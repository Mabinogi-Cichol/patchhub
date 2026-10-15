"""Isolated installer fault tests: no Steam, network or real plugin writes."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import socket
import stat
import struct
import tempfile
import unittest
from unittest.mock import patch
import zipfile

SPEC = importlib.util.spec_from_file_location("install_decky", Path(__file__).with_name("install-decky.py"))
m = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(m)


def create_package(path, version="0.2.0", changes=None):
    elf = b"\x7fELF\x02\x01\x01" + b"\0" * 11 + struct.pack("<H", 62) + b"fixture"
    files = {name: b"fixture" for name in (
        "main.py", "dist/index.js", "backend/bridge.py", "backend/tasks.py", "backend/protocol.py", "backend/v4.schema.json",
        "bin/catalog.json", "bin/signed-catalog.json", "bin/trust-keys.json", "LICENSE", "licenses/sfextract-MIT.txt", "dependencies.cdx.json")}
    files["plugin.json"] = json.dumps({"name": m.NAME, "api_version": 1, "flags": []}).encode()
    files["package.json"] = json.dumps({"name": "steam-patch-decky", "version": version, "type": "module"}).encode()
    entries = []
    for name in ("bin/steam-patch", "bin/sf-worker"):
        files[name] = elf
        entries.append({"path": name, "size": len(elf), "sha256": m.hashlib.sha256(elf).hexdigest()})
    files["bin/runtime.json"] = json.dumps({"protocol_version": 4, "worker_protocol": 1, "files": entries}).encode()
    files.update(changes or {})
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as bundle:
        for name, data in files.items():
            if data is None:
                continue
            info = zipfile.ZipInfo(m.FOLDER + "/" + name)
            info.create_system = 3
            info.external_attr = (0o100755 if name in ("bin/steam-patch", "bin/sf-worker") else 0o100644) << 16
            bundle.writestr(info, data)
    return {"tag": "v" + version, "sha256": m.digest(path), **m.validate_package(path)} if not changes else None


def unpack_fixture(archive, root):
    if root.exists():
        shutil.rmtree(root)
    root.mkdir(parents=True)
    with zipfile.ZipFile(archive) as bundle:
        for info in bundle.infolist():
            relative = Path(*Path(info.filename).parts[1:])
            target = root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(bundle.read(info))
            target.chmod((info.external_attr >> 16) & 0o777)


class FakeLoader(m.Loader):
    """Uses real durable mutate/settle code; only the external Loader is fake."""
    def __init__(self, root):
        self.plugin = root / "plugins" / m.FOLDER
        self.timeout = 0
        self.settings = {key: [] for key in m.SETTINGS}
        self.jobs = {}
        self.calls = []
        self.boot = "loader-1"
        self.fail_install = None
        self.fail_verify = False
        self.fail_rollback = False
        self.fail_cleanup = False
        self.active = False
        self.lost = False
        self.busy = False
        self.guard_id = None
        self.cdp = type("Connection", (), {"close": lambda _: None})()

    def identity(self):
        return self.boot

    def root(self):
        return self.plugin.parent.parent

    def call(self, op, **args):
        self.calls.append((op, args))
        if op == "acquire":
            self.guard_id = args["id"]
            return True
        if op == "guard":
            return self.guard_id == args["id"]
        if op == "release":
            self.guard_id = None
            return True
        if op == "probe":
            present = self.plugin.exists()
            version = m.json_read(self.plugin / "package.json")["version"] if present else None
            return {"plugins": [{"version": version, "disabled": m.NAME in self.settings["disabled_plugins"]}] if present else [],
                    "settings": {key: list(value) for key, value in self.settings.items()},
                    "instances": [{"version": version, "valid": True}] if present else [], "user": {"path": str(Path.home())}}
        if op == "tasks":
            m.require(not self.active, "active_or_unknown_tasks")
            return True
        if op == "status":
            if self.lost:
                return {"known": False, "done": False, "ok": False, "busy": self.busy}
            job = self.jobs.get(args["id"])
            return {"known": bool(job), "done": bool(job), "ok": bool(job and job["ok"]), "error": job.get("error") if job else None, "busy": self.busy}
        if op == "forget":
            self.jobs.pop(args["id"], None)
            return True
        if op == "launch":
            kind = args["kind"]
            job = self.jobs[args["id"]] = {"ok": True}
            if kind == "install":
                rollback = args["artifact"].endswith("rollback.zip")
                if rollback and self.fail_rollback:
                    job.update(ok=False, error="rollback_failed")
                    return True
                if self.fail_install == "before" and not rollback:
                    job.update(ok=False, error="install_failed")
                    return True
                source = Path(m.urllib.request.url2pathname(m.urllib.parse.urlsplit(args["artifact"]).path))
                unpack_fixture(source, self.plugin)
                # Simulate Loader's uninstall resetting just this plugin's settings.
                for key in m.SETTINGS:
                    self.settings[key] = [item for item in self.settings[key] if item != m.NAME]
                if self.fail_install == "after" and not rollback:
                    job.update(ok=False, error="reload_failed")
                if self.fail_install == "lost" and not rollback:
                    self.lost = self.busy = True
                if self.fail_install == "cancel" and not rollback:
                    raise KeyboardInterrupt()
            elif kind == "settings":
                for key in m.SETTINGS:
                    own = m.NAME in args["settings"][key]
                    self.settings[key] = [item for item in self.settings[key] if item != m.NAME]
                    if own:
                        self.settings[key].append(m.NAME)
            elif kind == "uninstall":
                shutil.rmtree(self.plugin)
            return True
        raise AssertionError(op)

    def verify(self, work, original, present=True):
        if self.fail_verify and original.get("version") == "0.2.0":
            raise m.InstallError("runtime_failed")
        if present:
            m.require(m.json_read(self.plugin / "package.json")["version"] == original["version"], "wrong_runtime")
        else:
            m.require(not self.plugin.exists(), "still_present")


@unittest.skipIf(os.name == "nt", "Installer recovery requires POSIX permissions; run in Linux/WSL")
class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name).resolve()
        self.work = self.root / "steam-patch-decky-install"
        self.work.mkdir(mode=0o700)
        self.loader = FakeLoader(self.root / "homebrew")
        self.installer = m.Installer(self.work, self.loader.plugin, self.loader)
        self.package = create_package(self.work / "package.zip")
        self.output = contextlib.redirect_stdout(io.StringIO())
        self.output.__enter__()

    def tearDown(self):
        self.output.__exit__(None, None, None)
        self.temporary.cleanup()

    def old(self):
        old = self.root / "original.zip"
        create_package(old, "0.1.0")
        unpack_fixture(old, self.loader.plugin)
        for key in ("pluginOrder", "frozenPlugins", "hiddenPlugins"):
            self.loader.settings[key] = ["other", m.NAME, "another"]
        self.original_files = m.tree_manifest(self.loader.plugin)

    def assert_original(self):
        self.assertEqual(m.tree_manifest(self.loader.plugin), self.original_files)
        self.assertFalse(self.work.exists())
        for key in ("pluginOrder", "frozenPlugins", "hiddenPlugins"):
            self.assertIn(m.NAME, self.loader.settings[key])
            self.assertIn("other", self.loader.settings[key])

    def test_first_install_and_update_clean_all_workspace_material(self):
        self.installer.install(self.package, yes=True)
        self.assertFalse(self.work.exists())
        self.assertEqual(m.json_read(self.loader.plugin / "package.json")["version"], "0.2.0")
        self.assertIn(m.NAME, self.loader.settings["pluginOrder"])
        self.assertFalse(self.loader.jobs)

    def test_update_keeps_external_state_and_other_plugins_settings(self):
        self.old()
        external = self.root / "business-state.json"
        external.write_text("preserve")
        self.installer.install(self.package, yes=True)
        self.assertFalse(self.work.exists())
        self.assertEqual(external.read_text(), "preserve")
        for key in ("pluginOrder", "frozenPlugins", "hiddenPlugins"):
            self.assertEqual(set(self.loader.settings[key]), {"other", "another", m.NAME})

    def test_install_and_reload_failures_restore_old_and_cleanup(self):
        self.old()
        self.loader.fail_install = "after"
        with self.assertRaisesRegex(m.InstallError, "已恢复"):
            self.installer.install(self.package, yes=True)
        self.assert_original()

    def test_install_rejection_before_uninstall_restores_and_cleans(self):
        self.old()
        self.loader.fail_install = "before"
        with self.assertRaisesRegex(m.InstallError, "已恢复"):
            self.installer.install(self.package, yes=True)
        self.assert_original()

    def test_formal_file_mismatch_triggers_restore(self):
        self.old()
        actual = self.loader.call
        def corrupt(op, **args):
            result = actual(op, **args)
            if op == "launch" and args.get("kind") == "install" and args["artifact"].endswith("package.zip"):
                (self.loader.plugin / "main.py").write_text("corrupt")
            return result
        with patch.object(self.loader, "call", side_effect=corrupt):
            with self.assertRaisesRegex(m.InstallError, "已恢复"):
                self.installer.install(self.package, yes=True)
        self.assert_original()

    def test_runtime_failure_restores_old_and_cleanup(self):
        self.old()
        self.loader.fail_verify = True
        with self.assertRaisesRegex(m.InstallError, "已恢复"):
            self.installer.install(self.package, yes=True)
        self.assert_original()

    def test_broken_new_backend_rpc_can_restore_under_retained_transaction_guard(self):
        self.old()
        self.loader.fail_verify = True
        actual = self.loader.call
        def broken_rpc(op, **args):
            if op == "tasks" and self.loader.plugin.exists() and m.json_read(self.loader.plugin / "package.json")["version"] == "0.2.0":
                raise m.InstallError("plugin_startup_failed")
            return actual(op, **args)
        with patch.object(self.loader, "call", side_effect=broken_rpc):
            with self.assertRaisesRegex(m.InstallError, "已恢复"):
                self.installer.install(self.package, yes=True)
        self.assert_original()
        self.assertIsNone(self.loader.guard_id)

    def test_first_install_runtime_failure_returns_to_absent(self):
        self.loader.fail_verify = True
        with self.assertRaisesRegex(m.InstallError, "已恢复"):
            self.installer.install(self.package, yes=True)
        self.assertFalse(self.loader.plugin.exists())
        self.assertFalse(self.work.exists())
        self.assertNotIn(m.NAME, self.loader.settings["pluginOrder"])

    def test_cancellation_restores_and_cleans(self):
        self.old()
        self.loader.fail_install = "cancel"
        with self.assertRaisesRegex(m.InstallError, "已恢复"):
            self.installer.install(self.package, yes=True)
        self.assert_original()

    def test_unknown_active_write_never_rolls_back_or_deletes_backup(self):
        self.old()
        self.loader.fail_install = "lost"
        with self.assertRaisesRegex(m.InstallError, "无法确认"):
            self.installer.install(self.package, yes=True)
        self.assertTrue((self.work / "rollback.zip").is_file())
        self.assertTrue((self.work / "package.zip").is_file())
        self.assertEqual(len([args for op, args in self.loader.calls if op == "launch" and args["kind"] == "install"]), 1)
        self.loader.lost = self.loader.busy = False
        self.installer.recover()
        self.assert_original()

    def test_missing_context_same_loader_retains_material_until_loader_restarts(self):
        self.old()
        self.loader.fail_install = "lost"
        with self.assertRaises(m.InstallError):
            self.installer.install(self.package, yes=True)
        self.loader.busy = False
        with self.assertRaisesRegex(m.InstallError, "无法确认"):
            self.installer.recover()
        self.assertTrue((self.work / "rollback.zip").exists())
        self.loader.boot = "loader-2"
        # The lost old context is now safe to recover, but new jobs have a context.
        real_mutate = self.loader.mutate
        def mutate(*args, **kwargs):
            self.loader.lost = False
            return real_mutate(*args, **kwargs)
        with patch.object(self.loader, "mutate", side_effect=mutate):
            self.installer.recover()
        self.assert_original()

    def test_failed_rollback_is_reentrant_and_preserves_unique_old_package(self):
        self.old()
        self.loader.fail_verify = self.loader.fail_rollback = True
        with self.assertRaisesRegex(m.InstallError, "rollback_failed"):
            self.installer.install(self.package, yes=True)
        backup_hash = m.digest(self.work / "rollback.zip")
        self.assertEqual({file.name for file in self.work.iterdir()}, {"rollback.zip", "journal.json"})
        self.loader.fail_rollback = False
        self.installer.recover()
        self.assert_original()
        self.assertTrue(backup_hash)
        # Recovery/cleanup is idempotent after successful completion.
        self.installer.recover()

    def test_cleanup_failure_keeps_complete_phase_and_retries_without_rollback(self):
        self.old()
        with patch.object(m, "cleanup", side_effect=PermissionError("cleanup_failed")):
            with self.assertRaisesRegex(m.InstallError, "新版已验证"):
                self.installer.install(self.package, yes=True)
        self.assertEqual(m.json_read(self.work / "journal.json")["phase"], "complete")
        installs = len([1 for op, args in self.loader.calls if op == "launch" and args["kind"] == "install"])
        self.installer.recover()
        self.assertFalse(self.work.exists())
        self.assertEqual(len([1 for op, args in self.loader.calls if op == "launch" and args["kind"] == "install"]), installs)

    def test_active_tasks_and_disabled_plugin_fail_before_mutation(self):
        self.old()
        self.loader.active = True
        with self.assertRaisesRegex(m.InstallError, "active"):
            self.installer.install(self.package, yes=True)
        self.loader.active = False
        self.loader.settings["disabled_plugins"] = [m.NAME]
        with self.assertRaisesRegex(m.InstallError, "已禁用"):
            self.installer.install(self.package, yes=True)
        self.assertFalse(any(op == "launch" for op, _ in self.loader.calls))

    def test_decline_does_not_mutate_or_create_recovery_record(self):
        with self.assertRaisesRegex(m.InstallError, "已取消"):
            self.installer.install(self.package, confirm=lambda _: "n")
        self.assertFalse((self.work / "journal.json").exists())
        self.assertFalse(any(op == "launch" for op, _ in self.loader.calls))

    def test_recovery_target_change_refuses_mutation(self):
        self.old()
        self.loader.fail_install = "lost"
        with self.assertRaises(m.InstallError):
            self.installer.install(self.package, yes=True)
        with patch.object(self.loader, "root", return_value=self.root / "different"):
            with self.assertRaisesRegex(m.InstallError, "根目录"):
                self.installer.recover()
        self.assertTrue((self.work / "rollback.zip").exists())

    def test_cleanup_rejects_unknown_files_without_touching_them(self):
        other = self.work / "user.zip"
        other.write_text("keep")
        with self.assertRaises(m.InstallError):
            m.cleanup(self.work)
        self.assertEqual(other.read_text(), "keep")

    def test_prepared_interruption_cleans_without_any_loader_mutation(self):
        self.old()
        original = m.backup_plugin(self.loader.plugin, self.work / "rollback.zip")
        m.State(self.work, {"schema": 1, "plugin": str(self.loader.plugin), "phase": "prepared", "original": original}).save()
        self.installer.recover()
        self.assert_original()
        self.assertFalse(any(op == "launch" for op, _ in self.loader.calls))

    @unittest.skipIf(os.name == "nt", "POSIX flock")
    def test_workspace_lock_leaves_no_lockfile_and_rejects_second_installer(self):
        cache = self.root / "cache"
        with m.workspace_lock(cache):
            with self.assertRaisesRegex(m.InstallError, "另一个"):
                with m.workspace_lock(cache):
                    self.fail("second lock acquired")
        self.assertEqual(list(cache.iterdir()), [])

    @unittest.skipIf(os.name == "nt", "Requires POSIX symlink permissions")
    def test_cleanup_never_follows_links(self):
        outside = self.root / "outside"
        outside.write_text("keep")
        (self.work / "rollback.zip").symlink_to(outside)
        with self.assertRaisesRegex(m.InstallError, "符号链接"):
            m.cleanup(self.work)
        self.assertEqual(outside.read_text(), "keep")

    def test_pending_is_durable_before_launch_and_cleared_before_forget(self):
        observed = []
        call = self.loader.call
        def spy(op, **args):
            if op in ("launch", "forget"):
                journal = m.json_read(self.work / "journal.json")
                observed.append((op, bool(journal.get("pending"))))
            return call(op, **args)
        with patch.object(self.loader, "call", side_effect=spy):
            self.installer.install(self.package, yes=True)
        self.assertTrue(all(pending == (op == "launch") for op, pending in observed))


class PackageTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.zip = self.root / "input.zip"

    def tearDown(self):
        self.tmp.cleanup()

    def test_rejects_unsafe_paths_duplicates_links_and_special_modes(self):
        for name, mode in [("../escape", 0o100644), ("/absolute", 0o100644), ("other/file", 0o100644),
                           ("steam-patch-manager/a/../file", 0o100644), ("steam-patch-manager\\file", 0o100644),
                           ("steam-patch-manager/link", 0o120777), ("steam-patch-manager/suid", 0o104755)]:
            create_package(self.zip)
            with zipfile.ZipFile(self.zip, "a") as bundle:
                info = zipfile.ZipInfo(name)
                info.filename = name  # Preserve a hostile backslash on Windows too.
                info.external_attr = mode << 16
                bundle.writestr(info, "bad")
            with self.subTest(name=name), self.assertRaises(m.InstallError):
                m.validate_package(self.zip)
        create_package(self.zip)
        with zipfile.ZipFile(self.zip, "a") as bundle:
            bundle.writestr("steam-patch-manager/MAIN.PY", "bad")
        with self.assertRaisesRegex(m.InstallError, "重复"):
            m.validate_package(self.zip)

    def test_rejects_wrong_version_runtime_and_identity(self):
        cases = [({"plugin.json": b'{"name":"other"}'}, None),
                 ({"bin/runtime.json": b'{"protocol_version":3}'}, None),
                 ({"bin/steam-patch": b"MZ"}, None),
                 ({"dist/index.js": None}, None),
                 ({}, "9.9.9")]
        for changes, expected in cases:
            create_package(self.zip, changes=changes)
            with self.subTest(changes=list(changes)), self.assertRaises(m.InstallError):
                m.validate_package(self.zip, expected)

    def test_runtime_must_be_executable_linux_x86_64(self):
        create_package(self.zip)
        with zipfile.ZipFile(self.zip) as bundle:
            entries = [(item, bundle.read(item)) for item in bundle.infolist()]
        with zipfile.ZipFile(self.zip, "w") as bundle:
            for item, data in entries:
                if item.filename.endswith("/bin/steam-patch"):
                    item.external_attr = 0o100644 << 16
                bundle.writestr(item, data)
        with self.assertRaisesRegex(m.InstallError, "runtime"):
            m.validate_package(self.zip)

    def test_release_freezes_latest_and_matches_tag_hash(self):
        create_package(self.zip)
        calls = []
        prefix = f"https://github.com/{m.REPOSITORY}/releases/download/v0.2.0/"
        release = {"tag_name": "v0.2.0", "assets": [{"name": n, "browser_download_url": prefix+n} for n in (m.ASSET, "SHA256SUMS.txt")]}
        def download(url, output=None, limit=None):
            calls.append(url)
            if "api.github.com" in url:
                return json.dumps(release).encode()
            if url.endswith("SHA256SUMS.txt"):
                return f"{m.digest(self.zip)}  {m.ASSET}\n".encode()
            output.write_bytes(self.zip.read_bytes())
        result = m.release_package(self.root, None, download)
        self.assertEqual(result["tag"], "v0.2.0")
        self.assertTrue(all("/v0.2.0/" in url for url in calls[1:]))
        release["tag_name"] = "v0.3.0"
        with self.assertRaisesRegex(m.InstallError, "Tag"):
            m.release_package(self.root, "v0.2.0", download)

    def test_bad_hash_and_download_interrupt_leave_only_owned_temporary_files(self):
        create_package(self.zip)
        prefix = f"https://github.com/{m.REPOSITORY}/releases/download/v0.2.0/"
        release = {"tag_name": "v0.2.0", "assets": [{"name": n, "browser_download_url": prefix+n} for n in (m.ASSET, "SHA256SUMS.txt")]}
        work = self.root / "steam-patch-decky-install"
        work.mkdir()
        def download(url, output=None, limit=None):
            if "api.github.com" in url:
                return json.dumps(release).encode()
            if url.endswith("SHA256SUMS.txt"):
                return ("0"*64 + "  " + m.ASSET).encode()
            output.write_bytes(self.zip.read_bytes())
        with self.assertRaisesRegex(m.InstallError, "SHA-256"):
            m.release_package(work, None, download)
        m.cleanup(work)
        self.assertFalse(work.exists())
        self.assertTrue(self.zip.exists())

    def test_partial_download_is_cleanable_without_deleting_callers_original(self):
        create_package(self.zip)
        work = self.root / "steam-patch-decky-install"
        work.mkdir()
        prefix = f"https://github.com/{m.REPOSITORY}/releases/download/v0.2.0/"
        release = {"tag_name": "v0.2.0", "assets": [{"name": n, "browser_download_url": prefix+n} for n in (m.ASSET, "SHA256SUMS.txt")]}
        def download(url, output=None, limit=None):
            if "api.github.com" in url:
                return json.dumps(release).encode()
            if url.endswith("SHA256SUMS.txt"):
                return f"{m.digest(self.zip)}  {m.ASSET}".encode()
            output.write_bytes(b"partial")
            raise ConnectionError("interrupted")
        with self.assertRaises(ConnectionError):
            m.release_package(work, None, download)
        m.cleanup(work)
        self.assertFalse(work.exists())
        self.assertTrue(self.zip.exists())


class AdapterTests(unittest.TestCase):
    @unittest.skipIf(os.name == "nt", "Loader paths are POSIX absolute paths")
    def test_discover_configured_and_standard_loader_roots_and_reject_environment_files(self):
        for configuration, expected in [
            ('Environment=PRIVILEGED_PATH=/home/deck/custom\nEnvironmentFiles=\nExecStart=ignored', '/home/deck/custom'),
            ('Environment=UNPRIVILEGED_PATH=/home/deck/homebrew\nEnvironmentFiles=\nExecStart=ignored', '/home/deck/homebrew'),
            ('Environment=PLUGIN_PATH=/home/deck/homebrew/plugins\nEnvironmentFiles=\nExecStart=ignored', '/home/deck/homebrew'),
            ('Environment=\nEnvironmentFiles=\nExecStart={ path=/home/deck/homebrew/service/PluginLoader ; argv[]=... }', '/home/deck/homebrew'),
        ]:
            with self.subTest(config=configuration), patch.object(m.subprocess, "run", return_value=type("Result", (), {"stdout": configuration})()):
                self.assertEqual(m.loader_root(), Path(expected).absolute())
        with patch.object(m.subprocess, "run", return_value=type("Result", (), {"stdout": "EnvironmentFiles=/etc/custom"})()):
            with self.assertRaises(m.InstallError):
                m.loader_root()

    @unittest.skipIf(os.name == "nt", "POSIX socketpair; Windows sandbox may block loopback setup")
    def test_websocket_masking_fragmentation_ping_and_cdp_errors(self):
        client, server = socket.socketpair()
        client.settimeout(2)
        server.settimeout(2)
        cdp = m.CDP(timeout=1)
        cdp.sock = client
        try:
            cdp._send(b"hello")
            frame = server.recv(1024)
            self.assertEqual(frame[:2], bytes([0x81, 0x85]))
            mask = frame[2:6]
            self.assertEqual(bytes(x ^ mask[i % 4] for i, x in enumerate(frame[6:])), b"hello")
            # Ping interspersed with a fragmented text message.
            server.sendall(b'\x01\x05{"ok"\x89\x01?\x80\x06:true}')
            self.assertEqual(cdp._message(), {"ok": True})
            pong = server.recv(1024)
            self.assertEqual(pong[0], 0x8a)
        finally:
            cdp.close()
            server.close()

    def test_cdp_returns_safe_actionable_bridge_error(self):
        cdp = m.CDP()
        cdp.sock = object()
        message = {"id": 1, "result": {"exceptionDetails": {"exception": {"description": "Error: unsupported_loader_version\nstack"}}}}
        with patch.object(cdp, "_send"), patch.object(cdp, "_message", return_value=message):
            with self.assertRaisesRegex(m.InstallError, "unsupported_loader_version"):
                cdp.evaluate("fixture")


if __name__ == "__main__":
    unittest.main()
