"""Trusted host state directories; never reads legacy caches or journals."""
import os
from pathlib import Path
import stat


def _plain(path):
    for item in (path, *path.parents):
        if item.is_symlink() or (hasattr(item, "is_junction") and item.is_junction()):
            raise ValueError("linked_state_path")
        if item.exists() and not item.is_dir():
            raise ValueError("invalid_state_path")


def prepare_host_state(state_dir, user_home, run_uid=None, run_gid=None):
    """Create only fixed directories; never recursively repair ownership.

    Core initializes its own atomic schema after this host boundary. Migration
    checks are deployment tooling and do not become a startup legacy reader.
    """
    home = Path(user_home).absolute()
    root = Path(state_dir).absolute()
    if root != home / ".local/share/steam-patch-manager":
        raise ValueError("invalid_state_path")
    _plain(root)
    if not home.is_dir():
        raise ValueError("invalid_user_home")
    euid = os.geteuid() if hasattr(os, "geteuid") else None
    uid = run_uid if run_uid is not None else euid
    if euid == 0 and (uid is None or uid == 0 or run_gid is None):
        raise ValueError("unprivileged_user_required")
    if uid is not None and home.stat().st_uid != uid:
        raise ValueError("state_owner_mismatch")
    for directory in (home / ".local", home / ".local/share", root, root / "host-v3"):
        _plain(directory)
        created = False
        try:
            directory.mkdir(mode=0o700)
            created = True
        except FileExistsError:
            if not directory.is_dir():
                raise ValueError("invalid_state_path") from None
        if created and euid == 0:
            os.chown(directory, uid, run_gid, follow_symlinks=False)
        observed = directory.stat()
        if uid is not None and observed.st_uid != uid:
            raise ValueError("state_owner_mismatch")
        if stat.S_IMODE(observed.st_mode) & 0o700 != 0o700:
            raise ValueError("state_permissions_invalid")
    return root / "host-v3"
