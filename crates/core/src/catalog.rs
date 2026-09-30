use crate::steam::GameInstall;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
};

pub const MAX_CATALOG_BYTES: u64 = 4 * 1024 * 1024;
const MAX_FINGERPRINT_BYTES: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub schema_version: u32,
    pub revision: u64,
    pub releases: Vec<PatchRelease>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Windows,
    Steamos,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SupportLevel {
    DiscoverOnly,
    ManualDownload,
    ManagedZip,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PatchRelease {
    pub patch_id: String,
    pub release_id: String,
    pub appid: u32,
    pub name: String,
    pub version: String,
    pub language: String,
    pub category: String,
    pub source_page: String,
    pub support_level: SupportLevel,
    pub platforms: Vec<Platform>,
    pub verified_buildids: Vec<String>,
    pub detection: Vec<Fingerprint>,
    pub recipe: Option<Recipe>,
    #[serde(default)]
    pub artifact: Option<Artifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub url: String,
    pub size: u64,
    pub sha256: String,
    pub allowed_redirect_hosts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Fingerprint {
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub artifact_sha256: String,
    pub files: Vec<FileMapping>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileMapping {
    pub source: String,
    pub destination: String,
    /// None means the destination must not already exist.
    pub original_sha256: Option<String>,
    pub installed_sha256: String,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct PatchMatch {
    pub release_id: String,
    pub name: String,
    pub source_page: String,
    pub support_level: SupportLevel,
    pub download_available: bool,
    pub compatibility: String,
    pub blockers: Vec<String>,
    pub detection: Detection,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct Detection {
    pub status: String,
    pub files: Vec<FileEvidence>,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct FileEvidence {
    pub path: String,
    pub status: String,
    pub actual_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct InstallPreview {
    pub instance_id: String,
    pub release_id: String,
    pub catalog_revision: u64,
    pub executable: bool,
    pub blockers: Vec<String>,
    pub changes: Vec<ProposedChange>,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct ProposedChange {
    pub path: String,
    pub action: String,
    pub baseline_matches: bool,
    pub actual_sha256: Option<String>,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

pub fn load(path: &Path) -> io::Result<Catalog> {
    let file = File::open(path)?;
    let mut bytes = Vec::new();
    file.take(MAX_CATALOG_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CATALOG_BYTES {
        return Err(invalid("catalog_too_large"));
    }
    let catalog: Catalog = serde_json::from_slice(&bytes).map_err(|e| invalid(e.to_string()))?;
    catalog.validate()?;
    Ok(catalog)
}

fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Portable file paths: forward slashes only; reject Windows aliases on Linux too.
pub fn relative_path(path: &str) -> io::Result<PathBuf> {
    if path.is_empty() || path.len() > 1024 || path.contains(['\\', ':', '\0']) {
        return Err(invalid("unsafe_relative_path"));
    }
    for part in path.split('/') {
        let device = part
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        let numbered_device = device.len() == 4
            && (device.starts_with("COM") || device.starts_with("LPT"))
            && device.as_bytes()[3].is_ascii_digit();
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with(['.', ' '])
            || part
                .chars()
                .any(|c| c.is_control() || "<>\"|?*".contains(c))
            || ["CON", "PRN", "AUX", "NUL"].contains(&device.as_str())
            || numbered_device
        {
            return Err(invalid("unsafe_relative_path"));
        }
    }
    Ok(PathBuf::from(path))
}

impl Catalog {
    pub fn validate(&self) -> io::Result<()> {
        if self.schema_version != 1 {
            return Err(invalid("unsupported_catalog_schema"));
        }
        let mut ids = HashSet::new();
        for release in &self.releases {
            if release.appid == 0
                || release.release_id.trim().is_empty()
                || release.patch_id.trim().is_empty()
                || release.name.trim().is_empty()
                || !ids.insert(&release.release_id)
                || release.platforms.is_empty()
            {
                return Err(invalid("invalid_release_identity"));
            }
            // URLs are descriptive only in P0; never fetched or executed.
            if !release.source_page.starts_with("https://")
                || release.source_page[8..].is_empty()
                || release.source_page.chars().any(|c| c.is_control())
            {
                return Err(invalid("invalid_source_page"));
            }
            let mut seen = HashSet::new();
            for rule in &release.detection {
                relative_path(&rule.path)?;
                if !valid_hash(&rule.sha256) || !seen.insert(rule.path.to_lowercase()) {
                    return Err(invalid("invalid_detection_rule"));
                }
            }
            if let Some(recipe) = &release.recipe {
                if !valid_hash(&recipe.artifact_sha256) || recipe.files.is_empty() {
                    return Err(invalid("invalid_recipe"));
                }
                let mut targets = HashSet::new();
                for file in &recipe.files {
                    relative_path(&file.source)?;
                    relative_path(&file.destination)?;
                    if !targets.insert(file.destination.to_lowercase())
                        || !valid_hash(&file.installed_sha256)
                        || file
                            .original_sha256
                            .as_ref()
                            .is_some_and(|h| !valid_hash(h))
                    {
                        return Err(invalid("invalid_file_mapping"));
                    }
                }
            }
            if release.support_level == SupportLevel::ManagedZip
                && (release.recipe.is_none()
                    || release.verified_buildids.is_empty()
                    || release.detection.is_empty())
            {
                return Err(invalid("managed_release_missing_evidence"));
            }
            if let Some(artifact) = &release.artifact {
                crate::download::validate_artifact(artifact)?;
                if release
                    .recipe
                    .as_ref()
                    .is_some_and(|r| r.artifact_sha256 != artifact.sha256)
                {
                    return Err(invalid("artifact_recipe_hash_mismatch"));
                }
            }
        }
        Ok(())
    }
}

fn linked(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn hash_file(root: &Path, library: &Path, relative: &str) -> io::Result<Option<String>> {
    let relative = relative_path(relative)?;
    if linked(&fs::symlink_metadata(root)?) {
        return Err(invalid("linked_install_root"));
    }
    let root = fs::canonicalize(root)?;
    let library = fs::canonicalize(library)?;
    if !root.starts_with(&library) {
        return Err(invalid("install_outside_library"));
    }
    let mut target = root.clone();
    for part in &relative {
        target.push(part);
        match fs::symlink_metadata(&target) {
            Ok(metadata) if linked(&metadata) => return Err(invalid("linked_target")),
            Ok(_) => (),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        }
    }
    if !fs::canonicalize(&target)?.starts_with(&root) {
        return Err(invalid("target_outside_game"));
    }
    let mut file = File::open(target)?;
    if !file.metadata()?.is_file() {
        return Err(invalid("fingerprint_not_a_file"));
    }
    if file.metadata()?.len() > MAX_FINGERPRINT_BYTES {
        return Err(invalid("fingerprint_too_large"));
    }
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > MAX_FINGERPRINT_BYTES {
            return Err(invalid("fingerprint_too_large"));
        }
        hasher.update(&buffer[..count]);
    }
    Ok(Some(format!("{:x}", hasher.finalize())))
}

pub fn detect(install: &GameInstall, release: &PatchRelease) -> Detection {
    if release.detection.is_empty() || install.availability != "installed" {
        return Detection {
            status: "unknown".into(),
            files: Vec::new(),
        };
    }
    let files: Vec<_> = release
        .detection
        .iter()
        .map(
            |rule| match hash_file(&install.install_path, &install.library_path, &rule.path) {
                Ok(Some(hash)) => FileEvidence {
                    path: rule.path.clone(),
                    status: if hash == rule.sha256 {
                        "match"
                    } else {
                        "mismatch"
                    }
                    .into(),
                    actual_sha256: Some(hash),
                },
                Ok(None) => FileEvidence {
                    path: rule.path.clone(),
                    status: "missing".into(),
                    actual_sha256: None,
                },
                Err(_) => FileEvidence {
                    path: rule.path.clone(),
                    status: "unreadable_or_unsafe".into(),
                    actual_sha256: None,
                },
            },
        )
        .collect();
    let status = if files.iter().any(|f| f.status == "unreadable_or_unsafe") {
        "unknown"
    } else if files.iter().all(|f| f.status == "match") {
        "installed_external"
    } else if files.iter().all(|f| f.status == "missing") {
        "not_detected"
    } else {
        "partial_or_conflict"
    };
    Detection {
        status: status.into(),
        files,
    }
}

fn compatibility(install: &GameInstall, release: &PatchRelease, platform: Platform) -> Vec<String> {
    let mut blockers = Vec::new();
    if !release.platforms.contains(&platform) {
        blockers.push("platform_not_verified".into());
    }
    if install.availability != "installed" {
        blockers.push("game_not_ready".into());
    }
    if !install
        .buildid
        .as_ref()
        .is_some_and(|b| release.verified_buildids.contains(b))
    {
        blockers.push("build_not_verified".into());
    }
    if release.support_level != SupportLevel::ManagedZip {
        blockers.push("source_requires_guidance".into());
    }
    blockers
}

pub fn matches(install: &GameInstall, catalog: &Catalog, platform: Platform) -> Vec<PatchMatch> {
    catalog
        .releases
        .iter()
        .filter(|r| r.appid == install.appid)
        .map(|r| {
            let blockers = compatibility(install, r, platform);
            PatchMatch {
                release_id: r.release_id.clone(),
                name: r.name.clone(),
                source_page: r.source_page.clone(),
                support_level: r.support_level,
                download_available: r.artifact.is_some(),
                compatibility: if blockers.is_empty() {
                    "verified_metadata"
                } else {
                    "needs_review"
                }
                .into(),
                blockers,
                detection: detect(install, r),
            }
        })
        .collect()
}

/// Read-only proposal. An unsigned development catalog never authorizes execution.
pub fn preview(
    install: &GameInstall,
    catalog: &Catalog,
    release_id: &str,
    platform: Platform,
) -> io::Result<InstallPreview> {
    let release = catalog
        .releases
        .iter()
        .find(|r| r.release_id == release_id && r.appid == install.appid)
        .ok_or_else(|| invalid("release_not_matched"))?;
    let mut blockers = compatibility(install, release, platform);
    blockers.push("catalog_unsigned".into());
    blockers.push("installer_not_implemented".into());
    let mut changes = Vec::new();
    if let Some(recipe) = &release.recipe {
        for file in &recipe.files {
            let actual = hash_file(
                &install.install_path,
                &install.library_path,
                &file.destination,
            );
            let baseline_matches = matches!(&actual, Ok(hash) if hash == &file.original_sha256);
            if !baseline_matches {
                blockers.push(format!("baseline_conflict:{}", file.destination));
            }
            changes.push(ProposedChange {
                path: file.destination.clone(),
                action: if file.original_sha256.is_some() {
                    "replace"
                } else {
                    "add"
                }
                .into(),
                baseline_matches,
                actual_sha256: actual.ok().flatten(),
            });
        }
    } else {
        blockers.push("recipe_missing".into());
    }
    Ok(InstallPreview {
        instance_id: install.instance_id.clone(),
        release_id: release_id.into(),
        catalog_revision: catalog.revision,
        executable: false,
        blockers,
        changes,
    })
}
