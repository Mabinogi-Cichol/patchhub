import asyncio
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch, AsyncMock
import os
import sys
import types

from sources import SourceRegistry, _BoundedRedirect


PATCH = {"id": "patchmoe", "name": "patch.moe", "adapter": "patchmoe", "enabled": True,
         "pages": ["https://www.patch.moe/?flag=1"], "page_hosts": ["www.patch.moe"],
         "download_hosts": ["download.patch.moe"], "root_layout": "game_root"}
TABLE = {"id": "second", "name": "Second source", "adapter": "table_html", "enabled": True,
         "pages": ["https://patches.example.org/list"], "page_hosts": ["patches.example.org"],
         "download_hosts": ["cdn.example.org"], "root_layout": "game_root",
         "columns": {"title": 0, "appid": 1, "download": 2, "date": 3, "language": 4}}


class SourcesTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.config = self.root / "sources.json"
        self.config.write_text(json.dumps({"schema_version": 1, "sources": [PATCH, TABLE]}))
        self.registry = SourceRegistry(self.root / "state", self.config)

    async def test_live_new_title_after_refresh_and_source_separation(self):
        rows = '''<table><tr><td>Brand New + Game</td><td>42</td><td><a href="https://cdn.example.org/new.zip">ZIP</a></td><td>2026/1/1</td><td>en</td></tr></table>'''
        self.registry.set_enabled("patchmoe", False)
        self.assertEqual(self.registry.matches({"name": "Brand New Plus Game", "appid": 42}), [])
        with patch.object(self.registry, "_fetch", return_value=(200, rows, {"ETag": "v1"}, TABLE["pages"][0])):
            diagnostics = await self.registry.refresh(force=True)
        self.assertEqual(diagnostics[0]["code"], "source_refreshed")
        match = self.registry.matches({"name": "Brand New Plus Game", "appid": 42})
        self.assertEqual(len(match), 1)
        self.assertEqual(match[0]["release_id"], "source-second-42")
        self.assertEqual(match[0]["source_id"], "second")
        self.assertEqual(self.registry.list_sources()[1]["entry_count"], 1)
        self.assertEqual(self.registry.source_for_release("source-second-42")["id"], "second")
        self.assertEqual(self.registry.source_for_release("patchmoe-42")["id"], "patchmoe")

    async def test_patchmoe_forms_language_pairing_only_same_url(self):
        form = '''<form name="form_en" method="post" action="/"><input type="hidden" name="lang"></form>
                  <form name="form_zh_cn" method="post" action="/"><input type="hidden" name="lang"></form>
                  <script>document.form_en.lang.value = 'en'; document.form_zh_cn.lang.value = 'zh_cn';</script>'''
        english = '''<table><tr><td class="title">New Game</td><td class="release">2026/1/1</td><td class="patch"><a href="https://download.patch.moe/new.zip\n">DLC</a></td></tr></table>'''
        chinese = (english.replace("New Game", "新游戏") +
                   '''<table><tr><td class="title">RAR only</td><td class="release">2026/1/1</td><td class="patch"><a href="https://download.patch.moe/rar_only.rar">RAR</a></td></tr></table>''')
        def fetch(url, rule, *, data=None, **kwargs):
            if data is None:
                return 200, form, {}, PATCH["pages"][0]
            return 200, chinese if b"zh_cn" in data else english, {}, "https://www.patch.moe/"
        self.registry.set_enabled("second", False)
        with patch.object(self.registry, "_fetch", side_effect=fetch):
            diagnostics = await self.registry.refresh(force=True)
        self.assertEqual(diagnostics[0]["code"], "source_refreshed")
        result = self.registry.matches({"name": "New Game", "appid": 7})
        self.assertEqual(len(result), 1)
        self.assertEqual(result[0]["language"], "zh-CN")
        self.assertEqual(result[0]["title"], "新游戏")
        self.assertIn("New Game", result[0]["aliases"])
        self.assertEqual(result[0]["release_id"], "patchmoe-7")
        self.assertEqual(self.registry.matches({"name": "RAR only", "appid": 8}), [])

    async def test_new_publication_replaces_cache_without_game_configuration(self):
        self.registry.set_enabled("patchmoe", False)
        def row(title, appid):
            return f'<tr><td>{title}</td><td>{appid}</td><td><a href="https://cdn.example.org/{appid}.zip">ZIP</a></td><td>2026/1/1</td><td>en</td></tr>'
        with patch.object(self.registry, "_fetch", return_value=(200, "<table>" + row("Old", 10) + "</table>", {"ETag": "v1"}, TABLE["pages"][0])):
            await self.registry.refresh(force=True)
        self.assertEqual(self.registry.matches({"name": "New", "appid": 11}), [])
        with patch.object(self.registry, "_fetch", return_value=(200, "<table>" + row("Old", 10) + row("New", 11) + "</table>", {"ETag": "v2"}, TABLE["pages"][0])):
            await self.registry.refresh(force=True)
        reopened = SourceRegistry(self.root / "state", self.config)
        self.assertEqual(reopened.matches({"name": "New", "appid": 11})[0]["release_id"], "source-second-11")
        with patch.object(reopened, "_fetch", return_value=(304, "", {}, TABLE["pages"][0])) as fetch:
            result = await reopened.refresh(force=True)
        self.assertEqual(result[0]["code"], "source_not_modified")
        self.assertEqual(fetch.call_args.kwargs["etag"], "v2")
        self.assertEqual(reopened.list_sources()[1]["entry_count"], 2)

    async def test_refresh_respects_ttl_and_force_bypasses_it(self):
        self.registry.set_enabled("patchmoe", False)
        rows = '<table><tr><td>Game</td><td>42</td><td><a href="https://cdn.example.org/a.zip">ZIP</a></td><td>2026/1/1</td><td>en</td></tr></table>'
        with patch.object(self.registry, "_fetch", return_value=(200, rows, {}, TABLE["pages"][0])) as fetch:
            await self.registry.refresh(force=False)
            self.assertEqual(fetch.call_count, 1)
            self.assertEqual((await self.registry.refresh(force=False))[0]["code"], "source_cache_fresh")
            self.assertEqual(fetch.call_count, 1)
            await self.registry.refresh(force=True)
            self.assertEqual(fetch.call_count, 2)

    async def test_live_chinese_pending_date_is_normalized(self):
        self.registry.set_enabled("second", False)
        forms = '<form name="form_zh_cn" method="post" action="/"><input name="lang" type="hidden" value="cn"></form>'
        rows = '<table><tr><td class="title">新游戏</td><td class="release">待定</td><td class="patch"><a href="https://download.patch.moe/new.zip">下载</a></td></tr></table>'
        with patch.object(self.registry, "_fetch", side_effect=[(200, forms, {}, PATCH["pages"][0]), (200, rows, {}, "https://www.patch.moe/")]):
            result = await self.registry.refresh(force=True)
        self.assertEqual(result[0]["code"], "source_refreshed")
        self.assertEqual(self.registry.matches({"name": "新游戏", "appid": 9})[0]["date"], "COMING SOON")

    async def test_ambiguous_urls_refuse_match_and_failed_parse_preserves_cache(self):
        self.registry.set_enabled("patchmoe", False)
        rows = '''<table>
          <tr><td>Same Game</td><td>42</td><td><a href="https://cdn.example.org/a.zip">A</a></td><td>2026/1/1</td><td>en</td></tr>
          <tr><td>Same Game</td><td>42</td><td><a href="https://cdn.example.org/b.zip">B</a></td><td>2026/1/1</td><td>en</td></tr></table>'''
        with patch.object(self.registry, "_fetch", return_value=(200, rows, {}, TABLE["pages"][0])):
            await self.registry.refresh(force=True)
        self.assertEqual(self.registry.matches({"name": "Same Game", "appid": 42}), [])
        cached = (self.registry.state_dir / "cache/second.json").read_bytes()
        with patch.object(self.registry, "_fetch", return_value=(200, "<html>broken</html>", {}, TABLE["pages"][0])):
            diagnostics = await self.registry.refresh(force=True)
        self.assertEqual(diagnostics[0]["code"], "source_offline_cached")
        self.assertEqual((self.registry.state_dir / "cache/second.json").read_bytes(), cached)

    def test_hostile_urls_and_redirects_rejected(self):
        with self.assertRaises(ValueError):
            self.registry.validate_download("second", "https://localhost/a.zip")
        with self.assertRaises(ValueError):
            self.registry.validate_download("second", "http://cdn.example.org/a.zip")
        with self.assertRaises(ValueError):
            self.registry.validate_download("second", "https://cdn.example.org.evil/a.zip")
        redirect = _BoundedRedirect(TABLE["page_hosts"])
        with self.assertRaises(ValueError):
            redirect.redirect_request(None, None, 302, "", {}, "https://evil.example/anything")

    async def test_malformed_date_or_appid_keeps_previous_cache(self):
        self.registry.set_enabled("patchmoe", False)
        good = '''<table><tr><td>Game</td><td>42</td><td><a href="https://cdn.example.org/g.zip">ZIP</a></td><td>2026/1/1</td><td>en</td></tr></table>'''
        with patch.object(self.registry, "_fetch", return_value=(200, good, {}, TABLE["pages"][0])):
            await self.registry.refresh(force=True)
        bad_date = good.replace("2026/1/1", "nonsense")
        with patch.object(self.registry, "_fetch", return_value=(200, bad_date, {}, TABLE["pages"][0])):
            self.assertEqual((await self.registry.refresh(force=True))[0]["code"], "source_offline_cached")
        bad_appid = good.replace(">42<", ">unknown<")
        with patch.object(self.registry, "_fetch", return_value=(200, bad_appid, {}, TABLE["pages"][0])):
            self.assertEqual((await self.registry.refresh(force=True))[0]["code"], "source_offline_cached")
        self.assertEqual(len(self.registry.matches({"name": "Game", "appid": 42})), 1)

    def test_add_and_toggle_declarative_source_persists(self):
        new = {**TABLE, "id": "third", "name": "Third source"}
        self.registry.add_source(new)
        self.registry.set_enabled("third", False)
        reopened = SourceRegistry(self.root / "state", self.config)
        self.assertFalse(next(r for r in reopened.list_sources() if r["id"] == "third")["enabled"])

    async def test_game_subscription_filters_only_one_app_and_survives_global_changes(self):
        self.registry.set_enabled("patchmoe", False)
        rows = '<table>' + ''.join(
            f'<tr><td>Game {appid}</td><td>{appid}</td><td><a href="https://cdn.example.org/{appid}.zip">ZIP</a></td><td>2026/1/1</td><td>en</td></tr>'
            for appid in (42, 43)) + '</table>'
        with patch.object(self.registry, "_fetch", return_value=(200, rows, {}, TABLE["pages"][0])):
            await self.registry.refresh(force=True)
        game = {"appid": 42, "name": "Game 42"}
        self.registry.set_game_source(42, "second", False)
        self.assertEqual(self.registry.matches(game), [])
        self.assertEqual(len(self.registry.matches({"appid": 43, "name": "Game 43"})), 1)
        source = next(s for s in self.registry.list_game_sources(game) if s["id"] == "second")
        self.assertTrue(source["enabled"], "game selection must not disable the global source")
        self.assertTrue(source["matched"], "excluded sources must remain available to select again")
        self.assertFalse(source["subscribed"])
        self.registry.set_enabled("second", False)
        self.registry.add_source({**TABLE, "id": "third", "name": "Third"})
        reopened = SourceRegistry(self.root / "state", self.config)
        self.assertFalse(reopened.game_subscribed(42, "second"))
        reopened.set_enabled("second", True)
        self.assertEqual(reopened.matches(game), [])
        reopened.set_game_source(42, "second", True)
        self.assertEqual(len(reopened.matches(game)), 1)
        reopened.set_enabled("second", False)
        self.assertEqual(reopened.matches(game), [], "a game cannot override global source disablement")

    def test_game_subscription_validates_inputs_and_legacy_file(self):
        # Existing override files predate game selections.
        self.registry.override_path.parent.mkdir(parents=True, exist_ok=True)
        self.registry.override_path.write_text(json.dumps({"enabled": {}, "added": []}))
        self.registry._reload()
        self.assertTrue(self.registry.game_subscribed(42, "second"))
        for appid, source, subscribed in ((True, "second", False), (0, "second", True),
                                          (42, "missing", True), (42, "second", "yes")):
            with self.assertRaises(ValueError):
                self.registry.set_game_source(appid, source, subscribed)
        self.registry.set_game_source(42, "second", False)
        overrides = json.loads(self.registry.override_path.read_text())
        overrides["games"]["42"]["second"] = "no"
        self.registry.override_path.write_text(json.dumps(overrides))
        with self.assertRaisesRegex(ValueError, "invalid_source_overrides"):
            SourceRegistry(self.root / "state", self.config)

    async def test_frozen_decky_uses_system_python_without_bundled_library_environment(self):
        diagnostics = [{"source_id": "patchmoe", "code": "source_refresh_failed"}]
        process = types.SimpleNamespace(returncode=0,
            communicate=AsyncMock(return_value=(json.dumps(diagnostics).encode(), b"")))
        with patch.object(sys, "frozen", True, create=True), patch.dict(os.environ,
            {"PYTHONHOME": "/frozen", "PYTHONPATH": "/frozen", "LD_LIBRARY_PATH": "/frozen", "LD_LIBRARY_PATH_ORIG": "/usr/lib"}), \
            patch("sources.asyncio.create_subprocess_exec", AsyncMock(return_value=process)) as spawn:
            self.assertEqual(await self.registry.refresh(force=True), diagnostics)
        self.assertEqual(spawn.call_args.args[:2], ("/usr/bin/python3", "-I"))
        worker_env = spawn.call_args.kwargs["env"]
        self.assertNotIn("PYTHONHOME", worker_env)
        self.assertNotIn("PYTHONPATH", worker_env)
        self.assertEqual(worker_env["LD_LIBRARY_PATH"], "/usr/lib")

    async def test_missing_system_worker_preserves_source_cache_and_reports_failure(self):
        self.registry.set_enabled("patchmoe", False)
        rows = '<table><tr><td>Game</td><td>42</td><td><a href="https://cdn.example.org/g.zip">ZIP</a></td><td>2026/1/1</td><td>en</td></tr></table>'
        with patch.object(self.registry, "_fetch", return_value=(200, rows, {}, TABLE["pages"][0])):
            await self.registry.refresh(force=True)
        with patch.object(sys, "frozen", True, create=True), patch("sources.asyncio.create_subprocess_exec", AsyncMock(side_effect=FileNotFoundError("worker unavailable"))):
            result = await self.registry.refresh(force=True)
        self.assertEqual(result[0]["code"], "source_offline_cached")
        self.assertEqual(len(self.registry.matches({"name": "Game", "appid": 42})), 1)


if __name__ == "__main__":
    unittest.main()
