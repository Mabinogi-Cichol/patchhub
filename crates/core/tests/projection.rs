use patchhub_core::{
    catalog::Platform,
    install::{InstallOperation, InstallPlan, PlannedChange, Transaction, TransactionSnapshot},
    projection::*,
    sources::{
        self,
        model::{
            GameMatchRecord, GameMatchStatus, SourceCandidate, SourceMatchRecord, SourceMatchStatus,
        },
        snapshot::SourceSnapshot,
    },
    steam,
};
use sha2::{Digest, Sha256};
use std::{fs, path::Path, sync::atomic::AtomicBool};

fn fixture() -> (tempfile::TempDir, steam::ScanReport) {
    let temp = tempfile::tempdir().unwrap();
    let library = temp.path().join("Steam");
    fs::create_dir_all(library.join("steamapps/common/Test")).unwrap();
    fs::write(library.join("steamapps/appmanifest_42.acf"),r#""AppState" { "appid" "42" "name" "Test" "installdir" "Test" "buildid" "100" "StateFlags" "4" }"#).unwrap();
    let report = steam::scan(&[library]);
    (temp, report)
}
fn inputs<'a>(
    steam: &'a steam::ScanReport,
    source: Option<&'a SourceSnapshot>,
    transactions: &'a TransactionSnapshot,
    capabilities: &'a HostCapabilities,
    cancel: &'a AtomicBool,
) -> ProjectionInputs<'a> {
    ProjectionInputs {
        steam,
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
    let mut candidate: SourceCandidate = serde_json::from_value(serde_json::json!({"candidate_id":"candidate-000000000000000000000000","source_id":"kagura-cn","source_record_id":"https://kagurafan.com/patches/game","game_appid":42,"parent_appid":null,"game_title":"Test","patch_title":"Test patch","variant":"base","requires_dlc":null,"excludes_dlc":null,"platforms":["steam"],"patch_kind":"adult_content","languages":["zh-CN"],"version_raw":"1","version_identity":"1","published_at":null,"patch_updated_at":null,"source_page":"https://kagurafan.com/patches/game","upstream_page":null,"mirrors":[{"url":"https://kaguraserver.com/a.zip","label":null,"format":"zip"}],"format_reported":"zip","format_observed":"zip","capability":"discover_only","match_basis":"appid","observed_at":"2026-01-01T00:00:00+00:00","cache_status":"cached","fingerprint":"f".repeat(64)})).unwrap();
    candidate.parser_fingerprint = sources::index::parser_fingerprint(&fixture_rule());
    candidate
}
fn fixture_rule() -> sources::SourceRule {
    let config: sources::RulesConfig = serde_json::from_str(sources::DEFAULT_RULES_JSON).unwrap();
    sources::validate::validate_rule(config.sources.iter().find(|r| r.id == "kagura-cn").unwrap())
        .unwrap()
}
fn seed(state: &Path, candidate: SourceCandidate) -> u64 {
    let handle = sources::Sources::open(state).unwrap();
    let source_id = candidate.source_id.clone();
    handle
        .store()
        .save_index(&sources::SourceIndex {
            parser_fingerprint: sources::index::parser_fingerprint(&fixture_rule()),
            schema_version: 1,
            source_id: source_id.clone(),
            revision: 1,
            complete: true,
            checked_at: 100,
            updated_at: 100,
            etag: None,
            modified: None,
            entries: vec![],
            pending: vec![],
        })
        .unwrap();
    handle
        .store()
        .save_match(&GameMatchRecord {
            schema_version: 1,
            appid: 42,
            name: "Test".into(),
            localized_name: None,
            input_key: "key".into(),
            status: GameMatchStatus::PublisherMatch,
            sources: std::collections::BTreeMap::from([(
                source_id,
                SourceMatchRecord {
                    index_revision: 1,
                    status: SourceMatchStatus::Matched,
                    candidates: vec![candidate],
                    detail_pages: Vec::new(),
                    checked_at: 100,
                    diagnostics: Vec::new(),
                },
            )]),
            fallback: None,
            updated_at: 100,
        })
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
        direct: false,
    };
    let p = InstallPlan {
        operation: InstallOperation::Overwrite,
        recipe_sha256: None,
        plan_id: id,
        instance_id: game.instance_id.clone(),
        release_id: t.release_id.clone(),
        catalog_revision: 0,
        created_at: time,
        expires_at: None,
        install: game.clone(),
        changes: vec![PlannedChange {
            path: path.into(),
            action: "add".into(),
            original_sha256: None,
            installed_sha256: Some(format!("{:x}", Sha256::digest(b"patched"))),
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
    let tx = TransactionSnapshot::default();
    let caps = HostCapabilities::default();
    let cancel = AtomicBool::new(false);
    let i = inputs(&steam, None, &tx, &caps, &cancel);
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
fn frozen_subscriptions_and_shared_stale_codes() {
    let (temp, steam) = fixture();
    let state = temp.path().join("state");
    let now = seed(&state, candidate());
    let source = load(&state, &steam, now);
    let tx = TransactionSnapshot::default();
    let caps = HostCapabilities::default();
    let cancel = AtomicBool::new(false);
    let first = project_instance(
        &inputs(&steam, Some(&source), &tx, &caps, &cancel),
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
        &inputs(&steam, Some(&source), &tx, &caps, &cancel),
        &steam.installs[0].instance_id,
    )
    .unwrap();
    assert!(old.patches[0].actions.automatic.available);
    let changed = load(&state, &steam, now);
    let row = project_instance(
        &inputs(&steam, Some(&changed), &tx, &caps, &cancel),
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
    // Advance well past the old 6h source TTL: time no longer expires a
    // subscription or invalidates the live selection (design D08/S04).
    let stale = load(&state, &steam, now + 6 * 3600 + 1);
    let row = project_instance(
        &inputs(&steam, Some(&stale), &tx, &caps, &cancel),
        &steam.installs[0].instance_id,
    )
    .unwrap();
    assert!(row.patches[0].actions.automatic.available);
    assert!(!row.patches[0]
        .blockers
        .iter()
        .any(|b| b == "selection_stale"));
}
#[test]
fn selection_instance_live_bytes_and_superseded_binding() {
    let (temp, steam) = fixture();
    let state = temp.path().join("state");
    let candidate = candidate();
    let selection = patchhub_core::installer::zip_selection_id(&candidate).unwrap();
    let now = seed(&state, candidate);
    let source = load(&state, &steam, now);
    let game = &steam.installs[0];
    let caps = HostCapabilities::default();
    let cancel = AtomicBool::new(false);
    fs::write(game.install_path.join("data.txt"), b"patched").unwrap();
    let (t, p) = records(game, &selection, "committed", 20, "data.txt");
    let (superseded, old) = records(game, &selection, "superseded", 40, "wrong.txt");
    let tx = TransactionSnapshot::from_records(vec![t, superseded], vec![p, old]);
    let row = project_instance(
        &inputs(&steam, Some(&source), &tx, &caps, &cancel),
        &game.instance_id,
    )
    .unwrap();
    assert_eq!(row.patches[0].detection.status, "installed_managed");
    fs::write(game.install_path.join("data.txt"), b"modified").unwrap();
    let row = project_instance(
        &inputs(&steam, Some(&source), &tx, &caps, &cancel),
        &game.instance_id,
    )
    .unwrap();
    assert_eq!(row.patches[0].detection.status, "partial_or_conflict");
    assert_eq!(row.patches[0].detection.files[0].status, "mismatch");
    fs::remove_file(game.install_path.join("data.txt")).unwrap();
    let row = project_instance(
        &inputs(&steam, Some(&source), &tx, &caps, &cancel),
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
            &inputs(&steam, Some(&source), &tx, &caps, &cancel),
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
    let selection = patchhub_core::installer::zip_selection_id(&candidate).unwrap();
    let now = seed(&state, candidate);
    let source = load(&state, &steam, now);
    let game = &steam.installs[0];
    fs::write(game.install_path.join("data.txt"), b"patched").unwrap();
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
            &inputs(&steam, Some(&source), &tx, &caps, &cancel),
            &game.instance_id,
        )
        .unwrap();
        assert_eq!(row.patches[0].detection.status, "unknown");
        assert!(!row.diagnostics.is_empty());
        assert_eq!(
            row.patches[0].actions.primary.label,
            patchhub_core::actions::PrimaryLabel::Overwrite
        );
        assert!(row.patches[0].actions.reinstall.available);
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
    let caps = HostCapabilities::default();
    let cancel = AtomicBool::new(false);
    let i = inputs(&steam, None, &tx, &caps, &cancel);
    assert!(scan(inputs(&steam, None, &tx, &caps, &cancel)).is_ok());
    let p = preview(
        &i,
        PreviewRequest {
            instance_id: steam.installs[0].instance_id.clone(),
            target: PreviewTarget::PublisherZipSelection(
                "zip-selection-0123456789abcdef01234567-42".into(),
            ),
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
    let tx = TransactionSnapshot::default();
    let caps = HostCapabilities::default();
    let cancel = AtomicBool::new(true);
    assert_eq!(
        scan(inputs(&steam, None, &tx, &caps, &cancel))
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
        let selection = patchhub_core::installer::zip_selection_id(&candidate).unwrap();
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
        let caps = HostCapabilities::default();
        let cancel = AtomicBool::new(false);
        let row = project_instance(
            &inputs(&steam, Some(&source), &tx, &caps, &cancel),
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
    let tx = TransactionSnapshot::default();
    let caps = HostCapabilities::default();
    let cancel = AtomicBool::new(false);
    let mut i = inputs(&steam, Some(&source), &tx, &caps, &cancel);
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
            patchhub_core::installer::exe_resolve::exe_selection_id(&candidate)
        } else {
            patchhub_core::installer::zip_selection_id(&candidate)
        }
        .unwrap();
        let now = seed(&state, candidate);
        let source = load(&state, &steam, now);
        let game = &steam.installs[0];
        let (t, p) = records(game, &selection, "committed", 20, "data.txt");
        let tx = TransactionSnapshot::from_records(vec![t], vec![p]);
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
        let i = inputs(&steam, Some(&source), &tx, &caps, &cancel);
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
            assert!(p.blockers.is_empty());
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
fn damaged_newer_overlay_timestamp_never_falls_back_to_older_receipt() {
    let (temp, steam) = fixture();
    let state = temp.path().join("state");
    let selected = candidate();
    let selection = patchhub_core::installer::zip_selection_id(&selected).unwrap();
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
    let caps = HostCapabilities::default();
    let cancel = AtomicBool::new(false);
    let row = project_instance(
        &inputs(&steam, Some(&source), &tx, &caps, &cancel),
        &game.instance_id,
    )
    .unwrap();
    assert_eq!(row.patches[0].detection.status, "unknown");
}

#[test]
fn scanner_generated_long_identity_is_preserved() {
    let (_temp, mut steam) = fixture();
    steam.installs[0].instance_id = "库".repeat(100);
    let tx = TransactionSnapshot::default();
    let caps = HostCapabilities::default();
    let cancel = AtomicBool::new(false);
    let report = scan(inputs(&steam, None, &tx, &caps, &cancel)).unwrap();
    assert_eq!(
        report.installs[0].game.instance_id,
        steam.installs[0].instance_id
    );
}

// P4 action fixtures use the actual projection rather than client permissions.
fn has_action_blocker(
    availability: &patchhub_core::actions::ActionAvailability,
    code: patchhub_core::actions::ActionBlockerCode,
) -> bool {
    availability.blockers.iter().any(|b| b.code == code)
}
#[test]
fn publisher_actions_resolve_urls_and_distinguish_format_links_and_unknown_detection() {
    use patchhub_core::actions::{ActionBlockerCode::*, InstallInputKind};
    for case in ["zip", "exe", "bad_url", "ambiguous", "zip_link"] {
        let (temp, steam) = fixture();
        let state = temp.path().join("state");
        let mut candidate = candidate();
        let exe = case == "exe";
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
        let tx = TransactionSnapshot::default();
        let mut caps = HostCapabilities::default();
        caps.exe.blockers.clear();
        if case == "zip_link" {
            caps.zip.available = false;
            caps.zip.blockers = vec![DomainError::code("unprivileged_user_required")];
        }
        let cancel = AtomicBool::new(false);
        let row = project_instance(
            &inputs(&steam, Some(&source), &tx, &caps, &cancel),
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
        assert_eq!(a.reinstall.available, a.automatic.available);
        if case == "bad_url" {
            assert!(has_action_blocker(&a.automatic, InvalidDownloadUrl));
        }
        if case == "ambiguous" {
            assert!(has_action_blocker(&a.automatic, ZipDownloadAmbiguous));
        }
        if case == "zip_link" {
            assert!(has_action_blocker(&a.automatic, UnprivilegedUserRequired));
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
fn reinstall_uses_selected_archive_even_when_old_receipts_or_files_changed() {
    for case in ["intact", "damaged", "selection", "content", "superseded"] {
        let (temp, steam) = fixture();
        let game = &steam.installs[0];
        let state = temp.path().join("state");
        let candidate = candidate();
        let selection = patchhub_core::installer::zip_selection_id(&candidate).unwrap();
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
        // Historical content bindings do not authorize the new extraction.
        if case == "selection" {
            t.selection_id = Some("zip-selection-ffffffffffffffffffffffff-42".into());
        }
        if case == "content" {
            t.content_id = Some("different-content".into());
        }
        let tx = TransactionSnapshot::from_records(vec![t], vec![p]);
        let caps = HostCapabilities::default();
        let cancel = AtomicBool::new(false);
        let row = project_instance(
            &inputs(&steam, Some(&source), &tx, &caps, &cancel),
            &game.instance_id,
        )
        .unwrap();
        let a = &row.patches[0].actions;
        assert!(a.automatic.available && a.overwrite.available);
        assert!(a.reinstall.available, "{case}");
    }
}
#[test]
fn action_instance_boundary_is_checked_even_without_dynamic_file_receipts() {
    use patchhub_core::actions::ActionBlockerCode::GameNotReady;
    let (temp, mut steam) = fixture();
    let state = temp.path().join("state");
    let now = seed(&state, candidate());
    let outside = temp.path().join("outside-game");
    fs::create_dir(&outside).unwrap();
    steam.installs[0].install_path = outside;
    let source = load(&state, &steam, now);
    let tx = TransactionSnapshot::default();
    let caps = HostCapabilities::default();
    let cancel = AtomicBool::new(false);
    let row = project_instance(
        &inputs(&steam, Some(&source), &tx, &caps, &cancel),
        &steam.installs[0].instance_id,
    )
    .unwrap();
    assert!(has_action_blocker(
        &row.patches[0].actions.automatic,
        GameNotReady
    ));
    assert!(!row.patches[0].actions.overwrite.available);
}

#[test]
fn manual_unknown_candidate_binds_download_and_instance_without_faking_metadata() {
    use patchhub_core::installer::{exe_resolve, manual_selection, resolve_zip};
    use sources::model::{ArtifactFormat, MatchBasis, Mirror, Variant};
    for format in ["zip", "exe"] {
        let (temp, steam) = fixture();
        let state = temp.path().join("state");
        let mut candidate = candidate();
        candidate.game_appid = None;
        candidate.match_basis = MatchBasis::Title;
        candidate.variant = Variant::Unknown;
        candidate.version_raw = None;
        candidate.version_identity = String::new();
        candidate.languages.clear();
        candidate.format_observed = if format == "zip" {
            ArtifactFormat::Zip
        } else {
            ArtifactFormat::Exe
        };
        candidate.format_reported = candidate.format_observed;
        candidate.mirrors = vec![
            Mirror {
                url: format!("https://kaguraserver.com/first.{format}"),
                label: None,
                format: format.into(),
            },
            Mirror {
                url: format!("https://kaguraserver.com/second.{format}"),
                label: None,
                format: format.into(),
            },
        ];
        let now = seed(&state, candidate.clone());
        let query = sources::GameQuery {
            appid: 42,
            name: "Test".into(),
            localized_name: None,
            instance_id: Some(steam.installs[0].instance_id.clone()),
        };
        let snapshot = load(&state, &steam, now + 30 * 24 * 3600);
        let source_read = sources::Sources::open(&state).unwrap();
        let id = manual_selection::selection_id(&candidate, &query, &candidate.mirrors[1]).unwrap();
        let resolved = if format == "zip" {
            resolve_zip(source_read.registry(), &query, &id)
        } else {
            exe_resolve::resolve_exe(source_read.registry(), &query, &id)
        }
        .unwrap();
        assert_eq!(resolved.download_url, candidate.mirrors[1].url);
        assert_eq!(resolved.appid, 42);
        assert_eq!(resolved.candidate.game_appid, None);
        assert_eq!(resolved.candidate.variant, Variant::Unknown);
        assert!(resolved.language.is_empty());
        let mut other = query.clone();
        other.instance_id = Some("other-instance".into());
        assert!(manual_selection::resolve(source_read.registry(), &other, &id, format).is_err());
        let mut changed = candidate.clone();
        changed.version_identity = "changed".into();
        assert_ne!(
            manual_selection::selection_id(&changed, &query, &changed.mirrors[1]).unwrap(),
            id
        );
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
        let tx = TransactionSnapshot::default();
        let cancel = AtomicBool::new(false);
        let row = project_instance(
            &inputs(&steam, Some(&snapshot), &tx, &caps, &cancel),
            &steam.installs[0].instance_id,
        )
        .unwrap();
        let manual: Vec<_> = row
            .patches
            .iter()
            .filter(|p| p.compatibility == "manual_selection_required")
            .collect();
        assert_eq!(manual.len(), 2);
        assert!(manual.iter().all(|p| p.name == candidate.patch_title));
        assert!(manual.iter().all(|p| p.actions.automatic.available));
        let mut source = sources::Sources::open(&state).unwrap();
        source
            .registry_mut()
            .set_game_source(42, "kagura-cn", false)
            .unwrap();
        assert!(manual_selection::resolve(source.registry(), &query, &id, format).is_err());
    }
}

#[test]
fn domain_error_detail_is_not_truncated() {
    let detail = "测".repeat(3000);
    let error = DomainError::new("exe_manifest_invalid", detail.clone());
    assert_eq!(error.detail.as_deref(), Some(detail.as_str()));
    assert!(error.to_string().ends_with(&detail));
}

#[test]
fn valid_target_ignores_build_state_dlc_language_and_old_recovery() {
    let (temp, mut steam) = fixture();
    let state = temp.path().join("state");
    steam.installs[0].buildid = None;
    steam.installs[0].availability = "updating".into();
    let mut candidate = candidate();
    candidate.requires_dlc = Some(vec![999]);
    candidate.languages = vec!["unmatched".into()];
    candidate.variant = sources::model::Variant::Unknown;
    candidate.cache_status = sources::model::CacheStatus::Stale;
    let now = seed(&state, candidate);
    let source = load(&state, &steam, now);
    let game = &steam.installs[0];
    let (t, p) = records(game, "old-selection", "needs_recovery", 20, "data.txt");
    let tx = TransactionSnapshot::from_records(vec![t], vec![p]);
    let caps = HostCapabilities::default();
    let cancel = AtomicBool::new(false);
    let row = project_instance(
        &inputs(&steam, Some(&source), &tx, &caps, &cancel),
        &game.instance_id,
    )
    .unwrap();
    assert!(!row.patches.is_empty());
    for patch in row.patches {
        assert!(
            patch.actions.automatic.available && patch.actions.reinstall.available,
            "{patch:?}"
        );
    }
}
