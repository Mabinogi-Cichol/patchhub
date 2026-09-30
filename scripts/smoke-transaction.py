"""Exercise the production CLI trust and transaction boundary in an isolated Steam fixture."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import zipfile


def main(binary):
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
        archive = root / "patch.zip"
        with zipfile.ZipFile(archive, "w") as package:
            package.writestr("original.dat", b"patched")
            package.writestr("new.dat", b"new")
        digest = lambda data: hashlib.sha256(data).hexdigest()
        release = dict(patch_id="test", release_id="test-1", appid=42, name="Synthetic patch", version="1",
                       language="test", category="content_restore", source_page="https://example.invalid/patch",
                       support_level="managed_zip", platforms=["windows", "steamos"], verified_buildids=["100"],
                       detection=[dict(path="original.dat", sha256=digest(b"patched"))], artifact=None,
                       recipe=dict(artifact_sha256=digest(archive.read_bytes()), files=[
                           dict(source="original.dat", destination="original.dat", original_sha256=digest(b"original"), installed_sha256=digest(b"patched")),
                           dict(source="new.dat", destination="new.dat", original_sha256=None, installed_sha256=digest(b"new"))]))
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
        assert call("start_install", plan_id=plan["plan_id"], confirmed_game_closed=True)["state"] == "committed"
        assert call("scan")["installs"][0]["patches"][0]["detection"]["status"] == "installed_managed"
        assert call("uninstall", transaction_id=plan["plan_id"], confirmed_game_closed=True)["state"] == "uninstalled"
        assert (game / "original.dat").read_bytes() == b"original"
        assert (game / "save.dat").read_bytes() == b"user save"
        assert not (game / "new.dat").exists()
        assert call("recover", confirmed_game_closed=True) == []
        print("PASS: signed catalog -> plan -> install -> managed detection -> uninstall -> recover; saves preserved")


if __name__ == "__main__":
    main(sys.argv[1])
