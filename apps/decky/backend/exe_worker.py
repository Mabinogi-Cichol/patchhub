"""Launch the fixed offline SF worker in an unprivileged Linux sandbox."""
import asyncio
import json
import os
from pathlib import Path
import signal
import sys

from acquisition import no_links
from tasks import current_task

MAX_OUTPUT = 16 * 1024 * 1024


def worker_command(tool_dir, archive, output):
    if sys.platform != "linux" or not Path("/usr/bin/bwrap").is_file():
        raise ValueError("exe_sandbox_unavailable")
    tool_dir, archive, output = map(lambda p: Path(p).absolute(), (tool_dir, archive, output))
    if (not all(no_links(p) for p in (tool_dir, archive, output)) or not archive.is_file()
            or not output.is_dir() or not tool_dir.is_dir()
            or not all((tool_dir / n).is_file() and no_links(tool_dir / n) for n in ("sf95_audit.py", "sf95_worker.py"))):
        raise ValueError("exe_worker_path_invalid")
    if output == archive.parent or output.is_relative_to(archive.parent) or archive.is_relative_to(output):
        raise ValueError("exe_worker_input_output_overlap")
    return ["/usr/bin/bwrap", "--unshare-all", "--die-with-parent", "--new-session", "--clearenv",
        "--setenv", "PATH", "/usr/bin", "--setenv", "HOME", "/tmp", "--cap-drop", "ALL",
        "--ro-bind", "/usr", "/usr", "--ro-bind", "/lib", "/lib", "--ro-bind", "/lib64", "/lib64",
        "--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp", "--dir", "/input",
        "--ro-bind", str(archive), "/input/archive.exe", "--ro-bind", str(tool_dir), "/tool",
        "--bind", str(output), "/output", "--", "/usr/bin/python3", "-I", "-B", "/tool/sf95_worker.py"]


async def _bounded(stream, limit):
    data = bytearray()
    while chunk := await stream.read(64 * 1024):
        data.extend(chunk)
        if len(data) > limit:
            raise ValueError("exe_worker_response_limit")
    return bytes(data)


async def extract(bridge, tool_dir, archive, output, profile, cancel, *, timeout=None):
    descriptor = {k: profile[k] for k in ("exe_sha256", "exe_size", "metadata_sha256", "payload")}
    if profile.get("generic"):
        descriptor.update(action="convert_behavior", behavior_sha256=profile["behavior_sha256"])
        _behavior_tools(tool_dir)
    else:
        descriptor["semantics_approved"] = profile["review"]["semantics"] is True
    return await _run_worker(bridge, tool_dir, archive, output, descriptor, cancel,
                             timeout=timeout if timeout is not None else (480 if profile.get("generic") else 180))


async def analyze(bridge, tool_dir, archive, output, exe_sha256, exe_size, cancel, *, timeout=180):
    """Run the content-bound, read-only SF9 diagnostic protocol in the same sandbox."""
    module = Path(tool_dir) / "sf9_metadata.py"
    if not no_links(module) or not module.is_file():
        raise ValueError("exe_worker_path_invalid")
    descriptor = {"action": "analyze", "exe_sha256": exe_sha256, "exe_size": exe_size}
    result = await _run_worker(bridge, tool_dir, archive, output, descriptor, cancel, timeout=timeout)
    report = result.get("report")
    package = report.get("exe") if isinstance(report, dict) else None
    if (result.get("schema_version") != 1 or result.get("action") != "analyze"
            or result.get("installable") is not False or not isinstance(report, dict)
            or not isinstance(package, dict) or report.get("installable") is not False
            or package.get("sha256") != exe_sha256 or package.get("size") != exe_size):
        raise ValueError("exe_worker_invalid_response")
    return report


def _behavior_tools(tool_dir):
    for name in ("sf9_graph.py", "sf9_lua.py", "sf9_behavior.py"):
        module = Path(tool_dir) / name
        if not no_links(module) or not module.is_file():
            raise ValueError("exe_worker_path_invalid")


async def analyze_behavior(bridge, tool_dir, archive, output, exe_sha256, exe_size, cancel, *, timeout=240):
    _behavior_tools(tool_dir)
    descriptor = {"action": "analyze_behavior", "exe_sha256": exe_sha256, "exe_size": exe_size}
    result = await _run_worker(bridge, tool_dir, archive, output, descriptor, cancel, timeout=timeout)
    report = result.get("report")
    if (result.get("action") != "analyze_behavior" or result.get("schema_version") != 1
            or result.get("installable") is not False or not isinstance(report, dict)
            or report.get("exe") != {"offset": 0, "size": exe_size, "sha256": exe_sha256}
            or report.get("installable") is not False
            or report.get("capability") != "behavior_supported"
            or not isinstance(report.get("behavior"), dict)
            or report["behavior"].get("coverage", {}).get("complete") is not True):
        raise ValueError("exe_worker_invalid_response")
    from exe_profiles import digest
    if digest(report["behavior"]) != report.get("behavior_sha256"):
        raise ValueError("exe_worker_invalid_response")
    return report


async def _run_worker(bridge, tool_dir, archive, output, descriptor, cancel, *, timeout):
    command = worker_command(tool_dir, archive, output)
    identity = bridge.process_identity
    if os.geteuid() == 0 and identity.get("user") in (None, 0):
        raise ValueError("worker_requires_linux_user")
    payload = json.dumps(descriptor, ensure_ascii=True).encode()
    if len(payload) > MAX_OUTPUT:
        raise ValueError("exe_descriptor_limit")
    if cancel.is_set():
        raise RuntimeError("download_cancelled")
    process = await asyncio.create_subprocess_exec(*command, stdin=asyncio.subprocess.PIPE,
        stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE,
        start_new_session=True, env={"PATH": "/usr/bin"}, **identity)

    async def send():
        process.stdin.write(payload)
        await process.stdin.drain()
        process.stdin.close()

    readers = asyncio.gather(_bounded(process.stdout, MAX_OUTPUT), _bounded(process.stderr, 65536),
                             process.wait(), send())
    started = asyncio.get_running_loop().time()
    task = current_task.get()
    if task:
        task.stage("analyzing_installer" if descriptor.get("action") in ("analyze", "analyze_behavior") else "extracting")
    try:
        while not readers.done():
            if cancel.is_set():
                raise RuntimeError("download_cancelled")
            if asyncio.get_running_loop().time() - started >= timeout:
                raise ValueError("exe_worker_timeout")
            await asyncio.wait({readers}, timeout=.1)
        stdout, stderr, code, _ = await readers
        if cancel.is_set():
            raise RuntimeError("download_cancelled")
        try:
            result = json.loads(stdout)
        except (ValueError, UnicodeDecodeError) as error:
            raise ValueError("exe_worker_invalid_response") from error
        if not isinstance(result, dict):
            raise ValueError("exe_worker_invalid_response")
        if code:
            raise ValueError(result.get("error", "exe_worker_failed"))
        if not isinstance(result, dict) or result.get("exe_sha256") != descriptor["exe_sha256"]:
            raise ValueError("exe_worker_invalid_response")
        return result
    finally:
        if process.returncode is None:
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                await asyncio.wait_for(process.wait(), 1)
            except asyncio.TimeoutError:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                await process.wait()
        if not readers.done():
            readers.cancel()
        await asyncio.gather(readers, return_exceptions=True)
