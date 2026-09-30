"""Read-only P0 bridge. This module does not yet constitute a Decky plugin."""
import asyncio
import json
from pathlib import Path
import uuid

MAX_RESPONSE_BYTES = 8 * 1024 * 1024


class CoreBridge:
    def __init__(self, binary: Path, catalog: Path, timeout: float = 30):
        # Configuration is host-owned, never supplied through a UI request.
        self.binary = binary.resolve(strict=True)
        self.catalog = catalog.resolve(strict=True)
        self.timeout = timeout

    async def scan(self):
        return await self._call("scan", {})

    async def preview(self, instance_id: str, release_id: str):
        if not isinstance(instance_id, str) or not isinstance(release_id, str):
            raise ValueError("invalid_preview_ids")
        return await self._call("preview", {"instance_id": instance_id, "release_id": release_id})

    async def _call(self, method: str, params: dict):
        request_id = str(uuid.uuid4())
        params = {**params, "catalog": str(self.catalog), "platform": "steamos"}
        request = {"protocol_version": 1, "request_id": request_id, "method": method, "params": params}
        payload = json.dumps(request, ensure_ascii=False).encode("utf-8") + b"\n"
        if len(payload) > 64 * 1024:
            raise ValueError("request_too_large")
        process = await asyncio.create_subprocess_exec(
            str(self.binary), "serve", stdin=asyncio.subprocess.PIPE,
            stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.DEVNULL,
        )

        async def exchange():
            process.stdin.write(payload)
            await process.stdin.drain()
            process.stdin.close()
            # Read through EOF but never accumulate an unbounded response.
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
            return await asyncio.wait_for(exchange(), self.timeout)
        finally:
            if process.returncode is None:
                try:
                    process.kill()
                except ProcessLookupError:
                    pass
                await process.wait()
