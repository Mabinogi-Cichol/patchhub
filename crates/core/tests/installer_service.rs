//! Public write-boundary regression tests. These never trust caller game facts.
use std::{fs, io, path::Path};
use steam_patch_core::{
    catalog::Platform,
    install::InstallOperation,
    installer::{outcome::CancellationCause, service::*, HttpResponse, Transport},
    projection::DomainError,
    trust::{
        revision::{TrustRevisionStore, TrustedCatalogMaterial, TrustedCatalogProvider},
        VerifiedCatalog,
    },
};
fn platform() -> Platform {
    if cfg!(windows) {
        Platform::Windows
    } else {
        Platform::Steamos
    }
}
struct Provider;
impl TrustedCatalogProvider for Provider {
    fn load(&self) -> Result<TrustedCatalogMaterial, DomainError> {
        panic!("invalid boundary must reject before reading trust")
    }
}
struct Revision;
impl TrustRevisionStore for Revision {
    fn highest_revision(&self) -> Result<u64, DomainError> {
        panic!("invalid boundary before reading revisions")
    }
    fn remember_verified_revision(&self, _: &VerifiedCatalog) -> Result<(), DomainError> {
        panic!("invalid boundary before writing revisions")
    }
}
struct Offline;
impl Transport for Offline {
    fn get(&self, _: &str, _: Option<&str>, _: Option<&str>) -> io::Result<HttpResponse> {
        panic!("invalid boundary before network")
    }
}
struct Lease;
impl TaskLease for Lease {
    fn cancelled(&self) -> bool {
        false
    }
    fn cancellation_cause(&self) -> Option<CancellationCause> {
        None
    }
}
fn manifest(root: &Path, name: &str) {
    fs::create_dir_all(root.join("steamapps/common").join(name)).unwrap();
    fs::write(root.join("steamapps/appmanifest_42.acf"),format!("\"AppState\" {{ \"appid\" \"42\" \"name\" \"Fixture\" \"installdir\" \"{name}\" \"buildid\" \"100\" \"StateFlags\" \"4\" }}")).unwrap();
}
fn invoke(host: &ValidatedHostContext, id: &str) -> DomainError {
    let mut emit = |_| {};
    install(
        InstallRequest {
            request_id: RequestId::new("request").unwrap(),
            instance_id: InstanceId::new(id).unwrap(),
            operation: InstallOperation::Overwrite,
            target: InstallTarget::SignedRelease {
                release_id: ReleaseId::new("release").unwrap(),
                archive: None,
            },
            cleanup_policy: CleanupPolicy::Disabled,
        },
        InstallerDeps {
            host,
            catalog: &Provider,
            revision_store: &Revision,
            transport: &Offline,
            worker: None,
            control: &Lease,
            clock: &SystemClock,
            emit: &mut emit,
        },
    )
    .unwrap_err()
}
#[test]
fn checked_ids_enforce_utf8_bytes_controls_and_selection_shapes() {
    assert!(RequestId::new(&"中".repeat(42)).is_ok());
    assert!(RequestId::new(&"中".repeat(43)).is_err());
    assert!(InstanceId::new(&"中".repeat(5461)).is_ok());
    assert!(InstanceId::new(&"中".repeat(5462)).is_err());
    assert!(RequestId::new("\u{85}").is_err());
    assert!(ZipSelectionId::new("zip-selection-foo-42").is_err());
    assert!(ExeSelectionId::new("exe-selection-foo-42").is_err());
}
#[test]
fn context_platform_and_state_overlap_reject_without_initialization() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("Steam");
    manifest(&root, "Game");
    let wrong = if cfg!(windows) {
        Platform::Steamos
    } else {
        Platform::Windows
    };
    let state = t.path().join("state");
    assert_eq!(
        ValidatedHostContext::new(&state, std::slice::from_ref(&root), wrong)
            .unwrap_err()
            .code,
        "platform_mismatch"
    );
    let nested = root.join("steamapps/common/Game/state");
    assert_eq!(
        ValidatedHostContext::new(&nested, std::slice::from_ref(&root), platform())
            .unwrap_err()
            .code,
        "state_overlaps_game"
    );
    assert!(!nested.exists());
    assert!(!state.exists());
}
#[test]
fn write_rescans_steam_instead_of_using_context_time_facts() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("Steam");
    manifest(&root, "Old");
    let id = steam_patch_core::steam::scan(std::slice::from_ref(&root))
        .installs
        .remove(0)
        .instance_id;
    let state = t.path().join("state");
    let host = ValidatedHostContext::new(&state, std::slice::from_ref(&root), platform()).unwrap();
    manifest(&root, "New");
    assert_eq!(invoke(&host, &id).code, "instance_not_found");
    assert!(!state.exists());
}
#[test]
fn unbound_trust_store_is_rejected_before_any_state_write() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("Steam");
    manifest(&root, "Game");
    let id = steam_patch_core::steam::scan(std::slice::from_ref(&root))
        .installs
        .remove(0)
        .instance_id;
    let state = t.path().join("state");
    let host = ValidatedHostContext::new(&state, &[root], platform()).unwrap();
    assert_eq!(invoke(&host, &id).code, "trust_state_mismatch");
    assert!(!state.exists());
}
#[test]
fn recovery_empty_state_is_readonly() {
    let t = tempfile::tempdir().unwrap();
    let state = t.path().join("state");
    let host = ValidatedHostContext::new(&state, &[t.path().to_owned()], platform()).unwrap();
    assert!(recover(&host).unwrap().is_empty());
    assert!(!state.exists());
}

#[test]
fn publisher_download_cancel_is_standard_cancel_before_any_transaction() {
    use steam_patch_core::{
        install,
        sources::{
            model::{GameQuery, SourceIndex, SourceIndexEntry},
            Sources,
        },
        trust::revision::SqliteTrustRevisionStore,
    };
    struct DownloadFailure(&'static str);
    impl Transport for DownloadFailure {
        fn get(&self, _: &str, _: Option<&str>, _: Option<&str>) -> io::Result<HttpResponse> {
            Err(io::Error::other(self.0))
        }
    }
    for (transport_code, expected) in [
        ("task_cancelled", "cancelled"),
        ("download_cancelled", "cancelled"),
        ("network_unavailable", "download_failed"),
    ] {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("Steam");
        manifest(&root, "Game");
        let game = root.join("steamapps/common/Game");
        fs::write(game.join("original.txt"), b"original").unwrap();
        let id = steam_patch_core::steam::scan(std::slice::from_ref(&root))
            .installs
            .remove(0)
            .instance_id;
        let state = t.path().join("state");
        let host = ValidatedHostContext::new(&state, &[root], platform()).unwrap();
        let now = SystemClock.now_seconds().unwrap();
        let sources = Sources::open(&state).unwrap();
        sources
            .store()
            .save_index(&SourceIndex {
                schema_version: 1,
                source_id: "patchmoe".into(),
                revision: 1,
                complete: true,
                checked_at: now,
                updated_at: now,
                etag: None,
                modified: None,
                entries: vec![SourceIndexEntry {
                    title: "Fixture".into(),
                    aliases: vec![],
                    page: "https://www.patch.moe/".into(),
                    kind: "entry".into(),
                    appid: Some(42),
                    url: Some("https://download.patch.moe/fixture.zip".into()),
                    language: Some("zh-CN".into()),
                    date: None,
                }],
                pending: Vec::new(),
            })
            .unwrap();
        drop(sources);
        let sources = Sources::open(&state).unwrap();
        let query = GameQuery {
            appid: 42,
            name: "Fixture".into(),
            localized_name: None,
            instance_id: Some(id.clone()),
        };
        let candidate = sources.registry().index_candidates(&query).remove(0);
        let selection = steam_patch_core::installer::zip_selection_id(&candidate).unwrap();
        drop(sources);
        let revisions = SqliteTrustRevisionStore::new(&state).unwrap();
        let mut emit = |_| {};
        let error = install(
            InstallRequest {
                request_id: RequestId::new("publisher-download-cancel").unwrap(),
                instance_id: InstanceId::new(&id).unwrap(),
                operation: InstallOperation::Overwrite,
                target: InstallTarget::PublisherZipSelection {
                    selection_id: ZipSelectionId::new(&selection).unwrap(),
                },
                cleanup_policy: CleanupPolicy::AfterCommit,
            },
            InstallerDeps {
                host: &host,
                catalog: &Provider,
                revision_store: &revisions,
                transport: &DownloadFailure(transport_code),
                worker: None,
                control: &Lease,
                clock: &SystemClock,
                emit: &mut emit,
            },
        )
        .unwrap_err();
        assert_eq!(error.code, expected, "{transport_code}");
        assert!(install::list_transactions_at(host.installer_paths().root())
            .unwrap()
            .is_empty());
        assert_eq!(fs::read(game.join("original.txt")).unwrap(), b"original");
        assert_eq!(fs::read_dir(&game).unwrap().count(), 1);
    }
}
#[cfg(target_os = "linux")]
#[test]
fn actual_root_identity_cannot_be_declared_away() {
    if unsafe { libc::geteuid() } != 0 {
        return;
    }
    let t = tempfile::tempdir().unwrap();
    let host =
        ValidatedHostContext::new(&t.path().join("state"), &[t.path().to_owned()], platform())
            .unwrap();
    assert_eq!(
        host.ensure_write_identity().unwrap_err().code,
        "installer_host_requires_non_root"
    );
    assert!(!host.state_dir().exists());
}
