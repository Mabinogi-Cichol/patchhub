import assert from "node:assert/strict";
import test from "node:test";
import { filterInstalls, gameDisplayName, type Install } from "@patchhub/ui";
import { gamePresentation, matchesGameQuery, transactionGameName } from "../src/steam.ts";

const oldInstall = (): Install => JSON.parse(JSON.stringify({
  game: {
    instance_id: "library:2303800", appid: 2303800, name: "My Life as a Cult Leader",
    library_path: "/steam", install_path: "/steam/game", buildid: null,
    availability: "installed", kind: "game",
  },
  match_status: "no_catalog_match", patches: [],
}));

function withAppStore(store: object | undefined, check: () => void) {
  const originalWindow = Object.getOwnPropertyDescriptor(globalThis, "window");
  try {
    Object.defineProperty(globalThis, "window", { configurable: true, value: store ? { appStore: store } : {} });
    check();
  } finally {
    if (originalWindow) Object.defineProperty(globalThis, "window", originalWindow);
    else Reflect.deleteProperty(globalThis, "window");
  }
}

test("old scan JSON and blank overrides retain the manifest name", () => {
  const item = oldInstall();
  assert.equal("localized_name" in item.game, false);
  assert.equal(gameDisplayName(item.game), "My Life as a Cult Leader");
  item.game.localized_name = "   ";
  assert.equal(gameDisplayName(item.game), "My Life as a Cult Leader");
  item.game.localized_name = " Default name ";
  item.game.name_language = null;
  assert.equal(gameDisplayName(item.game), "Default name");
});

test("shared search includes manifest, localized name, and AppID", () => {
  const item = oldInstall();
  item.game.localized_name = " 妃神会秘史 ";
  for (const query of ["my life", "妃神会", "2303800"]) {
    assert.deepEqual(filterInstalls([item], "all", query), [item]);
  }
  assert.deepEqual(filterInstalls([item], "all", "missing"), []);
});

test("Decky prefers Steam name, then core override, then manifest", () => {
  const item = oldInstall();
  item.game.localized_name = " 妃神会秘史 ";
  withAppStore({
    GetAppOverviewByAppID: () => ({ display_name: "Steam 名称" }),
    GetIconURLForApp: () => "",
  }, () => {
    assert.equal(gamePresentation(item.game.appid, gameDisplayName(item.game)).name, "Steam 名称");
    assert.equal(matchesGameQuery(item, "Steam 名称"), true);
    assert.equal(matchesGameQuery(item, "My Life"), true);
    assert.equal(matchesGameQuery(item, "2303800"), true);
    assert.equal(transactionGameName([item], item.game.instance_id), "Steam 名称");
  });
  withAppStore({ GetAppOverviewByAppID: () => null }, () => {
    assert.equal(gamePresentation(item.game.appid, gameDisplayName(item.game)).name, "妃神会秘史");
    assert.equal(matchesGameQuery(item, "妃神会"), true);
    assert.equal(transactionGameName([item], item.game.instance_id), "妃神会秘史");
    assert.equal(transactionGameName([item], "missing"), "missing");
  });
  withAppStore({
    GetAppOverviewByAppID: () => ({ display_name: "   " }),
    GetIconURLForApp: () => "",
  }, () => {
    assert.equal(gamePresentation(item.game.appid, gameDisplayName(item.game)).name, "妃神会秘史");
    assert.equal(transactionGameName([item], item.game.instance_id), "妃神会秘史");
  });
  withAppStore({ GetAppOverviewByAppID: () => { throw new Error("unavailable"); } }, () => {
    assert.equal(gamePresentation(item.game.appid, gameDisplayName(item.game)).name, "妃神会秘史");
    assert.equal(transactionGameName([item], item.game.instance_id), "妃神会秘史");
  });
  item.game.localized_name = null;
  withAppStore(undefined, () => {
    assert.equal(gamePresentation(item.game.appid, gameDisplayName(item.game)).name, item.game.name);
    assert.equal(transactionGameName([item], item.game.instance_id), item.game.name);
  });
});