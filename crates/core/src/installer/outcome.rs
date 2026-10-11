//! The authoritative interpretation of durable installer transaction facts.
use crate::install::Transaction;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    Completed,
    Cancelled,
    Failed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CancellationCause {
    UserCancelled,
    LeaseExpired,
}
impl CancellationCause {
    pub fn code(self) -> &'static str {
        match self {
            Self::UserCancelled => "cancelled",
            Self::LeaseExpired => "lease_expired",
        }
    }
    /// Only the writer's exact cleanup suffix may accompany a durable cancel code.
    /// Other free-form error details never establish cancellation.
    pub(crate) fn from_transaction_error(error: &str) -> Option<Self> {
        let head = match error.split_once("; cleanup_pending:") {
            Some((head, detail)) if !detail.is_empty() => head,
            Some(_) => return None,
            None => error,
        };
        Self::from_code(head)
    }
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "cancelled" | "task_cancelled" => Some(Self::UserCancelled),
            "lease_expired" => Some(Self::LeaseExpired),
            _ => None,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    UserCancelled,
    LeaseExpired,
    DomainFailure,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FailureCause {
    pub kind: FailureKind,
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecoveryRequirement {
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CleanupStatus {
    NotRequested,
    Completed,
    Partial,
    Failed,
    Busy,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CleanupErrorCode {
    CleanupFailed,
    CleanupLinkedPath,
    CleanupUnknownPath,
    CleanupSizeOverflow,
    InstallerBusy,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CleanupFailure {
    pub code: CleanupErrorCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CleanupResult {
    pub status: CleanupStatus,
    pub performed: bool,
    pub removed_bytes: u64,
    pub failures: Vec<CleanupFailure>,
}
impl Default for CleanupResult {
    fn default() -> Self {
        Self {
            status: CleanupStatus::NotRequested,
            performed: false,
            removed_bytes: 0,
            failures: Vec::new(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InstallerOutcome {
    pub transaction_id: String,
    pub plan_id: String,
    pub instance_id: String,
    pub selection_id: Option<String>,
    pub content_id: Option<String>,
    pub release_id: String,
    pub state: String,
    pub error: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    pub has_replacements: bool,
    pub disposition: Disposition,
    pub cause: Option<FailureCause>,
    pub recovery: Option<RecoveryRequirement>,
    pub cleanup: CleanupResult,
}
fn bounded_detail(mut detail: String) -> String {
    if detail.len() > 2048 {
        let mut end = 2048;
        while !detail.is_char_boundary(end) {
            end -= 1;
        }
        detail.truncate(end);
    }
    detail
}

impl InstallerOutcome {
    /// Route identities must be verified by the caller; durable identities take precedence.
    /// No identity is inferred from a release-id prefix, including old journals.
    pub fn from_transaction(
        t: &Transaction,
        selection_id: Option<&str>,
        content_id: Option<&str>,
    ) -> Self {
        let cancellation = t
            .error
            .as_deref()
            .and_then(CancellationCause::from_transaction_error);
        let disposition = match t.state.as_str() {
            "committed" => Disposition::Completed,
            "rolled_back" if cancellation.is_some() => Disposition::Cancelled,
            _ => Disposition::Failed,
        };
        let cause = if disposition == Disposition::Completed {
            None
        } else {
            let (code, detail) = if cancellation.is_some() {
                let raw = t.error.as_deref().unwrap();
                match raw.split_once("; cleanup_pending:") {
                    Some((code, detail)) => {
                        (code.to_owned(), Some(format!("cleanup_pending:{detail}")))
                    }
                    None => (raw.to_owned(), None),
                }
            } else {
                let raw = t.error.as_deref().unwrap_or("installer_failed");
                let (raw, cleanup_detail) = raw
                    .split_once("; cleanup_pending:")
                    .map_or((raw, None), |(head, detail)| {
                        (head, Some(format!("cleanup_pending:{detail}")))
                    });
                let (head, detail) = raw.split_once(": ").map_or((raw, None), |(code, detail)| {
                    (code, Some(detail.to_owned()))
                });
                if !head.is_empty()
                    && head.len() <= 128
                    && head
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
                {
                    let detail = match (detail, cleanup_detail) {
                        (Some(detail), Some(cleanup)) => Some(format!("{detail}; {cleanup}")),
                        (detail, cleanup) => detail.or(cleanup),
                    };
                    (head.to_owned(), detail)
                } else {
                    ("installer_failed".into(), t.error.clone())
                }
            };
            Some(FailureCause {
                kind: match cancellation {
                    Some(CancellationCause::UserCancelled) => FailureKind::UserCancelled,
                    Some(CancellationCause::LeaseExpired) => FailureKind::LeaseExpired,
                    None => FailureKind::DomainFailure,
                },
                code,
                detail: detail.map(bounded_detail),
            })
        };
        let recovery = (t.state == "needs_recovery").then(|| RecoveryRequirement {
            required: true,
            code: Some("needs_recovery".into()),
            detail: t.error.clone().map(bounded_detail),
        });
        Self {
            transaction_id: t.transaction_id.clone(),
            plan_id: t.plan_id.clone(),
            instance_id: t.instance_id.clone(),
            selection_id: t
                .selection_id
                .clone()
                .or_else(|| selection_id.map(str::to_owned)),
            content_id: t
                .content_id
                .clone()
                .or_else(|| content_id.map(str::to_owned)),
            release_id: t.release_id.clone(),
            state: t.state.clone(),
            error: t.error.clone(),
            created_at: t.created_at,
            updated_at: t.updated_at,
            has_replacements: t.has_replacements,
            disposition,
            cause,
            recovery,
            cleanup: CleanupResult::default(),
        }
    }
}
