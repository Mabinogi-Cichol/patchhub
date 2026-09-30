"""Fixed-method Decky to Rust JSON-lines bridge. UI cannot select a binary or catalog."""
import asyncio
import os
import json
from pathlib import Path
import uuid

MAX_RESPONSE_BYTES = 8 * 1024 * 1024
MAX_REQUEST_BYTES = 64 * 1024
METHODS = {"scan", "preview", "plan_install", "fetch_artifact", "start_install", "list_transactions", "uninstall", "recover"}


class CoreBridge:
    def __init__(self, binary: Path, catalog: Path, timeout: float = 300,
                 signed_catalog: Path | None = None, trust_keys: Path | None = None,
                 state_dir: Path | None = None, run_uid: int | None = None, run_gid: int | None = None, user_home: Path | None = None):
        self.binary = binary.resolve(strict=True)
        self.catalog = catalog.resolve(strict=True)
        self.signed_catalog = signed_catalog.resolve(strict=True) if signed_catalog and signed_catalog.is_file() else None
        self.trust_keys = trust_keys.resolve(strict=True) if trust_keys and trust_keys.is_file() else None
        self.state_dir = state_dir.resolve() if state_dir else None
        self.timeout = timeout
        self.process_identity = {"user": run_uid, "group": run_gid, "extra_groups": []} if run_uid is not None else {}
        self.downloads = set()
        self.process_env = os.environ.copy()
        if user_home is not None:
            home = str(user_home)
            self.process_env.update(HOME=home, USER=user_home.name, LOGNAME=user_home.name,
                                    XDG_CONFIG_HOME=str(user_home / ".config"),
                                    XDG_DATA_HOME=str(user_home / ".local/share"),
                                    XDG_CACHE_HOME=str(user_home / ".cache"))
            if run_uid is not None:
                runtime = Path("/run/user") / str(run_uid)
                if runtime.is_dir() and runtime.stat().st_uid == run_uid:
                    self.process_env["XDG_RUNTIME_DIR"] = str(runtime)
                else:
                    self.process_env.pop("XDG_RUNTIME_DIR", None)

    @staticmethod
    def _id(value: str) -> str:
        if not isinstance(value, str) or not value or len(value) > 256 or any(ord(c) < 32 for c in value):
            raise ValueError("invalid_id")
        return value

    async def scan(self):
        return await self._call("scan", {})

    async def preview(self, instance_id: str, release_id: str):
        return await self._call("preview", {"instance_id": self._id(instance_id), "release_id": self._id(release_id)})

    async def plan_install(self, instance_id: str, release_id: str, archive: Path | None = None):
        params = {"instance_id": self._id(instance_id), "release_id": self._id(release_id)}
        if archive is not None:
            params["archive"] = str(archive.resolve(strict=True))
        return await self._call("plan_install", params)

    async def fetch_artifact(self, instance_id: str, release_id: str):
        return await self._call("fetch_artifact", {"instance_id": self._id(instance_id), "release_id": self._id(release_id)})

    async def cancel_download(self):
        for process in tuple(self.downloads):
            if process.returncode is None:
                try:
                    process.kill()
                except ProcessLookupError:
                    pass
        return {"canceled": bool(self.downloads)}

    async def start_install(self, plan_id: str, confirmed_game_closed: bool = False):
        return await self._call("start_install", {"plan_id": self._id(plan_id), "confirmed_game_closed": confirmed_game_closed})

    async def list_transactions(self):
        return await self._call("list_transactions", {})

    async def uninstall(self, transaction_id: str, confirmed_game_closed: bool = False):
        return await self._call("uninstall", {"transaction_id": self._id(transaction_id), "confirmed_game_closed": confirmed_game_closed})

    async def recover(self, confirmed_game_closed: bool = False):
        return await self._call("recover", {"confirmed_game_closed": confirmed_game_closed})

    async def _call(self, method: str, params: dict):
        if method not in METHODS:
            raise ValueError("unsupported_method")
        request_id = str(uuid.uuid4())
        host_params = {"catalog": str(self.catalog), "platform": "steamos"}
        if self.signed_catalog:
            host_params["signed_catalog"] = str(self.signed_catalog)
        if self.trust_keys:
            host_params["trust_keys"] = str(self.trust_keys)
        if self.state_dir:
            host_params["state_dir"] = str(self.state_dir)
        request = {"protocol_version": 1, "request_id": request_id, "method": method,
                   "params": {**host_params, **params}}
        payload = json.dumps(request, ensure_ascii=False).encode("utf-8") + b"\n"
        if len(payload) > MAX_REQUEST_BYTES:
            raise ValueError("request_too_large")
        process = await asyncio.create_subprocess_exec(
            str(self.binary), "serve", stdin=asyncio.subprocess.PIPE,
            stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.DEVNULL, env=self.process_env, **self.process_identity,
        )

        if method == "fetch_artifact":
            self.downloads.add(process)

        async def exchange():
            process.stdin.write(payload)
            await process.stdin.drain()
            process.stdin.close()
            chunks = []
            size = 0
            while True:
                chunk = await process.stdout.read(min(65536, MAX_RESPONSE_BYTES + 1 - size))
                if not chunk:
                    break
                size += len(chunk)
                if size > MAX_RESPONSE_BYTES:
                    raise RuntimeError("response_too_large")
                chunks.append(chunk)
            await process.wait()
            if process.returncode != 0:
                raise RuntimeError("core_process_failed")
            response = json.loads(b"".join(chunks))
            if (response.get("protocol_version") != 1 or response.get("request_id") != request_id
                    or not isinstance(response.get("ok"), bool)):
                raise RuntimeError("invalid_core_response")
            if not response["ok"]:
                raise RuntimeError(response.get("error", {}).get("code", "core_error"))
            return response["result"]

        try:
            return await asyncio.wait_for(exchange(), self.timeout if method in {"scan", "preview", "list_transactions"} else None)
        finally:
            self.downloads.discard(process)
            if process.returncode is None:
                try:
                    process.kill()
                except ProcessLookupError:
                    pass
                await process.wait()
