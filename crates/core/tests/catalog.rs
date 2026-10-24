use patchhub_core::{catalog::*, steam::GameInstall};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn game(root: PathBuf) -> GameInstall {
    GameInstall {
        instance_id: "test:42".into(),
        appid: 42,
        name: "Synthetic game".into(),
        localized_name: None,
        name_language: None,
        library_path: root.clone(),
        install_path: root,
        buildid: Some("100".into()),
        availability: "installed".into(),
        kind: "unknown".into(),
    }
}
fn release() -> PatchRelease {
    PatchRelease {
        patch_id: "synthetic".into(),
        release_id: "synthetic-v1".into(),
        appid: 42,
        name: "Synthetic patch".into(),
        version: "1".into(),
        language: "zh-CN".into(),
        category: "content_restore".into(),
        source_page: "https://example.invalid/test".into(),
        support_level: SupportLevel::ManagedZip,
        platforms: vec![Platform::Windows],
        verified_buildids: vec!["100".into()],
        detection: vec![Fingerprint {
            path: "game/patch.rpa".into(),
            sha256: hash(b"patch"),
        }],
        recipe: Some(Recipe {
            artifact_sha256: hash(b"synthetic archive"),
            files: vec![FileMapping {
                source: "patch.rpa".into(),
                destination: "game/patch.rpa".into(),
                original_sha256: None,
                installed_sha256: hash(b"patch"),
            }],
        }),
        artifact: None,
    }
}
fn catalog(r: PatchRelease) -> Catalog {
    Catalog {
        schema_version: 1,
        revision: 1,
        releases: vec![r],
    }
}

#[test]
fn detection_distinguishes_missing_external_conflict_and_unknown() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("game")).unwrap();
    let mut install = game(temp.path().into());
    let mut patch = release();
    assert_eq!(detect(&install, &patch).status, "not_detected");
    fs::write(temp.path().join("game/patch.rpa"), b"patch").unwrap();
    assert_eq!(detect(&install, &patch).status, "installed_external");
    fs::write(temp.path().join("game/patch.rpa"), b"modified").unwrap();
    assert_eq!(detect(&install, &patch).status, "partial_or_conflict");
    fs::create_dir(temp.path().join("game/directory")).unwrap();
    patch.detection[0].path = "game/directory".into();
    assert_eq!(detect(&install, &patch).status, "unknown");
    install.availability = "downloading".into();
    assert!(detect(&install, &patch).files.is_empty());
}

#[test]
fn precise_appid_and_explicit_build_platform_guards() {
    let temp = tempfile::tempdir().unwrap();
    let mut install = game(temp.path().into());
    let cat = catalog(release());
    install.appid = 43;
    assert!(matches(&install, &cat, Platform::Windows).is_empty());
    install.appid = 42;
    install.buildid = Some("new-build".into());
    let found = matches(&install, &cat, Platform::Steamos);
    assert!(found[0].blockers.contains(&"build_not_verified".into()));
    assert!(found[0].blockers.contains(&"platform_not_verified".into()));
}

#[test]
fn preview_is_read_only_and_never_authorizes_unsigned_catalog() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("game")).unwrap();
    let install = game(temp.path().into());
    let cat = catalog(release());
    let result = preview(&install, &cat, "synthetic-v1", Platform::Windows).unwrap();
    assert!(!result.executable);
    assert!(result.blockers.contains(&"catalog_unsigned".into()));
    assert!(result.changes[0].baseline_matches);
    assert!(!temp.path().join("game/patch.rpa").exists());
    fs::write(temp.path().join("game/patch.rpa"), b"user data").unwrap();
    let result = preview(&install, &cat, "synthetic-v1", Platform::Windows).unwrap();
    assert!(!result.changes[0].baseline_matches);
    assert_eq!(
        fs::read(temp.path().join("game/patch.rpa")).unwrap(),
        b"user data"
    );
}

#[test]
fn rejects_unsafe_portable_paths_and_conflicting_recipes() {
    for path in [
        "../escape",
        "/absolute",
        "C:/file",
        "a\\b",
        "a//b",
        "NUL.txt",
        "COM1",
        "a:stream",
        "a.",
        "a ",
        "a/*",
    ] {
        assert!(relative_path(path).is_err(), "{path}");
    }
    let mut patch = release();
    let recipe = patch.recipe.as_mut().unwrap();
    let mut duplicate = recipe.files[0].clone();
    duplicate.destination = "GAME/PATCH.RPA".into();
    recipe.files.push(duplicate);
    assert!(catalog(patch).validate().is_err());
}

#[test]
fn loader_rejects_unknown_schema_fields_and_bad_hashes() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("catalog.json");
    fs::write(&path, br#"{"schema_version":2,"revision":0,"releases":[]}"#).unwrap();
    assert!(load(&path).is_err());
    fs::write(
        &path,
        br#"{"schema_version":1,"revision":0,"releases":[],"execute":"cmd"}"#,
    )
    .unwrap();
    assert!(load(&path).is_err());
    let mut patch = release();
    patch.detection[0].sha256 = "bogus".into();
    assert!(catalog(patch).validate().is_err());
}

#[test]
fn detection_rechecks_install_boundary_even_after_scan() {
    let library = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::create_dir(outside.path().join("game")).unwrap();
    fs::write(outside.path().join("game/patch.rpa"), b"patch").unwrap();
    let mut install = game(outside.path().into());
    install.library_path = library.path().into();
    assert_eq!(detect(&install, &release()).status, "unknown");
}

#[cfg(unix)]
#[test]
fn fingerprint_does_not_follow_links_outside_game() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("patch.rpa"), b"patch").unwrap();
    symlink(outside.path(), temp.path().join("game")).unwrap();
    assert_eq!(
        detect(&game(temp.path().into()), &release()).status,
        "unknown"
    );
}

#[test]
fn installed_detection_requires_every_recipe_target_even_when_hints_are_subset() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("game")).unwrap();
    let install = game(temp.path().into());
    let mut patch = release();
    patch.recipe.as_mut().unwrap().files.push(FileMapping {
        source: "second.dat".into(),
        destination: "game/second.dat".into(),
        original_sha256: None,
        installed_sha256: hash(b"second"),
    });
    fs::write(temp.path().join("game/patch.rpa"), b"patch").unwrap();
    assert_eq!(detect(&install, &patch).status, "partial_or_conflict");
    fs::write(temp.path().join("game/second.dat"), b"second").unwrap();
    assert_eq!(detect(&install, &patch).status, "installed_external");
}

#[test]
fn display_name_overrides_do_not_change_appid_matches_or_detection() {
    let temp = tempfile::tempdir().unwrap();
    let mut install = game(temp.path().to_owned());
    let catalog = catalog(release());
    let before = serde_json::to_value(matches(&install, &catalog, Platform::Windows)).unwrap();
    assert_eq!(before.as_array().unwrap().len(), 1);
    install.localized_name = Some("妃神会秘史".into());
    install.name_language = Some("schinese".into());
    let after = serde_json::to_value(matches(&install, &catalog, Platform::Windows)).unwrap();
    assert_eq!(after, before);
}
