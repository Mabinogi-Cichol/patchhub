import type { Transaction } from "@steam-patch/ui";

export type TaskSnapshot = {
  task_id: string;
  kind: "install";
  state: "running" | "cancelling" | "rolling_back" | "cancelled" | "completed" | "failed";
  stage: string;
  content_release_id?: string;
  bytes_done: number;
  total_bytes: number | null;
  files_done: number;
  total_files: number | null;
  current_file: string | null;
  archive_bytes: number | null;
  elapsed_seconds: number;
  speed_bps: number;
  eta_seconds: number | null;
  result: Transaction | null;
  error: string | null;
  instance_id?: string;
  release_id?: string;
};
export type TaskApi = {
  install(instance: string, release: string, operation: "install" | "reinstall", requestId: string, archive: string): Promise<{ task_id: string }>;
  status(id: string): Promise<TaskSnapshot>;
  cancel(id: string): Promise<TaskSnapshot>;
  list(): Promise<TaskSnapshot[]>;
};
export const taskIsTerminal = (state: TaskSnapshot["state"]) => ["cancelled", "completed", "failed"].includes(state);

// Only the foreground installation UI observes work; leaving requests cancellation.
export function createTaskController(api: TaskApi, pollMs = 750, retryMs = 1500) {
  let current: TaskSnapshot | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let generation = 0;
  let starting = false;
  let cancelling = false;
  let disposed = false;
  const handled = new Set<string>();
  const listeners = new Set<() => void>();
  const publish = (value: TaskSnapshot | null) => { current = value; listeners.forEach(listener => listener()); };
  const stopTimer = () => { if (timer) clearTimeout(timer); timer = null; };
  function schedule(action: () => void, delay: number) { stopTimer(); if (!disposed) timer = setTimeout(action, delay); }
  async function poll(id: string, epoch: number) {
    if (epoch !== generation || disposed || cancelling) return;
    try {
      const value = await api.status(id);
      if (epoch !== generation || disposed || cancelling) return;
      publish(value);
      if (!taskIsTerminal(value.state)) schedule(() => void poll(id, epoch), pollMs);
    } catch {
      // Transient polling failures do not cancel the backend task.
      if (epoch === generation && !disposed && !cancelling) schedule(() => void poll(id, epoch), retryMs);
    }
  }
  async function requestCancel() {
    if (disposed) return;
    if (starting) { cancelling = true; return; }
    const value = current;
    if (!value || taskIsTerminal(value.state) || cancelling) return;
    cancelling = true;
    stopTimer();
    publish({ ...value, state: value.state === "rolling_back" ? "rolling_back" : "cancelling" });
    const id = value.task_id, epoch = generation;
    async function attempt() {
      if (epoch !== generation) return;
      try {
        const result = await api.cancel(id);
        if (epoch !== generation) return;
        publish(result);
        cancelling = false;
        if (!disposed && !taskIsTerminal(result.state)) schedule(() => void poll(id, epoch), pollMs);
      } catch {
        // Retry explicit cancellation until the backend acknowledges it.
        if (epoch === generation && !disposed) schedule(() => void attempt(), retryMs);
      }
    }
    void attempt();
  }
  async function begin(instance: string, release: string, action: () => Promise<{ task_id: string }>) {
    if (starting || (current && !taskIsTerminal(current.state))) throw new Error("installer_busy");
    starting = true; cancelling = false; disposed = false; stopTimer();
    publish(null);
    const epoch = ++generation;
    try {
      const value = await action();
      if (epoch !== generation || disposed) {
        // The user may leave before the start RPC returns its task ID.
        await api.cancel(value.task_id).catch(() => {});
        return value.task_id;
      }
      publish({ task_id: value.task_id, kind: "install", state: "running", stage: "starting", bytes_done: 0,
        total_bytes: null, files_done: 0, total_files: null, current_file: null, archive_bytes: null,
        elapsed_seconds: 0, speed_bps: 0, eta_seconds: null, result: null, error: null,
        instance_id: instance, release_id: release });
      starting = false;
      if (cancelling) { cancelling = false; void requestCancel(); }
      else void poll(value.task_id, epoch);
      return value.task_id;
    } finally { starting = false; }
  }
  async function resume() {
    if (starting || (!disposed && current && !taskIsTerminal(current.state))) return;
    disposed = false; cancelling = false;
    const epoch = generation;
    const values = await api.list();
    if (epoch !== generation || disposed) return;
    const latest = values.filter(value => !handled.has(value.task_id)).at(-1);
    if (!latest) return;
    publish(latest);
    // Reconnection must not revive work that lost its foreground UI.
    if (!taskIsTerminal(latest.state)) { ++generation; void requestCancel(); }
  }
  return {
    getSnapshot: () => current,
    isBusy: () => starting || !!current && !taskIsTerminal(current.state),
    subscribe(listener: () => void) { listeners.add(listener); return () => { listeners.delete(listener); }; },
    startInstall: (instance: string, release: string, operation: "install" | "reinstall", requestId: string, archive = "") => begin(instance, release, () => api.install(instance, release, operation, requestId, archive)),
    requestCancel, resume,
    clear() { if (current && taskIsTerminal(current.state)) { handled.add(current.task_id); publish(null); } },
    dismount() {
      const value = current;
      if (starting) cancelling = true;
      if (value && !taskIsTerminal(value.state)) {
        publish({ ...value, state: value.state === "rolling_back" ? "rolling_back" : "cancelling" });
        // Stop renewing the foreground lease; the backend guards lost cancel RPCs.
        void api.cancel(value.task_id).catch(() => {});
      }
      disposed = true; stopTimer();
      generation++;
    },
  };
}
