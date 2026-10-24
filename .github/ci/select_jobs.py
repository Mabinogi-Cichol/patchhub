"""Select CI jobs from changed build inputs, without third-party actions."""
import fnmatch
import json
import os
from pathlib import Path
import subprocess


JOBS = ("core", "windows", "decky")
# Keep build dependencies here when adding a new source tree or build input.
RULES = (
    (JOBS, (
        "crates/**", "packages/contracts/**", "catalog/trust-keys.json",
        "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".cargo/**",
        ".github/workflows/**", ".github/ci/**", ".gitattributes",
    )),
    (("windows", "decky"), (
        "packages/ui/**", "package.json", "package-lock.json", ".npmrc",
        "scripts/test-ui-contract.mjs", "scripts/test-flutter-presentation.mjs",
        "scripts/package-artifacts.py", "scripts/test_package_artifacts.py",
        "scripts/build-sbom.py", "scripts/test_build_sbom.py",
        "scripts/licenses/**", "LICENSE",
    )),
    (("windows",), (
        "apps/windows-flutter/**", "scripts/build-windows-*.ps1",
        "scripts/windows-installer.nsi", "scripts/flutter_sbom.py",
        "scripts/test_flutter_sbom.py",
        "scripts/windows-update.ps1", "scripts/update_manifest.py", "scripts/test_windows_update.py",
        "scripts/fixtures/app-update.json",
        "catalog/catalog.json", "catalog/signed-catalog.json",
        "docs/client-guide.md", "docs/privacy.md",
    )),
    (("decky",), (
        "apps/decky/**", "install.sh", "scripts/build-linux.sh",
        "catalog/catalog.json", "catalog/signed-catalog.json",
        "scripts/install-decky.py", "scripts/test_install_decky.py",
        "scripts/test_decky_bootstrap.py", "scripts/test-install-decky-bridge.mjs",
        "scripts/fixtures/app-update.json",
    )),
    (("core",), (
        "scripts/sf*.py", "scripts/test_sf*.py", "scripts/fixtures/**",
        "scripts/freeze_sf*.py", "scripts/verify_sf*.py",
    )),
)


def select_jobs(paths):
    selected = set()
    for path in paths:
        # These two documents are shipped inside the Windows archives/installers.
        if path.endswith(".md") and path not in ("docs/client-guide.md", "docs/privacy.md"):
            continue
        for jobs, patterns in RULES:
            if any(fnmatch.fnmatchcase(path, pattern) for pattern in patterns):
                selected.update(jobs)
    return {job: job in selected for job in JOBS}


def changed_paths(base, head, *, pull_request=False):
    # --no-renames includes both names of a rename, including moves out of a module.
    revisions = [f"{base}...{head}"] if pull_request else [base, head]
    result = subprocess.run(
        ["git", "diff", "--name-only", "--no-renames", "-z", *revisions, "--"],
        check=True, stdout=subprocess.PIPE,
    )
    return os.fsdecode(result.stdout).split("\0")[:-1]


def select_for_event(name, event, ref):
    if name == "workflow_dispatch" or ref.startswith("refs/tags/v"):
        return dict.fromkeys(JOBS, True)
    if name == "pull_request":
        pr = event["pull_request"]
        paths = changed_paths(pr["base"]["sha"], pr["head"]["sha"], pull_request=True)
    elif name == "push":
        if event["before"] == "0" * 40:
            # The first main push has no previous tree to compare against.
            return dict.fromkeys(JOBS, True)
        paths = changed_paths(event["before"], event["after"])
    else:
        raise ValueError(f"Unsupported CI event: {name}")
    print("Changed paths:", json.dumps(paths, ensure_ascii=True))
    return select_jobs(paths)


if __name__ == "__main__":
    selected = select_for_event(
        os.environ["GITHUB_EVENT_NAME"],
        json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text(encoding="utf-8")),
        os.environ["GITHUB_REF"],
    )
    print("Selected jobs:", json.dumps(selected))
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
        for job, enabled in selected.items():
            output.write(f"{job}={str(enabled).lower()}\n")
