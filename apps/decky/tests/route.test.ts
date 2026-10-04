import test from "node:test";
import assert from "node:assert/strict";
import { decodeInstance, encodeInstance, settingsPage, settingsRoutes } from "../src/route.ts";

test("instance ids with separators and non-ASCII survive a URL-safe round trip", () => {
  const values = [
    "3239270|/home/deck/.local/share/Steam|/home/deck/.local/share/Steam/steamapps/common/Night Shift Nurses",
    "游戏一|/media/deck/sd/steamapps/common/游戏名称",
    "42|/home/deck/.local/share/Steam|/x",
  ];
  for (const value of values) {
    const encoded = encodeInstance(value);
    assert.match(encoded, /^[A-Za-z0-9_-]+$/, `encoded segment must be URL-safe: ${encoded}`);
    assert.equal(decodeInstance(encoded), value);
  }
});

test("the encoded segment never contains '/' or '%' route delimiters", () => {
  const encoded = encodeInstance("1|/a/b|/c d");
  assert.ok(!encoded.includes("/"), "no path separator may leak into the segment");
  assert.ok(!encoded.includes("%"), "no percent escape may leak into the segment");
});

test("an undecodable legacy segment falls back to its raw value", () => {
  assert.equal(decodeInstance("3239270%7C%2Fhome"), "3239270%7C%2Fhome");
});

test("global settings pages cannot shadow each other under Steam prefix matching", () => {
  const paths = Object.values(settingsRoutes);
  for (const current of paths) {
    const matches = paths.filter(route => current === route || current.startsWith(`${route}/`));
    assert.deepEqual(matches, [current]);
  }
  assert.equal(settingsPage("subscriptions"), settingsRoutes.sources);
  assert.equal(settingsPage("cache"), settingsRoutes.cache);
  assert.equal(settingsPage(), settingsRoutes.sources, "legacy settings entry still opens sources");
});
