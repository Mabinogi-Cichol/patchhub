//! Public write-boundary regression tests. These never trust caller game facts.
use patchhub_core::{
    catalog::Platform,
    install::InstallOperation,
    installer::{outcome::CancellationCause, service::*, HttpResponse, Transport},
    projection::DomainError,
    trust::{
        revision::{TrustRevisionStore, TrustedCatalogMaterial, TrustedCatalogProvider},
        VerifiedCatalog,
    },
};
use std::{fs, io, path::Path};
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
    let id = patchhub_core::steam::scan(std::slice::from_ref(&root))
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
    let id = patchhub_core::steam::scan(std::slice::from_ref(&root))
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
    use patchhub_core::{
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
        let id = patchhub_core::steam::scan(std::slice::from_ref(&root))
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
                parser_fingerprint: patchhub_core::sources::index::parser_fingerprint(
                    sources
                        .registry()
                        .rules()
                        .iter()
                        .find(|r| r.id == "patchmoe")
                        .unwrap(),
                ),
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
        let selection = patchhub_core::installer::zip_selection_id(&candidate).unwrap();
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

/// P7 crosses the Lua -> index -> detail -> selection -> public write boundary.
/// The only payloads are an inert ZIP and the repository's synthetic SF fixture.
#[test]
#[ignore = "requires SF_WORKER_TEST_BIN pointing to the Release worker"]
fn script_source_update_to_isolated_zip_and_exe_install() {
    use patchhub_core::{
        installer::{exe_resolve::exe_selection_id, zip_selection_id},
        sources::{
            http::{FakeFetcher, FetchResponse, RequestContext},
            matching, update, GameQuery, Sources,
        },
        trust::revision::SqliteTrustRevisionStore,
    };
    use patchhub_installer_worker::production::WorkerRuntime;
    use serde_json::json;
    use std::{
        io::Write,
        sync::{atomic::AtomicBool, Arc},
        time::Duration,
    };
    struct PayloadTransport(std::sync::Mutex<Option<HttpResponse>>);
    impl Transport for PayloadTransport {
        fn get(&self, _: &str, _: Option<&str>, _: Option<&str>) -> io::Result<HttpResponse> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .take()
                .expect("one artifact download"))
        }
    }
    for format in ["zip", "exe"] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("Steam");
        manifest(&root, "Game");
        let game = patchhub_core::steam::scan(std::slice::from_ref(&root))
            .installs
            .remove(0);
        fs::write(game.install_path.join("engine.exe"), b"inert").unwrap();
        fs::write(game.install_path.join("identity.dat"), [87, 164, 242, 220]).unwrap();
        let manifest_path = root.join("steamapps/appmanifest_42.acf");
        let text = fs::read_to_string(&manifest_path).unwrap();
        let end = text.rfind('}').unwrap();
        let mut text = text;
        text.insert_str(end, " \"InstalledDepots\" {} ");
        fs::write(manifest_path, text).unwrap();
        let state = temp.path().join("state");
        let mut sources = Sources::open(&state).unwrap();
        let raw = serde_json::from_value(json!({"id":"script-install","name":"Script install","adapter":"lua","enabled":true,"discovery_only":true,"pages":["https://script.example/list"],"page_hosts":["script.example"],"download_hosts":["kaguraserver.com"]})).unwrap();
        let code = format!(
            r#"return {{
          collect_index=function(ctx,rule,previous)
            local title=ctx.html(ctx.get(rule.pages[1]).text):select('h1')[1]:text()
            return {{entries={{{{title=title,page='https://script.example/game',kind='article'}}}},complete=true}}
          end,
          resolve_page=function(ctx,rule,page)
            local title=ctx.html(ctx.get(page).text):select('h1')[1]:text()
            return {{title=title,page=page,kind='article',steam_ids={{42}},text='English patch {format}',mirrors={{{{url='https://kaguraserver.com/a.{format}'}}}}}}
          end
        }}"#
        );
        sources.import_script_source(&raw, 1, &code, false).unwrap();
        let fake = Arc::new(FakeFetcher::new());
        for page in ["list", "game"] {
            fake.push(FetchResponse {
                status: 200,
                final_url: format!("https://script.example/{page}"),
                body: b"<h1>Fixture</h1>".to_vec(),
                ..Default::default()
            });
        }
        let mut ctx = RequestContext::with_settings(Box::new(Arc::clone(&fake)), Duration::ZERO);
        let flag = Arc::new(AtomicBool::new(false));
        let rule = sources
            .registry()
            .rules()
            .iter()
            .find(|r| r.id == "script-install")
            .unwrap()
            .clone();
        update::update_source(sources.store(), &rule, true, &mut ctx, &flag, |_| {}).unwrap();
        let query = GameQuery {
            appid: 42,
            name: "Fixture".into(),
            localized_name: None,
            instance_id: Some(game.instance_id.clone()),
        };
        let matched = matching::match_games(
            sources.store(),
            sources.registry().rules(),
            sources.registry().overrides(),
            std::slice::from_ref(&query),
            &[],
            false,
            &mut ctx,
            &flag,
            100,
            |_| {},
            |_| {},
        )
        .unwrap();
        let candidate = matched[0]
            .candidates
            .iter()
            .find(|c| c.source_id == "script-install")
            .unwrap();
        assert!(candidate.parser_fingerprint.is_some());
        let (selection, payload) = if format == "zip" {
            let selection = zip_selection_id(candidate).unwrap();
            let reopened = Sources::open(&state).unwrap();
            patchhub_core::installer::resolve_zip(reopened.registry(), &query, &selection).unwrap();
            let mut writer = zip::ZipWriter::new(io::Cursor::new(Vec::new()));
            writer
                .start_file("payload.bin", zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"script ZIP payload").unwrap();
            (
                InstallTarget::PublisherZipSelection {
                    selection_id: ZipSelectionId::new(&selection).unwrap(),
                },
                writer.finish().unwrap().into_inner(),
            )
        } else {
            let selection = exe_selection_id(candidate).unwrap();
            let reopened = Sources::open(&state).unwrap();
            patchhub_core::installer::exe_resolve::resolve_exe(
                reopened.registry(),
                &query,
                &selection,
            )
            .unwrap();
            let golden: serde_json::Value = serde_json::from_str(include_str!(
                "../../installer-format/tests/fixtures/sf/worker-golden.json"
            ))
            .unwrap();
            let bytes = golden["cases"][0]["data_hex"]
                .as_str()
                .unwrap()
                .as_bytes()
                .chunks_exact(2)
                .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
                .collect();
            (
                InstallTarget::PublisherExeSelection {
                    selection_id: ExeSelectionId::new(&selection).unwrap(),
                },
                bytes,
            )
        };
        assert!(
            !game.install_path.join("payload.bin").exists(),
            "resolution is read-only"
        );
        let transport = PayloadTransport(std::sync::Mutex::new(Some(HttpResponse {
            status: 200,
            location: None,
            content_type: Some("application/octet-stream".into()),
            content_length: Some(payload.len() as u64),
            etag: None,
            last_modified: None,
            final_url: format!("https://kaguraserver.com/a.{format}"),
            body: Box::new(io::Cursor::new(payload)),
        })));
        let host = ValidatedHostContext::new(&state, &[root], platform()).unwrap();
        let revisions = SqliteTrustRevisionStore::new(&state).unwrap();
        let worker = WorkerRuntime::new(std::path::PathBuf::from(
            std::env::var_os("SF_WORKER_TEST_BIN").unwrap(),
        ))
        .unwrap();
        let mut emit = |_| {};
        let result = install(
            InstallRequest {
                request_id: RequestId::new("script-install").unwrap(),
                instance_id: InstanceId::new(&game.instance_id).unwrap(),
                operation: InstallOperation::Overwrite,
                target: selection,
                cleanup_policy: CleanupPolicy::AfterCommit,
            },
            InstallerDeps {
                host: &host,
                catalog: &Provider,
                revision_store: &revisions,
                transport: &transport,
                worker: Some(&worker),
                control: &Lease,
                clock: &SystemClock,
                emit: &mut emit,
            },
        )
        .unwrap();
        assert_eq!(result.state, "committed");
        let bytes = fs::read(game.install_path.join("payload.bin")).unwrap();
        if format == "zip" {
            assert_eq!(bytes, b"script ZIP payload");
        } else {
            assert_eq!(bytes.len(), 15);
        }
        assert_eq!(
            fs::read(game.install_path.join("engine.exe")).unwrap(),
            b"inert"
        );
    }
}
