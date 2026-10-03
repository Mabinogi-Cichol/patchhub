import assert from "node:assert/strict";
import test from "node:test";
import { getListPreferences, setListPreferences, subscribeListPreferences } from "../src/preferences.ts";
import { gamePresentation } from "../src/steam.ts";

test("a selection arriving after the old panel unmounts updates the new panel", () => {
  setListPreferences({ filter: "all", query: "夜勤" });
  const selectFromOldPanel = () => setListPreferences({ filter: "matched" });
  let oldNotifications = 0;
  const unsubscribeOld = subscribeListPreferences(() => oldNotifications++);
  unsubscribeOld();
  let nextPanel = getListPreferences();
  const unsubscribeNew = subscribeListPreferences(() => { nextPanel = getListPreferences(); });
  selectFromOldPanel();
  assert.equal(oldNotifications, 0);
  assert.deepEqual(nextPanel, { filter: "matched", query: "夜勤" });
  for (const filter of ["installed", "attention", "all"] as const) {
    setListPreferences({ filter });
    assert.equal(nextPanel.filter, filter);
    assert.equal(nextPanel.query, "夜勤");
  }
  unsubscribeNew();
});

test("Steam's localized library name and icon take priority, with safe fallbacks", () => {
  const originalWindow = Object.getOwnPropertyDescriptor(globalThis, "window");
  try {
    Object.defineProperty(globalThis, "window", { configurable: true, value: {
      appStore: {
        GetAppOverviewByAppID: () => ({ display_name: "夜勤病栋" }),
        GetIconURLForApp: () => "https://example.test/icon.jpg",
      },
    } });
    assert.deepEqual(gamePresentation(3239270, "Night Shift"), { name: "夜勤病栋", icon: "https://example.test/icon.jpg" });
    window.appStore.GetIconURLForApp = () => { throw new Error("icon unavailable"); };
    assert.deepEqual(gamePresentation(3239270, "Night Shift"), { name: "夜勤病栋" });
    window.appStore.GetAppOverviewByAppID = () => null;
    assert.deepEqual(gamePresentation(3239270, "Night Shift"), { name: "Night Shift" });
    Object.defineProperty(globalThis, "window", { configurable: true, value: {} });
    assert.deepEqual(gamePresentation(3239270, "Night Shift"), { name: "Night Shift" });
  } finally {
    if (originalWindow) Object.defineProperty(globalThis, "window", originalWindow);
    else Reflect.deleteProperty(globalThis, "window");
  }
});
