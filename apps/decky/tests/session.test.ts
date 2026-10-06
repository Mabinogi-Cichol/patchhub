import assert from "node:assert/strict";
import test from "node:test";
import type { DiscoveryResult, Install, Patch, ScanResult, Transaction } from "@steam-patch/ui";
import { createManagerSession, needsRecovery } from "../src/session.ts";
import { setListPreferences } from "../src/preferences.ts";

const report: ScanResult = {
  roots: [], catalog_revision: 1, catalog_trust: "signed", platform: "linux",
  installs: [], diagnostics: [],
};

test("menu remounts and filter changes reuse the snapshot; explicit refresh scans again", async () => {
  let scans = 0;
  const session = createManagerSession(async () => {
    scans++;
    return { report, transactions: [] };
  });
  const unmount = session.subscribe(() => {});
  await session.ensureSnapshot();
  unmount();
  const remount = session.subscribe(() => {});
  for (const filter of ["all", "installed", "attention", "matched"] as const) {
    setListPreferences({ filter, query: "game" });
    await session.ensureSnapshot();
    assert.equal(session.getSnapshot().report, report);
  }
  assert.equal(scans, 1);
  await session.refresh();
  assert.equal(scans, 2);
  remount();
});

test("scan status updates the same generation without a detail mount", async () => {
  const initial: ScanResult = { ...report, generation: 7, settings_revision: 2, scan_progress: { completed: 0, total: 1, running: true } };
  let polls = 0;
  const session = createManagerSession(async () => ({ report: initial, transactions: [] }), undefined, async generation => {
    assert.equal(generation, 7);
    polls++;
    return { ...initial, scan_progress: { completed: 1, total: 1, running: false } };
  });
  await session.ensureSnapshot();
  await new Promise(resolve => setTimeout(resolve, 1050));
  assert.equal(polls, 1);
  assert.equal(session.getSnapshot().report?.scan_progress?.completed, 1);
});

test("starting a refresh fences errors from the previous scan before its snapshot arrives", async () => {
  const initial = { ...report, generation: 7, settings_revision: 2, scan_progress: { completed: 0, total: 1, running: true } };
  let finish!: (value: { report: ScanResult; transactions: Transaction[] }) => void;
  let calls = 0;
  const session = createManagerSession(() => ++calls === 1 ? Promise.resolve({ report: initial, transactions: [] })
    : new Promise(resolve => { finish = resolve; }), undefined, async () => { throw new Error("scan_stale"); });
  await session.refresh();
  const refreshing = session.refresh();
  await new Promise(resolve => setTimeout(resolve, 1050));
  assert.equal(session.getSnapshot().error, null);
  assert.equal(session.getSnapshot().busy, "正在扫描…");
  finish({ report: { ...initial, generation: 8, scan_progress: { completed: 1, total: 1, running: false } }, transactions: [] });
  await refreshing;
  assert.equal(session.getSnapshot().error, null);
  assert.equal(session.getSnapshot().report?.generation, 8);
});

test("an in-flight scan survives unmount and blocks duplicate scans and operations", async () => {
  let finish!: (value: { report: ScanResult; transactions: Transaction[] }) => void;
  let scans = 0;
  const session = createManagerSession(() => {
    scans++;
    return new Promise(resolve => { finish = resolve; });
  });
  const firstScan = session.ensureSnapshot();
  let current = session.getSnapshot();
  const unmount = session.subscribe(() => { current = session.getSnapshot(); });
  unmount();
  let remounted = session.getSnapshot();
  const unsubscribe = session.subscribe(() => { remounted = session.getSnapshot(); });
  await session.ensureSnapshot();
  await session.refresh();
  let mutations = 0;
  await session.work("install", async () => { mutations++; }, () => {});
  assert.equal(scans, 1);
  assert.equal(mutations, 0);
  assert.equal(remounted.busy, "正在扫描…");
  finish({ report, transactions: [] });
  await firstScan;
  assert.equal(remounted.report, report);
  assert.equal(remounted.busy, null);
  assert.equal(current.report, null);
  unsubscribe();
});

test("a failed initial scan keeps its error across remounts and can be explicitly retried", async () => {
  let scans = 0;
  const session = createManagerSession(async () => {
    if (++scans === 1) throw new Error("library_unavailable");
    return { report, transactions: [] };
  });
  await session.ensureSnapshot();
  assert.match(session.getSnapshot().error!, /Steam 库不可访问/);
  await session.ensureSnapshot();
  assert.equal(scans, 1);
  await session.refresh();
  assert.equal(session.getSnapshot().error, null);
  assert.equal(session.getSnapshot().report, report);
});

test("recovery is offered for interrupted writes, but not prepared or completed transactions", () => {
  for (const state of ["backing_up", "backed_up", "applying", "rolling_back", "uninstalling", "needs_recovery"]) {
    assert.equal(needsRecovery({ state } as Transaction), true, state);
  }
  for (const state of ["planned", "committed", "rolled_back", "uninstalled"]) {
    assert.equal(needsRecovery({ state } as Transaction), false, state);
  }
});

const game = (instance: string): Install => ({ game: { instance_id: instance, appid: 1227890, name: "Summer Memories", library_path: instance, install_path: `${instance}/game`, buildid: null, availability: "installed", kind: "game" }, patches: [], match_status: "unmatched" });
const discoveryReport = (): ScanResult => ({ ...report, generation: 3, settings_revision: 4, installs: [game("a"), game("b")] });
const discovery = (fields: Partial<DiscoveryResult> = {}): DiscoveryResult => ({ instance_id: "a", appid: 1227890, generation: 3, settings_revision: 4, status: "complete", candidates: [{ candidate_id: "candidate", source_id: "kagura-cn", game_title: "Summer Memories", patch_title: "补丁", capability: "discover_only", source_page: "https://kagurafan.com", match_basis: "appid" }], diagnostics: [], ...fields });
const release = (id: string, support_level: string, source_id = "kagura-cn"): Patch => ({
  release_id: id, name: id, source_id, source_page: "https://kagurafan.com", support_level,
  compatibility: "verified", blockers: [], detection: { status: "absent", files: [] },
  ...(support_level === "publisher_exe" ? { exe: { profile_id: "reviewed", backup_required: true as const, terms: { text: "terms", url: "https://kagurafan.com/terms" } } } : {}),
});

test("approved EXE discovery adds a release while keeping existing ZIP patches and candidate guidance", async () => {
  const base = discoveryReport();
  base.installs[0].patches = [release("zip", "managed_zip")];
  const session = createManagerSession(async () => ({ report: base, transactions: [] }), async () =>
    discovery({ exe_patches: [release("exe-reviewed", "publisher_exe")], exe_reviews: [{ candidate_id: "candidate", release_id: "exe-reviewed", profile_id: "reviewed", status: "installable", blockers: [] }] }));
  await session.refresh(); await session.discover("a", []);
  const row = session.getSnapshot().report!.installs[0];
  assert.deepEqual(row.patches.map(patch => patch.release_id), ["zip", "exe-reviewed"]);
  assert.equal(row.discovery?.candidates[0].capability, "discover_only");
  assert.equal(row.exe_reviews?.[0].status, "installable");
});

test("repeated approved EXE release IDs appear only once", async () => {
  const exe = release("exe-reviewed", "publisher_exe");
  const session = createManagerSession(async () => ({ report: discoveryReport(), transactions: [] }), async () =>
    discovery({ exe_patches: [exe, { ...exe }] }));
  await session.refresh(); await session.discover("a", []);
  assert.deepEqual(session.getSnapshot().report!.installs[0].patches.map(patch => patch.release_id), ["exe-reviewed"]);
});

test("source subscription revocation removes its approved EXE release and preserves other patches", async () => {
  const base = discoveryReport();
  base.installs[0].patches = [release("zip", "managed_zip")];
  const session = createManagerSession(async () => ({ report: base, transactions: [] }), async () =>
    discovery({ exe_patches: [release("exe-reviewed", "publisher_exe")] }));
  await session.refresh(); await session.discover("a", []);
  session.invalidateDiscovery("kagura-cn", 1227890);
  assert.deepEqual(session.getSnapshot().report!.installs[0].patches.map(patch => patch.release_id), ["zip"]);
});

test("a stale discovery response clears a previously approved EXE release", async () => {
  let calls = 0;
  const session = createManagerSession(async () => ({ report: discoveryReport(), transactions: [] }), async () =>
    ++calls === 1 ? discovery({ exe_patches: [release("exe-reviewed", "publisher_exe")] })
      : discovery({ status: "stale", exe_patches: [] }));
  await session.refresh(); await session.discover("a", []);
  assert.equal(session.getSnapshot().report!.installs[0].patches[0].release_id, "exe-reviewed");
  await session.discover("a", [], null, true);
  assert.deepEqual(session.getSnapshot().report!.installs[0].patches, []);
});

test("failed refresh revokes a previously approved EXE without losing ZIP releases", async () => {
  let calls = 0;
  const base = discoveryReport();
  base.installs[0].patches = [release("zip", "managed_zip")];
  const session = createManagerSession(async () => ({ report: base, transactions: [] }), async () => {
    if (++calls === 1) return discovery({ exe_patches: [release("exe-reviewed", "publisher_exe")] });
    throw new Error("source_refresh_failed");
  });
  await session.refresh(); await session.discover("a", []);
  assert.deepEqual(session.getSnapshot().report!.installs[0].patches.map(patch => patch.release_id), ["zip", "exe-reviewed"]);
  await session.discover("a", [], null, true);
  assert.deepEqual(session.getSnapshot().report!.installs[0].patches.map(patch => patch.release_id), ["zip"]);
  assert.match(session.getSnapshot().discoveries.a.error!, /查询失败/);
});

test("a failed request from an older generation cannot revoke the newer scan's EXE", async () => {
  let rejectOld!: (error: Error) => void;
  const first = discoveryReport();
  first.installs[0].patches = [release("old-exe", "publisher_exe")];
  const newer = { ...discoveryReport(), generation: 5, installs: [game("a"), game("b")] };
  newer.installs[0].patches = [release("new-exe", "publisher_exe")];
  let scans = 0;
  const session = createManagerSession(async () => ({ report: ++scans === 1 ? first : newer, transactions: [] }),
    () => new Promise((_resolve, reject) => { rejectOld = reject; }));
  await session.refresh();
  const pending = session.discover("a", []);
  await session.refresh();
  rejectOld(new Error("source_refresh_failed"));
  await pending;
  assert.equal(session.getSnapshot().report?.generation, 5);
  assert.deepEqual(session.getSnapshot().report!.installs[0].patches.map(patch => patch.release_id), ["new-exe"]);
});

test("single-game discovery survives remount, is deduplicated and preserves transactions and installation work", async () => {
  let finish!: (result: DiscoveryResult) => void;
  let calls = 0;
  let names: string[] = [];
  let language: string | null = "unexpected";
  const session = createManagerSession(async () => ({ report: discoveryReport(), transactions: [] }), async (_instance, hints, preference) => {
    calls++; names = hints; language = preference;
    return new Promise(resolve => { finish = resolve; });
  });
  await session.refresh();
  const tx = { transaction_id: "receipt", instance_id: "b", release_id: "release", state: "committed" };
  session.update({ busy: "正在安装…", transactions: [tx] });
  const originalOther = session.getSnapshot().report!.installs[1];
  const first = session.discover("a", ["夏日狂想曲", "Summer Memories", "夏日狂想曲", "あまえんぼ", "ignored"]);
  const unsubscribe = session.subscribe(() => {}); unsubscribe();
  const second = session.discover("a", ["Summer Memories"]);
  assert.equal(first, second);
  assert.equal(calls, 1);
  assert.deepEqual(names, ["夏日狂想曲", "Summer Memories", "あまえんぼ"]);
  assert.equal(language, null);
  finish(discovery()); await first;
  assert.equal(session.getSnapshot().busy, "正在安装…");
  assert.deepEqual(session.getSnapshot().transactions, [tx]);
  assert.equal(session.getSnapshot().report!.installs[1], originalOther);
  assert.equal(session.getSnapshot().report!.installs[0].discovery!.candidates[0].candidate_id, "candidate");
  await session.discover("a", []); assert.equal(calls, 1);
});

test("generation, subscription revision, instance and AppID fencing reject stale discovery responses", async () => {
  for (const response of [discovery({ generation: 2 }), discovery({ settings_revision: 3 }), discovery({ instance_id: "b" }), discovery({ appid: 100 })]) {
    const session = createManagerSession(async () => ({ report: discoveryReport(), transactions: [] }), async () => response);
    await session.refresh(); await session.discover("a", []);
    assert.equal(session.getSnapshot().report!.installs[0].discovery, undefined);
  }
  let finish!: (result: DiscoveryResult) => void;
  const session = createManagerSession(async () => ({ report: discoveryReport(), transactions: [] }), () => new Promise(resolve => { finish = resolve; }));
  await session.refresh();
  const pending = session.discover("a", []);
  session.update({ report: { ...session.getSnapshot().report!, generation: 5 } });
  finish(discovery()); await pending;
  assert.equal(session.getSnapshot().report!.installs[0].discovery, undefined);
});

test("disabling a source clears its candidate projection and blocks in-flight results without removing receipts", async () => {
  let finish!: (result: DiscoveryResult) => void;
  const session = createManagerSession(async () => ({ report: discoveryReport(), transactions: [] }), () => new Promise(resolve => { finish = resolve; }));
  await session.refresh();
  const tx = { transaction_id: "receipt", instance_id: "a", release_id: "old", state: "committed" };
  session.update({ transactions: [tx] });
  const pending = session.discover("a", []);
  session.invalidateDiscovery("kagura-cn", 1227890);
  finish(discovery()); await pending;
  assert.deepEqual(session.getSnapshot().report!.installs[0].discovery!.candidates, []);
  assert.deepEqual(session.getSnapshot().transactions, [tx]);
  assert.deepEqual(session.getSnapshot().discoveries, {});
});

test("discovery failure keeps cached metadata and can be explicitly retried without changing installation state", async () => {
  let calls = 0;
  const cachedReport = discoveryReport();
  cachedReport.installs[0].discovery = { status: "cache_complete", candidates: discovery().candidates, diagnostics: [] };
  const session = createManagerSession(async () => ({ report: cachedReport, transactions: [] }), async () => { if (++calls === 1) throw new Error("source_refresh_failed"); return discovery(); });
  await session.refresh();
  session.update({ busy: "正在安装…", note: "安装已准备" });
  await session.discover("a", []);
  assert.equal(session.getSnapshot().report!.installs[0].discovery!.candidates.length, 1);
  assert.match(session.getSnapshot().discoveries.a.error!, /查询失败/);
  assert.equal(session.getSnapshot().busy, "正在安装…");
  assert.equal(session.getSnapshot().note, "安装已准备");
  await session.discover("a", [], null, true);
  assert.equal(calls, 2);
  assert.equal(session.getSnapshot().discoveries.a.error, null);
});

test("a stale backend result cannot replace metadata and leaves discovery retryable", async () => {
  let calls = 0;
  const session = createManagerSession(async () => ({ report: discoveryReport(), transactions: [] }), async () => ++calls === 1 ? discovery({ status: "stale" }) : discovery());
  await session.refresh(); await session.discover("a", []);
  assert.equal(session.getSnapshot().report!.installs[0].discovery, undefined);
  await session.discover("a", []);
  assert.equal(calls, 2);
  assert.equal(session.getSnapshot().report!.installs[0].discovery!.candidates.length, 1);
});
