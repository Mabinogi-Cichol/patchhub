//! Authoritative action facts for a frozen projection snapshot. Availability is
//! permission to enter an install flow, never a substitute for execution checks.
use crate::{install::InstallOperation, projection::DomainError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ActionBlockerCode {
    UnsignedCatalog,
    CatalogUnavailable,
    CatalogExpired,
    CatalogRevisionRollback,
    CatalogRollback,
    ReleaseRevoked,
    ReleaseNotMatched,
    PlatformNotVerified,
    GameNotReady,
    BuildNotVerified,
    SourceRequiresGuidance,
    ArtifactRequiresManualDownload,
    RecipeMissing,
    SourceNotEnabled,
    SourceNotSubscribed,
    ExeSourceDisabled,
    SelectionNotFound,
    InvalidSelectionId,
    SelectionStale,
    ZipDownloadUnavailable,
    ZipDownloadAmbiguous,
    ExeDownloadAmbiguous,
    InvalidDownloadUrl,
    ExeCandidateAmbiguous,
    ExeCandidateVariantUnsupported,
    ExeSandboxUnavailable,
    InstallerHostRequiresNonRoot,
    UnprivilegedUserRequired,
    ReinstallUnownedOrIncomplete,
    ReinstallContentIdentityChanged,
    NeedsRecovery,
    LocalArchiveSignedOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActionBlocker {
    pub code: ActionBlockerCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}
impl ActionBlocker {
    pub fn code(code: ActionBlockerCode) -> Self {
        Self { code, detail: None }
    }
    pub fn new(code: ActionBlockerCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: DomainError::new("action", detail).detail,
        }
    }
    /// Domain diagnostics remain open; action reasons are deliberately closed.
    /// Unknown refusals stay refusals, with their original reason in detail.
    pub(crate) fn from_domain(error: &DomainError) -> Self {
        match serde_json::from_value::<ActionBlockerCode>(serde_json::Value::String(
            error.code.clone(),
        )) {
            Ok(code) => Self {
                code,
                detail: error
                    .detail
                    .as_ref()
                    .and_then(|d| DomainError::new("action", d).detail),
            },
            Err(_) => Self::new(ActionBlockerCode::SourceRequiresGuidance, error.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActionAvailability {
    pub available: bool,
    pub blockers: Vec<ActionBlocker>,
}
impl ActionAvailability {
    fn from_blockers(blockers: Vec<ActionBlocker>) -> Self {
        let mut unique = Vec::new();
        for blocker in blockers {
            if !unique
                .iter()
                .any(|b: &ActionBlocker| b.code == blocker.code)
            {
                unique.push(blocker);
            }
        }
        Self {
            available: unique.is_empty(),
            blockers: unique,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PrimaryLabel {
    Install,
    Overwrite,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InstallInputKind {
    Automatic,
    LocalArchive,
    Unavailable,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PrimaryAction {
    pub operation: InstallOperation,
    pub label: PrimaryLabel,
    pub input: InstallInputKind,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PatchActions {
    pub automatic: ActionAvailability,
    pub local_archive: ActionAvailability,
    pub reinstall: ActionAvailability,
    pub overwrite: ActionAvailability,
    pub primary: PrimaryAction,
}
impl PatchActions {
    pub(crate) fn evaluate(
        automatic: Vec<ActionBlocker>,
        local_archive: Vec<ActionBlocker>,
        reinstall_blocker: Option<ActionBlockerCode>,
        installed_evidence: bool,
        needs_recovery: bool,
    ) -> Self {
        let add_recovery = |mut blockers: Vec<ActionBlocker>| {
            if needs_recovery {
                blockers.push(ActionBlocker::code(ActionBlockerCode::NeedsRecovery));
            }
            ActionAvailability::from_blockers(blockers)
        };
        let automatic = add_recovery(automatic);
        let local_archive = add_recovery(local_archive);
        // Any admitted input suffices for overwrite. Both rejected inputs retain
        // their ordered reasons, deduplicated by stable machine code.
        let overwrite = if automatic.available || local_archive.available {
            ActionAvailability::from_blockers(vec![])
        } else {
            ActionAvailability::from_blockers(
                automatic
                    .blockers
                    .iter()
                    .chain(&local_archive.blockers)
                    .cloned()
                    .collect(),
            )
        };
        let mut reinstall = overwrite.blockers.clone();
        if let Some(code) = reinstall_blocker {
            reinstall.push(ActionBlocker::code(code));
        }
        let reinstall = ActionAvailability::from_blockers(reinstall);
        let input = if automatic.available {
            InstallInputKind::Automatic
        } else if local_archive.available {
            InstallInputKind::LocalArchive
        } else {
            InstallInputKind::Unavailable
        };
        Self {
            automatic,
            local_archive,
            reinstall,
            overwrite,
            primary: PrimaryAction {
                operation: InstallOperation::Overwrite,
                label: if installed_evidence {
                    PrimaryLabel::Overwrite
                } else {
                    PrimaryLabel::Install
                },
                input,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn input_truth_table_and_stable_deduplication() {
        use ActionBlockerCode::*;
        for automatic_ok in [false, true] {
            for local_ok in [false, true] {
                for receipt_ok in [false, true] {
                    for recovery in [false, true] {
                        let automatic = if automatic_ok {
                            vec![]
                        } else {
                            vec![
                                ActionBlocker::new(ArtifactRequiresManualDownload, "first"),
                                ActionBlocker::new(ArtifactRequiresManualDownload, "second"),
                            ]
                        };
                        let local = if local_ok {
                            vec![]
                        } else {
                            vec![ActionBlocker::code(LocalArchiveSignedOnly)]
                        };
                        let actions = PatchActions::evaluate(
                            automatic,
                            local,
                            (!receipt_ok).then_some(ReinstallUnownedOrIncomplete),
                            receipt_ok,
                            recovery,
                        );
                        let input_ok = (automatic_ok || local_ok) && !recovery;
                        assert_eq!(actions.automatic.available, automatic_ok && !recovery);
                        assert_eq!(actions.local_archive.available, local_ok && !recovery);
                        assert_eq!(actions.overwrite.available, input_ok);
                        assert_eq!(actions.reinstall.available, input_ok && receipt_ok);
                        assert_eq!(actions.primary.operation, InstallOperation::Overwrite);
                        assert_eq!(
                            actions.primary.label,
                            if receipt_ok {
                                PrimaryLabel::Overwrite
                            } else {
                                PrimaryLabel::Install
                            }
                        );
                        assert_eq!(
                            actions.primary.input,
                            if automatic_ok && !recovery {
                                InstallInputKind::Automatic
                            } else if local_ok && !recovery {
                                InstallInputKind::LocalArchive
                            } else {
                                InstallInputKind::Unavailable
                            }
                        );
                        for availability in [
                            &actions.automatic,
                            &actions.local_archive,
                            &actions.reinstall,
                            &actions.overwrite,
                        ] {
                            assert_eq!(availability.available, availability.blockers.is_empty());
                            let codes: std::collections::HashSet<_> =
                                availability.blockers.iter().map(|b| b.code).collect();
                            assert_eq!(codes.len(), availability.blockers.len());
                        }
                        if !automatic_ok {
                            assert_eq!(
                                actions.automatic.blockers[0].detail.as_deref(),
                                Some("first")
                            );
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn required_fields_closed_codes_and_utf8_detail_bound() {
        let actions = PatchActions::evaluate(vec![], vec![], None, false, false);
        let json = serde_json::to_value(&actions).unwrap();
        assert_eq!(
            json["primary"],
            serde_json::json!({"operation":"overwrite","label":"install","input":"automatic"})
        );
        for field in [
            "automatic",
            "local_archive",
            "reinstall",
            "overwrite",
            "primary",
        ] {
            let mut missing = json.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(serde_json::from_value::<PatchActions>(missing).is_err());
        }
        for alias in [
            "zip_runtime_unavailable",
            "platform_unsupported",
            "link_unsupported",
            "catalog_unsigned",
            "new_unknown_code",
        ] {
            assert!(serde_json::from_value::<ActionBlockerCode>(serde_json::json!(alias)).is_err());
        }
        let blocker =
            ActionBlocker::new(ActionBlockerCode::SourceRequiresGuidance, "字".repeat(1000));
        assert!(blocker.detail.unwrap().len() <= 2048);
        let unknown = ActionBlocker::from_domain(&DomainError::new(
            "zip_runtime_unavailable",
            "missing worker",
        ));
        assert_eq!(unknown.code, ActionBlockerCode::SourceRequiresGuidance);
        assert!(unknown.detail.unwrap().contains("zip_runtime_unavailable"));
    }
}
