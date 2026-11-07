//! Authoritative action facts for a frozen projection snapshot. Availability is
//! permission to enter an install flow, never a substitute for execution checks.
use crate::{install::InstallOperation, projection::DomainError};
use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
pub enum ActionBlockerCode {
    GameNotReady,
    BuildNotVerified,
    SourceRequiresGuidance,
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
    InstallerHostRequiresNonRoot,
    UnprivilegedUserRequired,
    ReinstallUnownedOrIncomplete,
    ReinstallContentIdentityChanged,
    NeedsRecovery,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct ActionBlocker {
    pub code: ActionBlockerCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
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

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
pub enum PrimaryLabel {
    Install,
    Overwrite,
}
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS,
)]
#[serde(rename_all = "snake_case")]
pub enum InstallInputKind {
    Automatic,
    Unavailable,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct PrimaryAction {
    #[ts(type = "\"overwrite\"")]
    pub operation: InstallOperation,
    pub label: PrimaryLabel,
    pub input: InstallInputKind,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct PatchActions {
    pub automatic: ActionAvailability,
    pub reinstall: ActionAvailability,
    pub overwrite: ActionAvailability,
    pub primary: PrimaryAction,
}
impl PatchActions {
    pub(crate) fn evaluate(automatic: Vec<ActionBlocker>, installed_evidence: bool) -> Self {
        let automatic = ActionAvailability::from_blockers(automatic);
        let overwrite = automatic.clone();
        let reinstall = automatic.clone();
        let input = if automatic.available {
            InstallInputKind::Automatic
        } else {
            InstallInputKind::Unavailable
        };
        Self {
            automatic,
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
    fn install_and_reinstall_share_the_same_input() {
        let ready = PatchActions::evaluate(vec![], true);
        assert!(
            ready.automatic.available && ready.reinstall.available && ready.overwrite.available
        );
        assert_eq!(ready.primary.label, PrimaryLabel::Overwrite);
        let blocked = PatchActions::evaluate(
            vec![ActionBlocker::code(ActionBlockerCode::InvalidDownloadUrl)],
            false,
        );
        assert!(!blocked.automatic.available && !blocked.reinstall.available);
        assert_eq!(blocked.primary.input, InstallInputKind::Unavailable);
    }
    #[test]
    fn required_fields_closed_codes_and_utf8_detail_bound() {
        let actions = PatchActions::evaluate(vec![], false);
        let json = serde_json::to_value(&actions).unwrap();
        assert_eq!(
            json["primary"],
            serde_json::json!({"operation":"overwrite","label":"install","input":"automatic"})
        );
        for field in ["automatic", "reinstall", "overwrite", "primary"] {
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
        let detail = "字".repeat(1000);
        let blocker = ActionBlocker::new(ActionBlockerCode::SourceRequiresGuidance, detail.clone());
        assert_eq!(blocker.detail.as_deref(), Some(detail.as_str()));
        let unknown = ActionBlocker::from_domain(&DomainError::new(
            "zip_runtime_unavailable",
            "missing worker",
        ));
        assert_eq!(unknown.code, ActionBlockerCode::SourceRequiresGuidance);
        assert!(unknown.detail.unwrap().contains("zip_runtime_unavailable"));
    }
}
