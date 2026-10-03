import assert from "node:assert/strict";
import test from "node:test";
import type { ScanResult, Transaction } from "@steam-patch/ui";
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
