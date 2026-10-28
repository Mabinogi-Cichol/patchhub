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

#[cfg(unix)]
fn open_metadata(root: &Path, directory: &str, filename: &str) -> Result<MetadataFile, String> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::fs::OpenOptionsExt;

    let root_file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root)
        .map_err(|e| e.to_string())?;
    let open_child = |parent: &File, name: &str, is_directory: bool| -> Result<File, String> {
        let name = CString::new(name).map_err(|e| e.to_string())?;
        let flags = libc::O_RDONLY
            | libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            | if is_directory {
                libc::O_DIRECTORY
            } else {
                libc::O_NONBLOCK
            };
        // Component names are fixed and have no separators; the parent handle
        // anchors lookup even if the path is replaced between operations.
        let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        // Successful openat transfers this fresh descriptor to File.
        Ok(unsafe { File::from_raw_fd(fd) })
    };
    let directory_file = open_child(&root_file, directory, true)?;
    let file = open_child(&directory_file, filename, false)?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Steam name metadata is not a regular file".into());
    }
    Ok(MetadataFile {
        file,
        _directories: vec![root_file, directory_file],
    })
}

#[cfg(windows)]
fn open_metadata(root: &Path, directory: &str, filename: &str) -> Result<MetadataFile, String> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    const FILE_READ_ATTRIBUTES: u32 = 0x80;
    const FILE_SHARE_READ: u32 = 1;

    let open_directory = |path: &Path| -> Result<File, String> {
        let file = fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
            .map_err(|e| e.to_string())?;
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err("Steam metadata directory is a reparse point or not a directory".into());
        }
        Ok(file)
    };
    let root_file = open_directory(root)?;
    let directory_path = root.join(directory);
    let directory_file = open_directory(&directory_path)?;
    let file = fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .open(directory_path.join(filename))
        .map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err("Steam metadata is a reparse point or not a regular file".into());
    }
    Ok(MetadataFile {
        file,
        _directories: vec![root_file, directory_file],
    })
}

#[cfg(not(any(unix, windows)))]
fn open_metadata(_root: &Path, _directory: &str, _filename: &str) -> Result<MetadataFile, String> {
    Err("Safe Steam name metadata reading is unavailable on this platform".into())
}

fn read_metadata<T>(
    root: &Path,
    directory: &str,
    filename: &str,
    max_bytes: u64,
    mut parse: impl FnMut(&mut File) -> Result<T, String>,
) -> Result<T, String> {
    let mut last_error = String::new();
    for _ in 0..2 {
        let attempt = (|| {
            let mut opened = open_metadata(root, directory, filename)?;
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

pub(crate) fn names_for_root(
    root: &Path,
    targets: &HashSet<u32>,
    language: Option<&str>,
) -> HashMap<u32, ResolvedName> {
    if targets.is_empty() {
        return HashMap::new();
    }
    read_metadata(root, "appcache", "appinfo.vdf", MAX_APPINFO_BYTES, |file| {
        appinfo::parse_names(file, targets, language)
    })
    .unwrap_or_default()
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
}
