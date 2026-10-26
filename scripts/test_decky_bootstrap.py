"""Run the exact README one-liner with a fake download, never a real install."""
import json
import os
from pathlib import Path
import re
import shlex
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
URL = "https://raw.githubusercontent.com/Mabinogi-Cichol/patchhub/main/scripts/install-decky.py"
ENTRY_URL = "https://raw.githubusercontent.com/Mabinogi-Cichol/patchhub/main/install.sh"


def read_command(path):
    commands = re.findall(r"```bash[ \t]*\n[ \t]*(curl [^\n]+?)[ \t]*\n[ \t]*```", path.read_text(encoding="utf-8"))
    return next(command for command in commands if command == f"curl -fsSL {ENTRY_URL} | bash")


class DocumentationTests(unittest.TestCase):
    def test_root_and_client_readme_share_one_complete_command(self):
        command = read_command(ROOT / "README.md")
        self.assertEqual(command, read_command(ROOT / "apps/decky/README.md"))
        self.assertEqual(shlex.split(command), ["curl", "-fsSL", ENTRY_URL, "|", "bash"])


@unittest.skipUnless(sys.platform == "linux", "README bootstrap targets SteamOS Bash")
class BootstrapTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.work = self.root / "run"
        self.work.mkdir()
        self.payload = self.root / "download.py"
        self.entry = self.root / "entry.sh"
        self.entry.write_text((ROOT / "install.sh").read_text(encoding="utf-8"), encoding="utf-8")
        self.curl_args = self.root / "curl-args.txt"
        curl = self.bin / "curl"
        curl.write_text('''#!/bin/sh
for target; do :; done
printf '%s\\n' "$target" >> "$BOOTSTRAP_CURL_ARGS"
case "$target" in
    "$BOOTSTRAP_ENTRY_URL")
        cat -- "$BOOTSTRAP_ENTRY_SCRIPT"
        exit 0 ;;
    "$BOOTSTRAP_SOURCE_URL")
        cat -- "$BOOTSTRAP_PAYLOAD"
        exit "$BOOTSTRAP_CURL_STATUS" ;;
    *) exit 90 ;;
esac
''', encoding="utf-8")
        curl.chmod(0o755)
        (self.bin / "python3").symlink_to(sys.executable)
        self.env = {**os.environ, "PATH": str(self.bin) + os.pathsep + os.environ["PATH"],
                    "BOOTSTRAP_PAYLOAD": str(self.payload), "BOOTSTRAP_CURL_ARGS": str(self.curl_args),
                    "BOOTSTRAP_CURL_STATUS": "0", "BOOTSTRAP_ENTRY_SCRIPT": str(self.entry),
                    "BOOTSTRAP_SOURCE_URL": URL, "BOOTSTRAP_ENTRY_URL": ENTRY_URL}

    def tearDown(self):
        self.temporary.cleanup()

    def run_command(self, source, args=()):
        self.payload.write_text(source, encoding="utf-8")
        command = read_command(ROOT / "README.md")
        if args:
            command += " -s -- " + shlex.join(args)
        return subprocess.run(["bash", "-c", command], cwd=self.work, env=self.env,
                              capture_output=True, text=True, timeout=10)

    def test_success_forwards_arguments_without_saving_script(self):
        args = ["--version", "v0.1.0", "--homebrew", "/tmp/has spaces"]
        result = self.run_command("import json,sys; print(json.dumps(sys.argv[1:]))", args)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout), args)
        self.assertEqual(self.curl_args.read_text().splitlines(), [ENTRY_URL, URL])
        self.assertEqual(list(self.work.iterdir()), [])

    def test_download_failure_never_executes_partial_script(self):
        marker = self.work / "must-not-execute"
        self.env["BOOTSTRAP_CURL_STATUS"] = "22"
        result = self.run_command(f"from pathlib import Path; Path({str(marker)!r}).write_text('bad')")
        self.assertEqual(result.returncode, 22)
        self.assertFalse(marker.exists())
        self.assertEqual(list(self.work.iterdir()), [])

    def test_empty_download_fails(self):
        result = self.run_command("")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(list(self.work.iterdir()), [])

    def test_installer_failure_preserves_exit_code_without_script_residue(self):
        result = self.run_command("raise SystemExit(7)")
        self.assertEqual(result.returncode, 7)
        self.assertEqual(list(self.work.iterdir()), [])

    def test_controlling_terminal_remains_available_for_confirmation(self):
        import fcntl
        import termios
        def claim_terminal():
            os.setsid()
            fcntl.ioctl(0, termios.TIOCSCTTY, 0)
        self.payload.write_text("with open('/dev/tty') as terminal: print(terminal.readline().strip())", encoding="utf-8")
        master, slave = os.openpty()
        try:
            process = subprocess.Popen(["bash", "-c", read_command(ROOT / "README.md")], cwd=self.work,
                                       env=self.env, stdin=slave, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                       text=True, preexec_fn=claim_terminal)
            try:
                os.write(master, b"confirm from terminal\n")
                stdout, stderr = process.communicate(timeout=10)
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait()
            self.assertEqual(process.returncode, 0, stderr)
            self.assertEqual(stdout, "confirm from terminal\n")
        finally:
            os.close(master)
            os.close(slave)

    def test_actual_installer_can_start_from_memory(self):
        result = self.run_command((ROOT / "scripts/install-decky.py").read_text(encoding="utf-8"), ["--help"])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("--version", result.stdout)
        self.assertIn("--recover", result.stdout)
        self.assertEqual(list(self.work.iterdir()), [])

    def test_truncated_entry_without_invocation_does_not_run_install(self):
        source = (ROOT / "install.sh").read_text(encoding="utf-8")
        self.entry.write_text(source.rsplit('patchhub_install "$@"', 1)[0], encoding="utf-8")
        marker = self.work / "must-not-execute"
        result = self.run_command(f"from pathlib import Path; Path({str(marker)!r}).write_text('bad')")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.curl_args.read_text().splitlines(), [ENTRY_URL])
        self.assertFalse(marker.exists())

if __name__ == "__main__":
    unittest.main()
