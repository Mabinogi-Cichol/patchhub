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
    fn cancellation_cause(&self) -> Option<CancellationCause> {
        (self.0.load(Ordering::Acquire) == 1).then_some(CancellationCause::UserCancelled)
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
        Self::with_library_name("Steam")
    }
    fn with_library_name(name: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let library = temp.path().join(name);
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
        self.run_policy(
            provider,
            lease,
            transport,
            id,
            local,
            operation,
            CleanupPolicy::Disabled,
            &mut |_| {},
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn run_policy(
        &self,
        provider: &Provider,
        lease: &Lease,
        transport: &dyn Transport,
        id: &str,
        local: bool,
        operation: InstallOperation,
        policy: CleanupPolicy,
        emit: &mut dyn FnMut(steam_patch_core::operation::Progress),
    ) -> Result<InstallerOutcome, DomainError> {
        let host =
            ValidatedHostContext::new(&self.state, std::slice::from_ref(&self.library), platform())
                .unwrap();
        let store = SqliteTrustRevisionStore::new(&self.state).unwrap();
        let mut request = self.request(id, local, operation);
        request.cleanup_policy = policy;
        service::install(
            request,
            InstallerDeps {
                host: &host,
                catalog: provider,
                revision_store: &store,
                transport,
                worker: None,
                control: lease,
                clock: &FixedClock(self.now),
                emit,
            },
        )
    }
    fn projected_actions(&self, provider: &Provider) -> serde_json::Value {
        use steam_patch_core::projection::{self, CatalogView, HostCapabilities, ProjectionInputs};
        let report = steam::scan(std::slice::from_ref(&self.library));
        let transactions = install::TransactionSnapshot::load_ro(&self.state, &report).unwrap();
        let verified =
            trust::VerifiedCatalog::from_bytes(&provider.initial, &provider.keys, self.now, 0)
                .unwrap();
        let projected = projection::scan(ProjectionInputs {
            steam: &report,
            catalog: CatalogView::Verified(&verified),
            sources: None,
            transactions: &transactions,
            roots: std::slice::from_ref(&self.library),
            now: self.now,
            platform: platform(),
            capabilities: &HostCapabilities::default(),
            cancel: &std::sync::atomic::AtomicBool::new(false),
        })
        .unwrap();
        serde_json::to_value(&projected.installs[0].patches[0]).unwrap()["actions"].clone()
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
fn signed_precheck_accepts_installed_dlc_beyond_the_former_manifest_limit() {
    if !can_install() {
        return;
    }
    let f = Fixture::new();
    let installed_dlc = (0..129u32)
        .map(|index| (4_000_000 + index).to_string())
        .collect::<Vec<_>>()
        .join(",");
    fs::write(
        f.library.join("steamapps/appmanifest_42.acf"),
        format!(
            r#""AppState" {{ "appid" "42" "name" "Signed Test" "installdir" "Signed Test" "buildid" "100" "StateFlags" "4" "UserConfig" {{ "language" "english" }} "installeddlc" "{installed_dlc}" }}"#
        ),
    )
    .unwrap();
    let result = f
        .run(
            &f.provider(),
            &Lease::default(),
            &Offline,
            "signed-dlc-large",
            true,
            InstallOperation::Overwrite,
        )
        .unwrap();
    assert_eq!(result.state, "committed");
    assert_eq!(result.disposition, Disposition::Completed);
    assert_eq!(fs::read(f.game.join("original.dat")).unwrap(), b"patched");
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
        if change_release {
            assert_eq!(result.state, "rolled_back");
            assert_eq!(result.disposition, Disposition::Failed);
            assert_eq!(result.cause.unwrap().code, "authorization_rule_changed");
            f.unchanged();
        } else {
            // A mutated live buildid is now caught by the real target-state
            // recheck (`game_changed_or_unavailable`), not by the removed
            // whole-snapshot facts gate (design D06).
            assert_eq!(result.state, "rolled_back");
            assert_eq!(result.cause.unwrap().code, "game_changed_or_unavailable");
        }
    }
}

#[test]
fn postplan_user_cancel_persists_exact_cause_and_replays() {
    if !can_install() {
        return;
    }
    let f = Fixture::new();
    let p = f.provider();
    let lease = Arc::new(Lease::default());
    let observed = lease.clone();
    p.on_second.replace(Some(Box::new(move || {
        observed.0.store(1, Ordering::Release)
    })));
    let result = f
        .run(
            &p,
            &lease,
            &Offline,
            "cancelled",
            true,
            InstallOperation::Overwrite,
        )
        .unwrap();
    assert_eq!(result.state, "rolled_back");
    assert_eq!(result.error.as_deref(), Some("cancelled"));
    assert_eq!(result.disposition, Disposition::Cancelled);
    assert_eq!(
        result.cause.as_ref().unwrap().kind,
        FailureKind::UserCancelled
    );
    f.unchanged();
    let replay = f
        .run(
            &p,
            &lease,
            &Offline,
            "cancelled",
            true,
            InstallOperation::Overwrite,
        )
        .unwrap();
    assert_eq!(replay.transaction_id, result.transaction_id);
    assert_eq!(replay.error, result.error);
    assert_eq!(replay.disposition, Disposition::Cancelled);
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
    for (signal, kind, code) in [(1, FailureKind::UserCancelled, "cancelled")] {
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
                    if stage.exists() {
                        fs::remove_file(&stage).unwrap();
                    }
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
    for signal in [0, 1] {
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
            if stage.exists() {
                fs::remove_file(&stage).unwrap();
            }
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

#[test]
fn after_commit_cleans_once_and_replay_preserves_new_cache() {
    if !can_install() {
        return;
    }
    let f = Fixture::new();
    let paths = steam_patch_core::installer::InstallerPaths::new(&f.state);
    fs::create_dir_all(paths.downloads_dir()).unwrap();
    let cache = paths.downloads_dir().join("disposable");
    fs::write(&cache, b"cache").unwrap();
    let p = f.provider();
    let host =
        ValidatedHostContext::new(&f.state, std::slice::from_ref(&f.library), platform()).unwrap();
    p.on_second.replace(Some(Box::new(move || {
        assert_eq!(
            steam_patch_core::installer::cache::CacheService::new(&host)
                .cleanup()
                .unwrap_err()
                .code,
            "installer_busy"
        );
    })));
    let result = f
        .run_policy(
            &p,
            &Lease::default(),
            &Offline,
            "cleanup-once",
            true,
            InstallOperation::Overwrite,
            CleanupPolicy::AfterCommit,
            &mut |_| {},
        )
        .unwrap();
    assert_eq!(result.state, "committed");
    assert_eq!(result.cleanup.status, CleanupStatus::Completed);
    assert_eq!(result.cleanup.removed_bytes, 5);
    assert!(!cache.exists());
    fs::write(&cache, b"new download").unwrap();
    let replay = f
        .run_policy(
            &p,
            &Lease::default(),
            &Offline,
            "cleanup-once",
            true,
            InstallOperation::Overwrite,
            CleanupPolicy::AfterCommit,
            &mut |_| {},
        )
        .unwrap();
    assert_eq!(replay.transaction_id, result.transaction_id);
    assert_eq!(replay.cleanup.status, CleanupStatus::NotRequested);
    assert_eq!(replay.cleanup.removed_bytes, 0);
    assert_eq!(fs::read(cache).unwrap(), b"new download");
}

#[test]
fn after_commit_busy_preserves_commit_and_does_not_upgrade_own_lock() {
    if !can_install() {
        return;
    }
    let f = Fixture::new();
    let paths = steam_patch_core::installer::InstallerPaths::new(&f.state);
    fs::create_dir_all(paths.downloads_dir()).unwrap();
    let cache = paths.downloads_dir().join("keep");
    fs::write(&cache, b"keep").unwrap();
    let mut other_writer = None;
    let mut emit = |progress: steam_patch_core::operation::Progress| {
        if progress.stage == "cleanup" {
            let lock = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(paths.cache_lock())
                .unwrap();
            fs2::FileExt::try_lock_shared(&lock).unwrap();
            other_writer = Some(lock);
        }
    };
    let result = f
        .run_policy(
            &f.provider(),
            &Lease::default(),
            &Offline,
            "cleanup-busy",
            true,
            InstallOperation::Overwrite,
            CleanupPolicy::AfterCommit,
            &mut emit,
        )
        .unwrap();
    assert_eq!(result.disposition, Disposition::Completed);
    assert_eq!(result.cleanup.status, CleanupStatus::Busy);
    assert!(!result.cleanup.performed);
    assert_eq!(result.cleanup.removed_bytes, 0);
    assert_eq!(fs::read(cache).unwrap(), b"keep");
    assert_eq!(fs::read(f.game.join("original.dat")).unwrap(), b"patched");
    drop(other_writer);
}

#[test]
fn after_commit_partial_or_front_refusal_cannot_rewrite_committed_install() {
    if !can_install() {
        return;
    }
    for front_refusal in [false, true] {
        let f = Fixture::new();
        let paths = steam_patch_core::installer::InstallerPaths::new(&f.state);
        fs::create_dir_all(paths.downloads_dir()).unwrap();
        fs::write(paths.downloads_dir().join("safe"), b"safe").unwrap();
        let outside = f._temp.path().join("outside");
        fs::write(&outside, b"outside").unwrap();
        if front_refusal {
            // The refusal is injected after the install committed below.
        } else {
            fs::hard_link(&outside, paths.downloads_dir().join("linked")).unwrap();
        }
        let result = f
            .run_policy(
                &f.provider(),
                &Lease::default(),
                &Offline,
                "cleanup-failure",
                true,
                InstallOperation::Overwrite,
                CleanupPolicy::AfterCommit,
                &mut |progress| {
                    if front_refusal && progress.stage == "cleanup" {
                        fs::remove_dir(paths.reports_dir()).unwrap();
                        fs::write(paths.reports_dir(), b"not a directory").unwrap();
                    }
                },
            )
            .unwrap();
        assert_eq!(result.state, "committed");
        assert_eq!(result.disposition, Disposition::Completed);
        assert_eq!(result.cause, None);
        assert_eq!(
            result.cleanup.status,
            if front_refusal {
                CleanupStatus::Failed
            } else {
                CleanupStatus::Partial
            }
        );
        assert_eq!(
            result.cleanup.removed_bytes,
            if front_refusal { 0 } else { 4 }
        );
        assert!(!result.cleanup.failures.is_empty());
        assert_eq!(fs::read(outside).unwrap(), b"outside");
    }
}

#[test]
fn after_commit_never_cleans_rollback_and_exclusive_blocks_install_recover_discard() {
    if !can_install() {
        return;
    }
    let f = Fixture::new();
    let mut p = f.provider();
    p.current = f.envelope(f.release.clone(), 5, f.now + 3600, true);
    let paths = steam_patch_core::installer::InstallerPaths::new(&f.state);
    fs::create_dir_all(paths.downloads_dir()).unwrap();
    let cache = paths.downloads_dir().join("keep");
    fs::write(&cache, b"keep").unwrap();
    let result = f
        .run_policy(
            &p,
            &Lease::default(),
            &Offline,
            "cleanup-rollback",
            true,
            InstallOperation::Overwrite,
            CleanupPolicy::AfterCommit,
            &mut |_| {},
        )
        .unwrap();
    assert_eq!(result.state, "rolled_back");
    assert_eq!(result.cleanup.status, CleanupStatus::NotRequested);
    assert_eq!(fs::read(&cache).unwrap(), b"keep");
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(paths.cache_lock())
        .unwrap();
    fs2::FileExt::try_lock_exclusive(&lock).unwrap();
    assert_eq!(
        f.run(
            &f.provider(),
            &Lease::default(),
            &Offline,
            "blocked",
            true,
            InstallOperation::Overwrite
        )
        .unwrap_err()
        .code,
        "installer_busy"
    );
    let host =
        ValidatedHostContext::new(&f.state, std::slice::from_ref(&f.library), platform()).unwrap();
    assert_eq!(service::recover(&host).unwrap_err().code, "installer_busy");
    assert_eq!(
        service::discard_plan(&host, &service::PlanId::new(&result.plan_id).unwrap())
            .unwrap_err()
            .code,
        "installer_busy"
    );
    f.unchanged();
}

#[test]
fn stale_local_actions_cannot_bypass_a_revoked_release_or_changed_build() {
    if !can_install() {
        return;
    }
    for changed_build in [false, true] {
        let f = Fixture::new();
        let mut provider = f.provider();
        let snapshot = f.projected_actions(&provider);
        assert_eq!(snapshot["local_archive"]["available"], true);
        assert_eq!(snapshot["primary"]["input"], "local_archive");
        if changed_build {
            f.manifest("101");
        } else {
            provider.initial = f.envelope(f.release.clone(), 5, f.now + 3600, true);
            provider.current = provider.initial.clone();
        }
        let error = f
            .run(
                &provider,
                &Lease::default(),
                &Offline,
                "stale-local-actions",
                true,
                InstallOperation::Overwrite,
            )
            .unwrap_err();
        assert_eq!(
            error.code,
            if changed_build {
                "build_not_verified"
            } else {
                "release_revoked"
            }
        );
        f.unchanged();
        assert!(install::list_transactions_at(
            steam_patch_core::installer::InstallerPaths::new(&f.state).root()
        )
        .unwrap()
        .is_empty());
    }
}

#[test]
fn stale_reinstall_actions_do_not_take_over_external_or_damaged_files() {
    if !can_install() {
        return;
    }
    let f = Fixture::new();
    let provider = f.provider();
    f.run(
        &provider,
        &Lease::default(),
        &Offline,
        "owned-for-actions",
        true,
        InstallOperation::Overwrite,
    )
    .unwrap();
    let snapshot = f.projected_actions(&provider);
    assert_eq!(snapshot["reinstall"]["available"], true);
    fs::write(f.game.join("original.dat"), b"external edit after scan").unwrap();
    let error = f
        .run(
            &provider,
            &Lease::default(),
            &Offline,
            "stale-reinstall",
            true,
            InstallOperation::Reinstall,
        )
        .unwrap_err();
    assert_eq!(error.code, "reinstall_unowned_or_incomplete");
    assert_eq!(
        fs::read(f.game.join("original.dat")).unwrap(),
        b"external edit after scan"
    );
    let fresh = f.projected_actions(&provider);
    assert_eq!(fresh["reinstall"]["available"], false);
    assert_eq!(fresh["local_archive"]["available"], true);
    assert_eq!(fresh["primary"]["label"], "overwrite");
    assert_eq!(
        f.run(
            &provider,
            &Lease::default(),
            &Offline,
            "explicit-overwrite",
            true,
            InstallOperation::Overwrite
        )
        .unwrap()
        .state,
        "committed"
    );
}

#[test]
fn recovery_created_after_scan_blocks_a_different_patch_and_preserves_journal() {
    if !can_install() {
        return;
    }
    let mut f = Fixture::new();
    let provider = f.provider();
    let first = f
        .run(
            &provider,
            &Lease::default(),
            &Offline,
            "first-before-recovery",
            true,
            InstallOperation::Overwrite,
        )
        .unwrap();
    f.release.release_id = "other-signed-release".into();
    f.release.patch_id = "other-signed-patch".into();
    let provider = f.provider();
    assert_eq!(
        f.projected_actions(&provider)["local_archive"]["available"],
        true
    );
    let paths = steam_patch_core::installer::InstallerPaths::new(&f.state);
    let db = rusqlite::Connection::open(paths.journal()).unwrap();
    let document: String = db
        .query_row(
            "SELECT document FROM jobs WHERE id=?1",
            [&first.plan_id],
            |row| row.get(0),
        )
        .unwrap();
    let mut document: serde_json::Value = serde_json::from_str(&document).unwrap();
    document["transaction"]["state"] = serde_json::json!("needs_recovery");
    db.execute(
        "UPDATE jobs SET document=?1 WHERE id=?2",
        rusqlite::params![document.to_string(), &first.plan_id],
    )
    .unwrap();
    drop(db);
    let before = fs::read(paths.journal()).unwrap();
    let error = f
        .run(
            &provider,
            &Lease::default(),
            &Offline,
            "other-after-recovery",
            true,
            InstallOperation::Overwrite,
        )
        .unwrap_err();
    assert_eq!(error.code, "needs_recovery");
    assert_eq!(fs::read(paths.journal()).unwrap(), before);
    assert_eq!(fs::read(f.game.join("original.dat")).unwrap(), b"patched");
    let fresh = f.projected_actions(&provider);
    for action in ["automatic", "local_archive", "reinstall", "overwrite"] {
        assert_eq!(fresh[action]["available"], false);
        assert!(fresh[action]["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b["code"] == "needs_recovery"));
    }
}

#[test]
fn complete_external_signed_files_still_cannot_reinstall_without_owned_receipt() {
    if !can_install() {
        return;
    }
    let f = Fixture::new();
    let provider = f.provider();
    fs::write(f.game.join("original.dat"), b"patched").unwrap();
    let actions = f.projected_actions(&provider);
    assert_eq!(actions["reinstall"]["available"], false);
    assert_eq!(actions["primary"]["label"], "overwrite");
    assert_eq!(
        f.run(
            &provider,
            &Lease::default(),
            &Offline,
            "external-reinstall",
            true,
            InstallOperation::Reinstall
        )
        .unwrap_err()
        .code,
        "reinstall_unowned_or_incomplete"
    );
    assert_eq!(fs::read(f.game.join("original.dat")).unwrap(), b"patched");
}

#[test]
fn long_instance_and_large_durable_error_replay_preserve_committed_truth() {
    if !can_install() {
        return;
    }
    let f = Fixture::with_library_name(&"库".repeat(40));
    let p = f.provider();
    let lease = Lease::default();
    let first = f
        .run(
            &p,
            &lease,
            &Offline,
            "long-replay",
            true,
            InstallOperation::Overwrite,
        )
        .unwrap();
    assert!(first.instance_id.len() > 256);
    let paths = steam_patch_core::installer::InstallerPaths::new(&f.state);
    let db = rusqlite::Connection::open(paths.journal()).unwrap();
    let raw: String = db
        .query_row(
            "SELECT document FROM jobs WHERE id=?1",
            [&first.plan_id],
            |row| row.get(0),
        )
        .unwrap();
    let mut document: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let legacy_error = "界".repeat(3 * 1024 * 1024);
    document["transaction"]["error"] = serde_json::json!(legacy_error);
    db.execute(
        "UPDATE jobs SET document=?1 WHERE id=?2",
        rusqlite::params![document.to_string(), first.plan_id],
    )
    .unwrap();
    drop(db);
    let ledger = steam_patch_core::installer::requests::RequestLedger::open(&paths).unwrap();
    let mut record = ledger.get("long-replay").unwrap().unwrap();
    record.error = Some(legacy_error.clone());
    ledger.record(&record).unwrap();
    for _ in 0..2 {
        let replay = f
            .run(
                &p,
                &lease,
                &Offline,
                "long-replay",
                true,
                InstallOperation::Overwrite,
            )
            .unwrap();
        assert_eq!(replay.instance_id, first.instance_id);
        assert_eq!(replay.transaction_id, first.transaction_id);
        assert_eq!(replay.disposition, Disposition::Completed);
        assert_eq!(replay.state, "committed");
        assert_eq!(replay.error.as_deref(), Some(legacy_error.as_str()));
        assert!(serde_json::to_vec(&replay).unwrap().len() >= legacy_error.len());
    }
    let db = rusqlite::Connection::open(paths.journal()).unwrap();
    let raw: String = db
        .query_row(
            "SELECT document FROM jobs WHERE id=?1",
            [&first.plan_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        serde_json::from_str::<serde_json::Value>(&raw).unwrap()["transaction"]["error"]
            == legacy_error,
        "durable journal diagnostic must remain unchanged"
    );
    let ledger = steam_patch_core::installer::requests::RequestLedger::open(&paths).unwrap();
    assert!(
        ledger.get("long-replay").unwrap().unwrap().error.as_deref() == Some(legacy_error.as_str()),
        "durable ledger diagnostic must remain unchanged"
    );
}
