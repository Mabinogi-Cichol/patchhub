use patchhub_core::sources::{snapshot::SourceSnapshot, store::SourceStore, GameQuery};

fn game(appid: u32) -> GameQuery {
    GameQuery {
        appid,
        name: format!("Game {appid}"),
        localized_name: None,
        instance_id: None,
    }
}

#[test]
fn missing_snapshot_state_does_not_initialize_any_files() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    assert!(SourceSnapshot::load(&state, &[game(42)], 1234)
        .unwrap()
        .is_none());
    assert!(!state.exists());
}

#[test]
fn snapshot_read_preserves_existing_state_bytes_and_uses_supplied_clock() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    let store = SourceStore::open(&state).unwrap();
    let settings_path = store.paths().settings();
    let schema_path = store.paths().schema();
    let before_settings = std::fs::read(&settings_path).unwrap();
    let before_schema = std::fs::read(&schema_path).unwrap();
    drop(store);

    let snapshot = SourceSnapshot::load(&state, &[game(42)], 1_700_000_000)
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.now_unix(), 1_700_000_000);
    assert_eq!(snapshot.settings_revision(), 1);
    assert_eq!(std::fs::read(settings_path).unwrap(), before_settings);
    assert_eq!(std::fs::read(schema_path).unwrap(), before_schema);
    assert!(!state.join("sources/rust-v1/settings.lock").exists());
}

#[test]
fn malformed_existing_settings_is_reported_without_repairing_it() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    let store = SourceStore::open(&state).unwrap();
    let settings_path = store.paths().settings();
    drop(store);
    std::fs::write(&settings_path, b"{broken").unwrap();

    let error = match SourceSnapshot::load(&state, &[game(42)], 1234) {
        Ok(_) => panic!("corrupt settings must be reported"),
        Err(error) => error,
    };
    assert!(!error.to_string().is_empty());
    assert_eq!(std::fs::read(settings_path).unwrap(), b"{broken");
}

#[test]
fn relative_existing_state_is_captured() {
    let workspace = std::env::current_dir().unwrap();
    let parent = workspace.join(".local");
    std::fs::create_dir_all(&parent).unwrap();
    let temp = tempfile::tempdir_in(parent).unwrap();
    let state = temp.path().join("state");
    let store = SourceStore::open(&state).unwrap();
    drop(store);
    let relative = state.strip_prefix(&workspace).unwrap();
    let snapshot = SourceSnapshot::load(relative, &[game(42)], 1234)
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.settings_revision(), 1);
    assert!(snapshot.state_root().is_absolute());
}

fn directory_link(target: &std::path::Path, link: &std::path::Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    {
        let output = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn linked_legacy_directories_are_ignored_and_new_state_directories_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    let store = SourceStore::open(&state).unwrap();
    let root = store.paths().root().to_owned();
    drop(store);
    // Legacy v1 trees are only read by the one-time migration; a read-only
    // snapshot no longer reads (or therefore rejects) them.
    for subdirectory in ["cache", "discovery", "discovery-index"] {
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("record.json"), b"{}").unwrap();
        let link = root.join(subdirectory);
        directory_link(outside.path(), &link);
        let error =
            patchhub_core::sources::store::read_bounded(&link.join("record.json")).unwrap_err();
        assert_eq!(error.to_string(), "linked_source_state");
        assert!(
            SourceSnapshot::load(&state, &[game(42)], 1234).is_ok(),
            "snapshot must ignore legacy {subdirectory}"
        );
        #[cfg(unix)]
        std::fs::remove_file(&link).unwrap();
        #[cfg(windows)]
        std::fs::remove_dir(&link).unwrap();
    }
    // The schema-2 read model must still reject linked state directories.
    for subdirectory in ["status", "index", "matches"] {
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("record.json"), b"{}").unwrap();
        let link = root.join(subdirectory);
        directory_link(outside.path(), &link);
        assert!(
            SourceSnapshot::load(&state, &[game(42)], 1234).is_err(),
            "snapshot must reject linked {subdirectory}"
        );
        #[cfg(unix)]
        std::fs::remove_file(&link).unwrap();
        #[cfg(windows)]
        std::fs::remove_dir(&link).unwrap();
    }
}
