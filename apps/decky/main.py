"""Decky entry point. Bundle bin/steam-patch and catalog files with this plugin."""
from pathlib import Path
import sys
import os

import decky

sys.path.insert(0, str(Path(__file__).parent / "backend"))
from bridge import CoreBridge


class Plugin:
    async def _main(self):
        root = Path(decky.DECKY_PLUGIN_DIR)
        catalog = root / "bin" / "catalog.json"
        state_dir = Path(decky.DECKY_USER_HOME) / ".local/share/steam-patch-manager"
        run_uid = run_gid = None
        if os.geteuid() == 0:
            import pwd
            account = pwd.getpwnam(Path(decky.DECKY_USER_HOME).name)
            if account.pw_uid == 0 or Path(account.pw_dir) != Path(decky.DECKY_USER_HOME):
                raise ValueError("unprivileged_user_required")
            run_uid, run_gid = account.pw_uid, account.pw_gid
        self.bridge = CoreBridge(
            root / "bin" / "steam-patch", catalog,
            signed_catalog=root / "bin" / "signed-catalog.json",
            trust_keys=root / "bin" / "trust-keys.json",
            state_dir=state_dir, run_uid=run_uid, run_gid=run_gid, user_home=Path(decky.DECKY_USER_HOME),
        )
        self.downloads = (Path(decky.DECKY_USER_HOME) / "Downloads").resolve()

    async def scan(self):
        return await self.bridge.scan()

    async def preview(self, instance_id: str, release_id: str):
        return await self.bridge.preview(instance_id, release_id)

    async def plan_install(self, instance_id: str, release_id: str, archive_path: str = ""):
        archive = None
        if archive_path:
            if archive_path == "~" or archive_path.startswith("~/"):
                archive = Path(decky.DECKY_USER_HOME) / archive_path[2:]
            elif archive_path.startswith("~"):
                raise ValueError("invalid_archive")
            else:
                archive = Path(archive_path)
            archive = archive.resolve(strict=True)
            if archive.suffix.lower() != ".zip" or not archive.is_file():
                raise ValueError("invalid_archive")
            if not (archive.is_relative_to(self.downloads) or archive.is_relative_to(self.bridge.state_dir / "downloads")):
                raise ValueError("archive_outside_downloads")
        return await self.bridge.plan_install(instance_id, release_id, archive)

    async def fetch_artifact(self, instance_id: str, release_id: str):
        return await self.bridge.fetch_artifact(instance_id, release_id)

    async def cancel_download(self):
        return await self.bridge.cancel_download()

    async def start_install(self, plan_id: str, confirmed_game_closed: bool = False):
        return await self.bridge.start_install(plan_id, confirmed_game_closed)

    async def list_transactions(self):
        return await self.bridge.list_transactions()

    async def uninstall(self, transaction_id: str, confirmed_game_closed: bool = False):
        return await self.bridge.uninstall(transaction_id, confirmed_game_closed)

    async def recover(self, confirmed_game_closed: bool = False):
        return await self.bridge.recover(confirmed_game_closed)
