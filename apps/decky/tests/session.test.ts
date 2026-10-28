import assert from "node:assert/strict";
import test from "node:test";
import type { GameMatch, Install, ProjectInstancesResult, ScanResult, SourceSubscription, SourceUpdateResult, Transaction } from "@patchhub/ui";
import { createManagerSession, feedbackFor, needsRecovery } from "../src/session.ts";
import type { SourceTaskApi, SourceTaskSnapshot } from "../src/source-tasks.ts";
import { setListPreferences } from "../src/preferences.ts";

const report: ScanResult = {
  roots: [], catalog_revision: 1, catalog_trust: "signed", platform: "steamos",
  installs: [], diagnostics: [],
};

const game = (instance: string, appid = 1227890): Install => ({
  game: { instance_id: instance, appid, name: "Summer Memories", library_path: instance,
    install_path: `${instance}/game`, buildid: null, availability: "installed", kind: "game" },
  patches: [], match_status: "no_catalog_match",
});
const rowReport = (): ScanResult => ({ ...report, generation: 3, settings_revision: 4, installs: [game("a")] });
const match = (fields: Partial<GameMatch> = {}): GameMatch => ({
  appid: 1227890, instance_id: "a", status: "publisher_match",
  candidates: [{ candidate_id: "candidate-0123456789abcdef01234567", source_id: "patchmoe",
    source_page: "https://www.patch.moe/", game_title: "Summer Memories", patch_title: "补丁",
    variant: "base", patch_kind: "adult_content", version_identity: "v1", format_reported: "zip",
    format_observed: "zip", capability: "discover_only", match_basis: "appid",
    observed_at: "2026-10-11T00:00:00Z", cache_status: "fresh", fingerprint: "f" }],
  diagnostics: [], source_revisions: {}, ...fields,
});
const task = (fields: Partial<SourceTaskSnapshot> = {}): SourceTaskSnapshot => ({
  task_id: "task-1", kind: "rematch_game", key: "a", state: "running",
  instances: ["a"], appid: 1227890, progress: null, matches: {}, result: null,
  error: null, ...fields,
});

function fakeApi(queue: SourceTaskSnapshot[][], overrides: Partial<SourceTaskApi> = {}) {
  const calls = { rematchGame: [] as unknown[][], updateSource: [] as unknown[][], cancelTask: [] as string[],
    projectInstances: [] as string[][], listSources: 0, setInterval: [] as unknown[][] };
  const api: SourceTaskApi = {
    listSources: async () => { calls.listSources++; return []; },
    updateSource: async (...args) => { calls.updateSource.push(args); return task({ kind: "update_source", key: "patchmoe", instances: [], appid: null }); },
    matchGames: async () => task({ kind: "match_games", key: "match" }),
    rematchGame: async (...args) => {
      assert.notEqual(args[0] === null, args[1] === null, "backend requires exactly one game selector");
      calls.rematchGame.push(args); return task();
    },
    setInterval: async (...args) => { calls.setInterval.push(args); return {}; },
    projectInstances: async (ids) => { calls.projectInstances.push(ids); return { settings_revision: 9, instances: [] }; },
    taskStatus: async () => queue.shift() ?? [],
    cancelTask: async (id) => { calls.cancelTask.push(id); return task({ state: "cancelling" }); },
    ...overrides,
  };
  return { api, calls };
}

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
  for (const state of ["planned", "committed", "rolled_back", "uninstalled", "failed"]) {
    assert.equal(needsRecovery({ state } as Transaction), false, state);
  }
});

test("rematch task merges live match updates into every row sharing the AppID, then projects the affected row", async () => {
  const initial = rowReport();
  initial.installs = [game("a"), game("a-dlc", 1227890), game("b", 500)];
  const projected = { ...game("a"), patches: [{ release_id: "source-patchmoe-1227890" } as Install["patches"][number]],
    discovery: { status: "publisher_match", candidates: match().candidates, diagnostics: [] } };
  const running = task({ matches: { "1227890": {
    appid: 1227890, instance_id: "a", status: "publisher_match",
    candidates: match().candidates, diagnostics: [] } } });
  const completed = task({ state: "completed", result: { settings_revision: 9, source_revisions: {}, results: [match(), match({ instance_id: "a-dlc" })] } });
  const { api, calls } = fakeApi([[running], [completed], []], {
    projectInstances: async (ids) => {
      calls.projectInstances.push(ids);
      return { settings_revision: 9, instances: [projected] };
    },
  });
  const session = createManagerSession(async () => ({ report: initial, transactions: [] }), api);
  await session.refresh();
  assert.equal(await session.rematchGame("a", 1227890, ["Summer Memories"], "zh-CN"), "task-1");
  assert.deepEqual(calls.rematchGame[0], ["a", null, ["Summer Memories"], "zh-CN"]);
  await session.pollSourceTasks();
  for (const instance of ["a", "a-dlc"]) {
    assert.equal(session.getSnapshot().report!.installs.find(row => row.game.instance_id === instance)!.discovery!.status, "publisher_match");
  }
  assert.equal(session.getSnapshot().report!.installs.find(row => row.game.instance_id === "b")!.discovery, undefined);
  await session.pollSourceTasks();
  assert.deepEqual(calls.projectInstances, [["a", "a-dlc"]]);
  assert.equal(session.getSnapshot().report!.installs[0].patches[0].release_id, "source-patchmoe-1227890");
  assert.equal(session.getSnapshot().report!.settings_revision, 9);
  assert.deepEqual(session.getSnapshot().sourceTasks, {});
  await session.pollSourceTasks();
  assert.deepEqual(calls.projectInstances, [["a", "a-dlc"]]);
});

test("update_source completion refreshes sources and projects the affected instances", async () => {
  const initial = rowReport();
  initial.installs = [game("a", 1), game("b", 2)];
  const summaries = [{ id: "patchmoe", name: "patch.moe", enabled: true, interval_hours: 6 }] as unknown as SourceSubscription[];
  const running = task({ task_id: "update", kind: "update_source", key: "patchmoe", instances: [], appid: null });
  const result: SourceUpdateResult = {
    source_id: "patchmoe", settings_revision: 9,
    status: { checked_at: 20, cached_at: 20, entry_count: 2, code: "source_refreshed" },
    revision: 2, complete: true, affected_appids: [1],
  };
  const completed = { ...running, state: "completed" as const, result };
  let sources = 0;
  const { api, calls } = fakeApi([[running], [completed], []], {
    listSources: async () => { sources++; return summaries; },
    projectInstances: async (ids) => { calls.projectInstances.push(ids); return { settings_revision: 9, instances: [game("a", 1)] }; },
  });
  const session = createManagerSession(async () => ({ report: initial, transactions: [] }), api);
  await session.refresh();
  await session.updateSource("patchmoe");
  await session.pollSourceTasks();
  await session.pollSourceTasks();
  assert.deepEqual(calls.projectInstances, [["a"]]);
  assert.equal(sources, 1);
  assert.equal(session.getSnapshot().report!.sources, summaries);
  assert.equal(session.getSnapshot().sourceTasks.update, undefined);
});

test("a failed query logs its domain error without publishing game feedback or projecting empty results", async t => {
  const log = t.mock.method(console, "error", () => {});
  const initial = rowReport();
  const failed = task({ state: "failed", error: "source_refresh_failed", error_detail: "offline" });
  const { api, calls } = fakeApi([[failed], []]);
  const session = createManagerSession(async () => ({ report: initial, transactions: [] }), api);
  await session.refresh();
  await session.rematchGame("a", 1227890);
  await session.pollSourceTasks();
  assert.equal(feedbackFor(session.getSnapshot(), "a").error, null);
  assert.deepEqual(session.getSnapshot().sourceTasks, {});
  assert.equal(feedbackFor(session.getSnapshot(), "other").error, null);
  assert.deepEqual(calls.projectInstances, []);
  assert.match(log.mock.calls.map(call => call.arguments.join(" ")).join("\n"), /source_refresh_failed.*offline/);
});

test("query failure retains patches and source candidates across live updates and projection refresh", async t => {
  t.mock.method(console, "error", () => {});
  const original = { ...game("a"), patches: [{ release_id: "previous-patch" }] as Install["patches"],
    discovery: { status: "publisher_match", candidates: match().candidates, diagnostics: [] } };
  const initial = { ...rowReport(), installs: [original] };
  const failedMatch = match({ status: "failed", candidates: [], diagnostics: [{ code: "source_refresh_failed", detail: "offline", source_id: null }] });
  const running = task({ matches: { "1227890": { ...failedMatch } } });
  const completed = task({ state: "completed", result: { settings_revision: 9, source_revisions: {}, results: [failedMatch] } });
  const { api } = fakeApi([[running], [completed]], {
    projectInstances: async () => ({ settings_revision: 9, instances: [{ ...game("a"), discovery: { status: "failed", candidates: [], diagnostics: failedMatch.diagnostics } }] }),
  });
  const session = createManagerSession(async () => ({ report: initial, transactions: [] }), api);
  await session.refresh();
  await session.pollSourceTasks();
  await session.pollSourceTasks();
  const retained = session.getSnapshot().report!.installs[0];
  assert.equal(retained.discovery!.status, "failed");
  assert.deepEqual(retained.discovery!.candidates, original.discovery.candidates);
  assert.deepEqual(retained.discovery!.diagnostics, failedMatch.diagnostics);
  assert.deepEqual(retained.patches, original.patches);
  assert.deepEqual(session.getSnapshot().sourceTasks, {});
  assert.equal(feedbackFor(session.getSnapshot(), "a").error, null);
  session.acceptSnapshot({ report: { ...initial, installs: [{ ...game("a"), discovery: { status: "no_match", candidates: [], diagnostics: [] } }] }, transactions: [] });
  assert.deepEqual(session.getSnapshot().report!.installs[0].patches, []);
  assert.deepEqual(session.getSnapshot().report!.installs[0].discovery!.candidates, []);
});

test("incomplete snapshots cannot restore a disabled or unsubscribed source or its executable actions", async () => {
  const sources = [
    { id: "patchmoe", enabled: true, subscribed: true },
    { id: "retained", enabled: true, subscribed: true },
  ] as Install["sources"];
  const previous = { ...game("a"), sources,
    patches: [{ release_id: "old-patch", source_id: "patchmoe", actions: { automatic: { available: true } } }, { release_id: "retained-patch", source_id: "retained" }, { release_id: "old-signed-without-source" }] as Install["patches"],
    discovery: { status: "publisher_match", candidates: [match().candidates[0], { ...match().candidates[0], candidate_id: "retained-candidate", source_id: "retained" }], diagnostics: [] } };
  for (const changed of [{ enabled: false, subscribed: true }, { enabled: true, subscribed: false }]) {
    const session = createManagerSession(async () => ({ report, transactions: [] }));
    session.acceptSnapshot({ report: { ...rowReport(), installs: [previous] }, transactions: [] });
    const next = { ...game("a"), sources: [{ ...sources![0], ...changed }, sources![1]],
      discovery: { status: "incomplete", candidates: [], diagnostics: [] } };
    session.acceptSnapshot({ report: { ...rowReport(), installs: [next] }, transactions: [] });
    const retained = session.getSnapshot().report!.installs[0];
    assert.deepEqual(retained.patches.map(patch => patch.release_id), ["retained-patch"]);
    assert.deepEqual(retained.discovery!.candidates.map(candidate => candidate.source_id), ["retained"]);
    session.acceptSnapshot({ report: { ...rowReport(), installs: [{ ...next, sources: [] }] }, transactions: [] });
    assert.deepEqual(session.getSnapshot().report!.installs[0].patches, []);
    assert.deepEqual(session.getSnapshot().report!.installs[0].discovery!.candidates, []);
  }
});

test("match_games tasks update rows for every reported AppID", async () => {
  const initial = rowReport();
  initial.installs = [game("a", 1), game("b", 1), game("c", 2)];
  const running = task({ kind: "match_games", key: "match", instances: [], appid: null,
    matches: { "1": { appid: 1, instance_id: null, status: "no_match", candidates: [], diagnostics: [] } } });
  const { api } = fakeApi([[running], []]);
  const session = createManagerSession(async () => ({ report: initial, transactions: [] }), api);
  await session.refresh();
  await session.matchGames(null, [], null, false);
  await session.pollSourceTasks();
  for (const instance of ["a", "b"]) {
    assert.deepEqual(session.getSnapshot().report!.installs.find(row => row.game.instance_id === instance)!.discovery,
      { status: "no_match", candidates: [], diagnostics: [] });
  }
  assert.equal(session.getSnapshot().report!.installs.find(row => row.game.instance_id === "c")!.discovery, undefined);
});

test("cancelling a source task keeps it visible until the host reports a terminal state", async () => {
  const initial = rowReport();
  const running = task();
  const cancelled = task({ state: "cancelled" });
  const { api, calls } = fakeApi([[running], [cancelled], []]);
  const session = createManagerSession(async () => ({ report: initial, transactions: [] }), api);
  await session.refresh();
  await session.rematchGame("a", 1227890);
  await session.pollSourceTasks();
  await session.cancelSourceTask("task-1");
  assert.deepEqual(calls.cancelTask, ["task-1"]);
  assert.equal(session.getSnapshot().sourceTasks["task-1"].state, "cancelling");
  await session.pollSourceTasks();
  assert.deepEqual(session.getSnapshot().sourceTasks, {});
});

test("source interval writes refresh the source summaries without a scan", async () => {
  const initial = rowReport();
  const summaries = [{ id: "patchmoe", name: "patch.moe", enabled: true, interval_hours: 24 }] as unknown as SourceSubscription[];
  let scans = 0;
  const { api, calls } = fakeApi([], { listSources: async () => { calls.listSources++; return summaries; } });
  const session = createManagerSession(async () => { scans++; return { report: initial, transactions: [] }; }, api);
  await session.refresh();
  await session.setSourceInterval("patchmoe", 24);
  assert.deepEqual(calls.setInterval, [["patchmoe", 24]]);
  assert.equal(session.getSnapshot().report!.sources, summaries);
  assert.equal(scans, 1);
  assert.equal(calls.listSources, 1);
});

test("feedback survives remounts and stays scoped to the original game", async () => {
  const session = createManagerSession(async () => ({ report, transactions: [] }));
  await session.ensureSnapshot();
  session.setFeedback({ note: "安装完成，文件已写入。" }, "night");
  const unmount = session.subscribe(() => {});
  unmount();
  await session.ensureSnapshot();
  assert.equal(feedbackFor(session.getSnapshot(), "night").note, "安装完成，文件已写入。");
  assert.equal(feedbackFor(session.getSnapshot(), "cult").note, null);
  assert.equal(feedbackFor(session.getSnapshot()).note, null);
  session.acceptSnapshot({ report: { ...report }, transactions: [] });
  assert.equal(feedbackFor(session.getSnapshot(), "cult").note, null);
  assert.equal(session.getSnapshot().note, "安装完成，文件已写入。");
});

test("late terminal feedback stays with its game after navigation during refresh", async () => {
  let finish!: (value: { report: ScanResult; transactions: Transaction[] }) => void;
  const session = createManagerSession(() => new Promise(resolve => { finish = resolve; }));
  const completion = session.refresh().then(() => session.setFeedback({ note: "任务已安全取消。", error: "安装仍需恢复。" }, "night"));
  assert.deepEqual(feedbackFor(session.getSnapshot(), "cult"), { note: null, error: null });
  finish({ report, transactions: [] });
  await completion;
  assert.deepEqual(feedbackFor(session.getSnapshot(), "cult"), { note: null, error: null });
  assert.deepEqual(feedbackFor(session.getSnapshot(), "night"), { note: "任务已安全取消。", error: "安装仍需恢复。" });
});

test("game operation failures are scoped while scan and settings feedback remains global", async () => {
  const session = createManagerSession(async () => ({ report, transactions: [] }));
  for (const label of ["preview", "subscription", "recovery"]) {
    await session.work(label, async () => { throw new Error("game_not_ready"); }, () => {}, "night");
    assert.ok(feedbackFor(session.getSnapshot(), "night").error);
    assert.equal(feedbackFor(session.getSnapshot(), "cult").error, null);
    assert.equal(feedbackFor(session.getSnapshot()).error, null);
  }
  await session.work("scan", async () => { throw new Error("library_unavailable"); }, () => {});
  assert.equal(feedbackFor(session.getSnapshot(), "cult").error, session.getSnapshot().error);
  session.update({ note: "已开启自动清理。" });
  assert.equal(feedbackFor(session.getSnapshot(), "night").note, "已开启自动清理。");
  assert.equal(feedbackFor(session.getSnapshot(), "cult").note, "已开启自动清理。");
  assert.equal(feedbackFor(session.getSnapshot()).note, "已开启自动清理。");
});

test("source cards keep independent expansion through navigation, snapshots and source changes", async () => {
  const { api, calls } = fakeApi([]);
  let scans = 0;
  const session = createManagerSession(async () => { scans++; return { report, transactions: [] }; }, api);
  assert.deepEqual(session.getSnapshot().expandedSources, {});
  const initial = session.getSnapshot();
  session.toggleSourceExpanded("patchmoe");
  session.toggleSourceExpanded("hgamer");
  assert.deepEqual(initial.expandedSources, {});
  assert.deepEqual(session.getSnapshot().expandedSources, { patchmoe: true, hgamer: true });
  const unmount = session.subscribe(() => {});
  unmount();
  let notifications = 0;
  const remount = session.subscribe(() => { notifications++; });
  session.acceptSnapshot({ report, transactions: [] });
  session.commitSources([{ id: "patchmoe", name: "Renamed source", enabled: false }, { id: "new-source" }] as SourceSubscription[]);
  session.toggleSourceExpanded("patchmoe");
  assert.deepEqual(session.getSnapshot().expandedSources, { patchmoe: false, hgamer: true });
  assert.equal(session.getSnapshot().expandedSources["new-source"], undefined);
  assert.ok(notifications > 0);
  assert.equal(scans, 0);
  assert.deepEqual(calls, { rematchGame: [], updateSource: [], cancelTask: [], projectInstances: [], listSources: 0, setInterval: [] });
  remount();
  assert.deepEqual(createManagerSession(async () => ({ report, transactions: [] })).getSnapshot().expandedSources, {});
});

test("collapsing an updating source neither cancels nor restarts its task", async () => {
  const running = task({ kind: "update_source", key: "patchmoe", instances: [], appid: null });
  const { api, calls } = fakeApi([[running]]);
  const session = createManagerSession(async () => ({ report, transactions: [] }), api);
  session.acceptSnapshot({ report, transactions: [] });
  await session.updateSource("patchmoe");
  session.toggleSourceExpanded("patchmoe");
  session.toggleSourceExpanded("patchmoe");
  await session.pollSourceTasks();
  assert.equal(session.getSnapshot().expandedSources.patchmoe, false);
  assert.equal(session.getSnapshot().sourceTasks["task-1"].state, "running");
  assert.deepEqual(calls.updateSource, [["patchmoe", false]]);
  assert.deepEqual(calls.cancelTask, []);
  assert.equal(calls.listSources, 0);
});

test("failed source updates refresh the latest attempt without opening the card or losing error details", async () => {
  const failed = task({ kind: "update_source", key: "patchmoe", instances: [], appid: null,
    state: "failed", error: "source_refresh_failed", error_detail: "HTTP 503" });
  const summaries = [{ id: "patchmoe", role: "publisher", status: "source_offline_cached" }] as SourceSubscription[];
  const { api, calls } = fakeApi([[failed]], { listSources: async () => { calls.listSources++; return summaries; } });
  const session = createManagerSession(async () => ({ report, transactions: [] }), api);
  session.acceptSnapshot({ report, transactions: [] });
  await session.pollSourceTasks();
  assert.equal(session.getSnapshot().report!.sources, summaries);
  assert.equal(calls.listSources, 1);
  assert.deepEqual(session.getSnapshot().expandedSources, {});
  assert.equal(session.getSnapshot().error, null);
  assert.deepEqual(session.getSnapshot().sourceTasks, {});
  await session.pollSourceTasks();
  assert.equal(calls.listSources, 1);
});

test("a failed attempt and status read stay out of page feedback", async () => {
  const { api } = fakeApi([[task({ kind: "update_source", instances: [], state: "failed", error_detail: "HTTP 503" })]],
    { listSources: async () => { throw Error("status unavailable"); } });
  const session = createManagerSession(async () => ({ report, transactions: [] }), api);
  await session.pollSourceTasks();
  assert.equal(session.getSnapshot().error, null);
  assert.equal(feedbackFor(session.getSnapshot(), "a").error, null);
});

test("retrying a failed rematch clears its previous error and sends one game selector", async () => {
  let attempts = 0;
  const { api } = fakeApi([], {
    rematchGame: async (instance, appid) => {
      assert.equal(instance, "a");
      assert.equal(appid, null);
      if (++attempts === 1) throw new Error("Python Exception");
      return task();
    },
  });
  const session = createManagerSession(async () => ({ report: rowReport(), transactions: [] }), api);
  await session.refresh();
  assert.equal(await session.rematchGame("a", 1227890), null);
  assert.equal(feedbackFor(session.getSnapshot(), "a").error, null);
  assert.equal(await session.rematchGame("a", 1227890), "task-1");
  assert.equal(feedbackFor(session.getSnapshot(), "a").error, null);
});
