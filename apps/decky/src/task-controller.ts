import type { Transaction } from "@steam-patch/ui";
import type { PreparedInstall } from "./install";

export type TaskSnapshot = {
  task_id: string;
  kind: "prepare" | "install";
  state: "running" | "cancelling" | "rolling_back" | "cancelled" | "completed" | "failed";
  stage: string;
  bytes_done: number;
  total_bytes: number | null;
  files_done: number;
  total_files: number | null;
  current_file: string | null;
  archive_bytes: number | null;
  elapsed_seconds: number;
  speed_bps: number;
  eta_seconds: number | null;
  result: PreparedInstall | Transaction | null;
  error: string | null;
  instance_id?: string;
  release_id?: string;
};
export type TaskApi = {
  prepare(instance: string, release: string, archive: string): Promise<{ task_id: string }>;
  install(plan: string, confirmed: boolean): Promise<{ task_id: string }>;
  status(id: string): Promise<TaskSnapshot>;
  cancel(id: string): Promise<TaskSnapshot>;
  list(): Promise<TaskSnapshot[]>;
};
export const taskIsTerminal = (state: TaskSnapshot["state"]) => ["cancelled", "completed", "failed"].includes(state);

// task_status renews the backend lease. The controller therefore stops status
// polling when the UI closes or cancellation cannot reach the backend.
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
      // Transient status failures are safe: without a successful status call,
      // the backend lease expires and stops the task.
      if (epoch === generation && !disposed && !cancelling) schedule(() => void poll(id, epoch), retryMs);
    }
  }
  async function requestCancel() {
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
        // cancel_task does not renew the lease. Never call task_status while
        // cancellation is unconfirmed; the watchdog must remain effective.
        if (epoch === generation && !disposed) schedule(() => void attempt(), retryMs);
      }
    }
    void attempt();
  }
  async function begin(kind: TaskSnapshot["kind"], instance: string, release: string, action: () => Promise<{ task_id: string }>) {
    if (starting || (current && !taskIsTerminal(current.state))) throw new Error("installer_busy");
    starting = true; cancelling = false; disposed = false; stopTimer();
    const epoch = ++generation;
    try {
      const value = await action();
      if (epoch !== generation || disposed || cancelling) {
        // UI left while start RPC was in flight. Do not poll or renew.
        try { await api.cancel(value.task_id); } catch { /* Lease will expire. */ }
        return value.task_id;
      }
      publish({ task_id: value.task_id, kind, state: "running", stage: "starting", bytes_done: 0,
        total_bytes: null, files_done: 0, total_files: null, current_file: null, archive_bytes: null,
        elapsed_seconds: 0, speed_bps: 0, eta_seconds: null, result: null, error: null,
        instance_id: instance, release_id: release });
      void poll(value.task_id, epoch);
      return value.task_id;
    } finally { starting = false; }
  }
  async function resume() {
    if (starting || (current && !taskIsTerminal(current.state))) return;
    const epoch = generation;
    const values = await api.list();
    if (epoch !== generation || disposed) return;
    const latest = values.filter(value => !handled.has(value.task_id)).at(-1);
    if (!latest) return;
    publish(latest);
    if (!taskIsTerminal(latest.state)) { const next = ++generation; void poll(latest.task_id, next); }
  }
  return {
    getSnapshot: () => current,
    subscribe(listener: () => void) { listeners.add(listener); return () => { listeners.delete(listener); }; },
    startPrepare: (instance: string, release: string, archive = "") => begin("prepare", instance, release, () => api.prepare(instance, release, archive)),
    startInstall: (plan: string, instance: string, release: string) => begin("install", instance, release, () => api.install(plan, true)),
    requestCancel, resume,
    clear() { if (current && taskIsTerminal(current.state)) { handled.add(current.task_id); publish(null); } },
    dismount() {
      disposed = true; stopTimer();
      if (starting) cancelling = true;
      else if (current && !taskIsTerminal(current.state)) {
        void api.cancel(current.task_id).catch(() => {});
      }
      generation++;
    },
  };
}
