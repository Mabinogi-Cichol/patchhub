"""Session-scoped LAN source management, backed by the plugin's existing RPCs."""
import asyncio
import errno
import hmac
import ipaddress
import json
from pathlib import Path
import secrets
import socket
import struct

from bridge import CoreError
from source_service import TERMINAL

PREFERRED_PORT = 47625


def local_ipv4_addresses():
    import fcntl
    addresses = {}
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as channel:
        for _, name in socket.if_nameindex():
            try:
                value = fcntl.ioctl(channel.fileno(), 0x8915,
                                    struct.pack("256s", name.encode()))
            except OSError:
                # Interfaces without IPv4 do not have a SIOCGIFADDR result.
                continue
            address = ipaddress.IPv4Address(value[20:24])
            if not address.is_loopback and not address.is_unspecified:
                physical = (Path("/sys/class/net") / name / "device").exists()
                addresses[str(address)] = physical or addresses.get(str(address), False)
    return sorted(addresses, key=lambda address: (not addresses[address], address))


class RemoteServer:
    def __init__(self, plugin, assets):
        self.plugin = plugin
        self.assets = Path(assets)
        self._runner = None
        self._url = None
        self._key = None
        self._last_pin = None
        self._lifecycle = asyncio.Lock()
        self._tasks = set()

    def status(self):
        return {"enabled": self._runner is not None, "addresses": local_ipv4_addresses(),
                "url": self._url, "access_key": self._key}

    async def start(self, address):
        from aiohttp import web
        async with self._lifecycle:
            if self._runner is not None:
                return self.status()
            if not isinstance(address, str) or address not in local_ipv4_addresses():
                raise ValueError("invalid_remote_address")
            # Catch incomplete packaging before publishing a listening address.
            for name in ("index.html", "app.js", "tabler.min.css", "LICENSE-tabler.txt", "style.css"):
                if not (self.assets / name).is_file():
                    raise FileNotFoundError(f"remote_asset_missing: {name}")
            app = web.Application(client_max_size=0)
            app.router.add_route("*", "/{path:.*}", self._handle)
            runner = web.AppRunner(app)
            await runner.setup()
            try:
                site = web.TCPSite(runner, address, PREFERRED_PORT)
                try:
                    await site.start()
                except OSError as failure:
                    if failure.errno != errno.EADDRINUSE:
                        raise
                    await site.stop()
                    await web.TCPSite(runner, address, 0).start()
            except BaseException:
                await runner.cleanup()
                raise
            port = runner.addresses[0][1]
            self._url = f"http://{address}:{port}"
            pin = secrets.randbelow(1_000_000)
            while pin == self._last_pin:
                pin = secrets.randbelow(1_000_000)
            self._last_pin = pin
            self._key = f"{pin:06d}"
            self._runner = runner
            return self.status()

    async def stop(self):
        async with self._lifecycle:
            runner = self._runner
            self._runner = self._url = self._key = None
            if runner is not None:
                await runner.cleanup()
            self._tasks.clear()
            return self.status()

    async def _handle(self, request):
        from aiohttp import web
        def error(status, code, detail=None):
            return web.json_response({"code": code, "detail": detail or code}, status=status)

        if self._url is None or request.headers.get("Host") != self._url.removeprefix("http://"):
            return error(403, "invalid_host")
        headers = {"Cache-Control": "no-store", "X-Content-Type-Options": "nosniff",
                   "Content-Security-Policy": "default-src 'self'; style-src 'self'; script-src 'self'; connect-src 'self'; img-src 'self' data:; frame-ancestors 'none'; base-uri 'none'"}
        if not request.path.startswith("/api/"):
            static = {"/": "index.html", "/index.html": "index.html", "/app.js": "app.js",
                      "/tabler.min.css": "tabler.min.css", "/style.css": "style.css",
                      "/LICENSE-tabler.txt": "LICENSE-tabler.txt"}
            if request.method not in ("GET", "HEAD") or request.path not in static:
                return error(404, "not_found")
            return web.FileResponse(self.assets / static[request.path], headers=headers)
        authorization = request.headers.get("Authorization", "")
        if not hmac.compare_digest(authorization.encode(), f"Bearer {self._key}".encode()):
            return error(401, "unauthorized")
        if request.method not in ("GET", "HEAD") and request.headers.get("Origin") != self._url:
            return error(403, "invalid_origin")
        try:
            data = {}
            if request.method in ("POST", "PUT") and request.can_read_body:
                if request.content_type != "application/json":
                    return error(400, "invalid_request")
                data = await request.json()
                if not isinstance(data, dict):
                    return error(400, "invalid_request")
            result = await self._dispatch(request.method, request.path, data)
        except (json.JSONDecodeError, UnicodeError):
            return error(400, "invalid_request")
        except CoreError as failure:
            return error(400, failure.code, failure.detail)
        except (ValueError, RuntimeError) as failure:
            code, separator, detail = str(failure).partition(": ")
            status = 404 if code in ("not_found", "task_not_found") else 409 if code in (
                "installer_busy", "plugin_update_in_progress", "source_task_running") else 400
            return error(status, code, detail if separator else None)
        except OSError as failure:
            return error(500, "storage_error", str(failure))
        return web.json_response(result, headers=headers)

    def _remember_task(self, task):
        # Retain only ids still owned by the existing bounded task registry.
        self._tasks.intersection_update(item["task_id"] for item in self.plugin.sources.task_status())
        self._tasks.add(task["task_id"])
        return task["task_id"]

    async def _import(self, code, source_id=None):
        if not isinstance(code, str):
            raise ValueError("invalid_request")
        result = await self.plugin.import_script_source(code, source_id)
        if source_id is None and result["rule"]["role"] == "publisher" and result["rule"]["enabled"]:
            try:
                task = await self.plugin.update_source(result["rule"]["id"], True)
                result = {**result, "task_id": self._remember_task(task)}
            except (ValueError, RuntimeError) as failure:
                code, _, detail = str(failure).partition(": ")
                result = {**result, "task_error": {"code": code, "detail": detail or code}}
        return result

    @staticmethod
    def _task_snapshot(task):
        return {"task_id": task["task_id"], "running": task["state"] not in TERMINAL,
                "cancel_requested": task["state"] in ("cancelling", "cancelled"),
                "progress": task["progress"], "result": task["result"],
                "error": {"code": task["error"], "detail": task["error_detail"] or task["error"]}
                    if task["error"] else None, "control_error": None}

    async def _dispatch(self, method, path, data):
        plugin = self.plugin
        parts = path.split("/")[2:]
        if parts == ["sources"]:
            if method == "GET":
                sources = await plugin.list_sources()
                return {"sources": sources, "settings_revision": plugin.sources.settings_revision}
            if method == "POST" and set(data) == {"code"}:
                return await self._import(data["code"])
        if parts == ["script-prompt"] and method == "GET":
            return await plugin.get_script_source_prompt()
        if parts == ["updates"] and method == "POST" and set(data) <= {"force"}:
            force = data.get("force", False)
            if type(force) is not bool:
                raise ValueError("invalid_request")
            sources = await plugin.list_sources()
            active = {task["key"] for task in await plugin.source_task_status()
                      if task["kind"] == "update_source" and task["state"] not in TERMINAL}
            ids = []
            for source in sources:
                if source["enabled"] and source["role"] == "publisher" and source["id"] not in active:
                    ids.append(self._remember_task(await plugin.update_source(source["id"], force)))
            return {"task_ids": ids}
        if len(parts) == 2 and parts[0] == "sources" and method == "DELETE":
            return await plugin.remove_source(parts[1], refresh=False)
        if len(parts) == 3 and parts[0] == "sources":
            source_id, operation = parts[1:]
            if operation == "script":
                if method == "GET":
                    return await plugin.get_script_source(source_id)
                if method == "PUT" and set(data) == {"code"}:
                    return await self._import(data["code"], source_id)
            if method == "PUT":
                if operation == "enabled" and set(data) == {"enabled"}:
                    return await plugin.set_source_enabled(source_id, data["enabled"], refresh=False)
                if operation == "name" and set(data) == {"name"}:
                    if not isinstance(data["name"], str):
                        raise ValueError("invalid_request")
                    return await plugin.set_source_name(source_id, data["name"], refresh=False)
                if operation == "interval" and set(data) == {"interval_hours"}:
                    return await plugin.set_source_update_interval(source_id, data["interval_hours"], refresh=False)
            if operation == "updates" and method == "POST" and set(data) <= {"force"}:
                return {"task_id": self._remember_task(await plugin.update_source(source_id, data.get("force", False)))}
        if len(parts) in (2, 3) and parts[0] == "tasks":
            task_id = parts[1]
            if task_id not in self._tasks:
                raise ValueError("task_not_found")
            if len(parts) == 2 and method == "GET":
                return self._task_snapshot(await plugin.source_task_status(task_id))
            if len(parts) == 3 and parts[2] == "cancel" and method == "POST" and not data:
                return self._task_snapshot(await plugin.cancel_source_task(task_id))
        raise ValueError("not_found")
