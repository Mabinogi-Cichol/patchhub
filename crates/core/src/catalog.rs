use crate::steam::GameInstall;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, io, path::PathBuf, sync::atomic::AtomicBool};

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub schema_version: u32,
    pub revision: u64,
    pub releases: Vec<PatchRelease>,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Windows,
    Steamos,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
pub enum SupportLevel {
    DiscoverOnly,
    ManualDownload,
    ManagedZip,
    /// Dynamic publisher ZIP candidate (P1). Never authoritative for apply; the
    /// selection still re-resolves inside `install_patch` (p1-zip-orchestration §1).
    PublisherZip,
    /// Dynamic publisher EXE candidate (reserved P2/P3; P1 only blocks).
    PublisherExe,
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

#[derive(Debug, Clone, Serialize, schemars::JsonSchema, ts_rs::TS, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchMatch {
    /// Chosen trusted download; especially important for explicit manual selection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub download_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub download_label: Option<String>,
    pub release_id: String,
    /// Display attribution from the current trusted source candidate.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub source_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub candidate_id: Option<String>,
    /// Dynamic selection route (`zip-selection-*`); absent for signed catalog
    /// releases (p1-zip-orchestration §4.2).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub selection_id: Option<String>,
    pub name: String,
    pub source_page: String,
    pub support_level: SupportLevel,
    pub actions: crate::actions::PatchActions,
    pub compatibility: String,
    pub blockers: Vec<String>,
    pub detection: Detection,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema, ts_rs::TS, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Detection {
    #[ts(type = "DetectionStatus")]
    pub status: String,
    pub files: Vec<FileEvidence>,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema, ts_rs::TS, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileEvidence {
    pub path: String,
    #[ts(type = "EvidenceStatus")]
    pub status: String,
    pub actual_sha256: Option<String>,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
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
        self.validate_with_build_evidence(true)
    }
    /// Dynamic candidates carry observed facts, never reviewed build IDs.
    #[cfg(test)]
    pub(crate) fn validate_dynamic(&self) -> io::Result<()> {
        self.validate_with_build_evidence(false)
    }
    fn validate_with_build_evidence(&self, require_build: bool) -> io::Result<()> {
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
                    || (require_build && release.verified_buildids.is_empty())
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

pub fn detect(install: &GameInstall, release: &PatchRelease) -> Detection {
    detect_with_cancel(install, release, &AtomicBool::new(false))
}

pub fn detect_with_cancel(
    install: &GameInstall,
    release: &PatchRelease,
    cancel: &AtomicBool,
) -> Detection {
    detect_with_diagnostics(install, release, cancel).0
}

pub fn detect_with_diagnostics(
    install: &GameInstall,
    release: &PatchRelease,
    cancel: &AtomicBool,
) -> (Detection, Vec<crate::detection::FileEvidence>) {
    if install.availability != "installed" {
        return (
            Detection {
                status: "unknown".into(),
                files: Vec::new(),
            },
            Vec::new(),
        );
    }
    // Audited destinations take precedence over subset detection hints.
    let mut rules: Vec<_> = release
        .recipe
        .as_ref()
        .map(|recipe| {
            recipe
                .files
                .iter()
                .map(|file| (&file.destination, &file.installed_sha256))
                .collect()
        })
        .unwrap_or_default();
    for hint in &release.detection {
        if !rules.iter().any(|(path, _)| *path == &hint.path) {
            rules.push((&hint.path, &hint.sha256));
        }
    }
    let mut diagnostics = Vec::new();
    let files: Vec<_> = rules
        .into_iter()
        .map(|(path, expected)| {
            let evidence = crate::detection::inspect_file(install, path, expected, None, cancel);
            let result = FileEvidence {
                path: path.clone(),
                status: evidence.status.as_str().into(),
                actual_sha256: evidence.actual_sha256.clone(),
            };
            if evidence.diagnostic.is_some() {
                diagnostics.push(evidence);
            }
            result
        })
        .collect();
    let status = if files.is_empty() || files.iter().any(|f| f.status == "unreadable_or_unsafe") {
        "unknown"
    } else if files.iter().all(|f| f.status == "match") {
        "installed_external"
    } else if files.iter().all(|f| f.status == "missing") {
        "not_detected"
    } else {
        "partial_or_conflict"
    };
    (
        Detection {
            status: status.into(),
            files,
        },
        diagnostics,
    )
}
