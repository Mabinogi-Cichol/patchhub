//! Optional Steam name metadata; failures leave the manifest scan intact.
use crate::appinfo::{self, ResolvedName};
use crate::keyvalues::{self, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

const MAX_CONFIG_BYTES: u64 = 4 * 1024 * 1024;
const MAX_APPINFO_BYTES: u64 = 256 * 1024 * 1024;

// Unix lookups stay relative to these handles. Windows directory handles
// exclude write/delete sharing to prevent traversal components being replaced.
struct MetadataFile {
    file: File,
    _directories: Vec<File>,
}

enum OpenDirError {
    NotFound(String),
    Blocked(String),
}

impl OpenDirError {
    fn into_message(self) -> String {
        match self {
            Self::NotFound(message) | Self::Blocked(message) => message,
        }
    }
}

#[cfg(unix)]
fn open_dir_chain(root: &Path, dirs: &[&str]) -> Result<Vec<File>, OpenDirError> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::fs::OpenOptionsExt;

    let root_file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                OpenDirError::NotFound(e.to_string())
            } else {
                OpenDirError::Blocked(e.to_string())
            }
        })?;
    let mut handles = vec![root_file];
    for &dir in dirs {
        let name = CString::new(dir).map_err(|e| OpenDirError::Blocked(e.to_string()))?;
        let flags = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_DIRECTORY;
        let parent = handles.last().unwrap();
        let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            let err = std::io::Error::last_os_error();
            return Err(if err.raw_os_error() == Some(libc::ENOENT) {
                OpenDirError::NotFound(err.to_string())
            } else {
                OpenDirError::Blocked(err.to_string())
            });
        }
        handles.push(unsafe { File::from_raw_fd(fd) });
    }
    Ok(handles)
}

#[cfg(unix)]
fn open_metadata_path(root: &Path, dirs: &[&str], filename: &str) -> Result<MetadataFile, String> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};

    let directories = open_dir_chain(root, dirs).map_err(OpenDirError::into_message)?;
    let name = CString::new(filename).map_err(|e| e.to_string())?;
    let flags = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK;
    let parent = directories.last().unwrap();
    // Component names are fixed and have no separators; the parent handle
    // anchors lookup even if the path is replaced between operations.
    let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    // Successful openat transfers this fresh descriptor to File.
    let file = unsafe { File::from_raw_fd(fd) };
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Steam name metadata is not a regular file".into());
    }
    Ok(MetadataFile {
        file,
        _directories: directories,
    })
}

#[cfg(windows)]
fn open_windows_dir(path: &Path) -> Result<File, OpenDirError> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    const FILE_READ_ATTRIBUTES: u32 = 0x80;
    const FILE_SHARE_READ: u32 = 1;

    let file = fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                OpenDirError::NotFound(e.to_string())
            } else {
                OpenDirError::Blocked(e.to_string())
            }
        })?;
    let metadata = file
        .metadata()
        .map_err(|e| OpenDirError::Blocked(e.to_string()))?;
    if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(OpenDirError::Blocked(
            "Steam metadata directory is a reparse point or not a directory".into(),
        ));
    }
    Ok(file)
}

#[cfg(windows)]
fn open_dir_chain(root: &Path, dirs: &[&str]) -> Result<Vec<File>, OpenDirError> {
    let mut handles = Vec::with_capacity(dirs.len() + 1);
    let mut current = root.to_path_buf();
    handles.push(open_windows_dir(&current)?);
    for &dir in dirs {
        current.push(dir);
        handles.push(open_windows_dir(&current)?);
    }
    Ok(handles)
}

#[cfg(windows)]
fn open_metadata_path(root: &Path, dirs: &[&str], filename: &str) -> Result<MetadataFile, String> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    const FILE_SHARE_READ: u32 = 1;

    let directories = open_dir_chain(root, dirs).map_err(OpenDirError::into_message)?;
    let mut file_path = root.to_path_buf();
    for &dir in dirs {
        file_path.push(dir);
    }
    file_path.push(filename);
    let file = fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .open(file_path)
        .map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err("Steam metadata is a reparse point or not a regular file".into());
    }
    Ok(MetadataFile {
        file,
        _directories: directories,
    })
}

#[cfg(not(any(unix, windows)))]
fn open_dir_chain(_root: &Path, _dirs: &[&str]) -> Result<Vec<File>, OpenDirError> {
    Err(OpenDirError::Blocked(
        "Safe Steam name metadata reading is unavailable on this platform".into(),
    ))
}

#[cfg(not(any(unix, windows)))]
fn open_metadata_path(
    _root: &Path,
    _dirs: &[&str],
    _filename: &str,
) -> Result<MetadataFile, String> {
    Err("Safe Steam name metadata reading is unavailable on this platform".into())
}

#[cfg(test)]
fn open_metadata(root: &Path, directory: &str, filename: &str) -> Result<MetadataFile, String> {
    open_metadata_path(root, &[directory], filename)
}

fn read_metadata_path<T>(
    root: &Path,
    dirs: &[&str],
    filename: &str,
    max_bytes: u64,
    mut parse: impl FnMut(&mut File) -> Result<T, String>,
) -> Result<T, String> {
    let mut last_error = String::new();
    for _ in 0..2 {
        let attempt = (|| {
            let mut opened = open_metadata_path(root, dirs, filename)?;
            let before = opened.file.metadata().map_err(|e| e.to_string())?;
            if before.len() > max_bytes {
                return Err("Steam name metadata exceeds its size limit".into());
            }
            let parsed = parse(&mut opened.file)?;
            let after = opened.file.metadata().map_err(|e| e.to_string())?;
            if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
                return Err("Steam name metadata changed while reading".into());
            }
            Ok(parsed)
        })();
        match attempt {
            Ok(value) => return Ok(value),
            Err(error) => last_error = error,
        }
    }
    Err(last_error)
}

fn read_metadata<T>(
    root: &Path,
    directory: &str,
    filename: &str,
    max_bytes: u64,
    parse: impl FnMut(&mut File) -> Result<T, String>,
) -> Result<T, String> {
    read_metadata_path(root, &[directory], filename, max_bytes, parse)
}

fn field<'a>(object: &'a BTreeMap<String, Value>, key: &str) -> Option<&'a Value> {
    object
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v)
}

fn normalize_language(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_ascii_lowercase())
}

fn config_language(root: &Path) -> Option<String> {
    read_metadata(root, "config", "config.vdf", MAX_CONFIG_BYTES, |file| {
        let mut bytes = Vec::new();
        file.take(MAX_CONFIG_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        let document = keyvalues::parse(&bytes).map_err(|e| e.to_string())?;
        let mut object = &document;
        for key in ["InstallConfigStore", "Software", "Valve", "Steam"] {
            object = field(object, key)
                .and_then(Value::object)
                .ok_or("missing language path")?;
        }
        Ok(field(object, "Language")
            .and_then(Value::text)
            .and_then(normalize_language))
    })
    .ok()
    .flatten()
}

struct RegistryLanguage {
    root: PathBuf,
    language: String,
}

#[cfg(windows)]
fn registry_language() -> Option<RegistryLanguage> {
    let key = winreg::HKCU.open_subkey(r"Software\Valve\Steam").ok()?;
    Some(RegistryLanguage {
        root: PathBuf::from(key.get_value::<String, _>("SteamPath").ok()?),
        language: key.get_value("Language").ok()?,
    })
}

#[cfg(not(windows))]
fn registry_language() -> Option<RegistryLanguage> {
    None
}

fn language_for_root(root: &Path, registry: Option<RegistryLanguage>) -> Option<String> {
    config_language(root).or_else(|| {
        let registry = registry?;
        let registry_root = fs::canonicalize(registry.root).ok()?;
        (registry_root == root)
            .then(|| normalize_language(&registry.language))
            .flatten()
    })
}

pub(crate) fn interface_language(root: &Path) -> Option<String> {
    language_for_root(root, registry_language())
}

enum ShardedDir {
    Missing,
    Blocked,
    Present(Vec<String>),
}

fn is_sharded_appinfo_filename(name: &str) -> bool {
    let Some(hex) = name
        .strip_prefix("appinfo_")
        .and_then(|rest| rest.strip_suffix(".vdf"))
    else {
        return false;
    };
    !hex.is_empty()
        && hex
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn list_appinfo_shards(root: &Path) -> ShardedDir {
    let _guards = match open_dir_chain(root, &["appcache", "appinfo"]) {
        Ok(handles) => handles,
        Err(OpenDirError::NotFound(_)) => return ShardedDir::Missing,
        Err(OpenDirError::Blocked(_)) => return ShardedDir::Blocked,
    };
    let Ok(entries) = fs::read_dir(root.join("appcache").join("appinfo")) else {
        return ShardedDir::Blocked;
    };
    let mut shards = Vec::new();
    for entry in entries.flatten() {
        if let Some(name) = entry.file_name().to_str() {
            if is_sharded_appinfo_filename(name) {
                shards.push(name.to_owned());
            }
        }
    }
    shards.sort();
    ShardedDir::Present(shards)
}

fn collect_missing_dlc_targets(
    names: &HashMap<u32, ResolvedName>,
    targets: &HashSet<u32>,
) -> HashSet<u32> {
    names
        .values()
        .flat_map(|app| app.dlc_appids.iter().copied())
        .filter(|appid| !targets.contains(appid))
        .collect()
}

pub(crate) fn names_for_root(
    root: &Path,
    targets: &HashSet<u32>,
    language: Option<&str>,
) -> HashMap<u32, ResolvedName> {
    if targets.is_empty() {
        return HashMap::new();
    }
    match list_appinfo_shards(root) {
        ShardedDir::Blocked => HashMap::new(),
        ShardedDir::Present(shards) if !shards.is_empty() => {
            let mut names = HashMap::new();
            for shard in &shards {
                if let Ok(parsed) = read_metadata_path(
                    root,
                    &["appcache", "appinfo"],
                    shard,
                    MAX_APPINFO_BYTES,
                    |file| appinfo::parse_names(file, targets, language),
                ) {
                    names.extend(parsed);
                }
            }
            let dlc_targets = collect_missing_dlc_targets(&names, targets);
            if !dlc_targets.is_empty() {
                for shard in &shards {
                    if let Ok(parsed) = read_metadata_path(
                        root,
                        &["appcache", "appinfo"],
                        shard,
                        MAX_APPINFO_BYTES,
                        |file| appinfo::parse_names(file, &dlc_targets, language),
                    ) {
                        names.extend(parsed);
                    }
                }
            }
            names
        }
        ShardedDir::Missing | ShardedDir::Present(_) => {
            read_metadata(root, "appcache", "appinfo.vdf", MAX_APPINFO_BYTES, |file| {
                let mut names = appinfo::parse_names(file, targets, language)?;
                let dlc_targets = collect_missing_dlc_targets(&names, targets);
                if !dlc_targets.is_empty() {
                    names.extend(appinfo::parse_names(file, &dlc_targets, language)?);
                }
                Ok(names)
            })
            .unwrap_or_default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(root: &Path, language: &str) {
        fs::create_dir_all(root.join("config")).unwrap();
        fs::write(
            root.join("config/config.vdf"),
            format!(r#""InstallConfigStore" {{ "Software" {{ "Valve" {{ "Steam" {{ "Language" "{language}" }} }} }} }}"#),
        ).unwrap();
    }

    fn registry(root: &Path, language: &str) -> Option<RegistryLanguage> {
        Some(RegistryLanguage {
            root: root.to_owned(),
            language: language.into(),
        })
    }

    #[test]
    fn language_sources_are_normalized_and_scoped_to_the_root() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        assert_eq!(
            language_for_root(&root, registry(&root, " SCHINESE ")).as_deref(),
            Some("schinese")
        );
        config(&root, " TCHINESE ");
        assert_eq!(
            language_for_root(&root, registry(&root, "schinese")).as_deref(),
            Some("tchinese")
        );
        config(&root, "  ");
        assert_eq!(
            language_for_root(&root, registry(&root, "schinese")).as_deref(),
            Some("schinese")
        );
        fs::write(root.join("config/config.vdf"), b"broken {").unwrap();
        assert_eq!(
            language_for_root(&root, registry(&root, "schinese")).as_deref(),
            Some("schinese")
        );
        let other = tempfile::tempdir().unwrap();
        assert_eq!(
            language_for_root(&root, registry(other.path(), "schinese")),
            None
        );
        assert_eq!(language_for_root(&root, registry(&root, " ")), None);
        fs::write(
            root.join("config/config.vdf"),
            br#""InstallConfigStore" { "Software" {} }"#,
        )
        .unwrap();
        assert_eq!(
            language_for_root(&root, registry(&root, "schinese")).as_deref(),
            Some("schinese")
        );
        config(&root, " UNKNOWN_LANGUAGE ");
        assert_eq!(
            language_for_root(&root, None).as_deref(),
            Some("unknown_language")
        );
    }

    #[test]
    fn retries_once_with_a_fresh_handle_and_enforces_file_size_before_parsing() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        fs::create_dir(root.join("config")).unwrap();
        fs::write(root.join("config/config.vdf"), b"abc").unwrap();
        let mut calls = 0;
        let value = read_metadata(&root, "config", "config.vdf", 3, |file| {
            calls += 1;
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).unwrap();
            if calls == 1 {
                Err("transient read failure".into())
            } else {
                Ok(bytes)
            }
        })
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(value, b"abc");
        calls = 0;
        assert!(read_metadata(&root, "config", "config.vdf", 3, |_| {
            calls += 1;
            Err::<(), _>("always fails".into())
        })
        .is_err());
        assert_eq!(calls, 2);
        calls = 0;
        assert!(read_metadata(&root, "config", "config.vdf", 2, |_| {
            calls += 1;
            Ok(())
        })
        .is_err());
        assert_eq!(calls, 0);
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
    fn rejects_linked_parent_and_non_regular_final_component() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("config.vdf"), b"outside").unwrap();
        directory_link(outside.path(), &root.join("config"));
        assert!(open_metadata(&root, "config", "config.vdf").is_err());
        fs::create_dir(root.join("appcache")).unwrap();
        directory_link(outside.path(), &root.join("appcache/appinfo.vdf"));
        assert!(open_metadata(&root, "appcache", "appinfo.vdf").is_err());
        directory_link(outside.path(), &root.join("appcache/appinfo"));
        assert!(matches!(list_appinfo_shards(&root), ShardedDir::Blocked));
        assert!(names_for_root(&root, &HashSet::from([1227890]), Some("schinese")).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_final_symlink_and_fifo_without_blocking() {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        fs::create_dir(root.join("config")).unwrap();
        let target = root.join("outside.vdf");
        fs::write(&target, b"outside").unwrap();
        let path = root.join("config/config.vdf");
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(open_metadata(&root, "config", "config.vdf").is_err());
        fs::remove_file(&path).unwrap();
        let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
        assert!(open_metadata(&root, "config", "config.vdf").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn held_handles_block_directory_replacement_and_file_writes() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        config(&root, "schinese");
        let opened = open_metadata(&root, "config", "config.vdf").unwrap();
        assert!(fs::rename(root.join("config"), root.join("moved")).is_err());
        assert!(fs::write(root.join("config/config.vdf"), b"changed").is_err());
        drop(opened);
        fs::rename(root.join("config"), root.join("moved")).unwrap();
    }

    fn v28_container(entries: Vec<(u32, &str, &str)>) -> Vec<u8> {
        let mut bytes = 0x0756_4428u32.to_le_bytes().to_vec();
        bytes.extend_from_slice(&1u32.to_le_bytes());
        for (appid, default_name, schinese_name) in entries {
            let mut payload = Vec::new();
            payload.extend_from_slice(b"\x00appinfo\x00\x00common\x00\x01name\x00");
            payload.extend_from_slice(default_name.as_bytes());
            payload.push(0);
            payload.extend_from_slice(b"\x00name_localized\x00\x01schinese\x00");
            payload.extend_from_slice(schinese_name.as_bytes());
            payload.extend_from_slice(b"\x00\x08\x08\x08\x08");
            bytes.extend_from_slice(&appid.to_le_bytes());
            bytes.extend_from_slice(&(60 + payload.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&[0; 60]);
            bytes.extend_from_slice(&payload);
        }
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes
    }

    fn v2a_shard(bucket: u32, entries: Vec<(u32, &str, &str, Option<u32>)>) -> Vec<u8> {
        let keys = [
            "appinfo",
            "common",
            "name",
            "name_localized",
            "schinese",
            "extended",
            "listofdlc",
        ];
        let mut key_table = Vec::new();
        for key in keys {
            key_table.extend_from_slice(key.as_bytes());
            key_table.push(0);
        }
        let mut dir = Vec::new();
        let mut data = Vec::new();
        for (appid, default_name, schinese_name, dlc) in entries {
            let mut vdf = Vec::new();
            vdf.push(0);
            vdf.extend_from_slice(&0i32.to_le_bytes()); // appinfo
            vdf.push(0);
            vdf.extend_from_slice(&1i32.to_le_bytes()); // common
            vdf.push(1);
            vdf.extend_from_slice(&2i32.to_le_bytes()); // name
            vdf.extend_from_slice(default_name.as_bytes());
            vdf.push(0);
            vdf.push(0);
            vdf.extend_from_slice(&3i32.to_le_bytes()); // name_localized
            vdf.push(1);
            vdf.extend_from_slice(&4i32.to_le_bytes()); // schinese
            vdf.extend_from_slice(schinese_name.as_bytes());
            vdf.extend_from_slice(&[0, 8, 8]); // end name_localized, end common
            if let Some(dlc_id) = dlc {
                vdf.push(0);
                vdf.extend_from_slice(&5i32.to_le_bytes()); // extended
                vdf.push(2);
                vdf.extend_from_slice(&6i32.to_le_bytes()); // listofdlc
                vdf.extend_from_slice(&dlc_id.to_le_bytes());
                vdf.push(8);
            }
            vdf.push(8); // end appinfo
            vdf.push(8); // end root

            let offset = data.len() as u32;
            let size = (20 + vdf.len()) as u32;
            let mut entry = vec![0u8; 80];
            entry[0..4].copy_from_slice(&appid.to_le_bytes());
            entry[4..8].copy_from_slice(&offset.to_le_bytes());
            entry[8..12].copy_from_slice(&size.to_le_bytes());
            dir.extend_from_slice(&entry);
            data.extend_from_slice(&[0u8; 20]);
            data.extend_from_slice(&vdf);
        }
        let mut out = Vec::new();
        out.extend_from_slice(&0x0756_442Au32.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&bucket.to_le_bytes());
        out.extend_from_slice(&256u32.to_le_bytes());
        out.extend_from_slice(&((dir.len() / 80) as u32).to_le_bytes());
        out.extend_from_slice(&(dir.len() as u32).to_le_bytes());
        out.extend_from_slice(&(keys.len() as u32).to_le_bytes());
        out.extend_from_slice(&(key_table.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&dir);
        out.extend_from_slice(&key_table);
        out.extend_from_slice(&data);
        out
    }

    #[test]
    fn sharded_appinfo_prefers_shards_isolates_corrupt_shards_and_falls_back_to_legacy() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        fs::create_dir(root.join("appcache")).unwrap();
        fs::write(
            root.join("appcache/appinfo.vdf"),
            v28_container(vec![(42, "Legacy", "老版本中文")]),
        )
        .unwrap();

        // Missing appcache/appinfo directory reads legacy appinfo.vdf.
        let legacy = names_for_root(&root, &HashSet::from([42]), Some("schinese"));
        assert_eq!(legacy[&42].name, "老版本中文");

        // Empty appcache/appinfo directory falls back to legacy appinfo.vdf.
        fs::create_dir(root.join("appcache/appinfo")).unwrap();
        let empty_dir = names_for_root(&root, &HashSet::from([42]), Some("schinese"));
        assert_eq!(empty_dir[&42].name, "老版本中文");

        // Once valid sharded files exist, shards win over stale appinfo.vdf and bad shards are isolated.
        fs::write(
            root.join("appcache/appinfo/appinfo_00.vdf"),
            v2a_shard(0, vec![(42, "Sharded Game", "分片中文名", Some(43))]),
        )
        .unwrap();
        fs::write(
            root.join("appcache/appinfo/appinfo_01.vdf"),
            v2a_shard(1, vec![(43, "Sharded DLC", "分片DLC中文", None)]),
        )
        .unwrap();
        fs::write(root.join("appcache/appinfo/appinfo_02.vdf"), b"corrupt").unwrap();
        fs::write(root.join("appcache/appinfo/ignore_me.vdf"), b"corrupt").unwrap();

        let sharded = names_for_root(&root, &HashSet::from([42]), Some("schinese"));
        assert_eq!(sharded[&42].name, "分片中文名");
        assert_eq!(sharded[&43].name, "分片DLC中文");
        assert_eq!(sharded[&42].dlc_appids, vec![43]);
        assert!(is_sharded_appinfo_filename("appinfo_0a.vdf"));
        assert!(!is_sharded_appinfo_filename("appinfo_0A.vdf"));
        assert!(!is_sharded_appinfo_filename("appinfo.vdf"));
    }
}
