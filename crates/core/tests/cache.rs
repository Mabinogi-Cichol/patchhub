//! Cache public boundary regression coverage using temporary state only.
use patchhub_core::{
    catalog::Platform,
    installer::{cache::CacheService, outcome::CleanupStatus, service::ValidatedHostContext},
};
#[cfg(windows)]
use std::fs;
fn platform() -> Platform {
    if cfg!(windows) {
        Platform::Windows
    } else {
        Platform::Steamos
    }
}
#[test]
fn missing_cache_inspection_is_readonly_and_manual_cleanup_returns_truthful_empty_result() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let host =
        ValidatedHostContext::new(&state, &[dir.path().join("no-steam")], platform()).unwrap();
    let inventory = CacheService::new(&host).inspect().unwrap();
    assert_eq!((inventory.total_bytes, inventory.entries), (0, 0));
    assert!(!state.exists());
    if patchhub_core::installer::host_identity_blocker().is_some() {
        assert_eq!(
            CacheService::new(&host).cleanup().unwrap_err().code,
            "installer_host_requires_non_root"
        );
        assert!(!state.exists());
        return;
    }
    let result = CacheService::new(&host).cleanup().unwrap();
    assert_eq!(result.status, CleanupStatus::Completed);
    assert!(result.performed);
    assert_eq!(result.removed_bytes, 0);
    assert!(result.failures.is_empty());
    assert!(host.installer_paths().cache_lock().is_file());
    assert!(!host.installer_paths().journal().exists());
    assert!(!host.installer_paths().requests().exists());
}
#[cfg(windows)]
#[test]
fn junction_cache_root_is_refused_before_deleting_other_safe_cache() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let host =
        ValidatedHostContext::new(&state, &[dir.path().join("no-steam")], platform()).unwrap();
    let paths = host.installer_paths();
    fs::create_dir_all(paths.downloads_dir()).unwrap();
    fs::write(paths.downloads_dir().join("safe"), b"keep").unwrap();
    let outside = dir.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("original"), b"outside").unwrap();
    let output = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(paths.reports_dir())
        .arg(&outside)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        CacheService::new(&host).cleanup().unwrap_err().code,
        "cleanup_linked_path"
    );
    assert_eq!(
        fs::read(paths.downloads_dir().join("safe")).unwrap(),
        b"keep"
    );
    assert_eq!(fs::read(outside.join("original")).unwrap(), b"outside");
    fs::remove_dir(paths.reports_dir()).unwrap();
}
