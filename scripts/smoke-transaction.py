"""Exercise the production CLI trust and transaction boundary in an isolated Steam fixture."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import zipfile


def main(binary, interrupt=False):
    binary = str(Path(binary).resolve(strict=True))
    with tempfile.TemporaryDirectory(prefix="steam-patch-smoke-") as temporary:
        root = Path(temporary)
        library = root / "Steam 库"
        game = library / "steamapps/common/Test Game"
        game.mkdir(parents=True)
        (game / "original.dat").write_bytes(b"original")
        (game / "save.dat").write_bytes(b"user save")
        (library / "steamapps/appmanifest_42.acf").write_text(
            '"AppState" { "appid" "42" "name" "Synthetic" "installdir" "Test Game" "buildid" "100" "StateFlags" "4" }', encoding="utf-8")
        new_bytes = b"new" if not interrupt else bytes(range(256)) * (1024 * 1024)
        archive = root / "patch.zip"
        with zipfile.ZipFile(archive, "w") as package:
            package.writestr("original.dat", b"patched")
            package.writestr("new.dat", new_bytes)
        digest = lambda data: hashlib.sha256(data).hexdigest()
        release = dict(patch_id="test", release_id="test-1", appid=42, name="Synthetic patch", version="1",
                       language="test", category="content_restore", source_page="https://example.invalid/patch",
                       support_level="managed_zip", platforms=["windows", "steamos"], verified_buildids=["100"],
                       detection=[dict(path="original.dat", sha256=digest(b"patched"))], artifact=None,
                       recipe=dict(artifact_sha256=digest(archive.read_bytes()), files=[
                           dict(source="original.dat", destination="original.dat", original_sha256=digest(b"original"), installed_sha256=digest(b"patched")),
                           dict(source="new.dat", destination="new.dat", original_sha256=None, installed_sha256=digest(new_bytes))]))
        catalog = root / "catalog.json"
        catalog.write_text(json.dumps(dict(schema_version=1, revision=1, releases=[release])), encoding="utf-8")
        private, public, signed = [root / name for name in ("private.key", "public.json", "signed.json")]
        subprocess.run([binary, "keygen", str(private), str(public), "test"], check=True)
        subprocess.run([binary, "sign-catalog", str(catalog), str(private), "test", str(signed), "3600"], check=True)
        params = dict(roots=[str(library)], signed_catalog=str(signed), trust_keys=str(public), state_dir=str(root / "state"))

        def call(method, **extra):
            request = dict(protocol_version=1, request_id="smoke", method=method, params={**params, **extra})
            output = subprocess.run([binary, "serve"], input=json.dumps(request).encode()+b"\n", capture_output=True, check=True)
            response = json.loads(output.stdout)
            assert response["request_id"] == "smoke", response
            assert response["ok"], response
            return response["result"]

        scan = call("scan")
        assert scan["catalog_trust"] == "signed", scan
        instance = scan["installs"][0]["game"]["instance_id"]
        plan = call("plan_install", instance_id=instance, release_id="test-1", archive=str(archive))
        assert not plan["blockers"], plan
        if interrupt:
            request = dict(protocol_version=1, request_id="kill-smoke", method="start_install",
                           params={**params, "plan_id": plan["plan_id"], "confirmed_game_closed": True})
            process = subprocess.Popen([binary, "serve"], stdin=subprocess.PIPE,
                                       stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            caught = None
            try:
                process.stdin.write(json.dumps(request).encode()+b"\n")
                process.stdin.flush()
                deadline = time.monotonic() + 60
                temporary_file = game / f".steam-patch-{plan['plan_id']}-1.tmp"
                while process.poll() is None and time.monotonic() < deadline:
                    try:
                        size = temporary_file.stat().st_size
                        if 0 < size < len(new_bytes):
                            process.kill()
                            process.wait(timeout=10)
                            caught = temporary_file.stat().st_size
                            break
                    except FileNotFoundError:
                        pass
                    time.sleep(0.001)
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=10)
                for stream in [process.stdin, process.stdout, process.stderr]:
                    stream.close()
            assert caught is not None and 0 < caught < len(new_bytes), "Copy interruption window not captured; inconclusive"
            recovered = call("recover", confirmed_game_closed=True)
            assert recovered[0]["state"] == "rolled_back", recovered
            assert (game / "original.dat").read_bytes() == b"original"
            assert (game / "save.dat").read_bytes() == b"user save"
            assert not (game / "new.dat").exists()
            assert not list(game.glob(".steam-patch-*.tmp"))
            print(f"PASS: actual child-process kill at {caught}/{len(new_bytes)} copy bytes; authenticated recovery and saves preserved")
            return
        assert call("start_install", plan_id=plan["plan_id"], confirmed_game_closed=True)["state"] == "committed"
        assert call("scan")["installs"][0]["patches"][0]["detection"]["status"] == "installed_managed"
        assert call("uninstall", transaction_id=plan["plan_id"], confirmed_game_closed=True)["state"] == "uninstalled"
        assert (game / "original.dat").read_bytes() == b"original"
        assert (game / "save.dat").read_bytes() == b"user save"
        assert not (game / "new.dat").exists()
        assert call("recover", confirmed_game_closed=True) == []
        print("PASS: signed catalog -> plan -> install -> managed detection -> uninstall -> recover; saves preserved")


if __name__ == "__main__":
    main(sys.argv[1], interrupt="--interrupt" in sys.argv[2:])
