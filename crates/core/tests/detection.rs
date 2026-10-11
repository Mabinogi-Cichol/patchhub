use sha2::{Digest, Sha256};
use std::{fs, path::Path, sync::atomic::AtomicBool};
use steam_patch_core::{
    catalog::{self, Catalog, PatchRelease, Platform, Recipe, SupportLevel},
    detection::{self, EvidenceStatus},
    steam::GameInstall,
};
fn game(root: &Path) -> GameInstall {
    GameInstall {
        instance_id: "test".into(),
        appid: 1,
        name: "test".into(),
        localized_name: None,
        name_language: None,
        library_path: root.into(),
        install_path: root.into(),
        buildid: None,
        availability: "installed".into(),
        kind: "unknown".into(),
    }
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn inspect(game: &GameInstall, path: &str, expected: &str) -> detection::FileEvidence {
    detection::inspect_file(
        game,
        path,
        expected,
        detection::DEFAULT_MAX_BYTES,
        &AtomicBool::new(false),
    )
}
fn code(e: &detection::FileEvidence) -> &str {
    &e.diagnostic.as_ref().unwrap().code
}
#[test]
fn direct_api_distinguishes_missing_matching_conflicting_and_unknown() {
    let temp = tempfile::tempdir().unwrap();
    let game = game(temp.path());
    let hash = digest(b"patch");
    assert_eq!(
        inspect(&game, "patch.dat", &hash).status,
        EvidenceStatus::Missing
    );
    fs::write(temp.path().join("patch.dat"), b"patch").unwrap();
    assert_eq!(
        inspect(&game, "patch.dat", &hash).status,
        EvidenceStatus::Match
    );
    let mismatch = inspect(&game, "patch.dat", &digest(b"other"));
    assert_eq!(mismatch.status, EvidenceStatus::Mismatch);
    assert_eq!(mismatch.actual_sha256, Some(hash.clone()));
    fs::create_dir(temp.path().join("directory")).unwrap();
    assert_eq!(
        code(&inspect(&game, "directory", &hash)),
        "evidence_not_regular_file"
    );
    assert_eq!(
        code(&inspect(&game, "patch.dat", "invalid")),
        "evidence_invalid_hash"
    );
    for path in [
        "../patch.dat",
        "/patch.dat",
        "a\\b",
        "C:/patch.dat",
        "patch.dat:stream",
        "NUL",
    ] {
        assert_eq!(code(&inspect(&game, path, &hash)), "evidence_invalid_path");
    }
}
#[test]
fn unsafe_boundaries_cannot_report_missing() {
    let library = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let mut game = game(outside.path());
    game.library_path = library.path().into();
    assert_eq!(
        code(&inspect(&game, "missing", &digest(b""))),
        "evidence_boundary_violation"
    );
    game.install_path = library.path().join("missing-game");
    let e = inspect(&game, "missing", &digest(b""));
    assert_eq!(e.status, EvidenceStatus::UnreadableOrUnsafe);
}
#[test]
fn size_limit_and_cancel_do_not_return_a_digest() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("patch.dat"), b"patch").unwrap();
    let game = game(temp.path());
    let e = detection::inspect_file(
        &game,
        "patch.dat",
        &digest(b"patch"),
        4,
        &AtomicBool::new(false),
    );
    assert_eq!(code(&e), "evidence_size_limit");
    assert!(e.actual_sha256.is_none());
    let e = detection::inspect_file(
        &game,
        "patch.dat",
        &digest(b"patch"),
        u64::MAX,
        &AtomicBool::new(true),
    );
    assert_eq!(code(&e), "evidence_cancelled");
    assert!(e.actual_sha256.is_none());
    // A sparse file verifies the frozen 16 GiB default without allocating it.
    let large = fs::OpenOptions::new()
        .write(true)
        .open(temp.path().join("patch.dat"))
        .unwrap();
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn DeviceIoControl(
                handle: *mut std::ffi::c_void,
                code: u32,
                input: *mut std::ffi::c_void,
                input_size: u32,
                output: *mut std::ffi::c_void,
                output_size: u32,
                returned: *mut u32,
                overlapped: *mut std::ffi::c_void,
            ) -> i32;
        }
        let mut returned = 0;
        // FSCTL_SET_SPARSE avoids allocating 16 GiB for a metadata-only limit fixture.
        assert_ne!(
            unsafe {
                DeviceIoControl(
                    large.as_raw_handle(),
                    0x000900c4,
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null_mut(),
                    0,
                    &mut returned,
                    std::ptr::null_mut(),
                )
            },
            0
        );
    }
    large.set_len(detection::DEFAULT_MAX_BYTES + 1).unwrap();
    drop(large);
    let e = inspect(&game, "patch.dat", &digest(b"patch"));
    assert_eq!(code(&e), "evidence_size_limit");
}
#[cfg(unix)]
#[test]
fn links_in_target_game_library_or_ancestors_are_unknown() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("patch.dat"), b"patch").unwrap();
    symlink(
        outside.path().join("patch.dat"),
        temp.path().join("patch.dat"),
    )
    .unwrap();
    assert_eq!(
        code(&inspect(&game(temp.path()), "patch.dat", &digest(b"patch"))),
        "evidence_linked_path"
    );
    symlink(outside.path(), temp.path().join("linked")).unwrap();
    let mut g = game(&temp.path().join("linked"));
    assert_eq!(
        code(&inspect(&g, "missing", &digest(b""))),
        "evidence_linked_path"
    );
    g.install_path = temp.path().to_owned();
    assert_eq!(
        code(&inspect(&g, "missing", &digest(b""))),
        "evidence_linked_path"
    );
}
#[cfg(windows)]
#[test]
fn junction_ancestor_is_rejected_even_when_target_is_missing() {
    let temp = tempfile::tempdir().unwrap();
    let real = temp.path().join("real");
    let linked = temp.path().join("linked");
    fs::create_dir(&real).unwrap();
    fs::create_dir(real.join("game")).unwrap();
    let status = std::process::Command::new("cmd")
        .arg("/c")
        .arg("mklink")
        .arg("/J")
        .arg(&linked)
        .arg(&real)
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let result = inspect(&game(&linked.join("game")), "missing", &digest(b""));
    // Remove only the junction itself; never traverse its target.
    fs::remove_dir(&linked).unwrap();
    assert_eq!(code(&result), "evidence_linked_path");
}
#[test]
fn empty_recipe_is_not_evidence_of_installation() {
    let temp = tempfile::tempdir().unwrap();
    let release = PatchRelease {
        patch_id: "p".into(),
        release_id: "r".into(),
        appid: 1,
        name: "p".into(),
        version: "1".into(),
        language: "zh".into(),
        category: "test".into(),
        source_page: "https://example.invalid".into(),
        support_level: SupportLevel::DiscoverOnly,
        platforms: vec![Platform::Windows],
        verified_buildids: vec![],
        detection: vec![],
        recipe: Some(Recipe {
            artifact_sha256: digest(b""),
            files: vec![],
        }),
        artifact: None,
    };
    let game = game(temp.path());
    assert_eq!(catalog::detect(&game, &release).status, "unknown");
    let catalog = Catalog {
        schema_version: 1,
        revision: 1,
        releases: vec![release],
    };
    let result =
        catalog::matches_with_cancel(&game, &catalog, Platform::Windows, &AtomicBool::new(true));
    assert_eq!(result[0].detection.status, "unknown");
}
