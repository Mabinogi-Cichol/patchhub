use base64::{engine::general_purpose::STANDARD, Engine};
use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};
use std::{fs, path::Path, sync::atomic::AtomicBool};
use steam_patch_core::trust::*;
use steam_patch_core::{
    catalog::{Catalog, Fingerprint, PatchRelease, Platform, SupportLevel},
    install::{InstallOperation, InstallPlan, PlannedChange, Transaction, TransactionSnapshot},
    projection::*,
    sources::{
        self,
        model::{DiscoveryRecord, SourceCandidate},
        registry::DiscoveryOutcome,
        snapshot::SourceSnapshot,
    },
    steam,
};

fn fixture() -> (tempfile::TempDir, steam::ScanReport) {
    let temp = tempfile::tempdir().unwrap();
    let library = temp.path().join("Steam");
    fs::create_dir_all(library.join("steamapps/common/Test")).unwrap();
    fs::write(library.join("steamapps/appmanifest_42.acf"),r#""AppState" { "appid" "42" "name" "Test" "installdir" "Test" "buildid" "100" "StateFlags" "4" }"#).unwrap();
    let report = steam::scan(&[library]);
    (temp, report)
}
fn catalog() -> Catalog {
    Catalog {
        schema_version: 1,
        revision: 5,
        releases: vec![],
    }
}
fn inputs<'a>(
    steam: &'a steam::ScanReport,
    catalog: &'a Catalog,
    source: Option<&'a SourceSnapshot>,
    transactions: &'a TransactionSnapshot,
    capabilities: &'a HostCapabilities,
    cancel: &'a AtomicBool,
) -> ProjectionInputs<'a> {
    ProjectionInputs {
        steam,
        catalog: CatalogView::Development(catalog),
        sources: source,
        transactions,
        roots: &[],
        now: source.map_or(1500, |s| s.now_unix()),
        platform: Platform::Windows,
        capabilities,
        cancel,
    }
}
fn candidate() -> SourceCandidate {
    serde_json::from_value(serde_json::json!({"candidate_id":"candidate-000000000000000000000000","source_id":"kagura-cn","source_record_id":"https://kagurafan.com/patches/game","game_appid":42,"parent_appid":null,"game_title":"Test","patch_title":"Test patch","variant":"base","requires_dlc":null,"excludes_dlc":null,"platforms":["steam"],"patch_kind":"adult_content","languages":["zh-CN"],"version_raw":"1","version_identity":"1","published_at":null,"patch_updated_at":null,"source_page":"https://kagurafan.com/patches/game","upstream_page":null,"mirrors":[{"url":"https://kaguraserver.com/a.zip","label":null,"format":"zip"}],"format_reported":"zip","format_observed":"zip","capability":"discover_only","match_basis":"appid","observed_at":"2026-01-01T00:00:00+00:00","cache_status":"cached","fingerprint":"f".repeat(64)})).unwrap()
}
fn seed(state: &Path, candidate: SourceCandidate) -> u64 {
    let mut handle = sources::Sources::open(state).unwrap();
    let rule = handle
        .registry()
        .rules()
        .iter()
        .find(|r| r.id == "kagura-cn")
        .unwrap()
        .clone();
    handle
        .registry_mut()
        .commit(
            &rule,
            42,
            DiscoveryOutcome {
                status: "complete".into(),
                candidates: vec![candidate],
                ..Default::default()
            },
            &DiscoveryRecord::default(),
            "key",
        )
        .unwrap();
    sources::Clock::now_unix(&sources::SystemClock)
}
fn load(state: &Path, steam: &steam::ScanReport, now: u64) -> SourceSnapshot {
    let queries: Vec<_> = steam
        .installs
        .iter()
        .map(|g| sources::GameQuery {
            appid: g.appid,
            name: g.name.clone(),
            localized_name: g.localized_name.clone(),
            instance_id: Some(g.instance_id.clone()),
        })
        .collect();
    SourceSnapshot::load(state, &queries, now).unwrap().unwrap()
}
fn release() -> PatchRelease {
    PatchRelease {
        patch_id: "p".into(),
        release_id: "source-kagura-cn-42".into(),
        appid: 42,
        name: "Signed priority".into(),
        version: "1".into(),
        language: "zh-CN".into(),
        category: "adult_content".into(),
        source_page: "https://kagurafan.com/patches/game".into(),
        support_level: SupportLevel::ManagedZip,
        platforms: vec![Platform::Windows],
        verified_buildids: vec!["100".into()],
        detection: vec![Fingerprint {
            path: "data.txt".into(),
            sha256: format!("{:x}", Sha256::digest(b"patched")),
        }],
        recipe: Some(steam_patch_core::catalog::Recipe {
            artifact_sha256: "a".repeat(64),
            files: vec![steam_patch_core::catalog::FileMapping {
                source: "data.txt".into(),
                destination: "data.txt".into(),
                original_sha256: None,
                installed_sha256: format!("{:x}", Sha256::digest(b"patched")),
            }],
        }),
        artifact: None,
    }
}
fn verified(catalog: Catalog) -> VerifiedCatalog {
    let key = SigningKey::from_bytes(&[7; 32]);
    let payload = serde_json::to_vec(&CatalogPayload {
        catalog,
        generated_at: 1000,
        expires_at: 2000,
        revoked_release_ids: vec![],
    })
    .unwrap();
    let envelope = SignedEnvelope {
        schema_version: 1,
        key_id: "test".into(),
        payload: STANDARD.encode(&payload),
        signature: STANDARD.encode(key.sign(&payload).to_bytes()),
    };
    let keys = TrustKeys {
        schema_version: 1,
        keys: vec![TrustKey {
            key_id: "test".into(),
            public_key: STANDARD.encode(key.verifying_key().to_bytes()),
            revoked: false,
        }],
    };
    VerifiedCatalog::from_bytes(&serde_json::to_vec(&envelope).unwrap(), &keys, 1500, 0).unwrap()
}
fn records(
    game: &steam::GameInstall,
    selection: &str,
    state: &str,
    time: u64,
    path: &str,
) -> (Transaction, InstallPlan) {
    let id = uuid::Uuid::new_v4().to_string();
    let t = Transaction {
        transaction_id: id.clone(),
        plan_id: id.clone(),
        instance_id: game.instance_id.clone(),
        release_id: "exe-content-test".into(),
        state: state.into(),
        error: None,
        created_at: time,
        updated_at: time,
        has_replacements: false,
        selection_id: Some(selection.into()),
        content_id: Some("exe-content-test".into()),
    };
    let p = InstallPlan {
        operation: InstallOperation::Overwrite,
        recipe_sha256: None,
        plan_id: id,
        instance_id: game.instance_id.clone(),
        release_id: t.release_id.clone(),
        catalog_revision: 0,
        created_at: time,
        expires_at: time + 900,
        install: game.clone(),
        changes: vec![PlannedChange {
            path: path.into(),
            action: "add".into(),
            original_sha256: None,
            installed_sha256: format!("{:x}", Sha256::digest(b"patched")),
            size: 7,
            source: None,
        }],
        disk_requirement: 7,
        space_budget: None,
        blockers: vec![],
        preconditions: vec![],
    };
    (t, p)
}
#[test]
fn batch_order_dedup_and_existing_steam_snapshot() {
    let (_temp, mut steam) = fixture();
    let mut second = steam.installs[0].clone();
    second.instance_id = "second-library-instance".into();
    steam.installs.push(second);
    let c = catalog();
    let tx = TransactionSnapshot::default();
    let caps = HostCapabilities::default();
    let cancel = AtomicBool::new(false);
    let i = inputs(&steam, &c, None, &tx, &caps, &cancel);
    let ids = vec![
        "second-library-instance".into(),
        steam.installs[0].instance_id.clone(),
        "second-library-instance".into(),
    ];
    let rows = project_batch(&i, &ids).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].game.instance_id, ids[0]);
    assert_eq!(rows[1].game.instance_id, ids[1]);
    assert!(project_batch(&i, &[]).unwrap().is_empty());
    assert_eq!(
        project_instance(&i, "unknown").unwrap_err().code,
        "instance_not_found"
    );
    let report = scan(i).unwrap();
    assert_eq!(report.installs.len(), 2);
    assert!(report.sources.is_none());
    assert_eq!(
        report.installs[0].game.instance_id,
        steam.installs[0].instance_id
    );
}
#[test]
fn signed_priority_zip_and_exe_and_injected_expiration() {
    for exe in [false, true] {
        let (temp, steam) = fixture();
        let state = temp.path().join("state");
        let mut candidate = candidate();
        if exe {
            candidate.mirrors[0].format = "exe".into();
            candidate.mirrors[0].url = "https://kaguraserver.com/a.exe".into();
        }
        seed(&state, candidate);
        let source = load(&state, &steam, 1500);
        let c = Catalog {
            releases: vec![release()],
            ..catalog()
        };
        let signed = verified(c.clone());
        let tx = TransactionSnapshot::default();
        let caps = HostCapabilities::default();
        let cancel = AtomicBool::new(false);
        let mut i = inputs(&steam, &c, Some(&source), &tx, &caps, &cancel);
        i.catalog = CatalogView::Verified(&signed);
        let row = project_instance(&i, &steam.installs[0].instance_id).unwrap();
        assert_eq!(row.patches.len(), 1);
        assert_eq!(row.patches[0].name, "Signed priority");
        assert!(row.patches[0].selection_id.is_none());
        let expired_source = load(&state, &steam, 2000);
        i.now = 2000;
        i.sources = Some(&expired_source);
        let row = project_instance(&i, &steam.installs[0].instance_id).unwrap();
        assert!(row.patches[0]
            .blockers
            .iter()
            .any(|b| b == "catalog_expired"));
    }
}
#[test]
fn frozen_subscriptions_and_shared_stale_codes() {
    let (temp, steam) = fixture();
    let state = temp.path().join("state");
    let now = seed(&state, candidate());
    let source = load(&state, &steam, now);
    let c = catalog();
    let tx = TransactionSnapshot::default();
    let caps = HostCapabilities::default();
    let cancel = AtomicBool::new(false);
    let first = project_instance(
        &inputs(&steam, &c, Some(&source), &tx, &caps, &cancel),
        &steam.installs[0].instance_id,
    )
    .unwrap();
    assert!(first.patches[0].actions.automatic.available);
    let mut handle = sources::Sources::open(&state).unwrap();
    handle
        .registry_mut()
        .set_game_source(42, "kagura-cn", false)
        .unwrap();
    drop(handle);
    let old = project_instance(
        &inputs(&steam, &c, Some(&source), &tx, &caps, &cancel),
        &steam.installs[0].instance_id,
    )
    .unwrap();
    assert!(old.patches[0].actions.automatic.available);
    let changed = load(&state, &steam, now);
    let row = project_instance(
        &inputs(&steam, &c, Some(&changed), &tx, &caps, &cancel),
        &steam.installs[0].instance_id,
    )
    .unwrap();
    assert!(row.patches.is_empty());
    assert!(
        !row.sources
            .as_ref()
            .unwrap()
            .iter()
            .find(|s| s.source.rule.id == "kagura-cn")
            .unwrap()
            .subscribed
    );
    let mut handle = sources::Sources::open(&state).unwrap();
    handle
        .registry_mut()
        .set_game_source(42, "kagura-cn", true)
        .unwrap();
    drop(handle);
    let stale = load(&state, &steam, now + sources::DISCOVERY_SUCCESS_TTL + 1);
    let row = project_instance(
        &inputs(&steam, &c, Some(&stale), &tx, &caps, &cancel),
        &steam.installs[0].instance_id,
    )
    .unwrap();
    assert!(row.patches[0]
        .blockers
        .iter()
        .any(|b| b == "selection_stale"));
}
#[test]
fn selection_instance_live_bytes_and_superseded_binding() {
    let (temp, steam) = fixture();
    let state = temp.path().join("state");
    let candidate = candidate();
    let selection = steam_patch_core::installer::zip_selection_id(&candidate).unwrap();
    let now = seed(&state, candidate);
    let source = load(&state, &steam, now);
    let game = &steam.installs[0];
    let c = catalog();
    let caps = HostCapabilities::default();
    let cancel = AtomicBool::new(false);
    fs::write(game.install_path.join("data.txt"), b"patched").unwrap();
    let (t, p) = records(game, &selection, "committed", 20, "data.txt");
    let (superseded, old) = records(game, &selection, "superseded", 40, "wrong.txt");
    let tx = TransactionSnapshot::from_records(vec![t, superseded], vec![p, old]);
    let row = project_instance(
        &inputs(&steam, &c, Some(&source), &tx, &caps, &cancel),
        &game.instance_id,
    )
    .unwrap();
    assert_eq!(row.patches[0].detection.status, "installed_managed");
    fs::write(game.install_path.join("data.txt"), b"modified").unwrap();
    let row = project_instance(
        &inputs(&steam, &c, Some(&source), &tx, &caps, &cancel),
        &game.instance_id,
    )
    .unwrap();
    assert_eq!(row.patches[0].detection.status, "partial_or_conflict");
    assert_eq!(row.patches[0].detection.files[0].status, "mismatch");
    fs::remove_file(game.install_path.join("data.txt")).unwrap();
    let row = project_instance(
        &inputs(&steam, &c, Some(&source), &tx, &caps, &cancel),
        &game.instance_id,
    )
    .unwrap();
    assert_eq!(row.patches[0].detection.files[0].status, "missing");
    for other in [false, true] {
        let (mut t, p) = records(game, "wrong-selection", "committed", 50, "data.txt");
        if other {
            t.instance_id = "other-instance".into();
            t.selection_id = Some(selection.clone());
        }
        let tx = TransactionSnapshot::from_records(vec![t], vec![p]);
        let row = project_instance(
            &inputs(&steam, &c, Some(&source), &tx, &caps, &cancel),
            &game.instance_id,
        )
        .unwrap();
        assert_eq!(row.patches[0].detection.status, "unknown");
    }
}
#[test]
fn newest_unreadable_empty_or_unsafe_plan_is_unknown() {
    let (temp, steam) = fixture();
    let state = temp.path().join("state");
    let candidate = candidate();
    let selection = steam_patch_core::installer::zip_selection_id(&candidate).unwrap();
    let now = seed(&state, candidate);
    let source = load(&state, &steam, now);
    let game = &steam.installs[0];
    fs::write(game.install_path.join("data.txt"), b"patched").unwrap();
    let c = catalog();
    let caps = HostCapabilities::default();
    let cancel = AtomicBool::new(false);
    for mode in 0..3 {
        let (old, old_plan) = records(game, &selection, "committed", 10, "data.txt");
        let (new, mut new_plan) = records(game, &selection, "committed", 20, "../escape.txt");
        let mut plans = vec![old_plan];
        if mode == 1 {
            new_plan.changes.clear();
        }
        if mode != 0 {
            plans.push(new_plan);
        }
        let tx = TransactionSnapshot::from_records(vec![old, new], plans);
        let row = project_instance(
            &inputs(&steam, &c, Some(&source), &tx, &caps, &cancel),
            &game.instance_id,
        )
        .unwrap();
        assert_eq!(row.patches[0].detection.status, "unknown");
        assert!(!row.diagnostics.is_empty());
        assert_eq!(
            row.patches[0].actions.primary.label,
            steam_patch_core::actions::PrimaryLabel::Overwrite
        );
        assert!(!row.patches[0].actions.reinstall.available);
        if mode == 2 {
            assert_eq!(
                row.patches[0].detection.files[0].status,
                "unreadable_or_unsafe"
            );
        }
    }
}
#[test]
fn preview_scan_absent_state_stays_absent() {
    let (temp, steam) = fixture();
    let state = temp.path().join("absent");
    let tx = TransactionSnapshot::load_ro(&state, &steam).unwrap();
    assert!(!state.exists());
    let c = Catalog {
        releases: vec![release()],
        ..catalog()
    };
    let caps = HostCapabilities::default();
    let cancel = AtomicBool::new(false);
    let i = inputs(&steam, &c, None, &tx, &caps, &cancel);
    assert!(scan(inputs(&steam, &c, None, &tx, &caps, &cancel)).is_ok());
    let p = preview(
        &i,
        PreviewRequest {
            instance_id: steam.installs[0].instance_id.clone(),
            target: PreviewTarget::SignedRelease(release().release_id),
        },
    )
    .unwrap();
    assert!(!p.executable);
    assert!(!state.exists());
    assert!(preview(
        &i,
        PreviewRequest {
            instance_id: steam.installs[0].instance_id.clone(),
            target: PreviewTarget::PublisherZipSelection("bad".into())
        }
    )
    .is_err());
}
#[test]
fn snapshot_overlap_and_cancel_are_rejected() {
    let (_temp, steam) = fixture();
    let state = steam.installs[0].install_path.join("state");
    assert_eq!(
        TransactionSnapshot::load_ro(&state, &steam)
            .unwrap_err()
            .to_string(),
        "state_overlaps_game"
    );
    let c = catalog();
    let tx = TransactionSnapshot::default();
    let caps = HostCapabilities::default();
    let cancel = AtomicBool::new(true);
    assert_eq!(
        scan(inputs(&steam, &c, None, &tx, &caps, &cancel))
            .unwrap_err()
            .code,
        "cancelled"
    );
}
#[test]
fn corrupt_journal_remains_unmodified_with_diagnostic() {
    let (temp, steam) = fixture();
    let state = temp.path().join("state");
    let root = state.join("installer/rust-v1");
    fs::create_dir_all(&root).unwrap();
    let db = root.join("journal.sqlite");
    fs::write(&db, b"corrupt").unwrap();
    let tx = TransactionSnapshot::load_ro(&state, &steam).unwrap();
    assert!(!tx.diagnostics().is_empty());
    assert_eq!(fs::read(db).unwrap(), b"corrupt");
    assert!(!root.join("journal.lock").exists());
}

#[test]
fn corrupted_latest_database_plan_preserves_receipt_or_suppresses_unknown_identity() {
    for corrupt_receipt in [false, true] {
        let (temp, steam) = fixture();
        let state = temp.path().join("state");
        let candidate = candidate();
        let selection = steam_patch_core::installer::zip_selection_id(&candidate).unwrap();
        let now = seed(&state, candidate);
        let source = load(&state, &steam, now);
        let game = &steam.installs[0];
        fs::write(game.install_path.join("data.txt"), b"patched").unwrap();
        let root = state.join("installer/rust-v1");
        fs::create_dir_all(&root).unwrap();
        let db = rusqlite::Connection::open(root.join("journal.sqlite")).unwrap();
        db.execute_batch("CREATE TABLE jobs(id TEXT PRIMARY KEY, document TEXT NOT NULL)")
            .unwrap();
        let (old, p) = records(game, &selection, "committed", 10, "data.txt");
        let (new, _) = records(game, &selection, "committed", 20, "data.txt");
        let old_doc = serde_json::json!({"plan":p,"transaction":old,"progress":["installed"],"created_dirs":[]});
        let new_doc = if corrupt_receipt {
            serde_json::json!({"plan":{},"transaction":{}})
        } else {
            serde_json::json!({"plan":{},"transaction":new})
        };
        db.execute(
            "INSERT INTO jobs(id,document) VALUES(?1,?2)",
            rusqlite::params![old.plan_id, old_doc.to_string()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO jobs(id,document) VALUES(?1,?2)",
            rusqlite::params![new.plan_id, new_doc.to_string()],
        )
        .unwrap();
        drop(db);
        let tx = TransactionSnapshot::load_ro(&state, &steam).unwrap();
        assert!(!tx.diagnostics().is_empty());
        assert_eq!(tx.receipts_complete(), !corrupt_receipt);
        let c = catalog();
        let caps = HostCapabilities::default();
        let cancel = AtomicBool::new(false);
        let row = project_instance(
            &inputs(&steam, &c, Some(&source), &tx, &caps, &cancel),
            &game.instance_id,
        )
        .unwrap();
        assert_eq!(row.patches[0].detection.status, "unknown");
    }
}
#[test]
fn source_snapshot_time_must_match_projection_clock() {
    let (temp, steam) = fixture();
    let state = temp.path().join("state");
    let now = seed(&state, candidate());
    let source = load(&state, &steam, now);
    let c = catalog();
    let tx = TransactionSnapshot::default();
    let caps = HostCapabilities::default();
    let cancel = AtomicBool::new(false);
    let mut i = inputs(&steam, &c, Some(&source), &tx, &caps, &cancel);
    i.now += 1;
    assert_eq!(scan(i).unwrap_err().code, "invalid_snapshot_time");
}
#[test]
fn zip_and_exe_selection_previews_are_typed_and_read_only() {
    for exe in [false, true] {
        let (temp, steam) = fixture();
        let state = temp.path().join("state");
        let mut candidate = candidate();
        if exe {
            candidate.mirrors[0].format = "exe".into();
            candidate.mirrors[0].url = "https://kaguraserver.com/a.exe".into();
        }
        let selection = if exe {
            steam_patch_core::installer::exe_resolve::exe_selection_id(&candidate)
        } else {
            steam_patch_core::installer::zip_selection_id(&candidate)
        }
        .unwrap();
        let now = seed(&state, candidate);
        let source = load(&state, &steam, now);
        let game = &steam.installs[0];
        let (t, p) = records(game, &selection, "committed", 20, "data.txt");
        let tx = TransactionSnapshot::from_records(vec![t], vec![p]);
        let c = catalog();
        let caps = HostCapabilities {
            zip: LinkCapability {
                available: true,
                blockers: vec![],
            },
            exe: LinkCapability {
                available: true,
                blockers: vec![],
            },
        };
        let cancel = AtomicBool::new(false);
        let i = inputs(&steam, &c, Some(&source), &tx, &caps, &cancel);
        let target = if exe {
            PreviewTarget::PublisherExeSelection(selection.clone())
        } else {
            PreviewTarget::PublisherZipSelection(selection.clone())
        };
        let p = preview(
            &i,
            PreviewRequest {
                instance_id: game.instance_id.clone(),
                target,
            },
        )
        .unwrap();
        assert!(!p.executable);
        assert_eq!(p.selection_id.as_deref(), Some(selection.as_str()));
        if exe {
            assert_eq!(p.blockers, vec!["exe_check_required"]);
            assert!(p.changes.is_empty());
        } else {
            assert!(p.blockers.is_empty());
            assert_eq!(p.changes[0].path, "data.txt");
            assert_eq!(p.changes[0].baseline_matches, None);
        }
        assert!(!state.join("installer").exists());
        assert!(!game.install_path.join("data.txt").exists());
    }
}

#[test]
fn unreadable_preview_baseline_is_null_and_diagnostic_not_conflict() {
    let (_temp, steam) = fixture();
    let game = &steam.installs[0];
    fs::create_dir(game.install_path.join("data.txt")).unwrap();
    let c = Catalog {
        releases: vec![release()],
        ..catalog()
    };
    let tx = TransactionSnapshot::default();
    let caps = HostCapabilities::default();
    let cancel = AtomicBool::new(false);
    let report = preview(
        &inputs(&steam, &c, None, &tx, &caps, &cancel),
        PreviewRequest {
            instance_id: game.instance_id.clone(),
            target: PreviewTarget::SignedRelease(release().release_id),
        },
    )
    .unwrap();
    assert_eq!(report.changes[0].baseline_matches, None);
    assert_eq!(report.changes[0].actual_sha256, None);
    assert!(report
        .diagnostics
        .iter()
        .any(|d| d.code == "evidence_not_regular_file"));
    assert!(!report
        .blockers
        .iter()
        .any(|b| b.starts_with("baseline_conflict:")));
}

#[test]
fn damaged_newer_overlay_timestamp_never_falls_back_to_older_receipt() {
    let (temp, steam) = fixture();
    let state = temp.path().join("state");
    let selected = candidate();
    let selection = steam_patch_core::installer::zip_selection_id(&selected).unwrap();
    let now = seed(&state, selected);
    let source = load(&state, &steam, now);
    let game = &steam.installs[0];
    fs::write(game.install_path.join("data.txt"), b"patched").unwrap();
    let root = state.join("installer/rust-v1");
    fs::create_dir_all(&root).unwrap();
    let db = rusqlite::Connection::open(root.join("journal.sqlite")).unwrap();
    db.execute_batch(
        "CREATE TABLE jobs(id TEXT PRIMARY KEY, document TEXT NOT NULL);
        CREATE TABLE job_progress(job_id TEXT,item INTEGER,state TEXT,updated_at INTEGER);
        CREATE TABLE job_directories(job_id TEXT,item INTEGER,path TEXT);",
    )
    .unwrap();
    let (old, old_plan) = records(game, &selection, "committed", 20, "data.txt");
    let (new, new_plan) = records(game, &selection, "committed", 10, "data.txt");
    for (t, p, format) in [(&old, old_plan, 0), (&new, new_plan, 1)] {
        let document = serde_json::json!({"plan":p,"transaction":t,"progress":["installed"],"created_dirs":[],"progress_format":format});
        db.execute(
            "INSERT INTO jobs(id,document) VALUES(?1,?2)",
            rusqlite::params![t.plan_id, document.to_string()],
        )
        .unwrap();
    }
    db.execute(
        "INSERT INTO job_progress VALUES(?1,99,'installed',30)",
        [&new.plan_id],
    )
    .unwrap();
    drop(db);
    let tx = TransactionSnapshot::load_ro(&state, &steam).unwrap();
    assert_eq!(
        tx.transactions()
            .iter()
            .find(|t| t.plan_id == new.plan_id)
            .unwrap()
            .updated_at,
        30
    );
    assert!(tx.plan(&new.plan_id).is_none());
    let c = catalog();
    let caps = HostCapabilities::default();
    let cancel = AtomicBool::new(false);
    let row = project_instance(
        &inputs(&steam, &c, Some(&source), &tx, &caps, &cancel),
        &game.instance_id,
    )
    .unwrap();
    assert_eq!(row.patches[0].detection.status, "unknown");
}

#[test]
fn scanner_generated_long_identity_is_preserved() {
    let (_temp, mut steam) = fixture();
    steam.installs[0].instance_id = "库".repeat(100);
    let c = catalog();
    let tx = TransactionSnapshot::default();
    let caps = HostCapabilities::default();
    let cancel = AtomicBool::new(false);
    let report = scan(inputs(&steam, &c, None, &tx, &caps, &cancel)).unwrap();
    assert_eq!(
        report.installs[0].game.instance_id,
        steam.installs[0].instance_id
    );
}

// P4 action fixtures use the actual projection rather than client permissions.
fn signed_records(
    game: &steam::GameInstall,
    release: &PatchRelease,
    state: &str,
) -> (Transaction, InstallPlan) {
    let (mut t, mut p) = records(game, "unused-selection", state, 20, "data.txt");
    t.release_id = release.release_id.clone();
    t.selection_id = None;
    t.content_id = None;
    p.release_id = release.release_id.clone();
    p.recipe_sha256 = Some(steam_patch_core::install::recipe_identity(release).unwrap());
    (t, p)
}
fn has_action_blocker(
    availability: &steam_patch_core::actions::ActionAvailability,
    code: steam_patch_core::actions::ActionBlockerCode,
) -> bool {
    availability.blockers.iter().any(|b| b.code == code)
}
#[test]
fn signed_actions_receipt_and_input_truth_table() {
    use steam_patch_core::actions::{ActionBlockerCode::*, InstallInputKind, PrimaryLabel};
    // No receipt, external, partial, complete managed, damaged managed, content
    // changed, build changed and historical superseded all remain distinct.
    for case in [
        "new",
        "external",
        "partial",
        "managed",
        "damaged",
        "content",
        "build",
        "superseded",
        "unreadable_plan",
        "unreadable_missing",
    ] {
        let (_temp, steam) = fixture();
        let game = &steam.installs[0];
        let r = release();
        if !matches!(case, "new" | "damaged" | "unreadable_missing") {
            fs::write(
                game.install_path.join("data.txt"),
                if case == "partial" {
                    b"changed".as_slice()
                } else {
                    b"patched".as_slice()
                },
            )
            .unwrap();
        }
        let (t, mut p) = signed_records(
            game,
            &r,
            if case == "superseded" {
                "superseded"
            } else {
                "committed"
            },
        );
        if case == "content" {
            p.recipe_sha256 = Some("b".repeat(64));
        }
        if case == "build" {
            p.install.buildid = Some("older".into());
        }
        let tx = if matches!(case, "new" | "external" | "partial") {
            TransactionSnapshot::default()
        } else {
            TransactionSnapshot::from_records(
                vec![t],
                if matches!(case, "unreadable_plan" | "unreadable_missing") {
                    vec![]
                } else {
                    vec![p]
                },
            )
        };
        let c = Catalog {
            releases: vec![r],
            ..catalog()
        };
        let signed = verified(c.clone());
        let caps = HostCapabilities::default();
        let cancel = AtomicBool::new(false);
        let mut i = inputs(&steam, &c, None, &tx, &caps, &cancel);
        i.catalog = CatalogView::Verified(&signed);
        let row = project_instance(&i, &game.instance_id).unwrap();
        let a = &row.patches[0].actions;
        assert!(!a.automatic.available, "{case}");
        assert!(has_action_blocker(
            &a.automatic,
            ArtifactRequiresManualDownload
        ));
        assert!(a.local_archive.available, "{case}");
        assert!(a.overwrite.available, "{case}");
        assert_eq!(a.primary.input, InstallInputKind::LocalArchive);
        assert_eq!(a.reinstall.available, case == "managed", "{case}");
        if matches!(case, "content" | "build") {
            assert!(has_action_blocker(
                &a.reinstall,
                ReinstallContentIdentityChanged
            ));
        } else if case != "managed" {
            assert!(has_action_blocker(
                &a.reinstall,
                ReinstallUnownedOrIncomplete
            ));
        }
        assert_eq!(
            a.primary.label,
            if case == "new" {
                PrimaryLabel::Install
            } else {
                PrimaryLabel::Overwrite
            },
            "{case}"
        );
    }
}
#[test]
fn signed_trust_links_and_recipe_never_bypassed_by_matching_files() {
    use steam_patch_core::actions::ActionBlockerCode::*;
    for case in [
        "unsigned",
        "expired",
        "identity",
        "unavailable_link",
        "platform",
        "build",
        "game",
    ] {
        let (_temp, mut steam) = fixture();
        let mut r = release();
        fs::write(steam.installs[0].install_path.join("data.txt"), b"patched").unwrap();
        if case == "platform" {
            r.platforms = vec![Platform::Steamos];
        }
        if case == "build" {
            r.verified_buildids = vec!["other".into()];
        }
        if case == "game" {
            steam.installs[0].availability = "downloading".into();
        }
        let game = &steam.installs[0];
        let c = Catalog {
            releases: vec![r],
            ..catalog()
        };
        let signed = verified(c.clone());
        let tx = TransactionSnapshot::default();
        let mut caps = HostCapabilities::default();
        if case == "identity" {
            caps.zip.available = false;
            caps.zip.blockers = vec![DomainError::code("installer_host_requires_non_root")];
        }
        if case == "unavailable_link" {
            caps.zip.available = false;
        }
        let cancel = AtomicBool::new(false);
        let mut i = inputs(&steam, &c, None, &tx, &caps, &cancel);
        if case != "unsigned" {
            i.catalog = CatalogView::Verified(&signed);
        }
        if case == "expired" {
            i.now = 2000;
        }
        let row = project_instance(&i, &game.instance_id).unwrap();
        let a = &row.patches[0].actions;
        assert!(
            !a.automatic.available
                && !a.local_archive.available
                && !a.reinstall.available
                && !a.overwrite.available,
            "{case}"
        );
        let expected = match case {
            "unsigned" => UnsignedCatalog,
            "expired" => CatalogExpired,
            "identity" => InstallerHostRequiresNonRoot,
            "unavailable_link" => SourceRequiresGuidance,
            "platform" => PlatformNotVerified,
            "build" => BuildNotVerified,
            _ => GameNotReady,
        };
        assert!(has_action_blocker(&a.local_archive, expected), "{case}");
    }
}
#[test]
fn recovery_blocks_all_current_instance_patches_but_other_instances_are_independent() {
    use steam_patch_core::actions::ActionBlockerCode::NeedsRecovery;
    for same_instance in [false, true] {
        let (_temp, steam) = fixture();
        let game = &steam.installs[0];
        let r = release();
        let (mut t, _) = signed_records(game, &r, "needs_recovery");
        t.release_id = "different-patch".into();
        if !same_instance {
            t.instance_id = "another-library".into();
        }
        let tx = TransactionSnapshot::from_records(vec![t], vec![]);
        let c = Catalog {
            releases: vec![r],
            ..catalog()
        };
        let signed = verified(c.clone());
        let caps = HostCapabilities::default();
        let cancel = AtomicBool::new(false);
        let mut i = inputs(&steam, &c, None, &tx, &caps, &cancel);
        i.catalog = CatalogView::Verified(&signed);
        let row = project_instance(&i, &game.instance_id).unwrap();
        let a = &row.patches[0].actions;
        for availability in [&a.automatic, &a.local_archive, &a.reinstall, &a.overwrite] {
            assert_eq!(
                has_action_blocker(availability, NeedsRecovery),
                same_instance
            );
            if same_instance {
                assert!(!availability.available);
            }
        }
        assert_eq!(a.local_archive.available, !same_instance);
    }
}
#[test]
fn publisher_actions_resolve_urls_and_distinguish_format_links_and_unknown_detection() {
    use steam_patch_core::actions::{ActionBlockerCode::*, InstallInputKind};
    for case in ["zip", "exe", "bad_url", "ambiguous", "zip_link", "exe_link"] {
        let (temp, steam) = fixture();
        let state = temp.path().join("state");
        let mut candidate = candidate();
        let exe = matches!(case, "exe" | "exe_link");
        if exe {
            candidate.mirrors[0].format = "exe".into();
            candidate.mirrors[0].url = "https://kaguraserver.com/a.exe".into();
        }
        if case == "bad_url" {
            candidate.mirrors[0].url = "https://untrusted.invalid/a.zip".into();
        }
        if case == "ambiguous" {
            let mut mirror = candidate.mirrors[0].clone();
            mirror.url = "https://kaguraserver.com/b.zip".into();
            candidate.mirrors.push(mirror);
        }
        let now = seed(&state, candidate);
        let source = load(&state, &steam, now);
        let c = catalog();
        let tx = TransactionSnapshot::default();
        let mut caps = HostCapabilities::default();
        caps.exe.available = !matches!(case, "exe_link");
        caps.exe.blockers.clear();
        if case == "zip_link" {
            caps.zip.available = false;
            caps.zip.blockers = vec![DomainError::code("unprivileged_user_required")];
        }
        let cancel = AtomicBool::new(false);
        let row = project_instance(
            &inputs(&steam, &c, Some(&source), &tx, &caps, &cancel),
            &steam.installs[0].instance_id,
        )
        .unwrap();
        let p = &row.patches[0];
        let a = &p.actions;
        assert_eq!(p.detection.status, "unknown");
        assert_eq!(
            a.automatic.available,
            matches!(case, "zip" | "exe"),
            "{case}"
        );
        assert_eq!(a.overwrite.available, a.automatic.available);
        assert!(!a.local_archive.available && !a.reinstall.available);
        assert!(has_action_blocker(&a.local_archive, LocalArchiveSignedOnly));
        if case == "bad_url" {
            assert!(has_action_blocker(&a.automatic, InvalidDownloadUrl));
        }
        if case == "ambiguous" {
            assert!(has_action_blocker(&a.automatic, ZipDownloadAmbiguous));
        }
        if case == "zip_link" {
            assert!(has_action_blocker(&a.automatic, UnprivilegedUserRequired));
        }
        if case == "exe_link" {
            assert!(has_action_blocker(&a.automatic, ExeSandboxUnavailable));
        }
        assert_eq!(
            a.primary.input,
            if a.automatic.available {
                InstallInputKind::Automatic
            } else {
                InstallInputKind::Unavailable
            }
        );
        // Raw discovery mirror facts still survive; actions do not replace discovery.
        assert!(!row.discovery.unwrap().candidates[0].mirrors.is_empty());
    }
}
#[test]
fn serialized_action_stress_records_actual_jsonl_bytes_without_raising_cap() {
    let (_temp, mut steam) = fixture();
    let first = steam.installs[0].clone();
    steam.installs = (0..500)
        .map(|n| {
            let mut g = first.clone();
            g.instance_id = format!("instance-{n}");
            g
        })
        .collect();
    let tx = TransactionSnapshot::default();
    let caps = HostCapabilities {
        zip: LinkCapability {
            available: false,
            blockers: vec![DomainError::new(
                "installer_host_requires_non_root",
                "x".repeat(2048),
            )],
        },
        exe: HostCapabilities::default().exe,
    };
    let cancel = AtomicBool::new(false);
    let mut measurements = Vec::new();
    for patches in [1, 4] {
        let c = Catalog {
            releases: (0..patches)
                .map(|n| {
                    let mut r = release();
                    r.release_id = format!("stress-release-{n}");
                    r
                })
                .collect(),
            ..catalog()
        };
        let signed = verified(c.clone());
        let mut i = inputs(&steam, &c, None, &tx, &caps, &cancel);
        i.catalog = CatalogView::Verified(&signed);
        let report = scan(i).unwrap();
        let envelope = serde_json::json!({"protocol_version":4,"request_id":"stress","ok":true,"result":report});
        let bytes = serde_json::to_vec(&envelope).unwrap().len() + 1;
        measurements.push(serde_json::json!({"instances":500,"patches_per_instance":patches,"bytes_including_lf":bytes,"limit_bytes":8*1024*1024,"within_limit":bytes <= 8*1024*1024}));
    }
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.artifacts/rust-core-consolidation/p6/wire-size.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec_pretty(&serde_json::json!({"producer":"actual_core_projection","detail_bytes":2048,"protocol_version":4,"measurements":measurements})).unwrap()).unwrap();
    assert!(measurements[0]["within_limit"].as_bool().unwrap());
    assert!(!measurements[1]["within_limit"].as_bool().unwrap());
}

#[test]
fn signed_automatic_is_preferred_and_revocation_blocks_both_inputs() {
    use steam_patch_core::actions::{
        ActionBlockerCode::ReleaseRevoked, InstallInputKind, PrimaryLabel,
    };
    for revoked in [false, true] {
        let (_temp, steam) = fixture();
        let mut r = release();
        r.artifact = Some(steam_patch_core::catalog::Artifact {
            url: "https://example.invalid/patch.zip".into(),
            size: 7,
            sha256: r.recipe.as_ref().unwrap().artifact_sha256.clone(),
            allowed_redirect_hosts: vec!["example.invalid".into()],
        });
        let c = Catalog {
            releases: vec![r.clone()],
            ..catalog()
        };
        let key = SigningKey::from_bytes(&[7; 32]);
        let payload = serde_json::to_vec(&CatalogPayload {
            catalog: c.clone(),
            generated_at: 1000,
            expires_at: 2000,
            revoked_release_ids: if revoked {
                vec![r.release_id.clone()]
            } else {
                vec![]
            },
        })
        .unwrap();
        let envelope = SignedEnvelope {
            schema_version: 1,
            key_id: "test".into(),
            payload: STANDARD.encode(&payload),
            signature: STANDARD.encode(key.sign(&payload).to_bytes()),
        };
        let keys = TrustKeys {
            schema_version: 1,
            keys: vec![TrustKey {
                key_id: "test".into(),
                public_key: STANDARD.encode(key.verifying_key().to_bytes()),
                revoked: false,
            }],
        };
        let signed =
            VerifiedCatalog::from_bytes(&serde_json::to_vec(&envelope).unwrap(), &keys, 1500, 0)
                .unwrap();
        let tx = TransactionSnapshot::default();
        let caps = HostCapabilities::default();
        let cancel = AtomicBool::new(false);
        let mut i = inputs(&steam, &c, None, &tx, &caps, &cancel);
        i.catalog = CatalogView::Verified(&signed);
        let row = project_instance(&i, &steam.installs[0].instance_id).unwrap();
        let a = &row.patches[0].actions;
        assert_eq!(a.automatic.available, !revoked);
        assert_eq!(a.local_archive.available, !revoked);
        assert_eq!(a.overwrite.available, !revoked);
        assert_eq!(
            a.primary.input,
            if revoked {
                InstallInputKind::Unavailable
            } else {
                InstallInputKind::Automatic
            }
        );
        assert_eq!(a.primary.label, PrimaryLabel::Install);
        if revoked {
            assert!(has_action_blocker(&a.automatic, ReleaseRevoked));
            assert!(has_action_blocker(&a.local_archive, ReleaseRevoked));
        }
        let json = serde_json::to_value(row).unwrap();
        assert!(json["patches"][0].get("actions").is_some());
        assert!(json["patches"][0].get("download_available").is_none());
    }
}
#[test]
fn dynamic_complete_receipt_reinstalls_but_changed_selection_or_files_cannot() {
    use steam_patch_core::actions::ActionBlockerCode::*;
    for case in ["intact", "damaged", "selection", "content", "superseded"] {
        let (temp, steam) = fixture();
        let game = &steam.installs[0];
        let state = temp.path().join("state");
        let candidate = candidate();
        let selection = steam_patch_core::installer::zip_selection_id(&candidate).unwrap();
        let now = seed(&state, candidate);
        let source = load(&state, &steam, now);
        fs::write(
            game.install_path.join("data.txt"),
            if case == "damaged" {
                b"changed".as_slice()
            } else {
                b"patched".as_slice()
            },
        )
        .unwrap();
        let (mut t, mut p) = records(
            game,
            &selection,
            if case == "superseded" {
                "superseded"
            } else {
                "committed"
            },
            20,
            "data.txt",
        );
        t.release_id = "source-kagura-cn-42".into();
        t.content_id = None;
        p.release_id = t.release_id.clone();
        p.recipe_sha256 = Some("a".repeat(64));
        // The selection is unchanged but a mismatched persisted content binding
        // must still block strict reinstall.
        if case == "selection" {
            t.selection_id = Some("zip-selection-ffffffffffffffffffffffff-42".into());
        }
        if case == "content" {
            t.content_id = Some("different-content".into());
        }
        let tx = TransactionSnapshot::from_records(vec![t], vec![p]);
        let c = catalog();
        let caps = HostCapabilities::default();
        let cancel = AtomicBool::new(false);
        let row = project_instance(
            &inputs(&steam, &c, Some(&source), &tx, &caps, &cancel),
            &game.instance_id,
        )
        .unwrap();
        let a = &row.patches[0].actions;
        assert!(a.automatic.available && a.overwrite.available);
        assert_eq!(a.reinstall.available, case == "intact", "{case}");
        if case == "damaged" || case == "superseded" {
            assert!(has_action_blocker(
                &a.reinstall,
                ReinstallUnownedOrIncomplete
            ));
        }
        if case == "selection" || case == "content" {
            assert!(has_action_blocker(
                &a.reinstall,
                ReinstallContentIdentityChanged
            ));
        }
    }
}
#[test]
fn action_instance_boundary_is_checked_even_without_dynamic_file_receipts() {
    use steam_patch_core::actions::ActionBlockerCode::GameNotReady;
    let (temp, mut steam) = fixture();
    let state = temp.path().join("state");
    let now = seed(&state, candidate());
    let outside = temp.path().join("outside-game");
    fs::create_dir(&outside).unwrap();
    steam.installs[0].install_path = outside;
    let source = load(&state, &steam, now);
    let c = catalog();
    let tx = TransactionSnapshot::default();
    let caps = HostCapabilities::default();
    let cancel = AtomicBool::new(false);
    let row = project_instance(
        &inputs(&steam, &c, Some(&source), &tx, &caps, &cancel),
        &steam.installs[0].instance_id,
    )
    .unwrap();
    assert!(has_action_blocker(
        &row.patches[0].actions.automatic,
        GameNotReady
    ));
    assert!(!row.patches[0].actions.overwrite.available);
}
