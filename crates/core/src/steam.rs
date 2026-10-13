//! Read-only Steam library and app manifest discovery.
use crate::keyvalues::{self, Value};
use crate::steam_metadata;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ScanReport {
    pub installs: Vec<GameInstall>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct GameInstall {
    pub instance_id: String,
    pub appid: u32,
    pub name: String,
    /// Appinfo display override, including its default name fallback.
    pub localized_name: Option<String>,
    /// Present only when the override matches a known interface language.
    pub name_language: Option<String>,
    pub library_path: PathBuf,
    pub install_path: PathBuf,
    pub buildid: Option<String>,
    pub availability: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Diagnostic {
    pub code: String,
    pub path: PathBuf,
    pub message: String,
}

impl ScanReport {
    fn diagnostic(&mut self, code: &str, path: &Path, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic {
            code: code.into(),
            path: path.to_owned(),
            message: message.into(),
        });
    }
}

/// Discover likely Steam roots without reading games or modifying Steam state.
pub fn discover_roots() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    #[cfg(windows)]
    {
        // Read Unicode values through the registry API, not reg.exe's console code page.
        for (hive, path) in [
            (&winreg::HKCU, r"Software\Valve\Steam"),
            (&winreg::HKLM, r"Software\Valve\Steam"),
            (&winreg::HKLM, r"Software\WOW6432Node\Valve\Steam"),
        ] {
            if let Ok(key) = hive.open_subkey(path) {
                for value in ["SteamPath", "InstallPath"] {
                    if let Ok(path) = key.get_value::<String, _>(value) {
                        candidates.push(PathBuf::from(path));
                    }
                }
            }
        }
        for var in ["PROGRAMFILES(X86)", "PROGRAMFILES", "LOCALAPPDATA"] {
            if let Some(base) = std::env::var_os(var) {
                candidates.push(PathBuf::from(base).join("Steam"));
            }
        }
    }
    #[cfg(not(windows))]
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        candidates.push(home.join(".steam/steam"));
        candidates.push(home.join(".local/share/Steam"));
        candidates.push(home.join(".steam/root"));
    }
    unique_existing_dirs(candidates)
}

/// Scan explicit roots in their given order. Pass an empty slice to use discovered roots.
pub fn scan(roots: &[PathBuf]) -> ScanReport {
    let mut report = ScanReport {
        installs: Vec::new(),
        diagnostics: Vec::new(),
    };
    let roots = if roots.is_empty() {
        discover_roots()
    } else {
        roots.to_vec()
    };
    let mut libraries = Vec::new();
    let mut seen = HashSet::new();
    for root in roots {
        let root = match fs::canonicalize(&root) {
            Ok(path) if path.is_dir() => path,
            Ok(_) => {
                report.diagnostic(
                    "library_unavailable",
                    &root,
                    "Steam root is not a directory",
                );
                continue;
            }
            Err(err) => {
                report.diagnostic("library_unavailable", &root, err.to_string());
                continue;
            }
        };
        add_library(&mut libraries, &mut seen, &mut report, &root, &root);
        let primary = root.join("steamapps/libraryfolders.vdf");
        let fallback = root.join("config/libraryfolders.vdf");
        let file = if primary.exists() {
            Some(primary)
        } else if fallback.exists() {
            Some(fallback)
        } else {
            None
        };
        if let Some(file) = file {
            match read_keyvalues(&file).and_then(|doc| {
                let folders = field(&doc, "libraryfolders")
                    .and_then(Value::object)
                    .ok_or_else(|| "missing LibraryFolders object".to_owned())?;
                let mut paths = Vec::new();
                for (index, value) in folders {
                    if !index.chars().all(|ch| ch.is_ascii_digit()) {
                        continue;
                    }
                    let path = match value {
                        Value::Text(path) => Some(path.as_str()),
                        Value::Object(fields) => field(fields, "path").and_then(Value::text),
                    };
                    if let Some(path) = path {
                        paths.push(PathBuf::from(path));
                    }
                }
                Ok(paths)
            }) {
                Ok(paths) => {
                    for path in paths {
                        add_library(&mut libraries, &mut seen, &mut report, &path, &root);
                    }
                }
                Err(err) => report.diagnostic("libraryfolders_invalid", &file, err),
            }
        }
    }
    let mut root_installs: HashMap<PathBuf, Vec<usize>> = HashMap::new();
    for (library, owner) in libraries {
        let start = report.installs.len();
        scan_library(&library, &mut report);
        root_installs
            .entry(owner)
            .or_default()
            .extend(start..report.installs.len());
    }
    for (root, indices) in root_installs {
        let targets = indices.iter().map(|&i| report.installs[i].appid).collect();
        let names = steam_metadata::names_for_root(&root, &targets);
        for i in indices {
            let game = &mut report.installs[i];
            if let Some(name) = names.get(&game.appid) {
                game.localized_name = Some(name.name.clone());
                game.name_language = name.language.clone();
            }
        }
    }
    report.installs.sort_by(|a, b| {
        a.appid
            .cmp(&b.appid)
            .then(a.library_path.cmp(&b.library_path))
    });
    report
}

fn unique_existing_dirs(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    paths
        .into_iter()
        .filter_map(|path| fs::canonicalize(path).ok())
        .filter(|path| path.is_dir() && seen.insert(path.clone()))
        .collect()
}

fn add_library(
    libraries: &mut Vec<(PathBuf, PathBuf)>,
    seen: &mut HashSet<PathBuf>,
    report: &mut ScanReport,
    path: &Path,
    owner: &Path,
) {
    match fs::canonicalize(path) {
        Ok(path) if path.is_dir() => {
            if seen.insert(path.clone()) {
                libraries.push((path, owner.to_owned()));
            }
        }
        Ok(_) => report.diagnostic("library_unavailable", path, "library is not a directory"),
        Err(err) => report.diagnostic("library_unavailable", path, err.to_string()),
    }
}

fn scan_library(library: &Path, report: &mut ScanReport) {
    let steamapps = library.join("steamapps");
    let entries = match fs::read_dir(&steamapps) {
        Ok(entries) => entries,
        Err(err) => {
            report.diagnostic("library_unavailable", &steamapps, err.to_string());
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                report.diagnostic("library_read_error", &steamapps, err.to_string());
                continue;
            }
        };
        let file = entry.path();
        let Some(filename) = file.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(appid_text) = filename
            .strip_prefix("appmanifest_")
            .and_then(|name| name.strip_suffix(".acf"))
        else {
            continue;
        };
        let Ok(filename_appid) = appid_text.parse::<u32>() else {
            continue;
        };
        match parse_manifest(&file, library, filename_appid) {
            Ok(install) => report.installs.push(install),
            Err(err) => report.diagnostic("manifest_invalid", &file, err),
        }
    }
}

fn parse_manifest(file: &Path, library: &Path, filename_appid: u32) -> Result<GameInstall, String> {
    let document = read_keyvalues(file)?;
    let state = field(&document, "AppState")
        .and_then(Value::object)
        .ok_or_else(|| "missing AppState object".to_owned())?;
    let appid = required(state, "appid")?
        .parse::<u32>()
        .map_err(|_| "invalid appid".to_owned())?;
    if appid != filename_appid {
        return Err("manifest appid differs from filename".into());
    }
    let name = required(state, "name")?.to_owned();
    let installdir = required(state, "installdir")?;
    if !valid_install_dir(installdir) {
        return Err("unsafe installdir".into());
    }
    let install_path = library.join("steamapps/common").join(installdir);
    if install_path.is_dir() {
        let common =
            fs::canonicalize(library.join("steamapps/common")).map_err(|err| err.to_string())?;
        let resolved = fs::canonicalize(&install_path).map_err(|err| err.to_string())?;
        if !common.starts_with(library) || !resolved.starts_with(&common) {
            return Err("installdir resolves outside library".into());
        }
    }
    let stateflags = field(state, "StateFlags")
        .and_then(Value::text)
        .and_then(|text| text.parse::<u64>().ok());
    let availability = if library
        .join("steamapps/downloading")
        .join(appid.to_string())
        .exists()
        || stateflags.is_some_and(|flags| flags != 4)
    {
        "downloading"
    } else if !install_path.is_dir() {
        "missing"
    } else if stateflags.is_some() {
        "installed"
    } else {
        "unknown"
    };
    let buildid = field(state, "buildid")
        .and_then(Value::text)
        .map(str::to_owned);
    let instance_id = format!("{appid}|{}|{}", library.display(), install_path.display());
    Ok(GameInstall {
        instance_id,
        appid,
        name,
        localized_name: None,
        name_language: None,
        library_path: library.to_owned(),
        install_path,
        buildid,
        availability: availability.into(),
        kind: "unknown".into(),
    })
}

fn valid_install_dir(name: &str) -> bool {
    !name.trim().is_empty()
        && name != "."
        && name != ".."
        && !name.ends_with('.')
        && !name.ends_with(' ')
        && !name.chars().any(|ch| {
            ch.is_control() || matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
        && Path::new(name)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn read_keyvalues(path: &Path) -> Result<BTreeMap<String, Value>, String> {
    let size = fs::metadata(path).map_err(|err| err.to_string())?.len();
    if size > MAX_FILE_BYTES {
        return Err("Steam metadata exceeds 4 MiB".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|err| err.to_string())?
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|err| err.to_string())?;
    keyvalues::parse(&bytes).map_err(|err| err.to_string())
}

fn field<'a>(object: &'a BTreeMap<String, Value>, key: &str) -> Option<&'a Value> {
    object
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(key))
        .map(|(_, value)| value)
}

fn required<'a>(object: &'a BTreeMap<String, Value>, key: &str) -> Result<&'a str, String> {
    field(object, key)
        .and_then(Value::text)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| format!("missing {key}"))
}
