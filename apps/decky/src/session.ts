import { reason, type DiscoveryResult, type ScanResult, type Transaction } from "@steam-patch/ui";

type Snapshot = { report: ScanResult; transactions: Transaction[] };
type SessionState = {
  report: ScanResult | null;
  transactions: Transaction[];
  busy: string | null;
  busySince: number | null;
  error: string | null;
  note: string | null;
  discoveries: Record<string, { busy: boolean; error: string | null }>;
};

// Steam menus and route changes can remount the UI. Keep the last scan and
// in-flight operation in the plugin session, rather than in a mounted panel.
export function createManagerSession(readSnapshot: () => Promise<Snapshot>, discoverGame?: (instance: string, names: string[], language: string | null, force: boolean) => Promise<DiscoveryResult>) {
  let state: SessionState = { report: null, transactions: [], busy: null, busySince: null, error: null, note: null, discoveries: {} };
  let attemptedInitialScan = false;
  let discoveryEpoch = 0;
  const pending = new Map<string, Promise<void>>();
  const attempted = new Set<string>();
  const listeners = new Set<() => void>();
  const update = (value: Partial<SessionState>) => {
    if (value.report && (value.report.generation !== state.report?.generation || value.report.settings_revision !== state.report?.settings_revision)) {
      attempted.clear();
      state = { ...state, discoveries: {} };
    }
    state = { ...state, ...value };
    listeners.forEach(listener => listener());
  };
  async function work<T>(label: string, action: () => Promise<T>, onSuccess: (value: T) => void | Promise<void>) {
    if (state.busy) return;
    update({ busy: label, busySince: Date.now(), error: null, note: null });
    try { await onSuccess(await action()); }
    catch (error) { update({ error: reason(String(error)) }); }
    finally { update({ busy: null, busySince: null }); }
  }
  function refresh() {
    return work("正在扫描…", readSnapshot, update);
  }
  function discover(instance: string, names: string[], language: string | null = null, force = false) {
    const report = state.report;
    const row = report?.installs.find(item => item.game.instance_id === instance);
    if (!discoverGame || !report || !row || report.generation == null || report.settings_revision == null) return Promise.resolve();
    const epoch = discoveryEpoch;
    const key = `${report.generation}:${report.settings_revision}:${epoch}:${instance}`;
    const existing = pending.get(key);
    if (existing) return existing;
    if (!force && attempted.has(key)) return Promise.resolve();
    attempted.add(key);
    const identityCurrent = () => state.report?.generation === report.generation && state.report?.settings_revision === report.settings_revision && epoch === discoveryEpoch;
    const clearExeProjection = () => {
      if (!identityCurrent() || !state.report) return;
      update({ report: { ...state.report, installs: state.report.installs.map(item => item.game.instance_id === instance
        ? { ...item, patches: item.patches.filter(patch => patch.support_level !== "publisher_exe"), exe_reviews: [] } : item) } });
    };
    update({ discoveries: { ...state.discoveries, [instance]: { busy: true, error: null } } });
    const operation = (async () => {
      try {
        const result = await discoverGame(instance, [...new Set(names.map(name => name.trim()).filter(Boolean))].slice(0, 3).map(name => name.slice(0, 256)), language, force);
        if (!identityCurrent() || result.generation !== report.generation || result.settings_revision !== report.settings_revision || result.instance_id !== instance || result.appid !== row.game.appid || result.status === "stale") {
          attempted.delete(key);
          clearExeProjection();
          return;
        }
        const current = state.report!;
        update({ report: { ...current, installs: current.installs.map(item => item.game.instance_id === instance ? { ...item,
          patches: [...new Map([...item.patches.filter(patch => patch.support_level !== "publisher_exe"), ...(result.exe_patches ?? [])]
            .map(patch => [patch.release_id, patch] as const)).values()],
          exe_reviews: result.exe_reviews ?? [], discovery: { status: result.status, candidates: result.candidates, diagnostics: result.diagnostics } } : item) } });
      } catch (error) {
        if (identityCurrent()) {
          clearExeProjection();
          update({ discoveries: { ...state.discoveries, [instance]: { busy: false, error: reason(String(error)) } } });
        }
      } finally {
        // A newer scan/request owns its own progress state.
        if (identityCurrent()) update({ discoveries: { ...state.discoveries, [instance]: { busy: false, error: state.discoveries[instance]?.error ?? null } } });
      }
    })();
    pending.set(key, operation);
    void operation.then(() => pending.delete(key));
    return operation;
  }
  function invalidateDiscovery(sourceId?: string, appid?: number) {
    discoveryEpoch++;
    attempted.clear();
    update({ discoveries: {}, report: state.report ? { ...state.report, installs: state.report.installs.map(item => {
      if (appid != null && item.game.appid !== appid) return item;
      return { ...item, patches: item.patches.filter(patch => patch.support_level !== "publisher_exe" || sourceId != null && patch.source_id !== sourceId),
        exe_reviews: [], discovery: { status: "not_queried", candidates: sourceId ? (item.discovery?.candidates ?? []).filter(candidate => candidate.source_id !== sourceId) : [], diagnostics: [] } };
    }) } : null });
  }
  return {
    getSnapshot: () => state,
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => { listeners.delete(listener); };
    },
    update,
    work,
    refresh,
    discover,
    invalidateDiscovery,
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
