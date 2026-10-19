#!/usr/bin/env python3
"""Install this project's Release ZIP through Steam's existing Decky connection.

Python 3.10+ standard library only. No direct writes to the plugin directory.
The internal Loader adapter is deliberately fail-closed; see the feature design.
"""
import argparse
import base64
import contextlib
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import secrets
import shlex
import signal
import socket
import stat
import struct
import subprocess
import sys
import time
import urllib.parse
import urllib.request
import zipfile

REPOSITORY = "Mabinogi-Cichol/steam-hentai-patch"
ASSET = "steam-patch-manager-decky.zip"
NAME = "Steam 黄油补丁助手"
LEGACY_NAME = "Steam 补丁管理"
PLUGIN_NAMES = (NAME, LEGACY_NAME)
FOLDER = "steam-patch-manager"
SETTINGS = ("pluginOrder", "frozenPlugins", "hiddenPlugins", "disabled_plugins")
TERMINAL = {"completed", "failed", "cancelled"}
MAX_ZIP = 512 * 1024 * 1024
MAX_EXPANDED = 1024 * 1024 * 1024


class InstallError(RuntimeError):
    pass


def require(condition, message):
    if not condition:
        raise InstallError(message)


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest() if hasattr(hashlib, "file_digest") else _digest(stream)


def _digest(stream):
    result = hashlib.sha256()
    for block in iter(lambda: stream.read(1024 * 1024), b""):
        result.update(block)
    return result.hexdigest()


def no_links(path):
    path = Path(os.path.abspath(path))
    for part in (path, *path.parents):
        require(not part.is_symlink(), f"拒绝符号链接：{part}")
    return path


def json_read(path):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            require(key not in result, "JSON 存在重复字段")
            result[key] = value
        return result
    return json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=unique)


def atomic_json(path, data):
    temporary = no_links(path.with_suffix(".tmp"))
    with temporary.open("w", encoding="utf-8") as stream:
        json.dump(data, stream, ensure_ascii=False)
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temporary, path)
    if sys.platform == "linux":
        fd = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)


def version_tag(value):
    require(isinstance(value, str) and re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?", value), "版本必须是明确的 vX.Y.Z Tag")
    return value


def fetch(url, output=None, limit=2 * 1024 * 1024):
    request = urllib.request.Request(url, headers={"User-Agent": "steam-patch-decky-installer", "Accept": "application/vnd.github+json" if "/api.github.com/" in url else "application/octet-stream"})
    require(urllib.parse.urlsplit(url).scheme == "https", "下载必须使用 HTTPS")
    with urllib.request.urlopen(request, timeout=30) as response:
        require(urllib.parse.urlsplit(response.url).scheme == "https", "拒绝非 HTTPS 重定向")
        target = output.open("wb") if output else None
        blocks, size = [], 0
        try:
            while True:
                block = response.read(1024 * 1024)
                if not block:
                    break
                size += len(block)
                require(size <= limit, "下载超过允许大小")
                if target:
                    target.write(block)
                else:
                    blocks.append(block)
        finally:
            if target:
                target.close()
        return b"".join(blocks)


def release_package(work, requested, download=fetch):
    suffix = "latest" if requested is None else "tags/" + urllib.parse.quote(version_tag(requested), safe="")
    release = json.loads(download(f"https://api.github.com/repos/{REPOSITORY}/releases/{suffix}"))
    tag = version_tag(release.get("tag_name"))
    require(not release.get("draft") and (requested is not None or not release.get("prerelease")), "不是正式发布版本")
    require(requested is None or requested == tag, "Release Tag 不匹配")
    assets = release.get("assets", [])
    selected = {}
    for name in (ASSET, "SHA256SUMS.txt"):
        matches = [item for item in assets if item.get("name") == name]
        require(len(matches) == 1, f"Release 缺少唯一附件：{name}")
        url = matches[0].get("browser_download_url")
        expected = f"https://github.com/{REPOSITORY}/releases/download/{tag}/{name}"
        require(url == expected, "附件不属于冻结后的仓库版本")
        selected[name] = url
    sums = download(selected["SHA256SUMS.txt"]).decode("utf-8")
    matches = []
    for line in sums.splitlines():
        match = re.fullmatch(r"([a-fA-F0-9]{64}) [ *](.+)", line)
        if match and match[2] == ASSET:
            matches.append(match[1].lower())
    require(len(matches) == 1, "校验清单缺少唯一 Decky ZIP 哈希")
    archive = work / "package.zip"
    download(selected[ASSET], archive, MAX_ZIP)
    require(digest(archive) == matches[0], "Release ZIP SHA-256 不匹配")
    package = validate_package(archive, tag[1:])
    return {"tag": tag, "sha256": matches[0], "files": package["files"], "version": package["version"]}


def validate_package(archive, expected_version=None):
    require(archive.stat().st_size <= MAX_ZIP, "ZIP 过大")
    with zipfile.ZipFile(archive) as bundle:
        entries = bundle.infolist()
        require(0 < len(entries) <= 10000, "ZIP 文件数量异常")
        files, infos, total, seen = {}, {}, 0, set()
        for item in entries:
            raw = item.orig_filename
            path = PurePosixPath(raw)
            require(raw and "\\" not in raw and "\x00" not in raw and ":" not in raw and not path.is_absolute()
                    and all(p not in ("", ".", "..") for p in raw.rstrip("/").split("/"))
                    and path.parts[0] == FOLDER, "ZIP 路径越界或插件目录错误")
            require(raw.casefold().rstrip("/") not in seen, "ZIP 存在重复路径")
            seen.add(raw.casefold().rstrip("/"))
            mode = item.external_attr >> 16
            require(stat.S_IFMT(mode) in (0, stat.S_IFREG, stat.S_IFDIR) and not mode & 0o7000 and not item.flag_bits & 1, "ZIP 含链接、特殊权限、特殊文件或加密成员")
            if item.is_dir():
                continue
            require(len(path.parts) > 1 and not stat.S_ISDIR(mode), "ZIP 文件类型异常")
            total += item.file_size
            require(total <= MAX_EXPANDED and item.file_size <= MAX_ZIP, "ZIP 解压大小超限")
            relative = "/".join(path.parts[1:])
            with bundle.open(item) as stream:
                sha = _digest(stream)
            files[relative] = {"sha256": sha, "size": item.file_size, "executable": bool(mode & 0o111)}
            infos[relative] = item
        for name in files:
            require(not any(str(parent) in files for parent in PurePosixPath(name).parents if str(parent) != "."), "ZIP 文件与目录冲突")

        def read(name):
            require(name in infos, f"ZIP 缺少 {name}")
            require(infos[name].file_size < 2 * 1024 * 1024, "元数据过大")
            return json.loads(bundle.read(infos[name]))

        plugin, package, runtime = read("plugin.json"), read("package.json"), read("bin/runtime.json")
        require(plugin.get("name") in PLUGIN_NAMES and plugin.get("api_version") == 1 and plugin.get("flags") == [], "插件身份/API/权限不匹配")
        version = package.get("version")
        version_tag("v" + str(version))
        require(package.get("name") == "steam-patch-decky" and package.get("type") == "module" and not package.get("remote_binary"), "插件 package.json 不匹配")
        require(expected_version is None or version == expected_version, "包内版本与 Release Tag 不一致")
        required = {"main.py", "dist/index.js", "backend/bridge.py", "backend/tasks.py", "backend/protocol.py", "backend/v5.schema.json", "bin/catalog.json", "bin/signed-catalog.json", "bin/trust-keys.json", "LICENSE", "licenses/sfextract-MIT.txt", "dependencies.cdx.json"}
        require(required <= files.keys(), "插件运行文件不完整")
        require(runtime.get("protocol_version") == 5 and runtime.get("worker_protocol") == 1, "不支持的 runtime 协议")
        binaries = runtime.get("files", [])
        require(isinstance(binaries, list) and len(binaries) == 2 and {entry.get("path") for entry in binaries} == {"bin/steam-patch", "bin/sf-worker"}, "runtime 清单不完整")
        for entry in binaries:
            name = entry["path"]
            require(name in files and files[name]["size"] == entry.get("size") and files[name]["sha256"] == entry.get("sha256") and files[name]["executable"], "runtime 哈希/大小/权限不匹配")
            with bundle.open(infos[name]) as stream:
                header = stream.read(20)
            require(len(header) == 20 and header[:7] == b"\x7fELF\x02\x01\x01" and struct.unpack("<H", header[18:20])[0] == 62, "runtime 必须是 Linux x86-64 ELF")
        return {"name": plugin["name"], "version": version, "files": files}


def tree_manifest(root):
    no_links(root)
    require(root.is_dir(), "插件目录不存在")
    result = {}
    for path in sorted(root.rglob("*")):
        no_links(path)
        mode = path.stat().st_mode
        require(stat.S_ISDIR(mode) or stat.S_ISREG(mode), "插件含特殊文件")
        if path.is_file():
            require(path.stat().st_nlink == 1, "插件含硬链接")
            result[path.relative_to(root).as_posix()] = {"sha256": digest(path), "size": path.stat().st_size, "executable": bool(mode & 0o111)}
    return result


def verify_files(root, expected):
    actual = tree_manifest(root)
    # Python may create bytecode while Loader imports the verified sources.
    actual = {key: value for key, value in actual.items() if "__pycache__" not in PurePosixPath(key).parts}
    expected = {key: value for key, value in expected.items() if "__pycache__" not in PurePosixPath(key).parts}
    require(actual.keys() == expected.keys(), "正式插件文件清单不一致")
    for name, value in expected.items():
        require(actual[name]["size"] == value["size"] and actual[name]["sha256"] == value["sha256"], f"正式文件校验失败：{name}")
        require(not value["executable"] or actual[name]["executable"], f"正式文件缺少执行权限：{name}")


def backup_plugin(root, output):
    manifest = tree_manifest(root)
    require(json_read(root / "plugin.json").get("name") in PLUGIN_NAMES, "旧插件身份不匹配")
    version = json_read(root / "package.json")["version"]
    with zipfile.ZipFile(output, "w", zipfile.ZIP_DEFLATED) as bundle:
        for name in manifest:
            if "__pycache__" in PurePosixPath(name).parts:
                continue
            source = root / name
            info = zipfile.ZipInfo(FOLDER + "/" + name)
            info.create_system = 3
            info.external_attr = (stat.S_IFREG | stat.S_IMODE(source.stat().st_mode)) << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            bundle.writestr(info, source.read_bytes())
    require(tree_manifest(root) == manifest, "备份时旧插件发生变化")
    validated = validate_package(output, version)
    with output.open("rb") as stream:
        os.fsync(stream.fileno())
    return {"name": validated["name"], "version": version, "files": validated["files"], "sha256": digest(output)}


def cleanup(work):
    """Only remove the installer's fixed, private workspace; never follow links."""
    no_links(work)
    require(work.name == "steam-patch-decky-install", "清理目录错误")
    if not work.exists():
        return
    require(work.is_dir(), "清理目标不是目录")
    for path in work.rglob("*"):
        no_links(path)
        require(path.is_file() and path.stat().st_nlink == 1, "暂存目录有非预期成员，拒绝清理")
        require(path.name in {"journal.json", "journal.tmp", "package.zip", "rollback.zip"} and path.parent == work, "暂存目录有非预期文件")
    for attempt in range(3):
        try:
            for path in work.iterdir():
                no_links(path).unlink()
            work.rmdir()
            require(not work.exists(), "清理后目录仍然存在")
            return
        except OSError:
            if attempt == 2:
                raise
            time.sleep(.2)


class CDP:
    """Small RFC 6455 client for the local Steam DevTools endpoint, not /ws."""
    def __init__(self, port=8080, timeout=15):
        self.port, self.timeout, self.sock, self.sequence = port, timeout, None, 0

    def close(self):
        if self.sock:
            self.sock.close()
            self.sock = None

    def connect(self):
        self.close()
        # Do not inherit HTTP_PROXY for a privileged local debugging endpoint.
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        with opener.open(f"http://127.0.0.1:{self.port}/json/list", timeout=self.timeout) as response:
            tabs = json.loads(response.read(2 * 1024 * 1024))
        tabs = [tab for tab in tabs if tab.get("title") == "SharedJSContext"]
        require(len(tabs) == 1, "未找到唯一 Steam SharedJSContext；请启动 Steam 并启用 CEF 调试")
        url = urllib.parse.urlsplit(tabs[0]["webSocketDebuggerUrl"])
        require(url.scheme == "ws" and url.hostname in ("localhost", "127.0.0.1", "::1") and not url.username and not url.fragment, "CEF WebSocket 地址异常")
        self.sock = socket.create_connection(("127.0.0.1", self.port), self.timeout)
        nonce = base64.b64encode(os.urandom(16)).decode()
        target = url.path + (("?" + url.query) if url.query else "")
        require("\r" not in target and "\n" not in target, "CEF 路径异常")
        self.sock.sendall((f"GET {target} HTTP/1.1\r\nHost: 127.0.0.1:{self.port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {nonce}\r\nSec-WebSocket-Version: 13\r\n\r\n").encode())
        header = b""
        while not header.endswith(b"\r\n\r\n"):
            header += self._read(1)
            require(len(header) < 16384, "CEF 握手响应过大")
        expected = base64.b64encode(hashlib.sha1((nonce + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest()).decode()
        require(header.startswith(b"HTTP/1.1 101 ") and f"sec-websocket-accept: {expected}".lower() in header.decode().lower(), "CEF WebSocket 握手失败")

    def _read(self, size):
        result = bytearray()
        while len(result) < size:
            block = self.sock.recv(size - len(result))
            require(block, "CEF 连接已断开")
            result.extend(block)
        return bytes(result)

    def _send(self, data, opcode=1):
        mask = os.urandom(4)
        length = len(data)
        header = bytes([0x80 | opcode, 0x80 | length]) if length < 126 else bytes([0x80 | opcode, 0xfe]) + struct.pack("!H", length) if length < 65536 else bytes([0x80 | opcode, 0xff]) + struct.pack("!Q", length)
        self.sock.sendall(header + mask + bytes(value ^ mask[i % 4] for i, value in enumerate(data)))

    def _message(self):
        chunks, total = [], 0
        while True:
            first, second = self._read(2)
            opcode = first & 15
            length = second & 127
            require(not second & 128 and not first & 0x70, "CEF 帧格式异常")
            if length == 126:
                length = struct.unpack("!H", self._read(2))[0]
            elif length == 127:
                length = struct.unpack("!Q", self._read(8))[0]
            total += length
            require(total <= 8 * 1024 * 1024, "CEF 响应过大")
            data = self._read(length)
            if opcode == 8:
                raise InstallError("CEF 连接已关闭")
            if opcode == 9:
                self._send(data, 10)
                continue
            if opcode == 10:
                continue
            require(opcode in (0, 1), "CEF 帧类型异常")
            chunks.append(data)
            if first & 128:
                return json.loads(b"".join(chunks))

    def evaluate(self, expression):
        if self.sock is None:
            self.connect()
        self.sequence += 1
        self._send(json.dumps({"id": self.sequence, "method": "Runtime.evaluate", "params": {"expression": expression, "returnByValue": True, "awaitPromise": True}}).encode())
        deadline = time.monotonic() + self.timeout
        while time.monotonic() < deadline:
            message = self._message()
            if message.get("id") != self.sequence:
                continue
            require("error" not in message, "CEF 调用失败")
            result = message["result"]
            if "exceptionDetails" in result:
                details = result["exceptionDetails"]
                reason = details.get("exception", {}).get("description", details.get("text", "未知错误")).splitlines()[0][:200]
                raise InstallError("Steam/Loader 接口调用失败：" + reason)
            return result.get("result", {}).get("value")
        raise InstallError("CEF 调用超时")


# No token access and no Loader WebSocket creation. Every write has a durable
# Python journal before launch and a retained JS promise until it settles.
BRIDGE = r"""async (op, a) => {
 const b = window.DeckyBackend, l = window.DeckyPluginLoader;
 const names = ['Steam 黄油补丁助手', 'Steam 补丁管理'];
 const currentName = l?.plugins?.find(p => names.includes(p.name))?.name || names[0];
 const name = a.name || currentName;
 const fail = m => { throw new Error(m); };
 if (!b || !l || !(b.runningCalls instanceof Map) || !(b.eventListeners instanceof Map) ||
     typeof b.call !== 'function' || typeof b.addEventListener !== 'function' ||
     typeof b.removeEventListener !== 'function' || !Array.isArray(l.plugins) || b.ws?.readyState !== 1)
   fail('unsupported_loader_adapter');
 const mutations = new Set(['utilities/install_plugin','utilities/install_plugins',
   'utilities/confirm_plugin_install','utilities/uninstall_plugin','loader/reload_plugin',
   'utilities/enable_plugin','utilities/disable_plugin','utilities/settings/set']);
 const busy = () => [...b.runningCalls.values()].some(r => mutations.has(r.message?.route));
 const jobs = window.__steamPatchTerminalJobs ||= Object.create(null);
 const reads=new Set(['list_tasks','get_capabilities','get_settings','list_sources','list_transactions','scan_status','task_status']);
 const pluginWrite = (route,args) => route==='loader/call_plugin_method' && names.includes(args[0]) && !reads.has(args[1]);
 if (op === 'guard') return window.__steamPatchTerminalGuard?.id===a.id;
 if (op === 'acquire') {
   const old=window.__steamPatchTerminalGuard;
   if (old) { if (old.id!==a.id) fail('another_terminal_guard'); return true; }
   if (busy() || [...b.runningCalls.values()].some(r=>pluginWrite(r.message?.route,r.message?.args||[]))) fail('pending_plugin_write');
   const previous=b.call;
   const guard=function(route,...args) {
     if (pluginWrite(route,args)) return Promise.reject(new Error('plugin_update_in_progress'));
     return previous.call(this,route,...args);
   };
   window.__steamPatchTerminalGuard={id:a.id,previous,guard}; b.call=guard; return true;
 }
 if (op === 'release') {
   const guard=window.__steamPatchTerminalGuard;
   if (!guard) { if (!Object.keys(jobs).length) delete window.__steamPatchTerminalJobs; return true; }
   if (guard.id!==a.id || busy() || b.call!==guard.guard) fail('cannot_release_terminal_guard');
   b.call=guard.previous; delete window.__steamPatchTerminalGuard;
   if (!Object.keys(jobs).length) delete window.__steamPatchTerminalJobs;
   return true;
 }
 if (op === 'status') {
   const j = jobs[a.id];
   return {known: !!j, done: j?.done ?? false, ok: j?.ok ?? false, busy: busy(), error: j?.error ?? null};
 }
 if (op === 'forget') { if (busy()) fail('loader_busy'); delete jobs[a.id]; return true; }
 if (op === 'probe') {
   if (busy()) fail('loader_busy');
   const plugins = await b.call('loader/get_plugins');
   if (!Array.isArray(plugins)) fail('invalid_plugin_list');
   const matches = plugins.filter(p => names.includes(p.name));
   if (matches.length > 1) fail('duplicate_plugin');
   const settings = {};
   for (const k of ['pluginOrder','frozenPlugins','hiddenPlugins','disabled_plugins']) {
     settings[k] = await b.call('utilities/settings/get', k, []);
     if (!Array.isArray(settings[k]) || settings[k].some(x => typeof x !== 'string')) fail('invalid_settings');
   }
   const version=await b.call('updater/get_version_info');
   if (!['v3.2.9','3.2.9'].includes(version?.current)) fail('unsupported_loader_version');
   return {plugins: matches, settings, instances: l.plugins.filter(p => names.includes(p.name)).map(p => ({version:p.version, valid:!!p.titleView})), user:await b.call('utilities/get_user_info')};
 }
 if (op === 'tasks') {
   const tasks = await b.call('loader/call_plugin_method',name,'list_tasks');
   if (!Array.isArray(tasks) || tasks.some(t => !['completed','failed','cancelled'].includes(t.state))) fail('active_or_unknown_tasks');
   return true;
 }
 if (op === 'runtime') {
   const plugins = (await b.call('loader/get_plugins')).filter(p => names.includes(p.name));
   const front = l.plugins.filter(p => names.includes(p.name));
   if (!a.present) { if (plugins.length || front.length) fail('plugin_still_present'); return true; }
   if (plugins.length !== 1 || plugins[0].name !== name || plugins[0].version !== a.version || !!plugins[0].disabled !== a.disabled) fail('backend_version_or_state');
   if (a.disabled) { if (front.length) fail('disabled_frontend_present'); return true; }
   if (front.length !== 1 || front[0].name !== name || front[0].version !== a.version || !front[0].titleView) fail('frontend_version_or_instance');
   const tasks = await b.call('loader/call_plugin_method',name,'list_tasks');
   if (!Array.isArray(tasks) || tasks.some(t => !['completed','failed','cancelled'].includes(t.state))) fail('active_or_unknown_tasks');
   const caps = await b.call('loader/call_plugin_method',name,'get_capabilities');
   if (!caps || caps.protocol_version!==5 || !Array.isArray(caps.methods)) fail('runtime_rpc_failed');
   const response = await fetch('http://127.0.0.1:1337/plugins/' + encodeURIComponent(name) + '/dist/index.js');
   if (!response.ok) fail('frontend_http_failed');
   const hash = [...new Uint8Array(await crypto.subtle.digest('SHA-256', await response.arrayBuffer()))].map(x=>x.toString(16).padStart(2,'0')).join('');
   if (hash !== a.bundle) fail('frontend_http_hash');
   return true;
 }
 if (op !== 'launch' || jobs[a.id] || busy()) fail('invalid_or_busy_operation');
 const j = jobs[a.id] = {done:false, ok:false};
 j.promise = (async () => {
   if (a.kind === 'install') {
     const installed=(await b.call('loader/get_plugins')).find(p=>names.includes(p.name));
     // Suppress only this exact terminal-confirmed prompt. Forward every other
     // event to the existing Loader UI, and restore listeners in finally.
     const event = 'loader/add_plugin_install_prompt';
     const original = [...(b.eventListeners.get(event) || [])];
     if (original.length !== 1 || !original[0].name.includes('addPluginInstallPrompt')) fail('unsupported_prompt_handler');
     if ([...b.runningCalls.values()].some(r=>pluginWrite(r.message?.route,r.message?.args||[]))) fail('pending_plugin_write');
     const previousCall=b.call;
     const guard=function(route,...args) {
       if (pluginWrite(route,args)) return Promise.reject(new Error('plugin_update_in_progress'));
       return previousCall.call(this,route,...args);
     };
     b.call=guard;
     let requestId, timer, resolvePrompt;
     const expected = args => args[0]===name && args[1]===a.version && args[3]===a.hash && args[4]===a.installType;
     const prompt = new Promise((resolve,reject) => {
       resolvePrompt = resolve;
       timer = setTimeout(()=>reject(new Error('install_prompt_timeout')),10000);
     });
     // Attach a rejection handler immediately; request RPC may itself be slow.
     prompt.catch(()=>{});
     const wrapper = (...args) => {
       if (expected(args) && typeof args[2]==='string' && !requestId) { requestId=args[2]; resolvePrompt(requestId); }
       else for (const f of original) f(...args);
     };
     b.removeEventListener(event,original[0]); b.addEventListener(event,wrapper);
     try {
       await b.call('utilities/install_plugin',a.artifact,name,a.version,a.hash,a.installType);
       await prompt;
       if (a.checkTasks) await b.call('loader/call_plugin_method',installed?.name || currentName,'list_tasks').then(tasks=>{
         if (!Array.isArray(tasks)||tasks.some(t=>!['completed','failed','cancelled'].includes(t.state))) fail('active_or_unknown_tasks');
       });
       // Loader finds replacements by metadata name, not ZIP directory.
       // Explicitly unload the old identity before extracting the renamed ZIP.
       if (installed && installed.name!==name) await b.call('utilities/uninstall_plugin',installed.name);
       const id=requestId; requestId=null;
       await b.call('utilities/confirm_plugin_install',id);
     } finally {
       clearTimeout(timer);
       try { if (requestId) await b.call('utilities/cancel_plugin_install',requestId); }
       finally {
         b.removeEventListener(event,wrapper); b.addEventListener(event,original[0]);
         if (b.call===guard) b.call=previousCall;
       }
     }
   } else if (a.kind === 'uninstall') {
     await b.call('utilities/uninstall_plugin',name);
   } else if (a.kind === 'settings') {
     for (const k of ['pluginOrder','frozenPlugins','hiddenPlugins','disabled_plugins']) {
       const current=await b.call('utilities/settings/get',k,[]), old=a.settings[k];
       if (!Array.isArray(current)||!Array.isArray(old)) fail('invalid_settings');
       const next=current.filter(x=>!names.includes(x)), index=old.indexOf(name);
       if (index>=0) {
         // Preserve other plugins' current order; restore only our membership
         // and place it next to a surviving original neighbour.
         let at=next.length;
         const after=old.slice(index+1).find(x=>next.includes(x));
         const before=old.slice(0,index).reverse().find(x=>next.includes(x));
         if (after!==undefined) at=next.indexOf(after);
         else if (before!==undefined) at=next.indexOf(before)+1;
         next.splice(at,0,name);
       }
       if (JSON.stringify(next)!==JSON.stringify(current)) await b.call('utilities/settings/set',k,next);
     }
     const plugin=(await b.call('loader/get_plugins')).find(p=>p.name===name);
     const disabled=a.settings.disabled_plugins.includes(name);
     if (plugin && !!plugin.disabled!==disabled)
       await b.call(disabled?'utilities/disable_plugin':'utilities/enable_plugin',name);
     for (const k of ['pluginOrder','frozenPlugins','hiddenPlugins','disabled_plugins']) {
       const current=await b.call('utilities/settings/get',k,[]);
       if (current.includes(name)!==a.settings[k].includes(name)) fail('settings_verification_failed');
     }
   } else fail('unknown_mutation');
 })().then(()=>{j.ok=true;},e=>{j.error=String(e?.message||e);}).finally(()=>{j.done=true;});
 return true;
}"""


def loader_identity():
    result = subprocess.run(["systemctl", "show", "plugin_loader", "--property=MainPID,ActiveState"], check=True, capture_output=True, text=True, timeout=10)
    values = dict(line.split("=", 1) for line in result.stdout.splitlines() if "=" in line)
    require(values.get("ActiveState") == "active" and values.get("MainPID", "0").isdigit() and int(values["MainPID"]) > 0, "Decky Loader 服务未运行")
    pid = values["MainPID"]
    start = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[19]
    boot = Path("/proc/sys/kernel/random/boot_id").read_text().strip()
    return f"{boot}:{pid}:{start}"


def loader_root():
    """Recognize the packaged systemd layout; reject unparsed configurations."""
    result = subprocess.run(["systemctl", "show", "plugin_loader", "--property=Environment,EnvironmentFiles,ExecStart"], check=True, capture_output=True, text=True, timeout=10)
    values = dict(line.split("=", 1) for line in result.stdout.splitlines() if "=" in line)
    require(not values.get("EnvironmentFiles"), "当前 Loader 使用额外环境文件，无法确认插件根目录")
    environment = dict(item.split("=", 1) for item in shlex.split(values.get("Environment", "")) if "=" in item)
    root = environment.get("PRIVILEGED_PATH") or environment.get("UNPRIVILEGED_PATH")
    if not root and environment.get("PLUGIN_PATH"):
        root = str(Path(environment["PLUGIN_PATH"]).parent)
    if not root:
        match = re.search(r"(?:^|[ {])path=([^ ;]+)", values.get("ExecStart", ""))
        require(match is not None and Path(match[1]).name == "PluginLoader" and Path(match[1]).parent.name == "service", "无法确认 Loader 的标准安装根目录")
        root = str(Path(match[1]).parent.parent)
    require(Path(root).is_absolute() and "%" not in root and "$" not in root, "Loader 路径配置不受支持")
    return no_links(Path(root))


class Loader:
    def __init__(self, port=8080, timeout=90):
        self.cdp, self.timeout = CDP(port), timeout

    def call(self, op, **args):
        return self.cdp.evaluate(f"({BRIDGE})({json.dumps(op)}, {json.dumps(args, ensure_ascii=True)})")

    def identity(self):
        return loader_identity()

    def root(self):
        return loader_root()

    def settle(self, journal):
        """A missing context cannot prove completion in the same Loader process."""
        operation = journal.get("pending")
        if not operation:
            return
        deadline = time.monotonic() + self.timeout
        while True:
            try:
                status = self.call("status", id=operation["id"])
                if not status["busy"]:
                    if status["known"] and status["done"]:
                        return status
                    if not status["known"] and self.identity() != operation["loader"]:
                        return {"ok": False, "done": True, "known": False}
            except (OSError, InstallError):
                self.cdp.close()
            require(time.monotonic() < deadline, "无法确认 Loader 已停止写入；保留必要恢复材料，下次运行自动继续")
            time.sleep(.5)

    def mutate(self, state, **operation):
        state.data["pending"] = {"id": secrets.token_hex(16), "loader": self.identity()}
        state.data.setdefault("jobs", []).append(state.data["pending"]["id"])
        state.save()
        identity = state.data["pending"]["id"]
        self.call("launch", id=identity, **operation)
        status = self.settle(state.data)
        require(status and status.get("ok"), "Loader 操作失败：" + str((status or {}).get("error") or "结果未确认"))
        state.data.pop("pending", None)
        state.save()
        # Persist completion before forgetting the live promise. A crash must
        # never leave a journal pointing at an intentionally forgotten job.
        self.call("forget", id=identity)
        state.data["jobs"].remove(identity)
        state.save()

    def verify(self, work, original, present=True):
        deadline = time.monotonic() + self.timeout
        while True:
            try:
                self.call("runtime", name=original.get("name", LEGACY_NAME), present=present, version=original.get("version"), disabled=original.get("disabled", False), bundle=original.get("files", {}).get("dist/index.js", {}).get("sha256"))
                return
            except (InstallError, OSError):
                self.cdp.close()
                require(time.monotonic() < deadline, "重载后版本、插件实例、HTTP 或 RPC 验证未通过")
                time.sleep(.5)


class State:
    def __init__(self, work, data):
        self.work, self.data = work, data

    def save(self):
        atomic_json(self.work / "journal.json", self.data)


class Installer:
    def __init__(self, work, plugin, loader):
        self.work, self.plugin, self.loader = work, plugin, loader

    def restore_settings(self, state, first_install_success=False):
        target = state.data["target"] if state.data["phase"] == "installing" else (state.data["original"] or {})
        name = target.get("name", LEGACY_NAME)
        settings = {key: [name if value in PLUGIN_NAMES else value for value in values] for key, values in state.data["settings"].items()}
        if first_install_success and name not in settings["pluginOrder"]:
            settings["pluginOrder"].append(name)
        self.loader.mutate(state, kind="settings", name=name, settings=settings)

    def release_guard(self, state):
        if state.data.get("guard_id"):
            self.loader.call("release", id=state.data["guard_id"])

    def finish(self, state):
        status = {"prepared": "正式插件未变", "complete": "新版已验证", "restored": "安装前状态已恢复"}[state.data["phase"]]
        try:
            self.release_guard(state)
            cleanup(self.work)
        except (OSError, InstallError) as error:
            raise InstallError(f"{status}，临时材料或界面保护清理尚未完成；下次运行自动重试（{error}）") from error

    def recover(self):
        journal = self.work / "journal.json"
        if not journal.exists():
            cleanup(self.work)
            return
        state = State(self.work, json_read(journal))
        require(state.data.get("schema") == 1 and state.data.get("plugin") == str(self.plugin), "恢复记录与目标目录不匹配")
        phase = state.data["phase"]
        if phase in ("prepared", "complete", "restored"):
            self.finish(state)
            return
        require(phase in ("installing", "restoring"), "未知恢复阶段")
        require(self.loader.root() / "plugins" / FOLDER == self.plugin, "Loader 插件根目录已变化，保留恢复材料")
        self.loader.settle(state.data)
        state.data.pop("pending", None)
        state.save()
        for identity in state.data.get("jobs", []):
            self.loader.call("forget", id=identity)
        state.data["jobs"] = []
        state.save()
        # The target ZIP is no longer in use. Recovery needs only its journal
        # and the single verified old-version ZIP.
        target = no_links(self.work / "package.zip")
        if target.exists():
            target.unlink()
        original = state.data["original"]
        state.data["phase"] = "restoring"
        state.save()
        probe = self.loader.call("probe")
        # A retained transaction guard proves the already-idle plugin could
        # not accept new writes through Steam during this update. In that case
        # a broken new backend RPC must not prevent restoring the old version.
        guarded = state.data.get("guard_id") and self.loader.call("guard", id=state.data["guard_id"])
        if not guarded and probe["plugins"] and not probe["plugins"][0].get("disabled"):
            self.loader.call("tasks")
        if original:
            archive = no_links(self.work / "rollback.zip")
            require(digest(archive) == original["sha256"], "唯一回滚包哈希异常，保留恢复材料")
            package = validate_package(archive, original["version"])
            require(package["files"] == original["files"], "回滚包清单异常")
            self.loader.mutate(state, kind="install", name=package["name"], artifact=archive.as_uri(), version=original["version"], hash=original["sha256"], installType=1, checkTasks=False)
            self.restore_settings(state)
            verify_files(self.plugin, original["files"])
            self.loader.verify(self.work, original)
        else:
            if self.plugin.exists() or probe["plugins"]:
                # Do not uninstall an unrelated directory or plugin.
                require(self.plugin.is_dir() and json_read(self.plugin / "plugin.json").get("name") in PLUGIN_NAMES, "首次安装失败后的目录身份无法确认")
                self.loader.mutate(state, kind="uninstall")
            self.restore_settings(state)
            require(not self.plugin.exists(), "首次安装的文件尚未移除")
            self.loader.verify(self.work, {}, present=False)
        state.data["phase"] = "restored"
        state.save()
        self.finish(state)

    def install(self, package, yes=False, confirm=None):
        probe = self.loader.call("probe")
        require(Path(probe["user"]["path"]).resolve() == Path.home().resolve(), "Steam 用户与终端用户不同")
        require(len(probe["plugins"]) == int(self.plugin.exists()), "Loader 与插件目录状态不一致")
        disabled = bool(probe["plugins"] and probe["plugins"][0].get("disabled"))
        require(not disabled and not any(name in probe["settings"]["disabled_plugins"] for name in PLUGIN_NAMES), "插件已禁用，无法确认运行中任务；请先在 Decky 启用后再更新")
        if self.plugin.exists():
            self.loader.call("tasks")
            require(len(probe["instances"]) == 1 and probe["instances"][0].get("valid"), "当前插件前端实例异常")
        old_version = probe["plugins"][0].get("version") if probe["plugins"] else "未安装"
        print(f"Steam 黄油补丁助手：{old_version} → {package['version']}")
        if not yes:
            prompt = "通过 Decky 安装此版本？[y/N] "
            if confirm is not None:
                answer = confirm(prompt)
            elif sys.stdin.isatty():
                answer = input(prompt)
            else:
                try:
                    with open("/dev/tty", "r+") as terminal:
                        terminal.write(prompt)
                        terminal.flush()
                        answer = terminal.readline()
                except OSError as error:
                    raise InstallError("非交互终端需要显式传入 --yes") from error
            require(answer.strip().lower() in ("y", "yes"), "已取消")
        original = backup_plugin(self.plugin, self.work / "rollback.zip") if self.plugin.exists() else None
        if original:
            require(original["version"] == old_version, "运行版本与旧包版本不一致")
            original["disabled"] = disabled
        state = State(self.work, {"schema": 1, "plugin": str(self.plugin), "phase": "prepared", "original": original, "settings": probe["settings"], "target": package, "guard_id": secrets.token_hex(16)})
        try:
            state.save()
            self.loader.call("acquire", id=state.data["guard_id"])
            if original:
                self.loader.call("tasks")
            state.data["phase"] = "installing"
            state.save()
            self.loader.mutate(state, kind="install", name=package["name"], artifact=(self.work / "package.zip").as_uri(), version=package["version"], hash=package["sha256"], installType=1 if original else 0, checkTasks=bool(original))
            self.restore_settings(state, first_install_success=not original)
            verify_files(self.plugin, package["files"])
            self.loader.verify(self.work, package)
            with recovery_signals():
                state.data["phase"] = "complete"
                state.save()
                self.finish(state)
        except BaseException as error:
            # A second Ctrl-C must not interrupt normal rollback. SIGKILL/power
            # loss are handled on the next invocation using the same journal.
            with recovery_signals():
                self.recover()
            if state.data["phase"] == "complete":
                return  # Verified installation; the automatic cleanup retry passed.
            raise InstallError(f"安装未完成（{error or '已取消'}）；已恢复安装前状态并清理本次材料") from error


@contextlib.contextmanager
def recovery_signals():
    previous = {sig: signal.signal(sig, signal.SIG_IGN) for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP) if hasattr(signal, sig.name)} if sys.platform == "linux" else {}
    try:
        yield
    finally:
        for sig, handler in previous.items():
            signal.signal(sig, handler)


@contextlib.contextmanager
def workspace_lock(cache):
    """Lock an existing cache directory inode, leaving no separate lock file."""
    import fcntl
    no_links(cache)
    cache.mkdir(mode=0o700, parents=True, exist_ok=True)
    fd = os.open(cache, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise InstallError("另一个终端安装器正在运行") from error
        yield
    finally:
        os.close(fd)


def main(argv=None):
    parser = argparse.ArgumentParser(description="在 Steam Deck 上通过 Decky 安装/更新 Steam 黄油补丁助手（Python 3.10+，Steam/Decky 已运行）")
    parser.add_argument("--version", type=version_tag, help="指定 Release Tag；省略时使用最新正式版")
    parser.add_argument("--yes", action="store_true", help="显式同意无人交互安装")
    parser.add_argument("--check", action="store_true", help="只下载并校验，不操作 Loader")
    parser.add_argument("--recover", action="store_true", help="只续做上次恢复和清理，不开始新安装")
    parser.add_argument("--cef-port", type=int, default=8080, help="本机 Steam CEF 调试端口（默认 8080）")
    parser.add_argument("--homebrew", type=Path, default=Path.home() / "homebrew", help="Decky homebrew 目录；必须与 Loader 实际目录一致")
    options = parser.parse_args(argv)
    require(sys.version_info >= (3, 10) and sys.platform == "linux", "请使用 Steam Deck 上的 Python 3.10+ 运行")
    require(os.geteuid() != 0, "请以 Steam 用户运行，不要使用 sudo")
    require(not options.recover or (not options.version and not options.check and not options.yes), "--recover 不能与版本、安装或检查参数组合")
    require(1 <= options.cef_port <= 65535, "CEF 端口无效")
    cache = no_links(Path.home() / ".cache")
    work = cache / "steam-patch-decky-install"
    plugin = no_links(options.homebrew.absolute() / "plugins" / FOLDER)
    loader = Loader(options.cef_port)
    installer = Installer(work, plugin, loader)
    for sig in (signal.SIGTERM, signal.SIGHUP):
        signal.signal(sig, lambda signum, frame: (_ for _ in ()).throw(KeyboardInterrupt()))
    with workspace_lock(cache):
        if work.exists():
            no_links(work)
            require(work.is_dir() and work.stat().st_uid == os.geteuid() and stat.S_IMODE(work.stat().st_mode) == 0o700, "恢复目录所有者/权限异常")
            print("先续做上次恢复与清理…")
            with recovery_signals():
                installer.recover()
        if options.recover:
            print("恢复与清理完成。")
            return
        work.mkdir(mode=0o700)
        cache_fd = os.open(cache, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(cache_fd)
        finally:
            os.close(cache_fd)
        try:
            package = release_package(work, options.version)
            if options.check:
                with recovery_signals():
                    cleanup(work)
                print(f"{package['tag']} 校验通过；本次下载与暂存已清理。")
                return
            require(plugin.parent.is_dir(), "找不到 Decky 插件目录，请核对 --homebrew")
            require(loader_root() / "plugins" / FOLDER == plugin, "--homebrew 与 Loader 实际插件根目录不同")
            loader.identity()
            installer.install(package, yes=options.yes)
            print(f"{package['tag']} 安装及运行验证通过；本次备份、下载与恢复记录已清理。")
        finally:
            loader.cdp.close()
            # Never clean a journal that still requires recovery.
            if work.exists() and not (work / "journal.json").exists():
                with recovery_signals():
                    cleanup(work)


if __name__ == "__main__":
    try:
        main()
    except (Exception, KeyboardInterrupt) as error:
        print(f"未完成：{error or '已取消'}。", file=sys.stderr)
        sys.exit(1)
