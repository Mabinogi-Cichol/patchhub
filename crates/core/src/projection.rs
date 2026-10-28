//! Typed read-only scan and preview service. Inputs are frozen caller facts;
//! this module never discovers Steam again or opens a writable source/store.
use crate::{
    actions::{ActionBlocker, ActionBlockerCode, PatchActions},
    catalog::{self, Catalog, PatchMatch, Platform, SupportLevel},
    detection,
    install::{self, TransactionSnapshot},
    installer,
    sources::{self, snapshot::SourceSnapshot},
    steam,
    trust::VerifiedCatalog,
};
use serde::Serialize;
use std::{
    collections::HashSet,
    fmt,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS, schemars::JsonSchema, serde::Deserialize,
)]
#[serde(deny_unknown_fields)]
pub struct DomainError {
    pub code: String,
    #[ts(optional = nullable)]
    pub detail: Option<String>,
}
impl DomainError {
    pub fn code(code: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            detail: None,
        }
    }
    pub fn new(code: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            detail: Some(detail.into()),
        }
    }
}
impl fmt::Display for DomainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.detail {
            Some(detail) => write!(f, "{}: {detail}", self.code),
            None => f.write_str(&self.code),
        }
    }
}
impl std::error::Error for DomainError {}
impl From<std::io::Error> for DomainError {
    fn from(error: std::io::Error) -> Self {
        let text = error.to_string();
        if text.len() <= 128
            && text
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            Self::code(text)
        } else {
            Self::new("projection_read_failed", text)
        }
    }
}
#[derive(Debug, Clone, Serialize, ts_rs::TS, schemars::JsonSchema, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionDiagnostic {
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub path: Option<PathBuf>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub instance_id: Option<String>,
}
#[derive(Debug, Clone)]
pub struct LinkCapability {
    pub available: bool,
    pub blockers: Vec<DomainError>,
}
#[derive(Debug, Clone)]
pub struct HostCapabilities {
    pub zip: LinkCapability,
    pub exe: LinkCapability,
}
impl Default for HostCapabilities {
    fn default() -> Self {
        Self {
            zip: LinkCapability {
                available: true,
                blockers: vec![],
            },
            exe: LinkCapability {
                available: true,
                blockers: vec![],
            },
        }
    }
}
#[derive(Clone, Copy)]
pub enum CatalogView<'a> {
    Verified(&'a VerifiedCatalog),
    Development(&'a Catalog),
    Unavailable { error: &'a DomainError },
}
pub struct ProjectionInputs<'a> {
    pub steam: &'a steam::ScanReport,
    pub catalog: CatalogView<'a>,
    pub sources: Option<&'a SourceSnapshot>,
    pub transactions: &'a TransactionSnapshot,
    pub roots: &'a [PathBuf],
    pub now: u64,
    pub platform: Platform,
    pub capabilities: &'a HostCapabilities,
    pub cancel: &'a AtomicBool,
}
#[derive(Debug, Clone, Copy, Serialize, ts_rs::TS, schemars::JsonSchema, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogTrust {
    Signed,
    UnsignedDevelopment,
    Unavailable,
}
#[derive(Debug, Clone, Copy, Serialize, ts_rs::TS, schemars::JsonSchema, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchStatus {
    Matched,
    NoCatalogMatch,
}
#[derive(Debug, Serialize, ts_rs::TS, schemars::JsonSchema, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanReport {
    pub catalog_revision: u64,
    pub catalog_trust: CatalogTrust,
    pub catalog_error: Option<DomainError>,
    pub platform: Platform,
    pub roots: Vec<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub settings_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub sources: Option<Vec<sources::SourceSummary>>,
    pub installs: Vec<InstanceProjection>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}
#[derive(Debug, Serialize, ts_rs::TS, schemars::JsonSchema, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceProjection {
    pub game: steam::GameInstall,
    pub match_status: MatchStatus,
    pub patches: Vec<PatchMatch>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub sources: Option<Vec<sources::GameSourceSummary>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub matches: Option<Vec<sources::SourceMatch>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub discovery: Option<sources::GameDiscovery>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}
#[derive(Debug, Clone)]
pub enum PreviewTarget {
    SignedRelease(String),
    PublisherZipSelection(String),
    PublisherExeSelection(String),
}
#[derive(Debug, Clone)]
pub struct PreviewRequest {
    pub instance_id: String,
    pub target: PreviewTarget,
}
#[derive(Debug, Serialize, ts_rs::TS, schemars::JsonSchema, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposedChange {
    pub path: String,
    pub action: String,
    pub baseline_matches: Option<bool>,
    pub actual_sha256: Option<String>,
}
#[derive(Debug, Serialize, ts_rs::TS, schemars::JsonSchema, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewReport {
    pub instance_id: String,
    pub release_id: Option<String>,
    pub selection_id: Option<String>,
    pub catalog_revision: u64,
    #[ts(type = "false")]
    pub executable: bool,
    pub blockers: Vec<String>,
    pub changes: Vec<ProposedChange>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
}
fn query(game: &steam::GameInstall) -> sources::GameQuery {
    sources::GameQuery {
        appid: game.appid,
        name: game.name.clone(),
        localized_name: game.localized_name.clone(),
        instance_id: Some(game.instance_id.clone()),
    }
}
fn catalog<'a>(inputs: &ProjectionInputs<'a>) -> Option<&'a Catalog> {
    match inputs.catalog {
        CatalogView::Verified(c) => Some(c.catalog()),
        CatalogView::Development(c) => Some(c),
        CatalogView::Unavailable { .. } => None,
    }
}
fn check_cancel(inputs: &ProjectionInputs<'_>) -> Result<(), DomainError> {
    if inputs.cancel.load(Ordering::Relaxed) {
        Err(DomainError::code("cancelled"))
    } else {
        Ok(())
    }
}
fn validate(inputs: &ProjectionInputs<'_>) -> Result<(), DomainError> {
    check_cancel(inputs)?;
    if inputs.roots.len() > 32 {
        return Err(DomainError::code("too_many_roots"));
    }
    if let Some(root) = inputs.transactions.state_root() {
        install::validate_projection_state(root, &inputs.steam.installs)?;
    }
    if let Some(snapshot) = inputs.sources {
        if snapshot.now_unix() != inputs.now {
            return Err(DomainError::code("invalid_snapshot_time"));
        }
        install::validate_projection_state(snapshot.state_root(), &inputs.steam.installs)?;
    }
    if let Some(catalog) = catalog(inputs) {
        catalog.validate()?;
    }
    Ok(())
}
fn game<'a>(
    inputs: &'a ProjectionInputs<'_>,
    id: &str,
) -> Result<&'a steam::GameInstall, DomainError> {
    if id.is_empty() || id.chars().any(char::is_control) {
        return Err(DomainError::code("invalid_instance_id"));
    }
    inputs
        .steam
        .installs
        .iter()
        .find(|game| game.instance_id == id)
        .ok_or_else(|| DomainError::code("instance_not_found"))
}
fn diagnostics(inputs: &ProjectionInputs<'_>) -> Vec<ProjectionDiagnostic> {
    let mut diagnostics: Vec<_> = inputs
        .steam
        .diagnostics
        .iter()
        .map(|d| ProjectionDiagnostic {
            code: d.code.clone(),
            path: Some(d.path.clone()),
            message: d.message.clone(),
            instance_id: None,
        })
        .collect();
    diagnostics.extend(
        inputs
            .transactions
            .diagnostics()
            .iter()
            .map(|d| ProjectionDiagnostic {
                code: d.code.clone(),
                path: Some(d.path.clone()),
                message: d.message.clone(),
                instance_id: None,
            }),
    );
    if let Some(snapshot) = inputs.sources {
        diagnostics.extend(snapshot.diagnostics().iter().map(|d| ProjectionDiagnostic {
            code: d.code.clone(),
            path: None,
            message: d.detail.clone().unwrap_or_default(),
            instance_id: None,
        }));
    }
    diagnostics
}
fn evidence_diagnostics(
    game: &steam::GameInstall,
    files: Vec<detection::FileEvidence>,
) -> Vec<ProjectionDiagnostic> {
    files
        .into_iter()
        .filter_map(|file| {
            file.diagnostic.map(|d| ProjectionDiagnostic {
                code: d.code,
                path: Some(PathBuf::from(file.path)),
                message: d.detail.unwrap_or_default(),
                instance_id: Some(game.instance_id.clone()),
            })
        })
        .collect()
}
pub fn scan(inputs: ProjectionInputs<'_>) -> Result<ScanReport, DomainError> {
    validate(&inputs)?;
    let ids: Vec<_> = inputs
        .steam
        .installs
        .iter()
        .map(|g| g.instance_id.clone())
        .collect();
    let installs = project_batch_impl(&inputs, &ids)?;
    let (catalog_trust, catalog_error) = match inputs.catalog {
        CatalogView::Verified(_) => (CatalogTrust::Signed, None),
        CatalogView::Development(_) => (CatalogTrust::UnsignedDevelopment, None),
        CatalogView::Unavailable { error } => (CatalogTrust::Unavailable, Some(error.clone())),
    };
    Ok(ScanReport {
        catalog_revision: catalog(&inputs).map_or(0, |c| c.revision),
        catalog_trust,
        catalog_error,
        platform: inputs.platform,
        roots: inputs.roots.to_vec(),
        settings_revision: inputs.sources.map(|s| s.settings_revision()),
        sources: inputs.sources.map(|s| s.registry().list_sources()),
        installs,
        diagnostics: diagnostics(&inputs),
    })
}
pub fn project_batch(
    inputs: &ProjectionInputs<'_>,
    instances: &[String],
) -> Result<Vec<InstanceProjection>, DomainError> {
    validate(inputs)?;
    project_batch_impl(inputs, instances)
}
fn project_batch_impl(
    inputs: &ProjectionInputs<'_>,
    instances: &[String],
) -> Result<Vec<InstanceProjection>, DomainError> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for instance in instances {
        if seen.insert(instance) {
            result.push(project_instance_impl(inputs, instance)?);
        }
    }
    Ok(result)
}
pub fn project_instance(
    inputs: &ProjectionInputs<'_>,
    instance_id: &str,
) -> Result<InstanceProjection, DomainError> {
    validate(inputs)?;
    project_instance_impl(inputs, instance_id)
}
fn project_instance_impl(
    inputs: &ProjectionInputs<'_>,
    instance_id: &str,
) -> Result<InstanceProjection, DomainError> {
    let game = game(inputs, instance_id)?;
    let (mut patches, evidence) = catalog(inputs)
        .map(|c| catalog::matches_with_diagnostics(game, c, inputs.platform, inputs.cancel))
        .unwrap_or_default();
    let mut diagnostics = evidence_diagnostics(game, evidence);
    let needs_recovery = inputs
        .transactions
        .transactions()
        .iter()
        .any(|t| t.instance_id == game.instance_id && t.state == "needs_recovery");
    let instance_blockers = instance_action_blockers(game, inputs.cancel);
    let signed_ids: HashSet<_> = patches.iter().map(|p| p.release_id.clone()).collect();
    for patch in &mut patches {
        if let CatalogView::Verified(verified) = inputs.catalog {
            if let Err(error) = verified.ensure_active(&patch.release_id, inputs.now) {
                patch.blockers.push(error.to_string());
            }
        }
        let owned = latest_plan(inputs, game, &patch.release_id, None)
            .is_some_and(|p| !p.changes.is_empty());
        if owned && patch.detection.status == "installed_external" {
            patch.detection.status = "installed_managed".into();
        } else if owned && patch.detection.status == "not_detected" {
            patch.detection.status = "partial_or_conflict".into();
        } else if !owned
            && inputs.transactions.transactions().iter().any(|t| {
                t.instance_id == game.instance_id
                    && t.release_id == patch.release_id
                    && t.state == "committed"
            })
        {
            patch.detection.status = "unknown".into();
            diagnostics.push(ProjectionDiagnostic {
                code: "committed_plan_unreadable".into(),
                path: None,
                message: patch.release_id.clone(),
                instance_id: Some(game.instance_id.clone()),
            });
        }
        let release = catalog(inputs)
            .and_then(|c| c.releases.iter().find(|r| r.release_id == patch.release_id))
            .expect("catalog match has a release");
        let mut common = instance_blockers.clone();
        common.extend(
            patch
                .blockers
                .iter()
                .map(|code| ActionBlocker::from_domain(&DomainError::code(code))),
        );
        match inputs.catalog {
            CatalogView::Verified(_) => (),
            CatalogView::Development(_) => {
                common.push(ActionBlocker::code(ActionBlockerCode::UnsignedCatalog))
            }
            CatalogView::Unavailable { error } => common.push(ActionBlocker::new(
                ActionBlockerCode::CatalogUnavailable,
                error.to_string(),
            )),
        }
        if release.recipe.is_none() {
            common.push(ActionBlocker::code(ActionBlockerCode::RecipeMissing));
        }
        common.extend(capability_blockers(&inputs.capabilities.zip));
        common.extend(unsafe_evidence_blockers(&patch.detection, &diagnostics));
        let mut automatic = common.clone();
        if release.artifact.is_none() {
            automatic.push(ActionBlocker::code(
                ActionBlockerCode::ArtifactRequiresManualDownload,
            ));
        }
        let reinstall = signed_reinstall_blocker(inputs, game, release, &patch.detection);
        patch.actions = PatchActions::evaluate(
            automatic,
            common,
            reinstall,
            receipt_evidence(inputs, game, &patch.release_id, None)
                || installed_evidence(&patch.detection),
            needs_recovery,
        );
    }
    let (source_summaries, matches, discovery) = if let Some(snapshot) = inputs.sources {
        let registry = snapshot.registry();
        let query = query(game);
        let mut choices = Vec::new();
        for candidate in registry.dynamic_zip_candidates(&query) {
            let automatic = if candidate.mirrors.iter().any(|m| m.format == "zip") {
                installer::zip_selection_id(&candidate)
            } else {
                installer::exe_resolve::exe_selection_id(&candidate)
            };
            if let Some(id) = &automatic {
                choices.push((candidate.clone(), id.clone(), false));
            }
            let usable = automatic.as_ref().is_some_and(|id| {
                if id.starts_with("zip-") {
                    installer::resolve_zip(registry, &query, id).is_ok()
                } else {
                    installer::exe_resolve::resolve_exe(registry, &query, id).is_ok()
                }
            });
            let artifacts: HashSet<_> = candidate
                .mirrors
                .iter()
                .filter(|m| matches!(m.format.as_str(), "zip" | "exe"))
                .map(|m| {
                    (
                        m.format.as_str(),
                        m.url.rsplit('/').next().unwrap_or(&m.url),
                    )
                })
                .collect();
            if !usable
                || game.buildid.is_none()
                || artifacts.len() > 1
                || candidate
                    .mirrors
                    .iter()
                    .filter(|m| matches!(m.format.as_str(), "zip" | "exe"))
                    .count()
                    > 1
            {
                for mirror in &candidate.mirrors {
                    if let Some(id) =
                        installer::manual_selection::selection_id(&candidate, &query, mirror)
                    {
                        choices.push((candidate.clone(), id, true));
                    }
                }
            }
        }
        for (candidate, selection_id, manual) in choices {
            check_cancel(inputs)?;
            let release_id =
                installer::resolve::source_release_id_from_source(&candidate.source_id, game.appid);
            if signed_ids.contains(&release_id)
                || patches
                    .iter()
                    .any(|p| p.selection_id.as_deref() == Some(&selection_id))
            {
                continue;
            }
            let has_zip = selection_id.starts_with("zip-");
            let mut blockers =
                installer::resolve::source_admission_blockers(registry, &candidate, game.appid);
            // Resolve the exact selection, including URL and mirror ambiguity.
            // Admission alone does not prove a usable automatic input.
            let resolved = if manual {
                installer::manual_selection::resolve(
                    registry,
                    &query,
                    &selection_id,
                    if has_zip { "zip" } else { "exe" },
                )
            } else if has_zip {
                installer::resolve_zip(registry, &query, &selection_id)
            } else {
                installer::exe_resolve::resolve_exe(registry, &query, &selection_id)
            };
            let mut automatic = instance_blockers.clone();
            automatic.extend(
                blockers
                    .iter()
                    .map(|code| ActionBlocker::from_domain(&DomainError::code(code))),
            );
            let download_url = resolved.as_ref().ok().map(|r| r.download_url.clone());
            let download_label = download_url
                .as_ref()
                .and_then(|url| candidate.mirrors.iter().find(|m| &m.url == url))
                .and_then(|m| m.label.clone());
            if let Err(error) = resolved {
                blockers.push(error.code.clone());
                automatic.push(ActionBlocker::from_domain(&DomainError {
                    code: error.code,
                    detail: error.detail,
                }));
            }
            if game.availability != "installed" {
                blockers.push("game_not_ready".into());
                automatic.push(ActionBlocker::code(ActionBlockerCode::GameNotReady));
            }
            if has_zip && !manual && game.buildid.is_none() {
                blockers.push("build_not_verified".into());
                automatic.push(ActionBlocker::code(ActionBlockerCode::BuildNotVerified));
            }
            let link = if has_zip {
                &inputs.capabilities.zip
            } else {
                &inputs.capabilities.exe
            };
            let capability = capability_blockers(link);
            blockers.extend(capability.iter().map(|b| {
                serde_json::to_value(b.code)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_owned()
            }));
            automatic.extend(capability);
            let detection =
                dynamic_detection(inputs, game, &release_id, &selection_id, &mut diagnostics);
            automatic.extend(unsafe_evidence_blockers(&detection, &diagnostics));
            let mut local_archive = instance_blockers.clone();
            local_archive.push(ActionBlocker::code(
                ActionBlockerCode::LocalArchiveSignedOnly,
            ));
            let reinstall = dynamic_reinstall_blocker(
                inputs,
                game,
                &release_id,
                &selection_id,
                &detection,
                has_zip,
            );
            let receipt = receipt_evidence(inputs, game, &release_id, Some(&selection_id));
            let actions = PatchActions::evaluate(
                automatic,
                local_archive,
                reinstall,
                receipt || installed_evidence(&detection),
                needs_recovery,
            );
            patches.push(PatchMatch {
                download_url,
                download_label,
                source_id: Some(candidate.source_id.clone()),
                candidate_id: Some(candidate.candidate_id.clone()),
                release_id,
                selection_id: Some(selection_id.clone()),
                name: candidate.patch_title,
                source_page: candidate.source_page,
                support_level: if has_zip {
                    SupportLevel::PublisherZip
                } else {
                    SupportLevel::PublisherExe
                },
                actions,
                compatibility: if manual {
                    "manual_selection_required"
                } else if has_zip {
                    if blockers.is_empty() {
                        "verified_metadata"
                    } else {
                        "needs_review"
                    }
                } else {
                    "check_required"
                }
                .into(),
                blockers,
                detection,
            });
        }
        (
            Some(registry.list_game_sources(&query)),
            Some(registry.matches(&query, false)),
            Some(registry.cached_discovery(&query)),
        )
    } else {
        (None, None, None)
    };
    check_cancel(inputs)?;
    Ok(InstanceProjection {
        game: game.clone(),
        match_status: if patches.is_empty() {
            MatchStatus::NoCatalogMatch
        } else {
            MatchStatus::Matched
        },
        patches,
        sources: source_summaries,
        matches,
        discovery,
        diagnostics,
    })
}
// A current committed receipt remains display evidence when its plan is
// unreadable. Ownership and strict reinstall still require a complete plan.
fn receipt_evidence(
    inputs: &ProjectionInputs<'_>,
    game: &steam::GameInstall,
    release: &str,
    selection: Option<&str>,
) -> bool {
    inputs.transactions.transactions().iter().any(|t| {
        t.instance_id == game.instance_id
            && t.state == "committed"
            && (t.release_id == release
                || selection.is_some_and(|s| t.selection_id.as_deref() == Some(s)))
    })
}
fn installed_evidence(detection: &catalog::Detection) -> bool {
    matches!(
        detection.status.as_str(),
        "installed_managed" | "installed_external" | "partial_or_conflict"
    ) || detection
        .files
        .iter()
        .any(|f| matches!(f.status.as_str(), "match" | "mismatch"))
}
fn capability_blockers(link: &LinkCapability) -> Vec<ActionBlocker> {
    if link.available && link.blockers.is_empty() {
        return vec![];
    }
    let mut blockers: Vec<_> = link
        .blockers
        .iter()
        .map(ActionBlocker::from_domain)
        .collect();
    if !link.available && blockers.is_empty() {
        blockers.push(ActionBlocker::new(
            ActionBlockerCode::SourceRequiresGuidance,
            "runtime_unavailable",
        ));
    }
    blockers
}
fn instance_action_blockers(game: &steam::GameInstall, cancel: &AtomicBool) -> Vec<ActionBlocker> {
    detection::validate_game_boundary(game, cancel)
        .err()
        .map(|e| ActionBlocker::new(ActionBlockerCode::GameNotReady, e.to_string()))
        .into_iter()
        .collect()
}
fn unsafe_evidence_blockers(
    detection: &catalog::Detection,
    diagnostics: &[ProjectionDiagnostic],
) -> Vec<ActionBlocker> {
    diagnostics
        .iter()
        .filter(|d| {
            matches!(
                d.code.as_str(),
                "evidence_linked_path"
                    | "evidence_boundary_violation"
                    | "evidence_invalid_path"
                    | "evidence_not_regular_file"
            )
        })
        .filter(|d| {
            d.path.as_ref().is_some_and(|path| {
                detection
                    .files
                    .iter()
                    .any(|f| path == &PathBuf::from(&f.path))
            })
        })
        .map(|d| ActionBlocker::new(ActionBlockerCode::GameNotReady, &d.code))
        .collect()
}
fn signed_reinstall_blocker(
    inputs: &ProjectionInputs<'_>,
    game: &steam::GameInstall,
    release: &catalog::PatchRelease,
    detection: &catalog::Detection,
) -> Option<ActionBlockerCode> {
    let Some(plan) = latest_plan(inputs, game, &release.release_id, None) else {
        return Some(ActionBlockerCode::ReinstallUnownedOrIncomplete);
    };
    let Some(recipe) = release.recipe.as_ref() else {
        return Some(ActionBlockerCode::ReinstallUnownedOrIncomplete);
    };
    let identity = install::recipe_identity(release).ok();
    let same_targets = plan.changes.len() == recipe.files.len()
        && plan.changes.iter().zip(&recipe.files).all(|(c, m)| {
            c.path == m.destination
                && c.installed_sha256.as_deref() == Some(m.installed_sha256.as_str())
        });
    if plan.install.buildid != game.buildid
        || !same_targets
        || plan
            .recipe_sha256
            .as_ref()
            .is_some_and(|h| Some(h) != identity.as_ref())
    {
        return Some(ActionBlockerCode::ReinstallContentIdentityChanged);
    }
    if plan.changes.is_empty()
        || !recipe.files.iter().all(|m| {
            detection.files.iter().any(|f| {
                f.path == m.destination
                    && f.status == "match"
                    && f.actual_sha256.as_deref() == Some(m.installed_sha256.as_str())
            })
        })
    {
        return Some(ActionBlockerCode::ReinstallUnownedOrIncomplete);
    }
    None
}
fn dynamic_reinstall_blocker(
    inputs: &ProjectionInputs<'_>,
    game: &steam::GameInstall,
    release: &str,
    selection: &str,
    detection: &catalog::Detection,
    zip: bool,
) -> Option<ActionBlockerCode> {
    let Some(plan) = latest_plan(inputs, game, release, Some(selection)) else {
        return Some(ActionBlockerCode::ReinstallUnownedOrIncomplete);
    };
    let transaction = inputs
        .transactions
        .transactions()
        .iter()
        .find(|t| t.plan_id == plan.plan_id && t.state == "committed");
    // ZIP retains source release_id and recipe_sha256; EXE retains its generated
    // exe-content identity in both release_id and content_id.
    let identity_changed = transaction.is_none_or(|t| {
        t.selection_id.as_deref() != Some(selection)
            || if zip {
                plan.release_id != release
                    || t.content_id
                        .as_ref()
                        .is_some_and(|h| Some(h) != plan.recipe_sha256.as_ref())
            } else {
                t.content_id.as_deref() != Some(plan.release_id.as_str())
            }
    });
    if plan.install.buildid != game.buildid || identity_changed {
        return Some(ActionBlockerCode::ReinstallContentIdentityChanged);
    }
    if plan.changes.is_empty()
        || (zip && plan.recipe_sha256.is_none())
        || detection.status != "installed_managed"
        || !detection.files.iter().all(|f| f.status == "match")
    {
        return Some(ActionBlockerCode::ReinstallUnownedOrIncomplete);
    }
    None
}
fn latest_plan<'a>(
    inputs: &'a ProjectionInputs<'_>,
    game: &steam::GameInstall,
    release: &str,
    selection: Option<&str>,
) -> Option<&'a install::InstallPlan> {
    if !inputs.transactions.receipts_complete() {
        return None;
    }
    let transaction = inputs
        .transactions
        .transactions()
        .iter()
        .filter(|t| {
            t.instance_id == game.instance_id
                && t.state == "committed"
                && (t.release_id == release
                    || selection.is_some_and(|s| t.selection_id.as_deref() == Some(s)))
        })
        .max_by_key(|t| (t.updated_at, &t.transaction_id))?;
    inputs.transactions.plan(&transaction.plan_id).filter(|p| {
        p.plan_id == transaction.plan_id
            && p.release_id == transaction.release_id
            && p.instance_id == game.instance_id
            && p.install.instance_id == game.instance_id
            && p.install.appid == game.appid
            && p.install.install_path == game.install_path
            && p.install.library_path == game.library_path
    })
}
fn dynamic_detection(
    inputs: &ProjectionInputs<'_>,
    game: &steam::GameInstall,
    release: &str,
    selection: &str,
    diagnostics: &mut Vec<ProjectionDiagnostic>,
) -> catalog::Detection {
    let Some(plan) = latest_plan(inputs, game, release, Some(selection)) else {
        if inputs.transactions.transactions().iter().any(|t| {
            t.instance_id == game.instance_id
                && t.state == "committed"
                && (t.release_id == release || t.selection_id.as_deref() == Some(selection))
        }) {
            diagnostics.push(ProjectionDiagnostic {
                code: "committed_plan_unreadable".into(),
                path: None,
                message: release.into(),
                instance_id: Some(game.instance_id.clone()),
            });
        }
        return catalog::Detection {
            status: "unknown".into(),
            files: vec![],
        };
    };
    if plan.changes.is_empty() {
        diagnostics.push(ProjectionDiagnostic {
            code: "committed_plan_empty".into(),
            path: None,
            message: plan.plan_id.clone(),
            instance_id: Some(game.instance_id.clone()),
        });
        return catalog::Detection {
            status: "unknown".into(),
            files: vec![],
        };
    }
    let mut files = Vec::new();
    for change in &plan.changes {
        let file = detection::inspect_file(
            game,
            &change.path,
            change.installed_sha256.as_deref().unwrap_or_default(),
            None,
            inputs.cancel,
        );
        let status = file.status.as_str().to_owned();
        let actual_sha256 = file.actual_sha256.clone();
        diagnostics.extend(evidence_diagnostics(game, vec![file]));
        files.push(catalog::FileEvidence {
            path: change.path.clone(),
            status,
            actual_sha256,
        });
    }
    let status = if files.iter().any(|f| f.status == "unreadable_or_unsafe") {
        "unknown"
    } else if files.iter().all(|f| f.status == "match") {
        "installed_managed"
    } else {
        "partial_or_conflict"
    };
    catalog::Detection {
        status: status.into(),
        files,
    }
}
pub fn preview(
    inputs: &ProjectionInputs<'_>,
    request: PreviewRequest,
) -> Result<PreviewReport, DomainError> {
    validate(inputs)?;
    let game = game(inputs, &request.instance_id)?;
    let revision = catalog(inputs).map_or(0, |c| c.revision);
    let mut report = PreviewReport {
        instance_id: game.instance_id.clone(),
        release_id: None,
        selection_id: None,
        catalog_revision: revision,
        executable: false,
        blockers: vec![],
        changes: vec![],
        diagnostics: diagnostics(inputs),
    };
    match request.target {
        PreviewTarget::SignedRelease(id) => {
            if id.is_empty()
                || id.len() > 256
                || id.chars().any(char::is_control)
                || id.starts_with("zip-selection-")
                || id.starts_with("exe-selection-")
            {
                return Err(DomainError::code("invalid_release_id"));
            }
            let catalog = catalog(inputs).ok_or_else(|| DomainError::code("release_not_found"))?;
            let (signed, evidence) = catalog::preview_with_diagnostics(
                game,
                catalog,
                &id,
                inputs.platform,
                inputs.cancel,
            )?;
            report.release_id = Some(id.clone());
            report.blockers = signed.blockers;
            if let CatalogView::Verified(verified) = inputs.catalog {
                if let Err(error) = verified.ensure_active(&id, inputs.now) {
                    report.blockers.push(error.to_string());
                }
            }
            let unknown_paths: HashSet<_> =
                evidence.iter().map(|file| file.path.as_str()).collect();
            report.changes = signed
                .changes
                .into_iter()
                .map(|c| ProposedChange {
                    baseline_matches: if unknown_paths.contains(c.path.as_str()) {
                        None
                    } else {
                        Some(c.baseline_matches)
                    },
                    path: c.path,
                    action: c.action,
                    actual_sha256: c.actual_sha256,
                })
                .collect();
            report
                .diagnostics
                .extend(evidence_diagnostics(game, evidence));
        }
        PreviewTarget::PublisherZipSelection(id) => {
            if installer::resolve::parse_zip_selection_id(&id).is_none() {
                return Err(DomainError::code("invalid_selection_id"));
            }
            report.selection_id = Some(id.clone());
            let Some(snapshot) = inputs.sources else {
                report.blockers.push("selection_not_found".into());
                return Ok(report);
            };
            match installer::resolve_zip(snapshot.registry(), &query(game), &id) {
                Ok(selection) => {
                    report.release_id = Some(selection.release_id.clone());
                    if let Some(plan) = latest_plan(inputs, game, &selection.release_id, Some(&id))
                    {
                        if plan.changes.is_empty() {
                            report.diagnostics.push(ProjectionDiagnostic {
                                code: "committed_plan_empty".into(),
                                path: None,
                                message: plan.plan_id.clone(),
                                instance_id: Some(game.instance_id.clone()),
                            });
                        } else {
                            for change in &plan.changes {
                                match catalog::relative_path(&change.path) {
                                    Ok(_) => report.changes.push(ProposedChange {
                                        path: change.path.clone(),
                                        action: change.action.clone(),
                                        baseline_matches: None,
                                        actual_sha256: None,
                                    }),
                                    Err(error) => report.diagnostics.push(ProjectionDiagnostic {
                                        code: "committed_plan_unsafe".into(),
                                        path: Some(PathBuf::from(&change.path)),
                                        message: error.to_string(),
                                        instance_id: Some(game.instance_id.clone()),
                                    }),
                                }
                            }
                        }
                    } else if inputs.transactions.transactions().iter().any(|t| {
                        t.instance_id == game.instance_id
                            && t.state == "committed"
                            && (t.release_id == selection.release_id
                                || t.selection_id.as_deref() == Some(&id))
                    }) {
                        report.diagnostics.push(ProjectionDiagnostic {
                            code: "committed_plan_unreadable".into(),
                            path: None,
                            message: selection.release_id,
                            instance_id: Some(game.instance_id.clone()),
                        });
                    }
                }
                Err(error) => report.blockers.push(error.code),
            }
        }
        PreviewTarget::PublisherExeSelection(id) => {
            if installer::exe_resolve::parse_exe_selection_id(&id).is_none() {
                return Err(DomainError::code("invalid_selection_id"));
            }
            let snapshot = inputs
                .sources
                .ok_or_else(|| DomainError::code("selection_not_found"))?;
            installer::exe_resolve::resolve_exe(snapshot.registry(), &query(game), &id).map_err(
                |e| DomainError {
                    code: e.code,
                    detail: e.detail,
                },
            )?;
            report.selection_id = Some(id);
            report.blockers.push("exe_check_required".into());
        }
    }
    check_cancel(inputs)?;
    Ok(report)
}
