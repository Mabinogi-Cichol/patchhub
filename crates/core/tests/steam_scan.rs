use patchhub_core::steam::scan;
use std::fs;
use std::path::{Path, PathBuf};

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn kv_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "\\\\")
}

fn manifest(appid: u32, name: &str, dir: &str, flags: Option<&str>) -> String {
    let flags = flags
        .map(|value| format!("\"StateFlags\" \"{value}\""))
        .unwrap_or_default();
    format!(
        r#"// synthetic Steam fixture
"AppState" {{
    "appid" "{appid}"
    "name" "{name}"
    "installdir" "{dir}"
    "buildid" "12345"
    {flags}
}}"#
    )
}

fn add_manifest(library: &Path, appid: u32, name: &str, dir: &str, flags: Option<&str>) {
    write(
        &library.join(format!("steamapps/appmanifest_{appid}.acf")),
        &manifest(appid, name, dir, flags),
    );
}

#[test]
fn scans_new_and_old_library_layouts_and_preserves_duplicate_appids() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Steam");
    let second = temp.path().join("SD Card 中文");
    let third = temp.path().join("Old Library");
    for lib in [&root, &second, &third] {
        fs::create_dir_all(lib.join("steamapps/common/Example Game")).unwrap();
    }
    write(
        &root.join("steamapps/libraryfolders.vdf"),
        &format!(
            r#"
"libraryfolders" {{
    "0" {{ "path" "{}" "apps" {{ "42" "1" }} }}
    "1" {{ "path" "{}" }}
    "2" "{}"
}}"#,
            kv_path(&root),
            kv_path(&second),
            kv_path(&third)
        ),
    );
    add_manifest(&root, 42, "First", "Example Game", Some("4"));
    add_manifest(&second, 42, "Second", "Example Game", Some("4"));
    add_manifest(&third, 7, "Third", "Example Game", Some("4"));
    let report = scan(&[root.clone(), second.clone()]);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    assert_eq!(report.installs.len(), 3);
    let copies: Vec<_> = report
        .installs
        .iter()
        .filter(|game| game.appid == 42)
        .collect();
    assert_eq!(copies.len(), 2);
    assert_ne!(copies[0].instance_id, copies[1].instance_id);
    assert!(report
        .installs
        .iter()
        .all(|game| game.availability == "installed" && game.kind == "unknown"));
}

#[test]
fn isolates_bad_manifests_and_classifies_incomplete_or_missing_installs() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Steam");
    fs::create_dir_all(root.join("steamapps/common/Downloading")).unwrap();
    add_manifest(&root, 1, "Download", "Downloading", Some("2"));
    add_manifest(&root, 2, "Absent", "Missing", Some("4"));
    add_manifest(&root, 3, "Unsafe", "../escape", Some("4"));
    write(
        &root.join("steamapps/appmanifest_4.acf"),
        "\"AppState\" { \"appid\" \"4\" \"appid\" \"4\" }",
    );
    add_manifest(&root, 5, "Wrong ID", "Wrong", Some("4"));
    let wrong = root.join("steamapps/appmanifest_5.acf");
    write(&wrong, &manifest(99, "Wrong ID", "Wrong", Some("4")));
    let report = scan(&[root]);
    assert_eq!(report.installs.len(), 2);
    assert_eq!(
        report
            .installs
            .iter()
            .find(|game| game.appid == 1)
            .unwrap()
            .availability,
        "downloading"
    );
    assert_eq!(
        report
            .installs
            .iter()
            .find(|game| game.appid == 2)
            .unwrap()
            .availability,
        "missing"
    );
    assert_eq!(
        report
            .diagnostics
            .iter()
            .filter(|diag| diag.code == "manifest_invalid")
            .count(),
        3
    );
}

#[test]
fn falls_back_to_config_libraryfolders_and_reports_offline_library() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Steam");
    let library = temp.path().join("Library");
    fs::create_dir_all(root.join("steamapps")).unwrap();
    fs::create_dir_all(library.join("steamapps/common/Valid")).unwrap();
    write(
        &root.join("config/libraryfolders.vdf"),
        &format!(
            "\"LibraryFolders\" {{ \"1\" \"{}\" \"2\" \"{}\" }}",
            kv_path(&library),
            kv_path(&temp.path().join("Offline"))
        ),
    );
    add_manifest(&library, 10, "Valid", "Valid", None);
    let report = scan(&[root]);
    assert_eq!(report.installs.len(), 1);
    assert_eq!(report.installs[0].availability, "unknown");
    assert!(report
        .diagnostics
        .iter()
        .any(|diag| diag.code == "library_unavailable"));
}

#[test]
fn reports_corrupt_library_index_but_still_scans_root() {
    let temp = tempfile::tempdir().unwrap();
    let root: PathBuf = temp.path().join("Steam");
    fs::create_dir_all(root.join("steamapps/common/Valid")).unwrap();
    write(
        &root.join("steamapps/libraryfolders.vdf"),
        "\"LibraryFolders\" { \"1\" \"bad\" ",
    );
    add_manifest(&root, 10, "Valid", "Valid", Some("4"));
    let report = scan(&[root]);
    assert_eq!(report.installs.len(), 1);
    assert!(report
        .diagnostics
        .iter()
        .any(|diag| diag.code == "libraryfolders_invalid"));
}

#[test]
fn fully_installed_bit_with_other_flags_is_not_ready() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Steam");
    fs::create_dir_all(root.join("steamapps/common/Updating")).unwrap();
    add_manifest(&root, 42, "Updating", "Updating", Some("1028"));
    let report = scan(&[root]);
    assert_eq!(report.installs[0].availability, "downloading");
}

#[cfg(any(windows, unix))]
#[test]
fn rejects_common_link_resolving_outside_library() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Steam");
    let outside = temp.path().join("Outside");
    fs::create_dir_all(root.join("steamapps")).unwrap();
    fs::create_dir_all(outside.join("Game")).unwrap();
    let link = root.join("steamapps/common");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    #[cfg(windows)]
    {
        // Junctions do not require Developer Mode or symlink privileges.
        let result = std::process::Command::new("cmd.exe")
            .args(["/C", "mklink", "/J"])
            .arg(link.to_string_lossy().replace('/', "\\"))
            .arg(outside.to_string_lossy().replace('/', "\\"))
            .output()
            .unwrap();
        assert!(result.status.success(), "{:?}", result);
    }
    add_manifest(&root, 42, "Outside", "Game", Some("4"));
    let report = scan(&[root]);
    assert!(report.installs.is_empty());
    assert_eq!(report.diagnostics[0].code, "manifest_invalid");
}
