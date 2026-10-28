import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from select_jobs import JOBS, changed_paths, select_for_event, select_jobs

SCRIPT = Path(__file__).with_name("select_jobs.py").resolve()


class SelectionTests(unittest.TestCase):
    def assert_jobs(self, paths, expected):
        actual = {job for job, enabled in select_jobs(paths).items() if enabled}
        self.assertEqual(actual, set(expected))

    def test_module_dependencies(self):
        cases = (
            ("apps/windows-flutter/lib/main.dart", ("windows",)),
            ("apps/windows-flutter/pubspec.lock", ("windows",)),
            ("scripts/build-windows-installer.ps1", ("windows",)),
            ("scripts/flutter_sbom.py", ("windows",)),
            ("scripts/test_windows_single_instance.py", ("windows",)),
            ("apps/decky/backend/tasks.py", ("decky",)),
            ("install.sh", ("decky",)),
            ("scripts/test-install-decky-bridge.mjs", ("decky",)),
            ("crates/core/src/lib.rs", JOBS),
            ("crates/contract-gen/src/main.rs", JOBS),
            ("crates/protocol/src/fixtures.rs", JOBS),
            ("crates/installer-worker/src/main.rs", JOBS),
            ("packages/contracts/fixtures/v5/cases.json", JOBS),
            ("catalog/trust-keys.json", JOBS),
            ("Cargo.lock", JOBS),
            ("rust-toolchain.toml", JOBS),
            (".cargo/config.toml", JOBS),
            (".github/workflows/ci.yml", JOBS),
            (".github/ci/select_jobs.py", JOBS),
            ("packages/ui/src/index.ts", ("windows", "decky")),
            ("scripts/test-flutter-presentation.mjs", ("windows", "decky")),
            ("package-lock.json", ("windows", "decky")),
            ("scripts/package-artifacts.py", ("windows", "decky")),
            ("scripts/build-sbom.py", ("windows", "decky")),
            ("scripts/licenses/sfextract-MIT.txt", ("windows", "decky")),
            ("catalog/catalog.json", ("windows", "decky")),
            ("catalog/signed-catalog.json", ("windows", "decky")),
            ("docs/client-guide.md", ("windows",)),
            ("docs/privacy.md", ("windows",)),
            ("docs/images/branding/logo.png", ("windows", "decky")),
            ("scripts/sf95_worker.py", ("core",)),
            ("scripts/test_sf9_metadata.py", ("core",)),
        )
        for path, jobs in cases:
            with self.subTest(path=path):
                self.assert_jobs([path], jobs)

    def test_non_build_inputs(self):
        self.assert_jobs([
            "README.md", "docs/features/example/design.md", "docs/example.png",
            "apps/decky/README.md", ".github/ci/README.md",
            ".agents/skills/example/agents/openai.yaml", ".gitignore",
        ], ())

    def test_combined_changes_and_empty_diff(self):
        self.assert_jobs([], ())
        self.assert_jobs(["apps/decky/main.py", "apps/windows-flutter/lib/main.dart"],
                         ("windows", "decky"))

    def test_release_manual_and_initial_push_always_run_all(self):
        for name, event, ref in (
            ("push", {}, "refs/tags/v0.3.1"),
            ("workflow_dispatch", {}, "refs/heads/main"),
            ("push", {"before": "0" * 40}, "refs/heads/main"),
        ):
            with self.subTest(name=name, ref=ref):
                self.assertEqual(select_for_event(name, event, ref), dict.fromkeys(JOBS, True))

    def test_event_ranges(self):
        with patch("select_jobs.changed_paths", return_value=["apps/decky/main.py"]) as diff:
            select_for_event("push", {"before": "old", "after": "new"}, "refs/heads/main")
            diff.assert_called_once_with("old", "new")
        with patch("select_jobs.changed_paths", return_value=[]) as diff:
            select_for_event("pull_request", {
                "pull_request": {"base": {"sha": "base"}, "head": {"sha": "head"}},
            }, "refs/pull/1/merge")
            diff.assert_called_once_with("base", "head", pull_request=True)


class GitDiffTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.addCleanup(os.chdir, os.getcwd())
        os.chdir(self.temp.name)
        self.git("init", "--initial-branch=main")
        self.git("config", "user.email", "ci-test@example.invalid")
        self.git("config", "user.name", "CI test")

    def git(self, *args):
        return subprocess.check_output(["git", *args], stderr=subprocess.PIPE).decode().strip()

    def commit(self, path):
        target = Path(path)
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(path, encoding="utf-8")
        self.git("add", "--all")
        self.git("commit", "-m", "fixture")
        return self.git("rev-parse", "HEAD")

    def test_push_includes_all_commits_and_deleted_rename_source(self):
        base = self.commit("apps/windows-flutter/lib/old.dart")
        self.git("mv", "apps/windows-flutter/lib/old.dart", "notes.txt")
        self.git("commit", "-m", "move out of module")
        head = self.commit("docs/change.md")
        paths = changed_paths(base, head)
        self.assertIn("apps/windows-flutter/lib/old.dart", paths)
        self.assertIn("notes.txt", paths)
        self.assertIn("docs/change.md", paths)
        self.assertEqual(select_jobs(paths), {"core": False, "windows": True, "decky": False})

    def test_pr_excludes_changes_only_on_base_branch(self):
        self.commit("README.md")
        self.git("checkout", "-b", "topic")
        head = self.commit("apps/decky/src/界面.tsx")
        self.git("checkout", "main")
        base = self.commit("apps/windows-flutter/lib/main.dart")
        self.assertEqual(changed_paths(base, head, pull_request=True), ["apps/decky/src/界面.tsx"])

    def test_missing_revision_fails_instead_of_skipping(self):
        head = self.commit("README.md")
        with self.assertRaises(subprocess.CalledProcessError):
            changed_paths("missing-revision", head)

    def test_cli_writes_github_job_outputs(self):
        base = self.commit("README.md")
        head = self.commit("apps/windows-flutter/lib/main.dart")
        event_path = Path("event.json").resolve()
        event_path.write_text(json.dumps({"before": base, "after": head}), encoding="utf-8")
        output_path = Path("outputs.txt").resolve()
        env = dict(os.environ, GITHUB_EVENT_NAME="push", GITHUB_REF="refs/heads/main",
                   GITHUB_EVENT_PATH=str(event_path), GITHUB_OUTPUT=str(output_path))
        subprocess.run([sys.executable, str(SCRIPT)], env=env, check=True,
                       stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        self.assertEqual(output_path.read_text(encoding="utf-8"),
                         "core=false\nwindows=true\ndecky=false\n")


if __name__ == "__main__":
    unittest.main()
