use base64::{engine::general_purpose::STANDARD, Engine};
use ed25519_dalek::{Signer, SigningKey};
use rusqlite::Connection;
use std::{
    fs,
    path::Path,
    sync::{Arc, Barrier},
    thread,
};
use steam_patch_core::{
    catalog::Catalog,
    projection::DomainError,
    steam::GameInstall,
    trust::{
        revision::{
            verify_catalog, SqliteTrustRevisionStore, TrustRevisionStore, TrustedCatalogMaterial,
            TrustedCatalogProvider,
        },
        CatalogPayload, SignedEnvelope, TrustKey, TrustKeys, VerifiedCatalog,
    },
};

struct Provider {
    envelope: Vec<u8>,
    keys: TrustKeys,
}
impl TrustedCatalogProvider for Provider {
    fn load(&self) -> Result<TrustedCatalogMaterial, DomainError> {
        Ok(TrustedCatalogMaterial {
            envelope: self.envelope.clone(),
            keys: self.keys.clone(),
        })
    }
}
fn provider(revision: u64) -> Provider {
    let key = SigningKey::from_bytes(&[42; 32]);
    let payload = serde_json::to_vec(&CatalogPayload {
        catalog: Catalog {
            schema_version: 1,
            revision,
            releases: vec![],
        },
        generated_at: 1000,
        expires_at: 2000,
        revoked_release_ids: vec![],
    })
    .unwrap();
    Provider {
        envelope: serde_json::to_vec(&SignedEnvelope {
            schema_version: 1,
            key_id: "revision-fixture".into(),
            signature: STANDARD.encode(key.sign(&payload).to_bytes()),
            payload: STANDARD.encode(payload),
        })
        .unwrap(),
        keys: TrustKeys {
            schema_version: 1,
            keys: vec![TrustKey {
                key_id: "revision-fixture".into(),
                public_key: STANDARD.encode(key.verifying_key().to_bytes()),
                revoked: false,
            }],
        },
    }
}
fn verified(revision: u64) -> VerifiedCatalog {
    let provider = provider(revision);
    VerifiedCatalog::from_bytes(&provider.envelope, &provider.keys, 1500, 0).unwrap()
}
fn legacy_database(state: &Path, revision: Option<i64>) {
    fs::create_dir_all(state).unwrap();
    let conn = Connection::open(state.join("trust.sqlite")).unwrap();
    conn.execute_batch(
        "CREATE TABLE catalog_revision(id INTEGER PRIMARY KEY, revision INTEGER NOT NULL);",
    )
    .unwrap();
    if let Some(revision) = revision {
        conn.execute("INSERT INTO catalog_revision VALUES (1, ?1)", [revision])
            .unwrap();
    }
}
fn game(path: &Path) -> GameInstall {
    GameInstall {
        instance_id: "fixture:1".into(),
        appid: 1,
        name: "fixture".into(),
        localized_name: None,
        name_language: None,
        library_path: path.parent().unwrap().to_owned(),
        install_path: path.to_owned(),
        buildid: None,
        availability: "installed".into(),
        kind: "game".into(),
    }
}

#[test]
fn construction_and_verification_do_not_create_missing_state() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("absent/nested");
    let store = SqliteTrustRevisionStore::new(&state).unwrap();
    assert_eq!(store.highest_revision().unwrap(), 0);
    assert_eq!(store.state_dir(), state);
    assert_eq!(TrustRevisionStore::state_dir(&store), Some(state.as_path()));
    assert_eq!(
        verify_catalog(&provider(7), &store, 1500)
            .unwrap()
            .catalog()
            .revision,
        7
    );
    assert!(!root.path().join("absent").exists());
}

#[test]
fn reads_existing_schema_without_mutating_database_or_creating_sidecars() {
    let root = tempfile::tempdir().unwrap();
    legacy_database(root.path(), Some(4));
    let database = root.path().join("trust.sqlite");
    let before = fs::read(&database).unwrap();
    let store = SqliteTrustRevisionStore::new(root.path()).unwrap();
    assert_eq!(store.highest_revision().unwrap(), 4);
    assert!(verify_catalog(&provider(4), &store, 1500).is_ok());
    assert_eq!(
        verify_catalog(&provider(3), &store, 1500).unwrap_err().code,
        "catalog_revision_rollback"
    );
    assert_eq!(fs::read(&database).unwrap(), before);
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn missing_singleton_is_zero_but_corrupt_database_and_negative_revision_fail() {
    let root = tempfile::tempdir().unwrap();
    legacy_database(root.path(), None);
    let store = SqliteTrustRevisionStore::new(root.path()).unwrap();
    assert_eq!(store.highest_revision().unwrap(), 0);
    let conn = Connection::open(root.path().join("trust.sqlite")).unwrap();
    conn.execute("INSERT INTO catalog_revision VALUES(1,-1)", [])
        .unwrap();
    drop(conn);
    assert_eq!(
        store.highest_revision().unwrap_err().code,
        "invalid_catalog_revision"
    );
    fs::write(root.path().join("trust.sqlite"), b"not a sqlite database").unwrap();
    assert_eq!(
        store.highest_revision().unwrap_err().code,
        "trust_store_error"
    );
}

#[test]
fn provider_cannot_bypass_core_signature_limits_time_or_key_revocation() {
    let root = tempfile::tempdir().unwrap();
    let store = SqliteTrustRevisionStore::new(&root.path().join("missing")).unwrap();
    let mut provider = provider(8);
    let mut envelope: SignedEnvelope = serde_json::from_slice(&provider.envelope).unwrap();
    envelope.payload = STANDARD.encode(b"{}");
    provider.envelope = serde_json::to_vec(&envelope).unwrap();
    assert_eq!(
        verify_catalog(&provider, &store, 1500).unwrap_err().code,
        "signature_mismatch"
    );
    provider = self::provider(8);
    provider.keys.keys[0].revoked = true;
    assert_eq!(
        verify_catalog(&provider, &store, 1500).unwrap_err().code,
        "signing_key_revoked"
    );
    provider = self::provider(8);
    assert_eq!(
        verify_catalog(&provider, &store, 2000).unwrap_err().code,
        "catalog_expired"
    );
    provider.envelope = vec![0; (steam_patch_core::catalog::MAX_CATALOG_BYTES * 2 + 1) as usize];
    assert_eq!(
        verify_catalog(&provider, &store, 1500).unwrap_err().code,
        "catalog_too_large"
    );
    assert!(!store.state_dir().exists());
}

#[test]
fn remember_preserves_legacy_table_and_prevents_verified_stale_race() {
    let root = tempfile::tempdir().unwrap();
    legacy_database(root.path(), Some(1));
    let store = SqliteTrustRevisionStore::new(root.path()).unwrap();
    let stale = verify_catalog(&provider(2), &store, 1500).unwrap();
    store.remember_verified_revision(&verified(3)).unwrap();
    assert_eq!(
        store.remember_verified_revision(&stale).unwrap_err().code,
        "catalog_rollback"
    );
    assert_eq!(store.highest_revision().unwrap(), 3);
    store.remember_verified_revision(&verified(3)).unwrap();
    let conn = Connection::open(root.path().join("trust.sqlite")).unwrap();
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(tables, vec!["catalog_revision"]);
    let rows: Vec<(i64, i64)> = conn
        .prepare("SELECT id, revision FROM catalog_revision")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(rows, vec![(1, 3)]);
}

#[test]
fn concurrent_remember_cannot_lower_high_water_mark() {
    let root = tempfile::tempdir().unwrap();
    let store = SqliteTrustRevisionStore::new(root.path()).unwrap();
    store.remember_verified_revision(&verified(1)).unwrap();
    let barrier = Arc::new(Barrier::new(9));
    let workers: Vec<_> = (2..=9)
        .map(|revision| {
            let store = store.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                let catalog = verified(revision);
                barrier.wait();
                match store.remember_verified_revision(&catalog) {
                    Ok(()) => (),
                    Err(error) => assert_eq!(error.code, "catalog_rollback"),
                }
            })
        })
        .collect();
    barrier.wait();
    for worker in workers {
        worker.join().unwrap();
    }
    assert_eq!(store.highest_revision().unwrap(), 9);
}

#[test]
fn revision_overflow_fails_before_creating_state() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("missing");
    let store = SqliteTrustRevisionStore::new(&state).unwrap();
    assert_eq!(
        store
            .remember_verified_revision(&verified(i64::MAX as u64 + 1))
            .unwrap_err()
            .code,
        "catalog_revision_out_of_range"
    );
    assert!(!state.exists());
}

#[test]
fn overlap_traversal_nonregular_database_and_sidecars_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let game_path = root.path().join("game");
    let games = [game(&game_path)];
    assert_eq!(
        SqliteTrustRevisionStore::new_for_games(&game_path.join("state"), &games)
            .unwrap_err()
            .code,
        "state_overlaps_game"
    );
    assert_eq!(
        SqliteTrustRevisionStore::new_for_games(root.path(), &games)
            .unwrap_err()
            .code,
        "state_overlaps_game"
    );
    assert_eq!(
        SqliteTrustRevisionStore::new(&root.path().join("a/../b"))
            .unwrap_err()
            .code,
        "unsafe_path_component"
    );
    let state = root.path().join("state");
    fs::create_dir_all(state.join("trust.sqlite")).unwrap();
    assert_eq!(
        SqliteTrustRevisionStore::new(&state).unwrap_err().code,
        "invalid_trust_state_path"
    );
    fs::remove_dir(state.join("trust.sqlite")).unwrap();
    fs::create_dir(state.join("trust.sqlite-wal")).unwrap();
    assert_eq!(
        SqliteTrustRevisionStore::new(&state).unwrap_err().code,
        "invalid_trust_state_path"
    );
}

#[test]
fn paths_are_revalidated_when_state_changes_after_construction() {
    let root = tempfile::tempdir().unwrap();
    let store = SqliteTrustRevisionStore::new(root.path()).unwrap();
    fs::create_dir(root.path().join("trust.sqlite-journal")).unwrap();
    assert_eq!(
        store.highest_revision().unwrap_err().code,
        "invalid_trust_state_path"
    );
    assert_eq!(
        store
            .remember_verified_revision(&verified(2))
            .unwrap_err()
            .code,
        "invalid_trust_state_path"
    );
    assert!(!root.path().join("trust.sqlite").exists());
}

#[test]
fn readonly_foreign_wal_state_fails_without_creating_shared_memory() {
    let root = tempfile::tempdir().unwrap();
    legacy_database(root.path(), Some(5));
    let conn = Connection::open(root.path().join("trust.sqlite")).unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
    drop(conn);
    let before = fs::read(root.path().join("trust.sqlite")).unwrap();
    let store = SqliteTrustRevisionStore::new(root.path()).unwrap();
    assert_eq!(
        store.highest_revision().unwrap_err().code,
        "trust_store_error"
    );
    assert_eq!(fs::read(root.path().join("trust.sqlite")).unwrap(), before);
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn symbolic_link_database_and_linked_state_are_rejected() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    legacy_database(outside.path(), Some(8));
    symlink(outside.path(), root.path().join("linked")).unwrap();
    assert_eq!(
        SqliteTrustRevisionStore::new(&root.path().join("linked"))
            .unwrap_err()
            .code,
        "linked_path"
    );
    symlink(
        outside.path().join("trust.sqlite"),
        root.path().join("trust.sqlite"),
    )
    .unwrap();
    assert_eq!(
        SqliteTrustRevisionStore::new(root.path()).unwrap_err().code,
        "linked_path"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn production_write_identity_matches_actual_process() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state");
    let store = SqliteTrustRevisionStore::new(&state).unwrap();
    let result = store.remember_verified_revision(&verified(1));
    if unsafe { libc::geteuid() } == 0 {
        assert_eq!(result.unwrap_err().code, "installer_host_requires_non_root");
        assert!(!state.exists());
    } else {
        result.unwrap();
        assert_eq!(store.highest_revision().unwrap(), 1);
    }
}

#[cfg(windows)]
#[test]
fn windows_junction_state_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let junction = root.path().join("junction");
    let result = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&junction)
        .arg(outside.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        SqliteTrustRevisionStore::new(&junction).unwrap_err().code,
        "linked_path"
    );
    fs::remove_dir(junction).unwrap();
}

#[test]
fn hardlinked_database_and_sidecars_are_rejected_without_external_mutation() {
    for name in [
        "trust.sqlite",
        "trust.sqlite-journal",
        "trust.sqlite-wal",
        "trust.sqlite-shm",
    ] {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state");
        let outside = root.path().join("outside");
        legacy_database(&outside, Some(8));
        let external_database = outside.join("trust.sqlite");
        let before = fs::read(&external_database).unwrap();
        fs::create_dir(&state).unwrap();
        let store = SqliteTrustRevisionStore::new(&state).unwrap();
        fs::hard_link(&external_database, state.join(name)).unwrap();
        assert_eq!(
            SqliteTrustRevisionStore::new(&state).unwrap_err().code,
            "linked_state_path",
            "{name}"
        );
        assert_eq!(
            store.highest_revision().unwrap_err().code,
            "linked_state_path",
            "{name}"
        );
        assert_eq!(
            store
                .remember_verified_revision(&verified(9))
                .unwrap_err()
                .code,
            "linked_state_path",
            "{name}"
        );
        assert_eq!(fs::read(&external_database).unwrap(), before, "{name}");
        if name != "trust.sqlite" {
            assert!(!state.join("trust.sqlite").exists(), "{name}");
        }
    }
}
