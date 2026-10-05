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


async def extract(bridge, tool_dir, archive, output, profile, cancel, *, timeout=180):
    command = worker_command(tool_dir, archive, output)
    identity = bridge.process_identity
    if os.geteuid() == 0 and identity.get("user") in (None, 0):
        raise ValueError("worker_requires_linux_user")
    descriptor = {k: profile[k] for k in ("exe_sha256", "exe_size", "metadata_sha256", "payload")}
    descriptor["semantics_approved"] = profile["review"]["semantics"] is True
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
        task.stage("extracting")
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
        if code:
            raise ValueError(result.get("error", "exe_worker_failed"))
        if not isinstance(result, dict) or result.get("exe_sha256") != profile["exe_sha256"]:
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
