import asyncio
import hashlib
import json
from pathlib import Path
import tempfile
import types
from threading import Event
import unittest
from unittest.mock import patch, Mock
from urllib.error import HTTPError
from io import BytesIO
from source_primitives import allowed_url, _normal, _split_names
import zipfile

from publisher import Publisher, _download, _recipe, _released


class FakeSourceService:
    """Duck-type stand-in for the Rust-backed adapter (no binary, no network)."""

    engine = "rust"

    def __init__(self, rules):
        self._rules = [dict(rule) for rule in rules]
        self.builtins = [dict(rule) for rule in self._rules]
        self.settings_revision = 1
        self.diagnostics = []
        self.entries = {rule["id"]: [] for rule in self._rules}

    def list_sources(self):
        return [{**rule, "status": "cached", "entry_count": len(self.entries.get(rule["id"], [])),
                 "cached_at": None, "checked_at": None, "origin": "builtin"} for rule in self._rules]

    def list_game_sources(self, game):
        return [{**source, "subscribed": self.game_subscribed(game.get("appid"), source["id"]),
                 "matched": False} for source in self.list_sources()]

    def matches(self, game, include_disabled=False):
        name = _normal(game["name"])
        result = []
        for rule in self._rules:
            if rule.get("discovery_only"):
                continue
            if not include_disabled and (not rule.get("enabled")
                                         or not self.game_subscribed(game["appid"], rule["id"])):
                continue
            candidates = []
            for entry in self.entries.get(rule["id"], []):
                names = set(_normal(value) for title in (entry["title"], *entry.get("aliases", []))
                            for value in _split_names(title))
                if entry.get("appid") == game["appid"] or (entry.get("appid") is None and name and name in names):
                    candidates.append(entry)
            if not candidates or len({entry["url"] for entry in candidates}) != 1:
                continue
            selected = next((entry for entry in candidates
                             if entry["language"].lower() in ("zh-cn", "zh_cn")), candidates[0])
            release_id = (f"patchmoe-{game['appid']}" if rule["id"] == "patchmoe"
                          else f"source-{rule['id']}-{game['appid']}")
            result.append({**selected, "source_id": rule["id"], "release_id": release_id,
                           "root_layout": rule["root_layout"]})
        return result

    def cached_candidates(self, game):
        return []

    def game_subscribed(self, appid, source_id):
        return True

    def source_for_release(self, release_id):
        return None

    def validate_download(self, source_id, url):
        rule = next((item for item in self._rules if item["id"] == source_id and item.get("enabled")), None)
        if rule is None:
            raise ValueError("source_not_enabled")
        return allowed_url(url, rule["download_hosts"], download=True)


def fixture_sources(root, entries):
    rule = {"id": "patchmoe", "name": "patch.moe", "adapter": "patchmoe", "enabled": True,
            "pages": ["https://www.patch.moe/?flag=1"], "page_hosts": ["www.patch.moe"],
            "download_hosts": ["download.patch.moe"], "root_layout": "game_root"}
    sources = FakeSourceService([rule])
    sources.entries["patchmoe"] = [{**entry, "source_page": "https://www.patch.moe/"} for entry in entries]
    return sources


class DummyBridge:
    def __init__(self, state_dir):
        self.state_dir = state_dir


class PublisherTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.index = self.root / "index.json"
        entries = [
            {"title": "Liminal Border Part III / Criminal Border 3rd Offence",
             "aliases": ["境界之限 第三部"], "url": "https://download.patch.moe/a.zip", "language": "zh-CN"},
            {"title": "Another Game", "aliases": ["另一个游戏"],
             "url": "https://download.patch.moe/b.zip", "language": "zh-CN"}]
        self.sources = fixture_sources(self.root, entries)
        self.publisher = Publisher(DummyBridge(self.root / "state"), self.sources)

    def test_exact_name_and_slash_alias_but_no_substring(self):
        self.assertEqual(self.publisher.match({"name": "Criminal Border: 3rd Offence", "appid": 12})["release_id"], "patchmoe-12")
        self.assertEqual(self.publisher.match({"name": "境界之限 第三部", "appid": 12})["url"], "https://download.patch.moe/a.zip")
        self.assertIsNone(self.publisher.match({"name": "Border", "appid": 12}))

    def test_different_urls_make_name_ambiguous(self):
        self.sources.entries["patchmoe"].append({"title": "Another Game", "aliases": [],
                                       "url": "https://download.patch.moe/c.zip", "language": "en"})
        self.assertIsNone(self.publisher.match({"name": "Another Game", "appid": 14}))

    def test_plus_is_a_different_game_not_decoration(self):
        self.sources.entries["patchmoe"].append({"title": "Another Game+", "aliases": [],
            "url": "https://download.patch.moe/plus.zip", "language": "en"})
        self.assertEqual(self.publisher.match({"name": "Another Game", "appid": 14})["url"],
            "https://download.patch.moe/b.zip")

    def test_unreleased_index_entries_are_not_eligible_for_prepare(self):
        self.assertFalse(_released({"date": "COMING SOON"}))
        self.assertFalse(_released({"date": "2999/1/1"}))
        self.assertTrue(_released({"date": "2000/1/1"}))

    def test_bridges_keep_same_app_in_two_libraries_isolated(self):
        for library in ("library-a", "library-b"):
            game = self.root / library / "game"
            game.mkdir(parents=True)
            instance = f"12|{game.parent}|{game}"
            state = self.publisher._state(instance, "patchmoe-12")
            state.mkdir(parents=True)
            for filename in ("catalog.json", "signed.json", "public.json"):
                (state / filename).write_text("{}")
            (state / "instance.json").write_text(json.dumps({"instance_id": instance,
                "release_id": "patchmoe-12", "install_path": str(game)}))
        with patch.object(self.publisher, "_bridge", side_effect=lambda state: state):
            bridges = self.publisher.bridges()
            self.assertEqual(len(bridges), 2)
            self.assertNotEqual(bridges[0][2], bridges[1][2])
            self.assertEqual({row[0] for row in bridges},
                {f"12|{self.root / library}|{self.root / library / 'game'}" for library in ("library-a", "library-b")})

    def test_zip_recipe_preserves_root_paths_and_baseline_hash(self):
        game = self.root / "game"
        game.mkdir()
        (game / "data.txt").write_bytes(b"original")
        archive = self.root / "patch.zip"
        with zipfile.ZipFile(archive, "w") as z:
            z.writestr("data.txt", b"patched")
            z.writestr("sub/new.txt", b"new")
        files = _recipe(archive, game, Event())
        self.assertEqual(files[0]["source"], "data.txt")
        self.assertEqual(files[0]["destination"], "data.txt")
        self.assertEqual(files[0]["original_sha256"], hashlib.sha256(b"original").hexdigest())
        self.assertEqual(files[0]["installed_sha256"], hashlib.sha256(b"patched").hexdigest())
        self.assertEqual(files[1]["destination"], "sub/new.txt")
        self.assertIsNone(files[1]["original_sha256"])
        self.assertEqual((game / "data.txt").read_bytes(), b"original")
        self.assertFalse((game / "sub").exists())

    def test_wrapped_xp3_patch_maps_to_root_without_title_rule(self):
        game = self.root / "game"
        game.mkdir()
        signature = bytes.fromhex("5850330d0a200a1a8b6701")
        (game / "data.xp3").write_bytes(signature + b"base")
        (game / "patch.xp3").write_bytes(signature + b"old")
        archive = self.root / "patch.zip"
        with zipfile.ZipFile(archive, "w") as z:
            z.writestr("Arbitrary wrapper/patch.xp3", signature + b"new")
            z.writestr("Arbitrary wrapper/Readme.txt", b"instructions")
        files = _recipe(archive, game, Event())
        self.assertEqual(len(files), 1)
        self.assertEqual(files[0]["source"], "Arbitrary wrapper/patch.xp3")
        self.assertEqual(files[0]["destination"], "patch.xp3")
        self.assertEqual(files[0]["original_sha256"], hashlib.sha256(signature + b"old").hexdigest())
        (game / "data.xp3").write_bytes(b"different engine")
        self.assertEqual(_recipe(archive, game, Event())[0]["destination"], "Arbitrary wrapper/patch.xp3")

    def test_wrapped_xp3_unknown_members_and_bad_signatures_blocked(self):
        game = self.root / "game"
        game.mkdir()
        signature = bytes.fromhex("5850330d0a200a1a8b6701")
        (game / "data.xp3").write_bytes(signature + b"base")
        archive = self.root / "patch.zip"
        for entries in ((("wrapper/patch.xp3", signature), ("wrapper/run.exe", b"unknown")),
                        (("wrapper/patch.xp3", b"not XP3"),)):
            with zipfile.ZipFile(archive, "w") as z:
                for name, content in entries:
                    z.writestr(name, content)
            with self.assertRaisesRegex(ValueError, "zip_wrapped_layout_unsupported"):
                _recipe(archive, game, Event())

    def test_zip_traversal_and_case_collision_rejected(self):
        game = self.root / "game"
        game.mkdir()
        archive = self.root / "bad.zip"
        with zipfile.ZipFile(archive, "w") as z:
            z.writestr("../escape", b"bad")
        with self.assertRaisesRegex(ValueError, "unsafe_relative_path"):
            _recipe(archive, game, Event())
        with zipfile.ZipFile(archive, "w") as z:
            z.writestr("A.txt", b"one")
            z.writestr("a.txt", b"two")
        with self.assertRaisesRegex(ValueError, "zip_duplicate_or_case_collision"):
            _recipe(archive, game, Event())

    def test_only_manifest_verified_download_is_reused(self):
        cache = self.root / "cache"
        cache.mkdir()
        archive = cache / "archive.zip"
        archive.write_bytes(b"not a download")
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        (cache / "download.json").write_text(json.dumps({"url": "https://download.patch.moe/a.zip",
                                                          "sha256": digest, "size": archive.stat().st_size}))
        opener = Mock()
        opener.open.side_effect = HTTPError("https://download.patch.moe/a.zip", 304, "unchanged", {}, None)
        receipt = json.loads((cache / "download.json").read_text())
        receipt["etag"] = '"v1"'
        (cache / "download.json").write_text(json.dumps(receipt))
        with patch("publisher.build_opener", return_value=opener):
            self.assertEqual(_download("https://download.patch.moe/a.zip", cache, Event())[3], "local")
        self.assertEqual(opener.open.call_args.args[0].get_header("If-none-match"), '"v1"')
        archive.write_bytes(b"changed")
        with patch("publisher.build_opener", side_effect=RuntimeError("must redownload")):
            with self.assertRaisesRegex(RuntimeError, "must redownload"):
                _download("https://download.patch.moe/a.zip", cache, Event())

    def test_same_url_is_revalidated_and_changed_content_is_downloaded(self):
        cache = self.root / "updated"
        cache.mkdir()
        (cache / "archive.zip").write_bytes(b"old")
        (cache / "download.json").write_text(json.dumps({"url": "https://download.patch.moe/a.zip",
            "sha256": hashlib.sha256(b"old").hexdigest(), "size": 3, "etag": '"old"'}))
        class Response(BytesIO):
            headers = {"ETag": '"new"'}
            def geturl(self):
                return "https://download.patch.moe/a.zip"
        opener = Mock()
        opener.open.return_value = Response(b"updated")
        with patch("publisher.build_opener", return_value=opener):
            archive, digest, size, mode = _download("https://download.patch.moe/a.zip", cache, Event())
        self.assertEqual((archive.read_bytes(), digest, size, mode),
            (b"updated", hashlib.sha256(b"updated").hexdigest(), 7, "download"))

    def test_clear_cache_removes_download_files_and_keeps_recipe(self):
        instance = f"12|{self.root}|{self.root / 'game'}"
        cache = self.publisher._state(instance, "patchmoe-12") / "cache"
        cache.mkdir(parents=True)
        (cache / "archive.zip").write_bytes(b"zip")
        (cache / "download.json").write_bytes(b"{}")
        (cache / "download-stale.tmp").write_bytes(b"tmp")
        (cache / "catalog.json").write_bytes(b"recipe")
        self.assertEqual(self.publisher.clear_cache(instance, "patchmoe-12"), 3)
        self.assertTrue((cache / "catalog.json").exists())
        self.assertEqual(self.publisher.clear_cache(instance, "patchmoe-12"), 0)

    def test_caches_enumerates_existing_cache_directories(self):
        instance = f"12|{self.root}|{self.root / 'game'}"
        cache = self.publisher._state(instance, "patchmoe-12") / "cache"
        cache.mkdir(parents=True)
        (cache / "archive.zip").write_bytes(b"zip")
        self.assertEqual([(release, path) for release, _state, path in self.publisher.caches()],
            [("patchmoe-12", cache)])


class ExistingTransactionTests(unittest.IsolatedAsyncioTestCase):
    async def test_committed_receipt_rejects_new_recipe_before_download(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            sources = fixture_sources(root, [{"title": "Game", "aliases": [],
                "url": "https://download.patch.moe/game.zip", "language": "en"}])
            publisher = Publisher(DummyBridge(root / "state"), sources)
            game_root = root / "game"
            game_root.mkdir()
            game = {"name": "Game", "appid": 9, "instance_id": f"9|{root}|{game_root}",
                    "install_path": str(game_root), "buildid": "42", "availability": "installed"}
            state = publisher._state(game["instance_id"], "patchmoe-9")
            state.mkdir(parents=True)
            (state / "catalog.json").write_text("{}")
            class Existing:
                async def list_transactions(self):
                    return [{"state": "committed"}]
            with patch.object(publisher, "_bridge", return_value=Existing()), patch("publisher._download") as download:
                with self.assertRaisesRegex(RuntimeError, "publisher_existing_transaction"):
                    await publisher.prepare(game, publisher.match(game), Event())
                download.assert_not_called()


class ReinstallContentTests(unittest.IsolatedAsyncioTestCase):
    async def test_changed_zip_keeps_previous_catalog_and_signed_detection(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            sources = fixture_sources(root, [{"title": "Game", "aliases": [],
                "url": "https://download.patch.moe/game.zip", "language": "en"}])
            publisher = Publisher(DummyBridge(root / "state"), sources)
            game_root = root / "game"
            game_root.mkdir()
            game = {"name": "Game", "appid": 9, "instance_id": f"9|{root}|{game_root}",
                    "install_path": str(game_root), "buildid": "42", "availability": "installed"}
            matched = publisher.match(game)
            state = publisher._state(game["instance_id"], matched["release_id"])
            state.mkdir(parents=True)
            previous = {"schema_version": 1, "revision": 1, "releases": [{
                "release_id": matched["release_id"], "appid": 9, "language": "en",
                "verified_buildids": ["42"], "recipe": {"artifact_sha256": "a" * 64,
                "files": [{"source": "patch.txt", "destination": "patch.txt", "installed_sha256": "b" * 64}]}}]}
            catalog = state / "catalog.json"
            catalog.write_text(json.dumps(previous), encoding="utf-8")
            signed = state / "signed.json"
            signed.write_bytes(b"previous-signature")
            archive = root / "new.zip"
            with zipfile.ZipFile(archive, "w") as package:
                package.writestr("patch.txt", b"updated-content")
            from unittest.mock import AsyncMock
            existing = types.SimpleNamespace(list_transactions=AsyncMock(return_value=[{"state": "committed"}]))
            with patch.object(publisher, "_bridge", return_value=existing), patch.object(publisher, "_native", new_callable=AsyncMock) as native:
                with self.assertRaisesRegex(ValueError, "reinstall_content_identity_changed"):
                    await publisher.prepare(game, matched, Event(), local_archive=archive, operation="reinstall")
                native.assert_not_awaited()
            self.assertEqual(json.loads(catalog.read_text(encoding="utf-8")), previous)
            self.assertEqual(signed.read_bytes(), b"previous-signature")


if __name__ == "__main__":
    unittest.main()
