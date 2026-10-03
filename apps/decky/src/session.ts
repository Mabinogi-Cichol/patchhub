import { reason, type ScanResult, type Transaction } from "@steam-patch/ui";

type Snapshot = { report: ScanResult; transactions: Transaction[] };
type SessionState = {
  report: ScanResult | null;
  transactions: Transaction[];
  busy: string | null;
  busySince: number | null;
  error: string | null;
  note: string | null;
};

// Steam menus and route changes can remount the UI. Keep the last scan and
// in-flight operation in the plugin session, rather than in a mounted panel.
export function createManagerSession(readSnapshot: () => Promise<Snapshot>) {
  let state: SessionState = { report: null, transactions: [], busy: null, busySince: null, error: null, note: null };
  let attemptedInitialScan = false;
  const listeners = new Set<() => void>();
  const update = (value: Partial<SessionState>) => {
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
  return {
    getSnapshot: () => state,
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => { listeners.delete(listener); };
    },
    update,
    work,
    refresh,
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
