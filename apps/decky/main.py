"""Decky entry point. Bundle bin/steam-patch and catalog files with this plugin."""
from pathlib import Path
import sys
import os
import asyncio
from threading import Event
import hashlib
import re

import decky

sys.path.insert(0, str(Path(__file__).parent / "backend"))
from bridge import CoreBridge
from acquisition import find_archive, release_metadata
from publisher import Publisher, _released, RELEASE_PATTERN
from sources import SourceRegistry


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
        self._prepare_cancel = None
        self.sources = SourceRegistry(state_dir, root / "backend/sources.json")
        self.publisher = Publisher(self.bridge, self.sources)

    async def scan(self, refresh_sources: bool = True):
        source_diagnostics = await self.sources.refresh(force=refresh_sources)
        report = await self.bridge.scan()
        report["diagnostics"].extend({"code": issue["code"], "path": issue.get("source_id", ""),
            "message": issue.get("detail", "")} for issue in source_diagnostics
            if issue["code"] not in ("source_refreshed", "source_not_modified"))
        report["sources"] = self.sources.list_sources()
        existing_transactions = await self.list_transactions()
        for row in report["installs"]:
            game = row["game"]
            # Source subscriptions own discovery. The bundled catalog remains
            # available to the core for existing receipts and CLI operations.
            row["patches"] = []
            row["match_status"] = "no_source_match"
            entries = self.publisher.matches(game)
            for entry in entries:
                patch = dict(release_id=entry["release_id"], name=f"{entry['source_id']} · 官方 ZIP · {entry['language']} · 默认游戏根目录",
                    source_page=entry["source_page"], support_level="publisher_zip", download_available=True,
                    compatibility="publisher_default", blockers=[], detection=dict(status="unknown", files=[]))
                if not _released(entry):
                    patch["blockers"].append("source_not_released")
                if game.get("availability") != "installed":
                    patch["blockers"].append("game_not_ready")
                if not game.get("buildid"):
                    patch["blockers"].append("build_not_verified")
                if any(t.get("instance_id") == game["instance_id"] and t.get("release_id") != entry["release_id"]
                       and t["state"] not in ("planned", "uninstalled", "rolled_back") for t in existing_transactions):
                    patch["blockers"].append("publisher_existing_transaction")
                local = self.publisher.bridge_for(game["instance_id"], entry["release_id"])
                if local is not None:
                    try:
                        observed = await local.scan()
                        if observed.get("catalog_error") == "catalog_expired":
                            local = await self.publisher.refresh_signature(game["instance_id"], entry["release_id"])
                            observed = await local.scan()
                        local_row = next((i for i in observed["installs"] if i["game"]["instance_id"] == game["instance_id"]), None)
                        local_patch = next((p for p in (local_row or {}).get("patches", []) if p["release_id"] == entry["release_id"]), None)
                        if observed.get("catalog_trust") != "signed":
                            patch["blockers"].append("catalog_untrusted")
                        if local_patch:
                            patch["detection"] = local_patch["detection"]
                            transactions = await local.list_transactions()
                            active = any(t["state"] not in ("planned", "uninstalled", "rolled_back") for t in transactions)
                            if active:
                                patch["blockers"].append("publisher_existing_transaction")
                            patch["blockers"].extend(b for b in local_patch["blockers"]
                                if active or b not in ("version_incompatible", "build_not_verified"))
                    except Exception:
                        patch["blockers"].append("publisher_state_unavailable")
                row["patches"].insert(0, patch)
                row["match_status"] = "matched"
        for issue in self.publisher.state_diagnostics:
            report["diagnostics"].append({"code": "publisher_state_unavailable", "path": issue.get("path", ""), "message": issue.get("code", "")})
        return report

    @staticmethod
    def _publisher_id(instance_id, release_id, core_id):
        return f"publisher:{release_id}:{hashlib.sha256(instance_id.encode()).hexdigest()[:16]}:{core_id}"

    def _dispatch_id(self, value):
        if not value.startswith("publisher:"):
            return self.bridge, value, None
        match = re.fullmatch(rf"publisher:({RELEASE_PATTERN}):([0-9a-f]{{16}}):([0-9a-f-]{{36}})", value)
        if not match:
            raise ValueError("invalid_id")
        release_id, instance_hash, core_id = match.groups()
        for instance_id, found_release, bridge in self.publisher.bridges():
            if found_release == release_id and hashlib.sha256(instance_id.encode()).hexdigest()[:16] == instance_hash:
                return bridge, core_id, (instance_id, release_id)
        raise ValueError("publisher_state_unavailable")

    def _wrap_publisher(self, value, instance_id, release_id):
        value = dict(value)
        for field in ("plan_id", "transaction_id"):
            if value.get(field):
                value[field] = self._publisher_id(instance_id, release_id, value[field])
        return value

    async def preview(self, instance_id: str, release_id: str):
        if re.fullmatch(RELEASE_PATTERN, release_id):
            bridge = self.publisher.bridge_for(instance_id, release_id)
            if bridge is None:
                raise ValueError("publisher_prepare_first")
            return await bridge.preview(instance_id, release_id)
        return await self.bridge.preview(instance_id, release_id)

    async def plan_install(self, instance_id: str, release_id: str, archive_path: str = ""):
        if re.fullmatch(RELEASE_PATTERN, release_id):
            raise ValueError("publisher_prepare_first")
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

    async def prepare_install(self, instance_id: str, release_id: str):
        """Acquire a reviewed ZIP and prepare a plan; never write game files."""
        if self._prepare_cancel is not None:
            raise RuntimeError("installer_busy")
        cancel = Event()
        self._prepare_cancel = cancel
        try:
            report = await self.scan()
            row = next((i for i in report["installs"] if i["game"]["instance_id"] == instance_id), None)
            if row is None:
                raise ValueError("instance_not_found")
            patch = next((p for p in row["patches"] if p["release_id"] == release_id), None)
            if patch is None:
                raise ValueError("release_not_found")
            if patch["blockers"]:
                raise ValueError(patch["blockers"][0])
            if patch["support_level"] == "publisher_zip":
                entry = self.publisher.match(row["game"], release_id)
                if entry is None or entry["release_id"] != release_id:
                    raise ValueError("publisher_match_changed")
                bridge, archive, plan, acquisition = await self.publisher.prepare(row["game"], entry, cancel)
                if cancel.is_set():
                    raise RuntimeError("download_cancelled")
                return {"archive": archive, "acquisition": acquisition, "mode": "publisher_root",
                    "plan": self._wrap_publisher(plan, instance_id, release_id)}
            if report.get("catalog_trust") != "signed":
                raise ValueError("catalog_untrusted")
            if patch["support_level"] != "managed_zip":
                raise ValueError("source_requires_guidance")
            digest, size, filename = release_metadata(self.bridge.signed_catalog, report["catalog_revision"], release_id)
            archive = await asyncio.to_thread(find_archive,
                [self.bridge.state_dir / "downloads", self.downloads], digest, size, filename, cancel)
            acquisition = "local"
            if cancel.is_set():
                raise RuntimeError("download_cancelled")
            if archive is None:
                if not patch.get("download_available"):
                    raise ValueError("artifact_requires_manual_download")
                result = await self.bridge.fetch_artifact(instance_id, release_id)
                archive = result["archive"]
                acquisition = "download"
            if cancel.is_set():
                raise RuntimeError("download_cancelled")
            plan = await self.plan_install(instance_id, release_id, str(archive))
            if cancel.is_set():
                raise RuntimeError("download_cancelled")
            return {"archive": str(archive), "acquisition": acquisition, "mode": "reviewed", "plan": plan}
        finally:
            self._prepare_cancel = None

    async def list_sources(self):
        return self.sources.list_sources()

    async def set_source_enabled(self, source_id: str, enabled: bool):
        if self._prepare_cancel is not None:
            raise RuntimeError("installer_busy")
        self.sources.set_enabled(source_id, enabled)
        return self.sources.list_sources()

    async def add_source(self, rule_json: str):
        if self._prepare_cancel is not None:
            raise RuntimeError("installer_busy")
        import json
        if not isinstance(rule_json, str) or len(rule_json) > 32768:
            raise ValueError("invalid_source_rule")
        self.sources.add_source(json.loads(rule_json))
        return self.sources.list_sources()

    async def cancel_download(self):
        if self._prepare_cancel is not None:
            self._prepare_cancel.set()
        return await self.bridge.cancel_download()

    async def start_install(self, plan_id: str, confirmed_game_closed: bool = False):
        bridge, value, owner = self._dispatch_id(plan_id)
        result = await bridge.start_install(value, confirmed_game_closed)
        return self._wrap_publisher(result, *owner) if owner else result

    async def list_transactions(self):
        result = await self.bridge.list_transactions()
        for instance_id, release_id, bridge in self.publisher.bridges():
            result.extend(self._wrap_publisher(tx, instance_id, release_id) for tx in await bridge.list_transactions())
        return result

    async def uninstall(self, transaction_id: str, confirmed_game_closed: bool = False):
        bridge, value, owner = self._dispatch_id(transaction_id)
        result = await bridge.uninstall(value, confirmed_game_closed)
        return self._wrap_publisher(result, *owner) if owner else result

    async def recover(self, confirmed_game_closed: bool = False):
        result = await self.bridge.recover(confirmed_game_closed)
        for instance_id, release_id, bridge in self.publisher.bridges():
            result.extend(self._wrap_publisher(tx, instance_id, release_id) for tx in await bridge.recover(confirmed_game_closed))
        return result
