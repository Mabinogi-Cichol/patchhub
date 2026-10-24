import assert from "node:assert/strict";
import test from "node:test";
import { candidateMatched, displayPatches, fallbackDisplayStatus, filterInstalls, gameMatchStatusLabel, gameMatchStatusTone, needsAttention, sourceCoverageText, sourceIntervalLabel, sourceIntervalOptions, sourceLastAttemptText, sourceNextDueText, sourcePhaseLabel, type Install, type SourceCandidate } from "@patchhub/ui";
import { candidateConditions, candidateSource, discoveryNotice, discoveryStatus, discoverySummary, safeSourceUrl, unqueriedCount } from "../src/discovery.ts";

const candidate = (fields: Partial<SourceCandidate> = {}): SourceCandidate => ({ candidate_id: "candidate-1", source_id: "kagura-cn", game_appid: 1227890, game_title: "Summer Memories", patch_title: "夏日狂想曲补丁", source_page: "https://kagurafan.com/patch/", capability: "discover_only", match_basis: "appid", ...fields });
const row = (candidates: SourceCandidate[] = []): Install => ({ game: { instance_id: "library-a", appid: 1227890, name: "Summer Memories", library_path: "/a", install_path: "/a/game", buildid: null, availability: "installed", kind: "game" }, match_status: "no_catalog_match", patches: [], discovery: { status: "complete", candidates, diagnostics: [] } });

test("verified discovery enters matched filter while title clues do not; unknown guidance needs no attention", () => {
  const verified = row([candidate()]);
  const clue = row([candidate({ match_basis: "title" })]);
  assert.deepEqual(filterInstalls([verified, clue], "matched", ""), [verified]);
  assert.equal(candidateMatched(clue.discovery!.candidates[0]), false);
  assert.equal(needsAttention(verified), false);
  verified.patches = [{ release_id: "old-guidance", name: "Old", source_page: "https://example.com", support_level: "discover_only", actions: { automatic: { available: false, blockers: [] }, local_archive: { available: false, blockers: [] }, reinstall: { available: false, blockers: [] }, overwrite: { available: false, blockers: [] }, primary: { operation: "overwrite", label: "install", input: "unavailable" } }, compatibility: "unknown", blockers: [], detection: { status: "unknown", files: [] } }];
  assert.equal(needsAttention(verified), false);
  verified.patches[0].detection.status = "partial_or_conflict";
  assert.equal(needsAttention(verified), true);
});

test("display union keeps candidate identities out of installation IDs and orders official candidates first", () => {
  const game = row([candidate({ candidate_id: "index", source_id: "hgamer" }), candidate({ candidate_id: "official" })]);
  game.patches = [{ release_id: "source-patchmoe-1227890", name: "Installable", source_page: "https://example.com", support_level: "publisher_zip", actions: { automatic: { available: false, blockers: [] }, local_archive: { available: false, blockers: [] }, reinstall: { available: false, blockers: [] }, overwrite: { available: false, blockers: [] }, primary: { operation: "overwrite", label: "install", input: "unavailable" } }, compatibility: "unknown", blockers: [], detection: { status: "not_detected", files: [] } }];
  const entries = displayPatches(game);
  assert.deepEqual(entries.map(entry => [entry.kind, entry.key]), [["installable", "source-patchmoe-1227890"], ["discovery", "official"], ["discovery", "index"]]);
  assert.equal("patch" in entries[1], false);
  assert.match(candidateSource(game.discovery!.candidates[0]), /HGamer.*索引/);
  game.discovery!.candidates.push(candidate({ candidate_id: "english", languages: ["en"] }), candidate({ candidate_id: "chinese", languages: ["zh-CN"] }));
  assert.deepEqual(displayPatches(game, "zh-CN").map(entry => entry.key), ["source-patchmoe-1227890", "chinese", "official", "english", "index"]);
});

test("cold cache games remain countable and DLC conditions stay explicit", () => {
  const cold = row(); cold.discovery = { status: "pending", candidates: [], diagnostics: [] };
  const queriedEmpty = row(); queriedEmpty.discovery!.status = "no_match";
  assert.equal(unqueriedCount([cold, queriedEmpty, row([candidate()])]), 1);
  assert.match(discoveryStatus("no_match"), /已启用来源中未找到/);
  assert.match(discoveryStatus("incomplete"), /尚需更新来源/);
  assert.match(discoveryStatus("source_offline_cached"), /显示上次结果/);
  assert.match(discoveryStatus("fallback_match"), /HGamer/);
  assert.match(candidateConditions(candidate({ variant: "dlc", requires_dlc: [1390150] })), /1390150.*待确认/);
  assert.match(candidateConditions(candidate({ variant: "base", excludes_dlc: [1390150] })), /未使用 DLC 1390150/);
});

test("an automatic EXE entry replaces its own duplicate source guidance", () => {
  const game = row([candidate({ candidate_id: "official" }), candidate({ candidate_id: "other", source_id: "hgamer" })]);
  game.patches = [{ release_id: "exe-selection-example", candidate_id: "official", name: "Installable", source_page: "https://example.com",
    support_level: "publisher_exe", actions: { automatic: { available: false, blockers: [] }, local_archive: { available: false, blockers: [] }, reinstall: { available: false, blockers: [] }, overwrite: { available: false, blockers: [] }, primary: { operation: "overwrite", label: "install", input: "unavailable" } }, compatibility: "check_required", blockers: [], detection: { status: "unknown", files: [] } }];
  assert.deepEqual(displayPatches(game).map(entry => [entry.kind, entry.key]), [["installable", "candidate::official"], ["discovery", "other"]]);
});

test("external navigation accepts public HTTPS pages and rejects local addresses and arbitrary schemes", () => {
  for (const url of ["https://kagurafan.com/patch/", "https://hgamer.tw/zh-CN/patches/game"]) assert.equal(safeSourceUrl(url), true);
  for (const url of ["http://example.com", "https://localhost/", "https://localhost./", "https://foo.localhost./", "https://example.local./", "https://127.0.0.1/", "https://10.0.0.1/", "https://[::1]/", "https://example.local/", "https://name:secret@example.com/", "javascript:alert(1)", "https://example.com:8443/"]) assert.equal(safeSourceUrl(url), false, url);
});

test("only a completed no_match says the enabled sources found no patch", () => {
  for (const status of ["no_match"]) {
    const game = row(); game.discovery!.status = status;
    assert.match(discoverySummary(game), /已启用来源中未找到/);
  }
  for (const status of ["pending", "publisher_match", "fallback_match", "incomplete", "failed"]) {
    const game = row(); game.discovery!.status = status;
    assert.doesNotMatch(discoverySummary(game), /未找到/);
  }
  const incomplete = row(); incomplete.discovery!.status = "incomplete";
  assert.match(discoveryNotice(incomplete)!, /尚需更新来源/);
  const publisher = row(); publisher.discovery!.status = "publisher_match";
  assert.equal(discoveryNotice(publisher), null);
});

test("shared six-state, interval and fallback helpers stay in the UI package", () => {
  assert.equal(gameMatchStatusLabel("publisher_match"), "发行商来源命中");
  assert.equal(gameMatchStatusLabel("no_match"), "已启用来源中未找到补丁");
  assert.equal(gameMatchStatusLabel("fallback_match"), "补丁索引（HGamer）命中");
  assert.equal(gameMatchStatusLabel("incomplete"), "来源覆盖不完整");
  assert.deepEqual(["pending", "publisher_match", "fallback_match", "no_match", "incomplete", "failed"].map(gameMatchStatusTone),
    ["pending", "ok", "fallback", "warning", "warning", "error"]);
  assert.equal(sourceIntervalLabel(0), "关闭");
  assert.equal(sourceIntervalLabel(6), "6 小时");
  assert.equal(sourceIntervalLabel(72), "3 天");
  assert.equal(sourceIntervalLabel(336), "14 天");
  assert.deepEqual(sourceIntervalOptions().map(option => option.data), [0, 6, 12, 24, 72, 168, 336]);
  assert.equal(sourceCoverageText({ complete: true, status: "source_refreshed" }), "索引覆盖完整");
  assert.match(sourceCoverageText({ complete: false, status: "source_partial" }), /尚需更新来源/);
  assert.equal(fallbackDisplayStatus("publisher_match"), null);
  assert.match(fallbackDisplayStatus("fallback_match")!, /HGamer/);
  assert.match(fallbackDisplayStatus("no_match")!, /已启用来源中未找到/);
  assert.match(sourceNextDueText({ interval_hours: 0 }), /已关闭/);
  assert.match(sourceNextDueText({ interval_hours: 24, next_due_at: 100 }, Date.parse("2026-10-11T00:00:00Z")), /已到更新周期/);
  assert.match(sourceNextDueText({ interval_hours: 24, next_due_at: Date.parse("2026-10-12T00:00:00Z") / 1000 }, Date.parse("2026-10-11T00:00:00Z")), /下次/);
  assert.match(sourceLastAttemptText({ checked_at: 100, status: "source_refreshed" }), /已实时查询/);
  assert.match(sourcePhaseLabel("details"), /补丁详情/);
});


test("dynamic variants with one display release retain distinct selections", () => {
  const game = row();
  game.patches = ["zip-selection-a", "zip-selection-b"].map(selection_id => ({
    release_id: "publisher-7", selection_id, name: "ZIP", source_page: "https://example.test/",
    support_level: "publisher_zip", actions: { automatic: { available: false, blockers: [] }, local_archive: { available: false, blockers: [] }, reinstall: { available: false, blockers: [] }, overwrite: { available: false, blockers: [] }, primary: { operation: "overwrite", label: "install", input: "unavailable" } }, compatibility: "unknown", blockers: [], detection: { status: "unknown", files: [] }
  }));
  assert.deepEqual(displayPatches(game).filter(item => item.kind === "installable").map(item => item.key), ["zip-selection-a", "zip-selection-b"]);
});


test("one candidate row retains both exact mirror choices while DLC remains distinct", () => {
  const game = row([candidate({candidate_id:"base",variant:"base"}),candidate({candidate_id:"dlc",variant:"dlc"})]);
  const available = {available:true,blockers:[]};
  const patch = (id:string,candidate_id:string,url:string) => ({release_id:"source-kagura-cn-1227890",selection_id:id,candidate_id,source_id:"kagura-cn",name:"Mirror",source_page:"https://kagurafan.com/patch/",support_level:"publisher_exe",compatibility:"manual_selection_required",download_url:url,actions:{automatic:available,overwrite:available,local_archive:{available:false,blockers:[]},reinstall:{available:false,blockers:[]},primary:{operation:"overwrite",label:"install",input:"automatic"}},blockers:[],detection:{status:"unknown",files:[]}});
  game.patches = [patch("exe-first","base","https://kaguraserver.com/same.exe"),patch("exe-second","base","https://kaguragamespatch.com/same.exe"),patch("exe-dlc","dlc","https://kaguraserver.com/dlc.exe")] as Install["patches"];
  const entries=displayPatches(game);
  assert.equal(entries.length,2);
  const base=entries[0];assert.equal(base.kind,"installable");
  if(base.kind!=="installable")throw Error("expected installation group");
  assert.equal(base.key,"candidate:kagura-cn:base");
  assert.deepEqual(base.choices.map(p=>p.selection_id),["exe-first","exe-second"]);
  assert.deepEqual(base.choices.map(p=>p.download_url),["https://kaguraserver.com/same.exe","https://kaguragamespatch.com/same.exe"]);
  const dlc=entries[1];assert.equal(dlc.kind,"installable");if(dlc.kind==="installable")assert.equal(dlc.choices[0].selection_id,"exe-dlc");
  game.patches.push({...game.patches[0],candidate_id:"different-version",selection_id:"exe-different"});
  assert.equal(displayPatches(game).length,3);
});
