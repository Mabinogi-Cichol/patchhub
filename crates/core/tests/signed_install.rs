//! Public signed-route acceptance: the caller supplies trust bytes, never a recipe.
use base64::{engine::general_purpose::STANDARD, Engine};
use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};
use std::{
    cell::{Cell, RefCell},
    fs::{self, File},
    io::{self, Cursor, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    },
};
use steam_patch_core::{
    catalog::{
        Artifact, Catalog, FileMapping, Fingerprint, PatchRelease, Platform, Recipe, SupportLevel,
    },
    install::{self, InstallOperation},
    installer::{
        outcome::{CancellationCause, CleanupStatus, Disposition, FailureKind, InstallerOutcome},
        service::{
            self, CleanupPolicy, Clock, InstallRequest, InstallTarget, InstallerDeps, InstanceId,
            LocalArchive, ReleaseId, RequestId, TaskLease, ValidatedHostContext,
        },
        HttpResponse, Transport,
    },
    projection::DomainError,
    steam,
    trust::{
        self,
        revision::{
            SqliteTrustRevisionStore, TrustRevisionStore, TrustedCatalogMaterial,
            TrustedCatalogProvider,
        },
        CatalogPayload, SignedEnvelope, TrustKey, TrustKeys,
    },
};
use zip::{write::SimpleFileOptions, ZipWriter};

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn platform() -> Platform {
    if cfg!(windows) {
        Platform::Windows
    } else {
        Platform::Steamos
    }
}

struct FixedClock(u64);
impl Clock for FixedClock {
    fn now_seconds(&self) -> Result<u64, DomainError> {
        Ok(self.0)
    }
}
#[derive(Default)]
struct Lease(AtomicU8);
impl TaskLease for Lease {
    fn cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire) == 1
    }
    fn lease_active(&self, _: u64) -> bool {
        self.0.load(Ordering::Acquire) != 2
    }
    fn deadline_ms(&self) -> u64 {
        u64::MAX
    }
    fn cancellation_cause(&self) -> Option<CancellationCause> {
        match self.0.load(Ordering::Acquire) {
            1 => Some(CancellationCause::UserCancelled),
            2 => Some(CancellationCause::LeaseExpired),
            _ => None,
        }
    }
}
struct Provider {
    initial: Vec<u8>,
    current: Vec<u8>,
    keys: TrustKeys,
    second_keys: Option<TrustKeys>,
    calls: Cell<usize>,
    on_second: RefCell<Option<Box<dyn FnOnce()>>>,
}
impl TrustedCatalogProvider for Provider {
    fn load(&self) -> Result<TrustedCatalogMaterial, DomainError> {
        let call = self.calls.get();
        self.calls.set(call + 1);
        if call == 1 {
            if let Some(hook) = self.on_second.borrow_mut().take() {
                hook();
            }
        }
        Ok(TrustedCatalogMaterial {
            envelope: if call == 0 {
                self.initial.clone()
            } else {
                self.current.clone()
            },
            keys: if call == 0 {
                self.keys.clone()
            } else {
                self.second_keys.as_ref().unwrap_or(&self.keys).clone()
            },
        })
    }
}
struct Offline;
impl Transport for Offline {
    fn get(&self, _: &str, _: Option<&str>, _: Option<&str>) -> io::Result<HttpResponse> {
        panic!("local signed archive must not use transport")
    }
}
struct BytesTransport {
    bytes: Vec<u8>,
    redirect: Option<String>,
    calls: Cell<usize>,
}
impl Transport for BytesTransport {
    fn get(&self, url: &str, _: Option<&str>, _: Option<&str>) -> io::Result<HttpResponse> {
        self.calls.set(self.calls.get() + 1);
        Ok(HttpResponse {
            status: if self.redirect.is_some() { 302 } else { 200 },
            location: self.redirect.clone(),
            content_type: Some("application/octet-stream".into()),
            content_length: Some(self.bytes.len() as u64),
            etag: None,
            last_modified: None,
            final_url: url.into(),
            body: Box::new(Cursor::new(self.bytes.clone())),
        })
    }
}
struct Fixture {
    _temp: tempfile::TempDir,
    library: PathBuf,
    game: PathBuf,
    state: PathBuf,
    archive: PathBuf,
    release: PatchRelease,
    now: u64,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let library = temp.path().join("Steam");
        let game = library.join("steamapps/common/Signed Test");
        fs::create_dir_all(&game).unwrap();
        fs::write(game.join("original.dat"), b"original").unwrap();
        let archive = temp.path().join("patch.zip");
        let mut zip = ZipWriter::new(File::create(&archive).unwrap());
        zip.start_file(
            "payload.dat",
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
        )
        .unwrap();
        zip.write_all(b"patched").unwrap();
        zip.finish().unwrap();
        let release = PatchRelease {
            patch_id: "signed-fixture".into(),
            release_id: "signed-release".into(),
            appid: 42,
            name: "Signed test patch".into(),
            version: "1".into(),
            language: "en".into(),
            category: "content_restore".into(),
            source_page: "https://publisher.example/patch".into(),
            support_level: SupportLevel::ManagedZip,
            platforms: vec![platform()],
            verified_buildids: vec!["100".into()],
            detection: vec![Fingerprint {
                path: "original.dat".into(),
                sha256: hash(b"patched"),
            }],
            recipe: Some(Recipe {
                artifact_sha256: hash(&fs::read(&archive).unwrap()),
                files: vec![FileMapping {
                    source: "payload.dat".into(),
                    destination: "original.dat".into(),
                    original_sha256: Some(hash(b"original")),
                    installed_sha256: hash(b"patched"),
                }],
            }),
            artifact: None,
        };
        let fixture = Self {
            state: temp.path().join("state"),
            _temp: temp,
            library,
            game,
            archive,
            release,
            now: trust::now().unwrap(),
        };
        fixture.manifest("100");
        fixture
    }
    fn manifest(&self, build: &str) {
        fs::write(self.library.join("steamapps/appmanifest_42.acf"), format!(
            r#""AppState" {{ "appid" "42" "name" "Signed Test" "installdir" "Signed Test" "buildid" "{build}" "StateFlags" "4" "UserConfig" {{ "language" "english" }} }}"#
        )).unwrap();
    }
    fn envelope(
        &self,
        release: PatchRelease,
        revision: u64,
        expiry: u64,
        revoked: bool,
    ) -> Vec<u8> {
        let key = SigningKey::from_bytes(&[7; 32]);
        let bytes = serde_json::to_vec(&CatalogPayload {
            catalog: Catalog {
                schema_version: 1,
                revision,
                releases: vec![release],
            },
            generated_at: self.now - 100,
            expires_at: expiry,
            revoked_release_ids: if revoked {
                vec![self.release.release_id.clone()]
            } else {
                vec![]
            },
        })
        .unwrap();
        serde_json::to_vec(&SignedEnvelope {
            schema_version: 1,
            key_id: "fixture".into(),
            payload: STANDARD.encode(&bytes),
            signature: STANDARD.encode(key.sign(&bytes).to_bytes()),
        })
        .unwrap()
    }
    fn provider(&self) -> Provider {
        let key = SigningKey::from_bytes(&[7; 32]);
        let envelope = self.envelope(self.release.clone(), 5, self.now + 3600, false);
        Provider {
            initial: envelope.clone(),
            current: envelope,
            keys: TrustKeys {
                schema_version: 1,
                keys: vec![TrustKey {
                    key_id: "fixture".into(),
                    public_key: STANDARD.encode(key.verifying_key().to_bytes()),
                    revoked: false,
                }],
            },
            second_keys: None,
            calls: Cell::new(0),
            on_second: RefCell::new(None),
        }
    }
    fn request(&self, id: &str, local: bool, operation: InstallOperation) -> InstallRequest {
        let game = steam::scan(std::slice::from_ref(&self.library))
            .installs
            .remove(0);
        InstallRequest {
            request_id: RequestId::new(id).unwrap(),
            instance_id: InstanceId::new(&game.instance_id).unwrap(),
            operation,
            target: InstallTarget::SignedRelease {
                release_id: ReleaseId::new(&self.release.release_id).unwrap(),
                archive: local.then(|| LocalArchive::new(&self.archive).unwrap()),
            },
            cleanup_policy: CleanupPolicy::Disabled,
        }
    }
    fn run(
        &self,
        provider: &Provider,
        lease: &Lease,
        transport: &dyn Transport,
        id: &str,
        local: bool,
        operation: InstallOperation,
    ) -> Result<InstallerOutcome, DomainError> {
        let host =
            ValidatedHostContext::new(&self.state, std::slice::from_ref(&self.library), platform())
                .unwrap();
        let store = SqliteTrustRevisionStore::new(&self.state).unwrap();
        service::install(
            self.request(id, local, operation),
            InstallerDeps {
                host: &host,
                catalog: provider,
                revision_store: &store,
                transport,
                worker: None,
                control: lease,
                clock: &FixedClock(self.now),
                emit: &mut |_| {},
            },
        )
    }
    fn unchanged(&self) {
        assert_eq!(
            fs::read(self.game.join("original.dat")).unwrap(),
            b"original"
        );
    }
}

// Production deliberately rejects privileged Linux hosts. These route fixtures
// run where the public host identity is supported (Windows or non-root Linux).
fn can_install() -> bool {
    steam_patch_core::installer::host_identity_blocker().is_none()
}

#[test]
fn local_signed_without_artifact_commits_and_replays_after_cancel_and_revocation() {
    if !can_install() {
        return;
    }
    let f = Fixture::new();
    let mut p = f.provider();
    let lease = Lease::default();
    let first = f
        .run(
            &p,
            &lease,
            &Offline,
            "signed-success",
            true,
            InstallOperation::Overwrite,
        )
        .unwrap();
    assert_eq!(first.state, "committed");
    assert_eq!(first.disposition, Disposition::Completed);
    assert_eq!(first.selection_id, None);
    assert_eq!(first.content_id, None);
    assert_eq!(first.cleanup.status, CleanupStatus::NotRequested);
    assert_eq!(fs::read(f.game.join("original.dat")).unwrap(), b"patched");
    assert_eq!(
        SqliteTrustRevisionStore::new(&f.state)
            .unwrap()
            .highest_revision()
            .unwrap(),
        5
    );
    assert_eq!(p.calls.get(), 2);
    p.initial = f.envelope(f.release.clone(), 5, f.now + 3600, true);
    lease.0.store(1, Ordering::Release);
    let replay = f
        .run(
            &p,
            &lease,
            &Offline,
            "signed-success",
            true,
            InstallOperation::Overwrite,
        )
        .unwrap();
    assert_eq!(replay.transaction_id, first.transaction_id);
    assert_eq!(replay.disposition, Disposition::Completed);
    assert_eq!(
        p.calls.get(),
        2,
        "terminal replay must not reload an expired/revoked catalog"
    );
}

#[test]
fn signed_auto_download_uses_injected_transport_and_accepts_extensionless_url() {
    if !can_install() {
        return;
    }
    let mut f = Fixture::new();
    let bytes = fs::read(&f.archive).unwrap();
    f.release.artifact = Some(Artifact {
        url: "https://publisher.example/download?id=42".into(),
        size: bytes.len() as u64,
        sha256: hash(&bytes),
        allowed_redirect_hosts: vec![],
    });
    let transport = BytesTransport {
        bytes,
        redirect: None,
        calls: Cell::new(0),
    };
    let result = f
        .run(
            &f.provider(),
            &Lease::default(),
            &transport,
            "signed-auto",
            false,
            InstallOperation::Overwrite,
        )
        .unwrap();
    assert_eq!(result.disposition, Disposition::Completed);
    assert_eq!(transport.calls.get(), 1);
}

#[test]
fn signed_download_rejects_unapproved_redirect_before_a_plan() {
    if !can_install() {
        return;
    }
    let mut f = Fixture::new();
    let bytes = fs::read(&f.archive).unwrap();
    f.release.artifact = Some(Artifact {
        url: "https://publisher.example/download".into(),
        size: bytes.len() as u64,
        sha256: hash(&bytes),
        allowed_redirect_hosts: vec!["cdn.example".into()],
    });
    let transport = BytesTransport {
        bytes,
        redirect: Some("https://evil.example/a".into()),
        calls: Cell::new(0),
    };
    assert_eq!(
        f.run(
            &f.provider(),
            &Lease::default(),
            &transport,
            "signed-redirect",
            false,
            InstallOperation::Overwrite
        )
        .unwrap_err()
        .code,
        "redirect_host_not_approved"
    );
    f.unchanged();
    assert!(install::list_transactions_at(
        steam_patch_core::installer::InstallerPaths::new(&f.state).root()
    )
    .unwrap()
    .is_empty());
}

#[test]
fn manual_missing_archive_and_tampered_local_hash_remain_preplan_errors() {
    if !can_install() {
        return;
    }
    let f = Fixture::new();
    assert_eq!(
        f.run(
            &f.provider(),
            &Lease::default(),
            &Offline,
            "signed-manual",
            false,
            InstallOperation::Overwrite
        )
        .unwrap_err()
        .code,
        "artifact_requires_manual_download"
    );
    fs::write(&f.archive, b"tampered").unwrap();
    assert_eq!(
        f.run(
            &f.provider(),
            &Lease::default(),
            &Offline,
            "signed-hash",
            true,
            InstallOperation::Overwrite
        )
        .unwrap_err()
        .code,
        "artifact_hash_mismatch"
    );
    f.unchanged();
}

#[test]
fn ordinary_postplan_baseline_conflict_returns_failed_transaction_and_replays() {
    if !can_install() {
        return;
    }
    let f = Fixture::new();
    fs::write(f.game.join("original.dat"), b"user bytes").unwrap();
    let p = f.provider();
    let lease = Lease::default();
    let first = f
        .run(
            &p,
            &lease,
            &Offline,
            "signed-blocked",
            true,
            InstallOperation::Install,
        )
        .unwrap();
    assert_eq!(first.state, "rolled_back");
    assert_eq!(first.disposition, Disposition::Failed);
    assert_eq!(
        first.cause.as_ref().unwrap().kind,
        FailureKind::DomainFailure
    );
    assert!(first
        .error
        .as_deref()
        .unwrap()
        .starts_with("plan_has_blockers"));
    let replay = f
        .run(
            &p,
            &lease,
            &Offline,
            "signed-blocked",
            true,
            InstallOperation::Install,
        )
        .unwrap();
    assert_eq!(replay.transaction_id, first.transaction_id);
    assert_eq!(replay.error, first.error);
    assert_eq!(replay.disposition, Disposition::Failed);
    assert_eq!(
        fs::read(f.game.join("original.dat")).unwrap(),
        b"user bytes"
    );
}

#[test]
fn apply_reloads_trust_for_revocation_expiry_rollback_and_revoked_key() {
    if !can_install() {
        return;
    }
    for code in [
        "release_revoked",
        "catalog_expired",
        "catalog_revision_rollback",
        "signing_key_revoked",
    ] {
        let f = Fixture::new();
        let mut p = f.provider();
        p.current = f.envelope(
            f.release.clone(),
            if code == "catalog_revision_rollback" {
                4
            } else {
                5
            },
            if code == "catalog_expired" {
                f.now
            } else {
                f.now + 3600
            },
            code == "release_revoked",
        );
        if code == "signing_key_revoked" {
            let mut keys = p.keys.clone();
            keys.keys[0].revoked = true;
            p.second_keys = Some(keys);
        }
        let result = f
            .run(
                &p,
                &Lease::default(),
                &Offline,
                code,
                true,
                InstallOperation::Overwrite,
            )
            .unwrap();
        assert_eq!(p.calls.get(), 2);
        assert_eq!(result.state, "rolled_back");
        assert_eq!(result.disposition, Disposition::Failed, "{code}");
        assert_eq!(result.cause.unwrap().code, code);
        f.unchanged();
        let replay = f
            .run(
                &p,
                &Lease::default(),
                &Offline,
                code,
                true,
                InstallOperation::Overwrite,
            )
            .unwrap();
        assert_eq!(replay.transaction_id, result.transaction_id);
        assert_eq!(replay.disposition, Disposition::Failed);
        assert_eq!(p.calls.get(), 2);
    }
}

#[test]
fn apply_rechecks_release_fingerprint_and_live_manifest() {
    if !can_install() {
        return;
    }
    for change_release in [true, false] {
        let f = Fixture::new();
        let mut p = f.provider();
        if change_release {
            let mut release = f.release.clone();
            release.version = "changed".into();
            p.current = f.envelope(release, 5, f.now + 3600, false);
        } else {
            let manifest = f.library.join("steamapps/appmanifest_42.acf");
            p.on_second.replace(Some(Box::new(move || {
                let text = fs::read_to_string(&manifest)
                    .unwrap()
                    .replace("\"100\"", "\"200\"");
                fs::write(manifest, text).unwrap();
            })));
        }
        let result = f
            .run(
                &p,
                &Lease::default(),
                &Offline,
                "signed-drift",
                true,
                InstallOperation::Overwrite,
            )
            .unwrap();
        assert_eq!(result.state, "rolled_back");
        assert_eq!(result.disposition, Disposition::Failed);
        assert_eq!(
            result.cause.unwrap().code,
            if change_release {
                "authorization_rule_changed"
            } else {
                "authorization_facts_changed"
            }
        );
        f.unchanged();
    }
}

#[test]
fn postplan_user_cancel_and_lease_expiry_persist_exact_causes_and_replay() {
    if !can_install() {
        return;
    }
    for (value, code, kind) in [
        (1, "cancelled", FailureKind::UserCancelled),
        (2, "lease_expired", FailureKind::LeaseExpired),
    ] {
        let f = Fixture::new();
        let p = f.provider();
        let lease = Arc::new(Lease::default());
        let observed = lease.clone();
        p.on_second.replace(Some(Box::new(move || {
            observed.0.store(value, Ordering::Release)
        })));
        let result = f
            .run(
                &p,
                &lease,
                &Offline,
                code,
                true,
                InstallOperation::Overwrite,
            )
            .unwrap();
        assert_eq!(result.state, "rolled_back");
        assert_eq!(result.error.as_deref(), Some(code));
        assert_eq!(result.disposition, Disposition::Cancelled);
        assert_eq!(result.cause.as_ref().unwrap().kind, kind);
        f.unchanged();
        let replay = f
            .run(
                &p,
                &lease,
                &Offline,
                code,
                true,
                InstallOperation::Overwrite,
            )
            .unwrap();
        assert_eq!(replay.transaction_id, result.transaction_id);
        assert_eq!(replay.error, result.error);
        assert_eq!(replay.disposition, Disposition::Cancelled);
    }
}

#[test]
fn preplan_cancel_has_no_synthetic_outcome_or_transaction() {
    if !can_install() {
        return;
    }
    let f = Fixture::new();
    let p = f.provider();
    let lease = Lease::default();
    lease.0.store(1, Ordering::Release);
    assert_eq!(
        f.run(
            &p,
            &lease,
            &Offline,
            "signed-pre-cancel",
            true,
            InstallOperation::Overwrite
        )
        .unwrap_err()
        .code,
        "cancelled"
    );
    assert_eq!(p.calls.get(), 0);
    f.unchanged();
}

#[test]
fn terminal_ledger_write_failure_keeps_real_journal_outcome_and_replay() {
    if !can_install() {
        return;
    }
    for ordinary_failure in [false, true] {
        let f = Fixture::new();
        let mut p = f.provider();
        if ordinary_failure {
            let mut changed = f.release.clone();
            changed.version = "changed".into();
            p.current = f.envelope(changed, 5, f.now + 3600, false);
        }
        let requests = steam_patch_core::installer::InstallerPaths::new(&f.state).requests();
        p.on_second.replace(Some(Box::new(move || {
            // The plan is already bound. Reject only the final ledger summary,
            // preserving the database, its plan_id and the separate real journal.
            let db = rusqlite::Connection::open(requests).unwrap();
            db.execute_batch(
                "CREATE TRIGGER reject_terminal_summary BEFORE UPDATE ON requests
                 WHEN NEW.state IN ('committed','rolled_back')
                 BEGIN SELECT RAISE(FAIL,'fixture_terminal_summary_failed'); END;",
            )
            .unwrap();
        })));
        let lease = Lease::default();
        let result = f
            .run(
                &p,
                &lease,
                &Offline,
                "signed-ledger-failure",
                true,
                InstallOperation::Overwrite,
            )
            .unwrap();
        assert_eq!(
            result.state,
            if ordinary_failure {
                "rolled_back"
            } else {
                "committed"
            }
        );
        assert_eq!(
            result.disposition,
            if ordinary_failure {
                Disposition::Failed
            } else {
                Disposition::Completed
            }
        );
        let paths = steam_patch_core::installer::InstallerPaths::new(&f.state);
        let recorded = steam_patch_core::installer::RequestLedger::open(&paths)
            .unwrap()
            .get("signed-ledger-failure")
            .unwrap()
            .unwrap();
        assert_eq!(
            recorded.state.as_deref(),
            Some("planned"),
            "the fixture must actually block terminal persistence"
        );
        assert_eq!(recorded.plan_id.as_deref(), Some(result.plan_id.as_str()));
        let persisted = install::list_transactions_at(paths.root()).unwrap();
        let transaction = persisted
            .iter()
            .find(|t| t.plan_id == result.plan_id)
            .unwrap();
        assert_eq!(transaction.state, result.state);
        let replay = f
            .run(
                &p,
                &lease,
                &Offline,
                "signed-ledger-failure",
                true,
                InstallOperation::Overwrite,
            )
            .unwrap();
        assert_eq!(replay.transaction_id, result.transaction_id);
        assert_eq!(replay.disposition, result.disposition);
        assert_eq!(p.calls.get(), 2);
        if ordinary_failure {
            f.unchanged();
        } else {
            assert_eq!(fs::read(f.game.join("original.dat")).unwrap(), b"patched");
        }
    }
}

#[test]
fn both_plan_binding_and_terminal_backfill_failure_replay_the_same_journal_transaction() {
    let f = Fixture::new();
    let paths = steam_patch_core::installer::InstallerPaths::new(&f.state);
    steam_patch_core::installer::init_atomic(&paths).unwrap();
    let _ledger = steam_patch_core::installer::RequestLedger::open(&paths).unwrap();
    rusqlite::Connection::open(paths.requests()).unwrap().execute_batch("CREATE TRIGGER reject_all_backfills BEFORE UPDATE ON requests BEGIN SELECT RAISE(FAIL,'fixture_all_backfills_failed'); END;").unwrap();
    let p = f.provider();
    let lease = Lease::default();
    let result = f
        .run(
            &p,
            &lease,
            &Offline,
            "signed-double-ledger-failure",
            true,
            InstallOperation::Overwrite,
        )
        .unwrap();
    assert_eq!(result.state, "rolled_back");
    assert_eq!(result.disposition, Disposition::Failed);
    let record = steam_patch_core::installer::RequestLedger::open(&paths)
        .unwrap()
        .get("signed-double-ledger-failure")
        .unwrap()
        .unwrap();
    assert!(record.plan_id.is_none());
    assert!(record.state.is_none());
    let replay = f
        .run(
            &p,
            &lease,
            &Offline,
            "signed-double-ledger-failure",
            true,
            InstallOperation::Overwrite,
        )
        .unwrap();
    assert_eq!(result.transaction_id, replay.transaction_id);
    assert_eq!(result.state, replay.state);
    assert_eq!(result.cause, replay.cause);
    assert_eq!(
        p.calls.get(),
        1,
        "replay must not re-load catalog or create a new plan"
    );
    assert_eq!(
        install::list_transactions_at(paths.root()).unwrap().len(),
        1
    );
    f.unchanged();
}

#[test]
fn preplan_expired_lease_uses_host_compatible_cancelled_code() {
    let f = Fixture::new();
    let p = f.provider();
    let lease = Lease::default();
    lease.0.store(2, Ordering::Release);
    assert_eq!(
        f.run(
            &p,
            &lease,
            &Offline,
            "preplan-expired",
            true,
            InstallOperation::Overwrite
        )
        .unwrap_err()
        .code,
        "cancelled"
    );
    assert!(install::list_transactions_at(
        steam_patch_core::installer::InstallerPaths::new(&f.state).root()
    )
    .unwrap()
    .is_empty());
    f.unchanged();
}

#[test]
fn recovery_revalidates_committed_uninstall_intent_against_trusted_steam_instances() {
    let f = Fixture::new();
    let p = f.provider();
    let lease = Lease::default();
    let outcome = f
        .run(
            &p,
            &lease,
            &Offline,
            "recovery-boundary",
            true,
            InstallOperation::Overwrite,
        )
        .unwrap();
    let paths = steam_patch_core::installer::InstallerPaths::new(&f.state);
    let db = rusqlite::Connection::open(paths.journal()).unwrap();
    let raw: String = db
        .query_row(
            "SELECT document FROM jobs WHERE id=?1",
            [&outcome.plan_id],
            |row| row.get(0),
        )
        .unwrap();
    let mut document: serde_json::Value = serde_json::from_str(&raw).unwrap();
    document["uninstall_requested"] = serde_json::json!(true);
    db.execute(
        "UPDATE jobs SET document=?1 WHERE id=?2",
        rusqlite::params![document.to_string(), &outcome.plan_id],
    )
    .unwrap();
    drop(db);
    fs::remove_file(f.library.join("steamapps/appmanifest_42.acf")).unwrap();
    let host =
        ValidatedHostContext::new(&f.state, std::slice::from_ref(&f.library), platform()).unwrap();
    assert_eq!(
        service::recover(&host).unwrap_err().code,
        "recovery_instance_not_found"
    );
    assert_eq!(fs::read(f.game.join("original.dat")).unwrap(), b"patched");
}

#[test]
fn typed_cancel_with_rollback_cleanup_failure_remains_cancelled_on_replay() {
    use std::sync::atomic::AtomicBool;
    for (signal, kind, code) in [
        (1, FailureKind::UserCancelled, "cancelled"),
        (2, FailureKind::LeaseExpired, "lease_expired"),
    ] {
        let f = Fixture::new();
        let p = f.provider();
        let lease = Arc::new(Lease::default());
        let changed_lease = lease.clone();
        let paths = steam_patch_core::installer::InstallerPaths::new(&f.state);
        let request = format!("cleanup-cancel-{signal}");
        let result = steam_patch_core::operation::run(
            Arc::new(AtomicBool::new(false)),
            move |progress| {
                if progress.stage == "writing" && progress.files_done == 1 {
                    let job = fs::read_dir(paths.jobs_dir())
                        .unwrap()
                        .map(Result::unwrap)
                        .find(|entry| entry.file_type().unwrap().is_dir())
                        .unwrap()
                        .path();
                    let stage = job.join("0.stage");
                    fs::remove_file(&stage).unwrap();
                    fs::create_dir(&stage).unwrap();
                    changed_lease.0.store(signal, Ordering::Release);
                    // Wait for the actual task monitor to acknowledge this signal.
                    // This fixture exercises cancellation before commit, not late cancellation.
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
                    while !job.join("revoked").is_file() && std::time::Instant::now() < deadline {
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                    assert!(
                        job.join("revoked").is_file(),
                        "task monitor must acknowledge cancellation"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
            },
            || {
                f.run(
                    &p,
                    &lease,
                    &Offline,
                    &request,
                    true,
                    InstallOperation::Overwrite,
                )
            },
        )
        .unwrap();
        assert_eq!(result.state, "rolled_back");
        assert_eq!(result.disposition, Disposition::Cancelled);
        assert_eq!(result.cause.as_ref().unwrap().kind, kind);
        assert_eq!(result.cause.as_ref().unwrap().code, code);
        assert!(result
            .error
            .as_deref()
            .unwrap()
            .contains("; cleanup_pending:"));
        assert!(result
            .cause
            .as_ref()
            .unwrap()
            .detail
            .as_deref()
            .unwrap()
            .starts_with("cleanup_pending:"));
        f.unchanged();
        let replay = f
            .run(
                &p,
                &lease,
                &Offline,
                &request,
                true,
                InstallOperation::Overwrite,
            )
            .unwrap();
        assert_eq!(replay.transaction_id, result.transaction_id);
        assert_eq!(replay.cause, result.cause);
        assert_eq!(replay.disposition, Disposition::Cancelled);
    }
}

#[test]
fn planned_discard_cleanup_failure_returns_real_cancel_or_domain_terminal_and_replays() {
    for signal in [0, 1, 2] {
        let f = Fixture::new();
        let mut p = f.provider();
        let lease = Arc::new(Lease::default());
        if signal == 0 {
            p.current = f.envelope(f.release.clone(), 5, f.now + 3600, true);
        }
        let paths = steam_patch_core::installer::InstallerPaths::new(&f.state);
        let changed_paths = paths.clone();
        let changed_lease = lease.clone();
        p.on_second.replace(Some(Box::new(move || {
            let job = fs::read_dir(changed_paths.jobs_dir())
                .unwrap()
                .map(Result::unwrap)
                .find(|entry| entry.file_type().unwrap().is_dir())
                .unwrap()
                .path();
            let stage = job.join("0.stage");
            fs::remove_file(&stage).unwrap();
            fs::create_dir(&stage).unwrap();
            changed_lease.0.store(signal, Ordering::Release);
        })));
        let request = format!("planned-cleanup-{signal}");
        let result = f
            .run(
                &p,
                &lease,
                &Offline,
                &request,
                true,
                InstallOperation::Overwrite,
            )
            .unwrap();
        assert_eq!(result.state, "rolled_back");
        let (disposition, kind, code) = match signal {
            1 => (
                Disposition::Cancelled,
                FailureKind::UserCancelled,
                "cancelled",
            ),
            2 => (
                Disposition::Cancelled,
                FailureKind::LeaseExpired,
                "lease_expired",
            ),
            _ => (
                Disposition::Failed,
                FailureKind::DomainFailure,
                "release_revoked",
            ),
        };
        assert_eq!(result.disposition, disposition);
        assert_eq!(result.cause.as_ref().unwrap().kind, kind);
        assert_eq!(result.cause.as_ref().unwrap().code, code);
        assert!(result
            .error
            .as_deref()
            .unwrap()
            .contains("; cleanup_pending:"));
        f.unchanged();
        let replay = f
            .run(
                &p,
                &lease,
                &Offline,
                &request,
                true,
                InstallOperation::Overwrite,
            )
            .unwrap();
        assert_eq!(replay.transaction_id, result.transaction_id);
        assert_eq!(replay.disposition, disposition);
        assert_eq!(replay.cause, result.cause);
    }
}
