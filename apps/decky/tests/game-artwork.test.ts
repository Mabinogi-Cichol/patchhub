import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { runInNewContext } from "node:vm";
import ts from "typescript";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

const require = createRequire(import.meta.url);
const output = ts.transpileModule(readFileSync(new URL("../src/game-artwork.tsx", import.meta.url), "utf8"), {
  compilerOptions: { jsx: ts.JsxEmit.ReactJSX, module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;

function fixture(appStore: any, available = true) {
  const props: any[] = [];
  const states: unknown[] = [];
  let hook = 0;
  const image = { current: { is_default_image: false } };
  class NativeImage extends React.Component<any> {
    GetSourcesForAsset() {}
    OnImageLoaded() {}
    render() { props.push(this.props); return React.createElement("img", { alt: this.props.alt }); }
  }
  const exports: Record<string, any> = {};
  runInNewContext(output, { exports, window: { appStore }, require: (id: string) => id === "@decky/ui" ? {
    findModuleExport: (predicate: (value: unknown) => boolean) => {
      assert.equal(predicate(() => null), false);
      assert.equal(predicate(NativeImage), true);
      return available ? NativeImage : undefined;
    },
    ErrorBoundary: ({ children }: any) => children,
  } : id === "react" ? { ...React,
    useRef: () => image,
    useState: (initial: unknown) => {
      const index = hook++;
      if (!(index in states)) states[index] = initial;
      return [states[index], (next: unknown) => { states[index] = next; }];
    },
  } : require(id) });
  return { props, image, render: (appid: number) => {
    hook = 0;
    return renderToStaticMarkup(React.createElement(exports.GameArtwork, { appid }));
  } };
}

test("native Hero receives the current Steam overview without enabling artwork customization", () => {
  const overviews = new Map([1, 2].map(appid => [appid, { appid, display_name: `Game ${appid}` }]));
  const value = fixture({ GetAppOverviewByAppID: (appid: number) => overviews.get(appid) });
  for (const appid of [1, 2]) {
    const html = value.render(appid);
    const props = value.props.at(-1);
    assert.equal(props.app, overviews.get(appid));
    assert.equal(props.eAssetType, 1);
    assert.equal(props.allowCustomization, false);
    assert.equal(props.neverShowTitle, true);
    assert.equal(props.alt, "");
    assert.equal(typeof props.onError, "function");
    assert.match(html, /hidden=""/); // No blank banner before the native image loads.
    assert.doesNotMatch(html, /button|tabindex|input/);
  }
});

test("missing Steam overview, client API or native component leaves the existing details intact", () => {
  for (const store of [undefined, {}, { GetAppOverviewByAppID: () => null },
    { GetAppOverviewByAppID: () => { throw new Error("Steam unavailable"); } }]) {
    assert.equal(fixture(store).render(1), "");
  }
  assert.equal(fixture({ GetAppOverviewByAppID: () => ({ appid: 1 }) }, false).render(1), "");
});

test("only loaded game artwork is shown; load errors and Steam's generic fallback remove the banner", () => {
  const store = { GetAppOverviewByAppID: () => ({ appid: 1 }) };
  const loaded = fixture(store);
  loaded.render(1);
  loaded.props.at(-1).onLoad();
  assert.doesNotMatch(loaded.render(1), /hidden=/);
  assert.match(loaded.render(1), /<img/);

  const failed = fixture(store);
  failed.render(1);
  failed.props.at(-1).onError();
  assert.equal(failed.render(1), "");

  const generic = fixture(store);
  generic.render(1);
  generic.image.current.is_default_image = true;
  generic.props.at(-1).onLoad();
  assert.equal(generic.render(1), "");
});
