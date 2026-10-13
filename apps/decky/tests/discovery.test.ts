import assert from "node:assert/strict";
import test from "node:test";
import { candidateMatched, displayPatches, filterInstalls, needsAttention, type Install, type SourceCandidate } from "@steam-patch/ui";
import { candidateConditions, candidateSource, discoveryStatus, discoverySummary, safeSourceUrl, unqueriedCount } from "../src/discovery.ts";

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
  const cold = row(); cold.discovery = { status: "not_queried", candidates: [], diagnostics: [] };
  const queriedEmpty = row(); queriedEmpty.discovery!.status = "empty";
  assert.equal(unqueriedCount([cold, queriedEmpty, row([candidate()])]), 1);
  assert.match(discoveryStatus("empty"), /已查询的来源未找到/);
  assert.match(discoveryStatus("source_offline_cached"), /显示上次结果/);
  assert.match(candidateConditions(candidate({ variant: "dlc", requires_dlc: [1390150] })), /1390150.*待确认/);
  assert.match(candidateConditions(candidate({ variant: "base", excludes_dlc: [1390150] })), /未使用 DLC 1390150/);
});

test("an automatic EXE entry replaces its own duplicate source guidance", () => {
  const game = row([candidate({ candidate_id: "official" }), candidate({ candidate_id: "other", source_id: "hgamer" })]);
  game.patches = [{ release_id: "exe-selection-example", candidate_id: "official", name: "Installable", source_page: "https://example.com",
    support_level: "publisher_exe", actions: { automatic: { available: false, blockers: [] }, local_archive: { available: false, blockers: [] }, reinstall: { available: false, blockers: [] }, overwrite: { available: false, blockers: [] }, primary: { operation: "overwrite", label: "install", input: "unavailable" } }, compatibility: "check_required", blockers: [], detection: { status: "unknown", files: [] } }];
  assert.deepEqual(displayPatches(game).map(entry => [entry.kind, entry.key]), [["installable", "exe-selection-example"], ["discovery", "other"]]);
});

test("external navigation accepts public HTTPS pages and rejects local addresses and arbitrary schemes", () => {
  for (const url of ["https://kagurafan.com/patch/", "https://hgamer.tw/zh-CN/patches/game"]) assert.equal(safeSourceUrl(url), true);
  for (const url of ["http://example.com", "https://localhost/", "https://localhost./", "https://foo.localhost./", "https://example.local./", "https://127.0.0.1/", "https://10.0.0.1/", "https://[::1]/", "https://example.local/", "https://name:secret@example.com/", "javascript:alert(1)", "https://example.com:8443/"]) assert.equal(safeSourceUrl(url), false, url);
});

test("only a complete empty query says no patch was found", () => {
  for (const status of ["complete", "cache_complete", "empty"]) {
    const game = row(); game.discovery!.status = status;
    assert.match(discoverySummary(game), /已查询的来源未找到/);
  }
  for (const status of ["not_queried", "partial", "failed", "source_refresh_failed"]) {
    const game = row(); game.discovery!.status = status;
    assert.doesNotMatch(discoverySummary(game), /未找到/);
  }
});


test("dynamic variants with one display release retain distinct selections", () => {
  const game = row();
  game.patches = ["zip-selection-a", "zip-selection-b"].map(selection_id => ({
    release_id: "publisher-7", selection_id, name: "ZIP", source_page: "https://example.test/",
    support_level: "publisher_zip", actions: { automatic: { available: false, blockers: [] }, local_archive: { available: false, blockers: [] }, reinstall: { available: false, blockers: [] }, overwrite: { available: false, blockers: [] }, primary: { operation: "overwrite", label: "install", input: "unavailable" } }, compatibility: "unknown", blockers: [], detection: { status: "unknown", files: [] }
  }));
  assert.deepEqual(displayPatches(game).filter(item => item.kind === "installable").map(item => item.key), ["zip-selection-a", "zip-selection-b"]);
});
