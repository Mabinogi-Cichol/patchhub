use std::fs;
use std::path::Path;
use steam_patch_core::steam::{scan, GameInstall};

fn write_manifest(root: &Path, appid: u32) {
    fs::create_dir_all(root.join("steamapps/common/Game")).unwrap();
    fs::write(root.join(format!("steamapps/appmanifest_{appid}.acf")), format!(
        r#""AppState" {{ "appid" "{appid}" "name" "Manifest {appid}" "installdir" "Game" "StateFlags" "4" }}"#
    )).unwrap();
}

fn language(root: &Path, value: &str) {
    fs::create_dir_all(root.join("config")).unwrap();
    fs::write(root.join("config/config.vdf"), format!(
        r#""InstallConfigStore" {{ "Software" {{ "Valve" {{ "Steam" {{ "Language" "{value}" }} }} }} }}"#
    )).unwrap();
}

fn string(payload: &mut Vec<u8>, key: &str, value: &str) {
    payload.push(1);
    payload.extend(key.as_bytes());
    payload.push(0);
    payload.extend(value.as_bytes());
    payload.push(0);
}

fn object(payload: &mut Vec<u8>, key: &str) {
    payload.push(0);
    payload.extend(key.as_bytes());
    payload.push(0);
}

// Independently authored V28 container fixture, not a copy of the parser.
fn appinfo(root: &Path, appid: u32, default: &str, localized: &str) {
    let mut payload = Vec::new();
    object(&mut payload, "appinfo");
    object(&mut payload, "common");
    string(&mut payload, "name", default);
    object(&mut payload, "name_localized");
    string(&mut payload, "schinese", localized);
    payload.extend([8, 8, 8, 8]);
    let mut header = vec![0; 68];
    header[..4].copy_from_slice(&appid.to_le_bytes());
    header[4..8].copy_from_slice(&(payload.len() as u32 + 60).to_le_bytes());
    let mut file = Vec::new();
    file.extend(0x0756_4428u32.to_le_bytes());
    file.extend(1u32.to_le_bytes());
    file.extend(header);
    file.extend(payload);
    file.extend(0u32.to_le_bytes());
    fs::create_dir_all(root.join("appcache")).unwrap();
    fs::write(root.join("appcache/appinfo.vdf"), file).unwrap();
}

fn libraries(root: &Path, shared: &Path) {
    let path = shared.display().to_string().replace('\\', "\\\\");
    fs::write(
        root.join("steamapps/libraryfolders.vdf"),
        format!(r#""libraryfolders" {{ "1" {{ "path" "{path}" }} }}"#),
    )
    .unwrap();
}

#[test]
fn preserves_install_identity_while_inheriting_external_libraries_and_isolating_roots() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("First Steam");
    let second = temp.path().join("Second Steam");
    let shared = temp.path().join("External Library");
    for root in [&first, &second, &shared] {
        write_manifest(root, 42);
    }
    libraries(&first, &shared);
    libraries(&second, &shared);
    language(&first, "schinese");
    language(&second, "schinese");
    appinfo(&first, 42, "Default A", "本地名甲");
    appinfo(&second, 42, "Default B", "本地名乙");
    let report = scan(&[first.clone(), second.clone()]);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    assert_eq!(report.installs.len(), 3);
    for game in &report.installs {
        assert_eq!(game.name, "Manifest 42");
        assert_eq!(game.name_language.as_deref(), Some("schinese"));
        let owner_b = game.library_path == fs::canonicalize(&second).unwrap();
        assert_eq!(
            game.localized_name.as_deref(),
            Some(if owner_b {
                "本地名乙"
            } else {
                "本地名甲"
            })
        );
    }
    let ids: Vec<_> = report
        .installs
        .iter()
        .map(|g| g.instance_id.clone())
        .collect();
    fs::write(first.join("appcache/appinfo.vdf"), b"corrupt").unwrap();
    let after = scan(&[first, second.clone()]);
    assert_eq!(
        after
            .installs
            .iter()
            .map(|g| &g.instance_id)
            .collect::<Vec<_>>(),
        ids.iter().collect::<Vec<_>>()
    );
    assert!(after.diagnostics.is_empty());
    for game in after.installs {
        if game.library_path != fs::canonicalize(&second).unwrap() {
            assert!(game.localized_name.is_none());
            assert!(game.name_language.is_none());
        } else {
            assert_eq!(game.localized_name.as_deref(), Some("本地名乙"));
        }
    }
}

#[test]
fn defaults_without_language_and_retains_manifest_when_names_are_empty_or_cache_absent() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write_manifest(root, 7);
    appinfo(root, 7, " Default ", "本地名");
    let report = scan(&[root.to_owned()]);
    assert_eq!(
        report.installs[0].localized_name.as_deref(),
        Some("Default")
    );
    assert!(report.installs[0].name_language.is_none());
    language(root, "english");
    let report = scan(&[root.to_owned()]);
    assert_eq!(
        report.installs[0].localized_name.as_deref(),
        Some("Default")
    );
    assert!(report.installs[0].name_language.is_none());
    language(root, "schinese");
    appinfo(root, 7, "", "  ");
    assert!(scan(&[root.to_owned()]).installs[0]
        .localized_name
        .is_none());
    fs::remove_file(root.join("appcache/appinfo.vdf")).unwrap();
    let report = scan(&[root.to_owned()]);
    assert_eq!(report.installs[0].name, "Manifest 7");
    assert!(report.installs[0].localized_name.is_none());
    assert!(report.diagnostics.is_empty());
}

fn directory_link(source: &Path, destination: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(source, destination).unwrap();
    #[cfg(windows)]
    {
        let output = std::process::Command::new("cmd.exe")
            .args(["/C", "mklink", "/J"])
            .arg(destination)
            .arg(source)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }
}

#[test]
fn root_aliases_work_but_linked_cache_and_config_sources_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Steam");
    let alias = temp.path().join("Alias");
    let outside = temp.path().join("Outside");
    write_manifest(&root, 42);
    language(&root, "schinese");
    appinfo(&root, 42, "Default", "中文显示");
    directory_link(&root, &alias);
    assert_eq!(
        scan(&[alias]).installs[0].localized_name.as_deref(),
        Some("中文显示")
    );
    fs::rename(root.join("config"), root.join("original-config")).unwrap();
    directory_link(&root.join("original-config"), &root.join("config"));
    let report = scan(std::slice::from_ref(&root));
    assert_eq!(
        report.installs[0].localized_name.as_deref(),
        Some("Default")
    );
    assert!(report.installs[0].name_language.is_none());
    fs::create_dir(&outside).unwrap();
    fs::rename(root.join("appcache"), outside.join("appcache")).unwrap();
    directory_link(&outside.join("appcache"), &root.join("appcache"));
    let report = scan(&[root]);
    assert!(report.installs[0].localized_name.is_none());
    assert!(report.diagnostics.is_empty());
}

#[test]
fn old_json_deserializes_with_optional_fields_and_serializes_nulls() {
    let json = r#"{"instance_id":"42|library|game","appid":42,"name":"Manifest","library_path":"library","install_path":"game","buildid":null,"availability":"installed","kind":"unknown"}"#;
    let game: GameInstall = serde_json::from_str(json).unwrap();
    assert!(game.localized_name.is_none());
    assert!(game.name_language.is_none());
    let report = serde_json::to_value(&game).unwrap();
    assert!(report["localized_name"].is_null());
    assert!(report["name_language"].is_null());
}
