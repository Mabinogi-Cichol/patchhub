import { reason, type GameMatch, type Install, type MatchGamesResult, type ProjectInstancesResult, type ScanResult, type SourceSubscription, type SourceUpdateResult, type Transaction } from "@steam-patch/ui";
import type { SourceTaskApi, SourceTaskSnapshot } from "./source-tasks";

// Decky has no push channel. `project_instances` replaces the rows a finished
// source task touched; match updates are merged while the task runs.
export type RowDiscovery = NonNullable<Install["discovery"]>;

type Snapshot = { report: ScanResult; transactions: Transaction[] };
type SessionState = {
  report: ScanResult | null;
  transactions: Transaction[];
  busy: string | null;
  busySince: number | null;
  error: string | null;
  note: string | null;
  errorInstance: string | null;
  noteInstance: string | null;
  sourceTasks: Record<string, SourceTaskSnapshot>;
  expandedSources: Record<string, boolean>;
};

const ACTIVE_POLL_MS = 1000;
const IDLE_POLL_MS = 3000;

// Local copies keep this module's runtime imports free of extension-pinned
// paths; the shared exported forms live in source-tasks.ts.
const sourceTaskTerminal = (state: SourceTaskSnapshot["state"]): boolean => ["completed", "cancelled", "failed"].includes(state);
// update_source re-matches affected games and streams match_update as well.
const sourceTaskMatches = (task: SourceTaskSnapshot) => Object.values(task.matches);

type Diagnostics = Array<{ code: string; detail?: string | null; source_id?: string | null }>;

function discovery(status: string, candidates: RowDiscovery["candidates"], diagnostics: Diagnostics): RowDiscovery {
  return {
    status,
    candidates,
    diagnostics: diagnostics.map(entry => ({ code: entry.code, detail: entry.detail ?? null, source_id: entry.source_id ?? null })),
  };
}

export function feedbackFor(state: Pick<SessionState, "error" | "note" | "errorInstance" | "noteInstance">, instance: string | null = null) {
  return {
    error: state.errorInstance === null || state.errorInstance === instance ? state.error : null,
    note: state.noteInstance === null || state.noteInstance === instance ? state.note : null,
  };
}

// Steam menus and route changes can remount the UI. Keep the last scan and
// in-flight operation in the plugin session, rather than in a mounted panel.
export function createManagerSession(readSnapshot: () => Promise<Snapshot>, sourceApi?: SourceTaskApi) {
  let state: SessionState = { report: null, transactions: [], busy: null, busySince: null,
    error: null, note: null, errorInstance: null, noteInstance: null, sourceTasks: {}, expandedSources: {} };
  let attemptedInitialScan = false;
  let pollTimer: ReturnType<typeof setTimeout> | null = null;
  let pollSubscribers = 0;
  const handledSourceTasks = new Set<string>();
  const listeners = new Set<() => void>();
  const update = (value: Partial<SessionState>) => {
    // Unscoped feedback remains global. Scoped async completions carry their
    // original instance even if the user has navigated to another game.
    if ("error" in value && !("errorInstance" in value)) value = { ...value, errorInstance: null };
    if ("note" in value && !("noteInstance" in value)) value = { ...value, noteInstance: null };
    state = { ...state, ...value };
    listeners.forEach(listener => listener());
  };
  function setFeedback(value: { error?: string | null; note?: string | null }, instance: string | null = null) {
    update({ ...value, ...("error" in value ? { errorInstance: instance } : {}), ...("note" in value ? { noteInstance: instance } : {}) });
  }
  async function work<T>(label: string, action: () => Promise<T>, onSuccess: (value: T) => void | Promise<void>, instance: string | null = null) {
    if (state.busy) return;
    update({ busy: label, busySince: Date.now(), error: null, note: null });
    try { await onSuccess(await action()); }
    catch (error) { setFeedback({ error: reason(String(error)) }, instance); }
    finally { update({ busy: null, busySince: null }); }
  }
  function acceptSnapshot(snapshot: Snapshot) {
    update(snapshot);
  }
  function refresh() {
    if (state.busy) return Promise.resolve();
    return work("正在扫描…", readSnapshot, acceptSnapshot);
  }
  function commitSources(sources: SourceSubscription[]) {
    if (state.report) update({ report: { ...state.report, sources } });
  }
  function mergeInstances(instances: Install[]) {
    if (!state.report || !instances.length) return;
    const byInstance = new Map(instances.map(instance => [instance.game.instance_id, instance]));
    update({ report: { ...state.report, installs: state.report.installs.map(row => byInstance.get(row.game.instance_id) ?? row) } });
  }
  function mergeMatch(appid: number, status: string, candidates: RowDiscovery["candidates"], diagnostics: Diagnostics) {
    if (!state.report) return;
    update({ report: { ...state.report, installs: state.report.installs.map(row => row.game.appid === appid
      ? { ...row, discovery: discovery(status, candidates, diagnostics) }
      : row) } });
  }
  async function refreshSources() {
    if (!sourceApi) return;
    commitSources(await sourceApi.listSources());
  }
  async function projectRows(instanceIds: string[]) {
    if (!sourceApi || !state.report) return;
    const known = new Set(state.report.installs.map(row => row.game.instance_id));
    const ids = [...new Set(instanceIds)].filter(instance => known.has(instance));
    if (!ids.length) return;
    const result: ProjectInstancesResult = await sourceApi.projectInstances(ids);
    mergeInstances(result.instances);
    if (typeof result.settings_revision === "number" && state.report && result.settings_revision !== state.report.settings_revision) {
      update({ report: { ...state.report, settings_revision: result.settings_revision } });
    }
  }
  async function finishSourceTask(task: SourceTaskSnapshot) {
    const instance = task.instances[0] ?? null;
    if (task.state === "failed") {
      const detail = task.error_detail ? ` · ${task.error_detail}` : "";
      let message = `${reason(task.error ?? "source_refresh_failed")}${detail}`;
      if (task.kind === "update_source") {
        try { await refreshSources(); }
        catch (error) { message += ` · 来源状态读取失败：${reason(String(error))}`; }
      }
      setFeedback({ error: message }, instance);
      return;
    }
    if (task.state !== "completed") return;
    try {
      if (task.kind === "update_source") {
        const result = task.result as SourceUpdateResult | null;
        await refreshSources();
        const affected = result?.affected_appids ?? [];
        const instances = state.report?.installs
          .filter(row => affected.includes(row.game.appid))
          .map(row => row.game.instance_id) ?? [];
        await projectRows(instances);
        return;
      }
      const matches = (task.result as MatchGamesResult | null)?.results ?? [];
      for (const match of matches) mergeMatch(match.appid, match.status, match.candidates, match.diagnostics);
      await projectRows(matches.flatMap((match: GameMatch) => match.instance_id ? [match.instance_id] : []));
    } catch (error) {
      setFeedback({ error: reason(String(error)) }, instance);
    }
  }
  function mergeTaskMatches(task: SourceTaskSnapshot) {
    for (const match of sourceTaskMatches(task)) {
      mergeMatch(match.appid, match.status, match.candidates, match.diagnostics);
    }
  }
  async function acceptSourceTasks(values: SourceTaskSnapshot[]) {
    const running: Record<string, SourceTaskSnapshot> = {};
    const finished: SourceTaskSnapshot[] = [];
    for (const value of values) {
      if (sourceTaskTerminal(value.state)) {
        if (!handledSourceTasks.has(value.task_id)) finished.push(value);
      } else {
        running[value.task_id] = value;
        mergeTaskMatches(value);
      }
    }
    update({ sourceTasks: running });
    for (const task of finished) {
      handledSourceTasks.add(task.task_id);
      mergeTaskMatches(task);
      await finishSourceTask(task);
    }
  }
  function schedulePoll() {
    if (!sourceApi || pollTimer || pollSubscribers === 0) return;
    const active = Object.values(state.sourceTasks).some(task => !sourceTaskTerminal(task.state));
    pollTimer = setTimeout(() => { pollTimer = null; void pollSourceTasks(); }, active ? ACTIVE_POLL_MS : IDLE_POLL_MS);
  }
  async function pollSourceTasks() {
    if (!sourceApi) return;
    try {
      const value = await sourceApi.taskStatus();
      await acceptSourceTasks(Array.isArray(value) ? value : [value]);
    } catch {
      // A transient host read failure must not cancel the backend task; the
      // next poll reports its actual state.
    }
    schedulePoll();
  }
  function startSourcePolling() {
    if (!sourceApi) return () => {};
    pollSubscribers += 1;
    if (pollSubscribers === 1) void pollSourceTasks();
    return () => {
      pollSubscribers = Math.max(0, pollSubscribers - 1);
      if (pollSubscribers === 0 && pollTimer) { clearTimeout(pollTimer); pollTimer = null; }
    };
  }
  async function beginSourceTask(scope: string | null, action: () => Promise<SourceTaskSnapshot>) {
    if (!sourceApi) return null;
    setFeedback({ error: null, note: null }, scope);
    try {
      const task = await action();
      update({ sourceTasks: { ...state.sourceTasks, [task.task_id]: task } });
      schedulePoll();
      return task.task_id;
    } catch (error) {
      setFeedback({ error: reason(String(error)) }, scope);
      return null;
    }
  }
  async function updateSource(sourceId: string, force = false) {
    return beginSourceTask(null, () => sourceApi!.updateSource(sourceId, force));
  }
  async function matchGames(instances: string[] | null, nameHints: string[] = [], language: string | null = null, forceDetails = false) {
    return beginSourceTask(null, () => sourceApi!.matchGames(instances, nameHints, language, forceDetails));
  }
  async function rematchGame(instance: string, _appid: number, nameHints: string[] = [], language: string | null = null) {
    return beginSourceTask(instance, () => sourceApi!.rematchGame(instance, null, nameHints, language));
  }
  async function cancelSourceTask(taskId: string) {
    if (!sourceApi) return;
    try {
      const task = await sourceApi.cancelTask(taskId);
      update({ sourceTasks: { ...state.sourceTasks, [task.task_id]: task } });
      schedulePoll();
    } catch (error) {
      setFeedback({ error: reason(String(error)) });
    }
  }
  async function setSourceInterval(sourceId: string, intervalHours: number) {
    if (!sourceApi) return;
    try {
      await sourceApi.setInterval(sourceId, intervalHours);
      await refreshSources();
    } catch (error) {
      setFeedback({ error: reason(String(error)) });
    }
  }
  function invalidateDiscovery(sourceId?: string, appid?: number) {
    if (!state.report) return;
    update({ report: { ...state.report, installs: state.report.installs.map(row => {
      if (appid != null && row.game.appid !== appid) return row;
      const candidates = sourceId
        ? (row.discovery?.candidates ?? []).filter(candidate => candidate.source_id !== sourceId)
        : [];
      return { ...row, discovery: discovery("pending", candidates, []) };
    }) } });
  }
  return {
    getSnapshot: () => state,
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => { listeners.delete(listener); };
    },
    update,
    toggleSourceExpanded(sourceId: string) {
      update({ expandedSources: { ...state.expandedSources, [sourceId]: !state.expandedSources[sourceId] } });
    },
    setFeedback,
    acceptSnapshot,
    work,
    refresh,
    invalidateDiscovery,
    commitSources,
    updateSource,
    matchGames,
    rematchGame,
    cancelSourceTask,
    setSourceInterval,
    refreshSources,
    pollSourceTasks,
    startSourcePolling,
    ensureSnapshot() {
      if (attemptedInitialScan || state.report || state.busy) return;
      attemptedInitialScan = true;
      return refresh();
    },
  };
}

// Match the journal states that the core can repair. A prepared plan has not
// changed files and a committed transaction is an ordinary installed patch.
export function needsRecovery(transaction: Transaction): boolean {
  return ["backing_up", "backed_up", "applying", "rolling_back", "uninstalling", "needs_recovery"].includes(transaction.state);
}
