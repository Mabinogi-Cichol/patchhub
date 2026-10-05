import asyncio
import json
from pathlib import Path
import tempfile
import time
import types
import unittest
import subprocess
import sys
from unittest.mock import AsyncMock, patch

import sources
from sources import SourceRegistry, _validate_rule, _write_json
from test_sources import PATCH


RULE = {"id": "official", "name": "Official", "adapter": "kagura_woocommerce", "enabled": True,
        "role": "publisher", "discovery_only": True, "priority": 10,
        "pages": ["https://publisher.example/patch/"], "page_hosts": ["publisher.example"],
        "download_hosts": [], "root_layout": None}
FALLBACK = {**RULE, "id": "hgamer", "name": "Indexer", "adapter": "hgamer_html",
            "role": "fallback_indexer", "priority": 100,
            "pages": ["https://hgamer.tw/zh-CN/patches"], "page_hosts": ["hgamer.tw"]}
GAME = {"appid": 42, "name": "A Game"}


def candidate(source="official", suffix="a", **fields):
    return {"candidate_id": "candidate-" + suffix * 24, "source_id": source,
            "game_appid": 42, "game_title": "A Game", "patch_title": "A Game Patch",
            "variant": "base", "match_basis": "appid", "capability": "discover_only",
            "aliases": [], "languages": ["en"], "warnings": [], "mirrors": [],
            "source_page": "https://publisher.example/game/" if source == "official" else "https://hgamer.tw/zh-CN/patches/game",
            **fields}


def response(status="complete", candidates=None, **extra):
    return {"schema_version": 1, "results": [{"source_id": "official", "status": status,
            "candidates": [candidate()] if candidates is None else candidates, "diagnostics": [], **extra}]}


class DiscoveryRegistryTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.config = self.root / "sources.json"
        self.config.write_text(json.dumps({"schema_version": 2, "sources": [RULE]}))
        self.registry = SourceRegistry(self.root / "state", self.config)

    async def asyncTearDown(self):
        await self.registry.close()

    async def seed(self, result=None):
        with patch.object(self.registry, "_discovery_sync", return_value=result or response()):
            return await self.registry.discover_game(GAME, force=True)

    def test_builtin_v2_and_v1_rules_keep_install_permissions_separate(self):
        builtins = SourceRegistry(self.root / "fresh", Path(sources.__file__).with_name("sources.json"))
        self.assertEqual({r["id"] for r in builtins.list_sources()}, {"patchmoe", "kagura-cn", "kagura-en", "hgamer"})
        self.assertEqual(next(r for r in builtins.list_sources() if r["id"] == "hgamer")["download_hosts"], [])
        migrated = _validate_rule(PATCH)
        self.assertFalse(migrated["discovery_only"])
        self.assertEqual(migrated["root_layout"], "game_root")
        for invalid in ({**FALLBACK, "download_hosts": ["download.example"]},
                        {**RULE, "root_layout": "game_root"}, {**PATCH, "download_hosts": []}):
            with self.assertRaises(ValueError):
                _validate_rule(invalid)

    async def test_complete_cached_force_and_expiry(self):
        self.assertEqual(self.registry.cached_discovery(GAME)["status"], "not_queried")
        result = await self.seed()
        self.assertEqual(result["status"], "complete")
        with patch.object(self.registry, "_discovery_sync", return_value=response()) as discover:
            self.assertEqual((await self.registry.discover_game(GAME))["status"], "cache_complete")
            discover.assert_not_called()
            await self.registry.discover_game(GAME, force=True)
            self.assertEqual(discover.call_count, 1)
            record = self.registry._discovery_cache[("official", 42)]
            record["success"]["checked_at"] = int(time.time()) - 7 * 3600
            await self.registry.discover_game(GAME)
            self.assertEqual(discover.call_count, 2)
        reopened = SourceRegistry(self.root / "state", self.config)
        self.assertEqual(len(reopened.cached_candidates(GAME)), 1)
        self.assertTrue(reopened.list_sources()[0]["cached_at"])
        self.assertIsNone(reopened.source_for_release("source-official-42"))
        self.assertEqual(reopened.matches(GAME), [])
        with self.assertRaises(ValueError):
            reopened.validate_download("official", "https://publisher.example/package.zip")

    async def test_negative_cache_invalidates_when_new_query_names_arrive(self):
        await self.seed(response(candidates=[]))
        with patch.object(self.registry, "_discovery_sync", return_value=response(candidates=[])) as discover:
            await self.registry.discover_game(GAME)
            discover.assert_not_called()
            await self.registry.discover_game(GAME, ["別名"])
            self.assertEqual(discover.call_count, 1)

    async def test_skipped_fallback_rechecks_when_official_install_match_disappears(self):
        self.registry.add_source(FALLBACK)
        skipped = {"schema_version": 1, "results": [response(candidates=[])["results"][0],
            {"source_id": "hgamer", "status": "skipped", "candidates": [], "diagnostics": []}]}
        with patch.object(self.registry, "matches", return_value=[{"language": "en"}]), patch.object(self.registry, "_discovery_sync", return_value=skipped):
            await self.registry.discover_game(GAME)
        found = {"schema_version": 1, "results": [response(candidates=[])["results"][0],
            {"source_id": "hgamer", "status": "complete", "candidates": [candidate(source="hgamer")], "diagnostics": []}]}
        with patch.object(self.registry, "matches", return_value=[]), patch.object(self.registry, "_discovery_sync", return_value=found) as discover:
            result = await self.registry.discover_game(GAME)
        discover.assert_called_once()
        self.assertEqual(result["candidates"][0]["source_id"], "hgamer")

    async def test_negative_query_rechecks_after_source_subscription_changes(self):
        self.registry.add_source(FALLBACK)
        empty = {"schema_version": 1, "results": [response(candidates=[])["results"][0],
            {"source_id": "hgamer", "status": "complete", "candidates": [], "diagnostics": []}]}
        with patch.object(self.registry, "_discovery_sync", return_value=empty):
            await self.registry.discover_game(GAME)
        self.registry.set_game_source(42, "official", False)
        found = {"schema_version": 1, "results": [{"source_id": "hgamer", "status": "complete", "candidates": [candidate(source="hgamer")], "diagnostics": []}]}
        with patch.object(self.registry, "_discovery_sync", return_value=found) as discover:
            result = await self.registry.discover_game(GAME)
        discover.assert_called_once()
        self.assertEqual(len(result["candidates"]), 1)

    def test_claimed_match_requires_appid_in_worker_candidate(self):
        with self.assertRaisesRegex(ValueError, "invalid_discovery_identity"):
            self.registry._validate_candidates(RULE, 42, [candidate(game_appid=None)])

    async def test_failure_preserves_success_and_partial_checkpoint(self):
        await self.seed()
        success = json.loads(self.registry._discovery_path("official", 42).read_text())["success"]
        await self.seed(response("partial", [candidate(suffix="b")], index={"parser_version": sources.DISCOVERY_PARSER_VERSION, "records": [], "pending": ["https://publisher.example/page/2"]}))
        self.assertEqual(len(self.registry.cached_candidates(GAME)), 2)
        self.assertEqual(self.registry.cached_discovery(GAME)["status"], "cache_partial")
        await self.seed(response("failed", []))
        self.assertEqual(len(self.registry.cached_candidates(GAME)), 1)
        record = json.loads(self.registry._discovery_path("official", 42).read_text())
        self.assertEqual(record["success"], success)
        self.assertEqual(self.registry.cached_discovery(GAME)["status"], "source_offline_cached")
        self.assertEqual(self.registry._game_previous(RULE, 42)["attempt"]["index"]["pending"], ["https://publisher.example/page/2"])

    async def test_partial_refresh_does_not_shadow_verified_candidate_and_keeps_new_leads(self):
        await self.seed()
        promoted = candidate(match_basis="official_title")
        unpromoted = candidate(match_basis="title", game_appid=None)
        _write_json(self.registry._discovery_path("official", 42), {
            "schema_version": 1, "source_id": "official", "appid": 42,
            "success": {"status": "complete", "candidates": [promoted], "checked_at": int(time.time())},
            "attempt": {"status": "partial", "checked_at": int(time.time()),
                        "candidates": [unpromoted, candidate(suffix="b")]}})
        self.registry._discovery_cache.clear()
        found = {c["candidate_id"]: c for c in self.registry.cached_candidates(GAME)}
        self.assertEqual(found[promoted["candidate_id"]]["match_basis"], "official_title")
        self.assertEqual(found[promoted["candidate_id"]]["game_appid"], 42)
        self.assertEqual(len(found), 2)

    async def test_global_and_game_toggle_filter_and_revision(self):
        await self.seed()
        self.registry.set_game_source(42, "official", False)
        self.assertEqual(self.registry.settings_revision, 1)
        self.assertEqual(self.registry.cached_candidates(GAME), [])
        self.assertTrue(self.registry.list_game_sources(GAME)[0]["matched"])
        with patch.object(self.registry, "_discovery_sync") as discover:
            await self.registry.discover_game(GAME, force=True)
            discover.assert_not_called()
        self.registry.set_game_source(42, "official", True)
        self.registry.set_enabled("official", False)
        self.assertEqual(self.registry.cached_candidates(GAME), [])
        self.registry.set_enabled("official", True)
        self.assertEqual(len(self.registry.cached_candidates(GAME)), 1)

    async def test_settings_and_generation_changes_during_worker_never_commit(self):
        for change_settings in (True, False):
            started, finish = asyncio.Event(), asyncio.Event()
            current = [True]
            async def delayed(payload):
                started.set()
                await finish.wait()
                return response()
            with patch.object(sources.sys, "frozen", True, create=True), patch.object(self.registry, "_discovery_worker", side_effect=delayed):
                task = asyncio.create_task(self.registry.discover_game(GAME, force=True, is_current=lambda: current[0]))
                await started.wait()
                if change_settings:
                    self.registry.set_game_source(42, "official", False)
                else:
                    current[0] = False
                finish.set()
                self.assertEqual((await task)["status"], "stale")
                self.assertFalse(self.registry._discovery_path("official", 42).exists())
            self.registry.set_game_source(42, "official", True)

    async def test_multiple_instances_share_appid_query_and_close_cancels(self):
        started, finish = asyncio.Event(), asyncio.Event()
        async def delayed(payload):
            started.set()
            await finish.wait()
            return response()
        with patch.object(sources.sys, "frozen", True, create=True), patch.object(self.registry, "_discovery_worker", side_effect=delayed) as worker:
            first = asyncio.create_task(self.registry.discover_game({**GAME, "instance_id": "one"}, generation=1))
            await started.wait()
            second = asyncio.create_task(self.registry.discover_game({**GAME, "instance_id": "two"}, generation=1))
            await asyncio.sleep(0)
            finish.set()
            results = await asyncio.gather(first, second)
            self.assertEqual(worker.call_count, 1)
            self.assertEqual([r["generation"] for r in results], [1, 1])
        started.clear()
        finish.clear()
        with patch.object(sources.sys, "frozen", True, create=True), patch.object(self.registry, "_discovery_worker", side_effect=delayed):
            task = asyncio.create_task(self.registry.discover_game(GAME, force=True))
            await started.wait()
            await self.registry.close()
            with self.assertRaises(asyncio.CancelledError):
                await task

    async def test_worker_response_cannot_grant_permission_or_claim_another_game(self):
        for bad in (candidate(capability="publisher_zip"), candidate(game_appid=43),
                    candidate(source_page="https://localhost/x"), candidate(mirrors=[{"url": "http://evil.example/file"}])):
            with patch.object(sources.sys, "frozen", True, create=True), patch.object(self.registry, "_discovery_worker", return_value=response(candidates=[bad])):
                result = await self.registry.discover_game(GAME, force=True)
            self.assertEqual(result["status"], "source_refresh_failed")
            self.assertEqual(result["candidates"], [])

    async def test_query_boundary_validation(self):
        for hints in (["x"] * 4, ["x" * 257], [""], "text"):
            with self.assertRaises(ValueError):
                await self.registry.discover_game(GAME, hints)
        with self.assertRaises(ValueError):
            await self.registry.discover_game(GAME, preferred_language="../../path")

    async def test_worker_uses_fixed_arguments_bounded_chunks_and_clean_environment(self):
        stdout = asyncio.StreamReader()
        encoded = json.dumps(response()).encode()
        stderr = asyncio.StreamReader()
        class Process:
            returncode = None
            def __init__(self):
                self.stdin = types.SimpleNamespace(write=lambda value: None, drain=AsyncMock(), close=lambda: None)
                self.stdout, self.stderr = stdout, stderr
            async def wait(self):
                self.returncode = 0
                return 0
            def kill(self):
                self.returncode = -9
        process = Process()
        async def stream_chunks():
            stdout.feed_data(encoded[:13])
            await asyncio.sleep(0)
            stdout.feed_data(encoded[13:])
            stdout.feed_eof()
            stderr.feed_eof()
        with patch("sources.asyncio.create_subprocess_exec", AsyncMock(return_value=process)) as spawn:
            streaming = asyncio.create_task(stream_chunks())
            result = await self.registry._discovery_worker({"schema_version": 1})
            await streaming
        self.assertEqual(result, response())
        self.assertEqual(spawn.call_args.args[:2], ("/usr/bin/python3", "-I"))
        self.assertEqual(spawn.call_args.args[3], "--discover")

    async def test_shared_index_reused_across_games_and_expires(self):
        await self.seed(response(index={"parser_version": sources.DISCOVERY_PARSER_VERSION, "records": [{"title": "A Game", "page": RULE["pages"][0], "kind": "product"}], "pending": []}))
        other = self.registry._game_previous(RULE, 43)
        self.assertEqual(other["attempt"]["index"]["records"][0]["title"], "A Game")
        self.registry._discovery_indexes["official"]["checked_at"] = int(time.time()) - 25 * 3600
        self.assertNotIn("index", self.registry._game_previous(RULE, 42)["attempt"])

    async def test_partial_failure_state_is_not_hidden_by_other_unqueried_source(self):
        self.registry.add_source(FALLBACK)
        with patch.object(self.registry, "_discovery_sync", return_value={"schema_version": 1, "results": [response("partial")["results"][0], {"source_id": "hgamer", "status": "skipped", "candidates": [], "diagnostics": []}]}):
            result = await self.registry.discover_game(GAME, force=True)
        self.assertEqual(result["status"], "partial")

    def test_fallback_uses_fresh_official_cache_and_explicit_force_overrides_skip(self):
        self.registry.add_source(FALLBACK)
        mocked = types.SimpleNamespace(RequestContext=lambda **kw: object(), needs_fallback=lambda values, query: not values,
            discover=lambda rule, query, old, context: {"status": "empty", "candidates": [], "diagnostics": []})
        previous = {"official": {"success": {"candidates": [candidate()]}}}
        with patch("sources._discovery_module", return_value=mocked):
            result = self.registry._discovery_sync({**GAME, "force": False}, [_validate_rule(FALLBACK)], previous)
            self.assertEqual(result["results"][0]["status"], "skipped")
            result = self.registry._discovery_sync({**GAME, "force": True, "legacy_match": True}, [_validate_rule(FALLBACK)], previous)
            self.assertEqual(result["results"][0]["status"], "complete")

    async def test_worker_timeout_and_cancellation_kill_process(self):
        class Process:
            returncode = None
            killed = False
            def __init__(self):
                self.stdin = types.SimpleNamespace(write=lambda value: None, drain=AsyncMock(), close=lambda: None)
                self.stdout, self.stderr = asyncio.StreamReader(), asyncio.StreamReader()
            async def wait(self):
                return self.returncode
            def kill(self):
                self.killed, self.returncode = True, -9
        process = Process()
        async def timed_out(awaitable, timeout):
            self.assertEqual(timeout, 30)
            awaitable.close()
            raise asyncio.TimeoutError()
        with patch("sources.asyncio.create_subprocess_exec", AsyncMock(return_value=process)), patch("sources.asyncio.wait_for", side_effect=timed_out):
            with self.assertRaises(asyncio.TimeoutError):
                await self.registry._discovery_worker({"schema_version": 1})
        self.assertTrue(process.killed)
        self.assertEqual(self.registry._worker_processes, set())
        process = Process()
        with patch("sources.asyncio.create_subprocess_exec", AsyncMock(return_value=process)):
            task = asyncio.create_task(self.registry._discovery_worker({"schema_version": 1}))
            await asyncio.sleep(0)
            task.cancel()
            with self.assertRaises(asyncio.CancelledError):
                await task
        self.assertTrue(process.killed)

    async def test_oversized_worker_output_kills_process(self):
        stdout, stderr = asyncio.StreamReader(), asyncio.StreamReader()
        stdout.feed_data(b"x" * (sources.MAX_DISCOVERY_OUTPUT + 1))
        stdout.feed_eof()
        stderr.feed_eof()
        class Process:
            returncode = None
            def __init__(self):
                self.stdin = types.SimpleNamespace(write=lambda value: None, drain=AsyncMock(), close=lambda: None)
                self.stdout, self.stderr = stdout, stderr
            async def wait(self):
                return self.returncode
            def kill(self):
                self.returncode = -9
        process = Process()
        with patch("sources.asyncio.create_subprocess_exec", AsyncMock(return_value=process)):
            with self.assertRaisesRegex(ValueError, "oversized"):
                await self.registry._discovery_worker({"schema_version": 1})
        self.assertEqual(process.returncode, -9)

    def test_isolated_worker_initializes_sibling_adapters_without_writing_cache(self):
        payload = {"schema_version": 1, "query": GAME, "source_ids": [], "refresh_source_ids": []}
        result = subprocess.run([sys.executable, "-I", sources.__file__, "--discover", str(self.root / "isolated"), str(self.config)],
                                input=json.dumps(payload).encode(), capture_output=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr.decode())
        self.assertEqual(json.loads(result.stdout), {"schema_version": 1, "results": []})
        self.assertFalse((self.root / "isolated").exists())

    def test_hg_identity_requeries_fresh_official_and_preserves_format_conflict(self):
        self.registry.add_source(FALLBACK)
        indexed = candidate("hgamer", version_raw="v1", format_reported="zip", format_observed="web",
                            upstream_page="https://publisher.example/game/", requires_dlc=[], excludes_dlc=[], patch_kind="adult_content")
        official = candidate(version_raw="v1", format_observed="exe", requires_dlc=[], excludes_dlc=[], patch_kind="adult_content", provenance=[])
        calls = []
        def discover(rule, query, old, context):
            calls.append(rule["id"])
            if rule["id"] == "hgamer":
                return {"status": "complete", "candidates": [indexed], "diagnostics": [],
                        "identities": [{"appid": 42, "aliases": ["A Game"], "page": indexed["source_page"]}]}
            return {"status": "complete", "candidates": [official], "diagnostics": []}
        mocked = types.SimpleNamespace(RequestContext=lambda **kw: object(), needs_fallback=lambda values, query: True,
            discover=discover, merge_candidates=lambda values: list({v["candidate_id"]: v for v in values}.values()),
            enrich_candidates=lambda values, identities: values, canonical_url=lambda value: value)
        previous = {"official": {"success": {"candidates": [official]}}}
        with patch("sources._discovery_module", return_value=mocked):
            result = self.registry._discovery_sync(GAME, [_validate_rule(FALLBACK)], previous)
        self.assertEqual(calls, ["hgamer", "official"])
        self.assertEqual({r["source_id"] for r in result["results"]}, {"hgamer", "official"})
        self.assertIn("artifact_format_conflict", result["results"][0]["candidates"][0]["warnings"])
        self.assertEqual(official["provenance"], [{"source_id": "hgamer", "page": indexed["source_page"]}])

    def test_refresh_without_fallback_identity_keeps_previously_verified_candidate(self):
        self.registry.add_source(FALLBACK)
        promoted = candidate(match_basis="official_title")
        unpromoted = candidate(match_basis="title", game_appid=None)
        def discover(rule, query, old, context):
            if rule["id"] == "hgamer":
                return {"status": "failed", "candidates": [], "diagnostics": [], "identities": []}
            return {"status": "complete", "candidates": [dict(unpromoted)], "diagnostics": []}
        context = types.SimpleNamespace(deadline=time.monotonic() + 30, budget=24, requests=0, detail_requests=0)
        mocked = types.SimpleNamespace(RequestContext=lambda **kw: context, needs_fallback=lambda values, query: True,
            discover=discover, merge_candidates=lambda values: list({v["candidate_id"]: v for v in values}.values()),
            enrich_candidates=lambda values, identities: values, canonical_url=lambda value: value)
        previous = {"official": {"success": {"candidates": [promoted]}}}
        rules = [_validate_rule(RULE), _validate_rule(FALLBACK)]
        with patch("sources._discovery_module", return_value=mocked):
            result = self.registry._discovery_sync({**GAME, "force": True}, rules, previous)
        official = next(r for r in result["results"] if r["source_id"] == "official")
        self.assertEqual(official["candidates"][0]["match_basis"], "official_title")
        self.assertEqual(official["candidates"][0]["game_appid"], 42)
        # A different record (new version) must not inherit the old identity.
        changed = candidate(suffix="c", match_basis="title", game_appid=None)
        def discover_changed(rule, query, old, context):
            if rule["id"] == "hgamer":
                return {"status": "failed", "candidates": [], "diagnostics": [], "identities": []}
            return {"status": "complete", "candidates": [changed], "diagnostics": []}
        with patch("sources._discovery_module", return_value=types.SimpleNamespace(**{**mocked.__dict__, "discover": discover_changed})):
            result = self.registry._discovery_sync({**GAME, "force": True}, rules, previous)
        official = next(r for r in result["results"] if r["source_id"] == "official")
        self.assertEqual(official["candidates"][0]["match_basis"], "title")
        self.assertIsNone(official["candidates"][0]["game_appid"])

    async def test_new_generation_waits_for_obsolete_query_then_retries(self):
        started, finish = asyncio.Event(), asyncio.Event()
        generation = [1]
        calls = []
        async def delayed(payload):
            calls.append(payload)
            if len(calls) == 1:
                started.set()
                await finish.wait()
            return response()
        with patch.object(sources.sys, "frozen", True, create=True), patch.object(self.registry, "_discovery_worker", side_effect=delayed):
            old = asyncio.create_task(self.registry.discover_game(GAME, generation=1, is_current=lambda: generation[0] == 1))
            await started.wait()
            generation[0] = 2
            new = asyncio.create_task(self.registry.discover_game(GAME, generation=2, is_current=lambda: generation[0] == 2))
            await asyncio.sleep(0)
            finish.set()
            old_result, new_result = await asyncio.gather(old, new)
        self.assertEqual(old_result["status"], "stale")
        self.assertEqual(new_result["status"], "complete")
        self.assertEqual(len(calls), 2)

    def test_complete_empty_official_refresh_replaces_old_success_before_fallback(self):
        self.registry.add_source(FALLBACK)
        calls = []
        context = types.SimpleNamespace(deadline=time.monotonic() + 30, budget=24, requests=0)
        def discover(rule, query, old, context):
            calls.append(rule["id"])
            return {"status": "complete", "candidates": [], "diagnostics": []}
        mocked = types.SimpleNamespace(RequestContext=lambda **kw: context, needs_fallback=lambda values, query: not values,
                                       discover=discover)
        previous = {"official": {"success": {"candidates": [candidate()], "checked_at": int(time.time())}}}
        with patch("sources._discovery_module", return_value=mocked):
            self.registry._discovery_sync(GAME, self.registry._effective_discovery_rules(42), previous)
        self.assertEqual(calls, ["official", "hgamer"])

    async def test_parser_and_query_version_invalidate_positive_and_negative_cache(self):
        await self.seed()
        with patch("sources.DISCOVERY_PARSER_VERSION", sources.DISCOVERY_PARSER_VERSION + 1), patch.object(self.registry, "_discovery_sync", return_value=response()) as discover:
            self.assertEqual(self.registry.cached_candidates(GAME)[0]["cache_status"], "stale")
            await self.registry.discover_game(GAME)
            discover.assert_called_once()
        await self.seed(response(candidates=[]))
        old_key = self.registry._query_key(GAME)
        with patch("sources.DISCOVERY_QUERY_VERSION", 2), patch.object(self.registry, "_discovery_sync", return_value=response(candidates=[])) as discover:
            self.assertNotEqual(old_key, self.registry._query_key(GAME))
            await self.registry.discover_game(GAME)
            discover.assert_called_once()

    def test_indexed_dlc_upstream_page_confirms_official_without_merging_base_alias(self):
        self.registry.add_source(FALLBACK)
        indexed = candidate("hgamer", variant="dlc", upstream_page="https://publisher.example/game-plus/",
                            patch_title="A Game+ Patch", game_title="A Game+", aliases=["A Game+"], requires_dlc=[123])
        official = candidate(variant="dlc", source_page="https://publisher.example/game-plus/", game_appid=None,
                             match_basis="title", game_title="A Game+", aliases=["A Game+"], requires_dlc=[123])
        def discover(rule, query, old, context):
            return {"status": "complete", "candidates": [indexed] if rule["id"] == "hgamer" else [official], "diagnostics": []}
        mocked = types.SimpleNamespace(RequestContext=lambda **kw: object(), needs_fallback=lambda values, query: True,
            discover=discover, merge_candidates=lambda values: list({v["candidate_id"]: v for v in values}.values()),
            enrich_candidates=lambda values, identities: values, canonical_url=lambda value: value)
        with patch("sources._discovery_module", return_value=mocked):
            self.registry._discovery_sync(GAME, [_validate_rule(FALLBACK)], {})
        self.assertEqual(official["game_appid"], 42)
        self.assertEqual(official["parent_appid"], 42)
        self.assertEqual(official["requires_dlc"], [123])
        self.assertEqual(official["aliases"], ["A Game+"])

    async def test_legacy_match_only_suppresses_fallback_for_preferred_language(self):
        self.registry.add_source(PATCH)
        self.registry._cache["patchmoe"] = {"entries": [{"title": "A Game", "aliases": [], "language": "zh-CN",
              "url": "https://download.patch.moe/game.zip", "source_page": PATCH["pages"][0], "date": None}]}
        with patch.object(self.registry, "_discovery_sync", return_value=response()) as discover:
            await self.registry.discover_game(GAME, preferred_language="ja")
            self.assertFalse(discover.call_args.args[0]["legacy_match"])
        with patch.object(self.registry, "_discovery_sync", return_value=response()) as discover:
            await self.registry.discover_game(GAME, preferred_language="zh-CN", force=True)
            self.assertTrue(discover.call_args.args[0]["legacy_match"])

    async def test_real_hg_adapter_candidates_validate_persist_and_hit_hot_cache(self):
        from test_discovery import Fetcher, fixture
        from urllib.parse import parse_qs, urlsplit
        self.registry.set_enabled("official", False)
        self.registry.add_source(FALLBACK)
        module = sources._discovery_module()
        original_context = module.RequestContext
        def routes(url):
            parsed = urlsplit(url)
            if parsed.path.endswith("/search"):
                term = parse_qs(parsed.query)["q"][0]
                name = "hg_other_traditional" if "來" in term else "hg_other_search"
                return 200, fixture(name), {}, url.replace("zh-CN", "zh-TW")
            return fixture("hg_other" if "/games/" in parsed.path else "hg_other_patch")
        fetcher = Fetcher({"*": routes})
        class FixtureContext(original_context):
            def __init__(self, **kwargs):
                super().__init__(fetcher=fetcher, interval=0, **kwargs)
        game = {"appid": 4483210, "name": "新搬来的大姐姐真是太可爱了"}
        with patch.object(module, "RequestContext", FixtureContext):
            result = await self.registry.discover_game(game)
            self.assertEqual(result["status"], "complete", result["diagnostics"])
            self.assertEqual(result["candidates"][0]["game_appid"], 4483210)
            self.assertEqual(result["candidates"][0]["capability"], "discover_only")
            calls = len(fetcher.calls)
            self.assertGreater(calls, 2)
            hot = await self.registry.discover_game(game)
            self.assertEqual(hot["status"], "cache_complete")
            self.assertEqual(len(fetcher.calls), calls)
        reopened = SourceRegistry(self.root / "state", self.config)
        self.assertEqual(reopened.cached_candidates(game)[0]["game_appid"], 4483210)

    def test_upgrade_preserves_custom_rule_whose_id_becomes_a_builtin(self):
        from test_sources import TABLE
        custom = {**TABLE, "id": "kagura-cn", "name": "My publisher"}
        self.registry.override_path.parent.mkdir(parents=True, exist_ok=True)
        overrides = {"enabled": {"kagura-cn": False}, "added": [custom], "games": {"42": {"kagura-cn": False}}}
        self.registry.override_path.write_text(json.dumps(overrides))
        upgraded = SourceRegistry(self.root / "state", Path(sources.__file__).with_name("sources.json"))
        source = next(r for r in upgraded.list_sources() if r["id"] == "kagura-cn")
        self.assertEqual(source["adapter"], "table_html")
        self.assertEqual(source["name"], "My publisher")
        self.assertFalse(source["enabled"])
        self.assertFalse(upgraded.game_subscribed(42, "kagura-cn"))
        self.assertEqual(json.loads(self.registry.override_path.read_text()), overrides)


if __name__ == "__main__":
    unittest.main()
