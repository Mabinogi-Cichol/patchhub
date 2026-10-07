//! New internal install authorization (design §5.2).
//!
//! An [`Authorization`] is generated server-side by Rust from the current
//! source/candidate/rule identity, the resolved content/transform artifacts, the
//! plan/task/request ownership, the adapter/policy/report versions and a
//! [`GameFacts`] snapshot. Before apply, [`validate`] re-reads the current facts
//! and re-resolves every binding; any difference (facts, content, policy,
//! settings revision) or expiry/revocation invalidates the authorization with a
//! diagnosable [`Invalidation`].
//!
//! Persistence lives at the protected `installer/rust-v1/jobs/<plan_id>/`
//! location (`authorization.json`, p0-contracts §3.1–§3.2). It never replaces the
//! `journal.sqlite` transaction truth source: the journal still owns the plan,
//! TTL and recovery material, and this file only records the authorization
//! bindings referenced by the same `plan_id`.
//!
//! Integration points:
//! - Content mappings come from `installer::zip_recipe::build_recipe`
//!   (`catalog::FileMapping`); [`ContentBinding`] stores their canonical digest
//!   in the same `(source, destination, installed_sha256)` order used by
//!   `install::recipe_identity`.
//! - `install::InstallPlan` remains untouched; callers pass the current plan's
//!   mappings into [`CurrentState::content`] for re-verification.

use super::facts::GameFacts;
use super::{invalid, InstallerPaths, Result};
use crate::catalog::FileMapping;
use crate::install::PLAN_TTL_SECONDS;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

/// `authorization.json` schema version (`INSTALLER_SELECTION/CONTENT_VERSION` are
/// carried separately in [`PolicyVersions`]).
pub const AUTHORIZATION_SCHEMA_VERSION: u32 = 1;
/// Maximum serialized authorization size (design §5.2; matches `MAX_POLICY`).
pub const MAX_AUTHORIZATION_BYTES: u64 = 4 * 1024 * 1024;

/// Independent adapter/policy/report and selection/content versions
/// (p0-contracts §0/§2.3; not part of `content_id`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PolicyVersions {
    pub adapter_version: u32,
    pub policy_schema_version: u32,
    pub report_schema_version: u32,
    pub selection_version: u32,
    pub content_version: u32,
}

impl Default for PolicyVersions {
    fn default() -> Self {
        Self {
            adapter_version: 1,
            policy_schema_version: 1,
            report_schema_version: 1,
            selection_version: 1,
            content_version: 1,
        }
    }
}

/// One install mapping captured for content re-verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MappingBinding {
    pub source: String,
    pub destination: String,
    pub installed_sha256: String,
}

/// Content/transform identity (design §5.2 "内容/转换产物摘要").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContentBinding {
    /// EXE dynamic route only; signature ZIP route binds `release_id` instead.
    pub content_id: Option<String>,
    pub exe_sha256: Option<String>,
    pub exe_size: Option<u64>,
    pub metadata_sha256: Option<String>,
    pub payload_sha256: Option<String>,
    /// Normalized ZIP artifact (publisher ZIP download or EXE transform output).
    pub artifact_sha256: Option<String>,
    pub artifact_size: Option<u64>,
    pub extractor: Option<String>,
    pub mappings: Vec<MappingBinding>,
    /// Canonical digest of `mappings` (see [`mapping_digest`]).
    pub mapping_sha256: String,
}

impl ContentBinding {
    /// Signature/publisher ZIP route (p0-contracts §2.4): artifact + mappings.
    pub fn publisher_zip(
        artifact_sha256: &str,
        artifact_size: u64,
        mappings: &[FileMapping],
    ) -> Self {
        Self {
            content_id: None,
            exe_sha256: None,
            exe_size: None,
            metadata_sha256: None,
            payload_sha256: None,
            artifact_sha256: Some(artifact_sha256.to_string()),
            artifact_size: Some(artifact_size),
            extractor: None,
            mappings: bindings(mappings),
            mapping_sha256: mapping_digest(mappings),
        }
    }

    /// EXE dynamic route (p0-contracts §2.2): EXE + metadata/payload + normalized
    /// artifact + mappings.
    #[allow(clippy::too_many_arguments)]
    pub fn publisher_exe(
        content_id: &str,
        exe_sha256: &str,
        exe_size: u64,
        metadata_sha256: &str,
        payload_sha256: &str,
        normalized_sha256: &str,
        normalized_size: u64,
        extractor: Option<&str>,
        mappings: &[FileMapping],
    ) -> Self {
        Self {
            content_id: Some(content_id.to_string()),
            exe_sha256: Some(exe_sha256.to_string()),
            exe_size: Some(exe_size),
            metadata_sha256: Some(metadata_sha256.to_string()),
            payload_sha256: Some(payload_sha256.to_string()),
            artifact_sha256: Some(normalized_sha256.to_string()),
            artifact_size: Some(normalized_size),
            extractor: extractor.map(str::to_string),
            mappings: bindings(mappings),
            mapping_sha256: mapping_digest(mappings),
        }
    }
}

fn bindings(mappings: &[FileMapping]) -> Vec<MappingBinding> {
    mappings
        .iter()
        .map(|mapping| MappingBinding {
            source: mapping.source.clone(),
            destination: mapping.destination.clone(),
            installed_sha256: mapping.installed_sha256.clone(),
        })
        .collect()
}

/// Deterministic digest of the install mappings, preserving recipe order (same
/// field triple as `install::recipe_identity`).
pub fn mapping_digest(mappings: &[FileMapping]) -> String {
    let rows: Vec<(&str, &str, &str)> = mappings
        .iter()
        .map(|mapping| {
            (
                mapping.source.as_str(),
                mapping.destination.as_str(),
                mapping.installed_sha256.as_str(),
            )
        })
        .collect();
    let bytes = serde_json::to_vec(&rows).unwrap_or_default();
    format!("{:x}", Sha256::digest(bytes))
}

/// Full authorization binding list (design §5.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Authorization {
    pub schema_version: u32,
    pub installer_schema_version: u32,
    /// Instance/path binding.
    pub instance_id: String,
    pub library_path: PathBuf,
    pub install_path: PathBuf,
    /// Candidate/source and settings revision binding.
    pub release_id: String,
    pub selection_id: Option<String>,
    pub source_id: String,
    pub source_record_id: Option<String>,
    pub candidate_sha256: String,
    pub settings_revision: u64,
    /// Rule/condition binding.
    pub rule_fingerprint: String,
    pub conditions_sha256: String,
    /// Content/transform artifact binding.
    pub content: ContentBinding,
    /// Plan/task/request ownership.
    pub plan_id: String,
    pub task_id: Option<String>,
    pub request_id: Option<String>,
    /// Policy versions.
    pub policy: PolicyVersions,
    /// Instance fact snapshot.
    pub facts: GameFacts,
    /// Validity window and revocation state.
    pub created_at: u64,
    pub expires_at: u64,
    pub lease_active: bool,
    pub revoked: bool,
}

/// Server-side input for [`authorize`].
#[derive(Debug, Clone)]
pub struct AuthorizeInput<'a> {
    pub release_id: &'a str,
    pub selection_id: Option<&'a str>,
    pub source_id: &'a str,
    pub source_record_id: Option<&'a str>,
    pub candidate_sha256: &'a str,
    pub rule_fingerprint: &'a str,
    pub settings_revision: u64,
    pub conditions_sha256: &'a str,
    pub content: ContentBinding,
    pub policy: PolicyVersions,
    pub plan_id: &'a str,
    pub task_id: Option<&'a str>,
    pub request_id: Option<&'a str>,
    pub facts: GameFacts,
    pub now: u64,
    pub ttl_seconds: u64,
}

/// Current instance state supplied to [`validate`].
#[derive(Debug, Clone)]
pub struct CurrentState {
    pub now: u64,
    pub plan_id: String,
    pub instance_id: String,
    /// Persistent revocation marker (`jobs/<plan_id>/revoked`).
    pub revoked: bool,
    pub lease_active: bool,
    pub facts: GameFacts,
    pub settings_revision: u64,
    pub policy: PolicyVersions,
    pub candidate_sha256: String,
    pub rule_fingerprint: String,
    pub conditions_sha256: String,
    pub content: ContentBinding,
}

/// A diagnosable invalidation reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invalidation {
    pub code: String,
    pub detail: Option<String>,
}

impl Invalidation {
    pub fn new(code: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            detail: Some(detail.into()),
        }
    }

    pub fn code(code: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            detail: None,
        }
    }
}

impl std::fmt::Display for Invalidation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.detail {
            Some(detail) => write!(formatter, "{}: {detail}", self.code),
            None => formatter.write_str(&self.code),
        }
    }
}

impl std::error::Error for Invalidation {}

pub type AuthResult<T> = std::result::Result<T, Invalidation>;

/// Generates a new authorization from verified server-side inputs.
///
/// Rejects invalid TTL, empty mappings, malformed plan ids and instances whose
/// facts already carry a failure.
pub fn authorize(input: AuthorizeInput<'_>) -> AuthResult<Authorization> {
    if input.ttl_seconds == 0 || input.ttl_seconds > PLAN_TTL_SECONDS {
        return Err(Invalidation::new(
            "authorization_ttl_invalid",
            "ttl outside the 1..=900 second plan bound",
        ));
    }
    let expires_at = input
        .now
        .checked_add(input.ttl_seconds)
        .ok_or_else(|| Invalidation::new("authorization_ttl_invalid", "expiry overflow"))?;
    if input.content.mappings.is_empty() {
        return Err(Invalidation::code("authorization_content_empty"));
    }
    for (name, value) in [
        ("release_id", input.release_id),
        ("source_id", input.source_id),
        ("candidate_sha256", input.candidate_sha256),
        ("rule_fingerprint", input.rule_fingerprint),
        ("conditions_sha256", input.conditions_sha256),
    ] {
        if value.is_empty() {
            return Err(Invalidation::new(
                "authorization_binding_missing",
                format!("{name} is empty"),
            ));
        }
    }
    if Uuid::parse_str(input.plan_id).is_err() {
        return Err(Invalidation::new(
            "authorization_plan_invalid",
            "plan id is not a UUID",
        ));
    }
    if let Some(failure) = input.facts.failures().first() {
        return Err(Invalidation::new(
            failure.code.clone(),
            failure.detail.clone(),
        ));
    }

    Ok(Authorization {
        schema_version: AUTHORIZATION_SCHEMA_VERSION,
        installer_schema_version: super::STORE_SCHEMA_VERSION,
        instance_id: input.facts.instance.instance_id.clone(),
        library_path: input.facts.instance.library_path.clone(),
        install_path: input.facts.instance.install_path.clone(),
        release_id: input.release_id.to_string(),
        selection_id: input.selection_id.map(str::to_string),
        source_id: input.source_id.to_string(),
        source_record_id: input.source_record_id.map(str::to_string),
        candidate_sha256: input.candidate_sha256.to_string(),
        settings_revision: input.settings_revision,
        rule_fingerprint: input.rule_fingerprint.to_string(),
        conditions_sha256: input.conditions_sha256.to_string(),
        content: input.content,
        plan_id: input.plan_id.to_string(),
        task_id: input.task_id.map(str::to_string),
        request_id: input.request_id.map(str::to_string),
        policy: input.policy,
        facts: input.facts,
        created_at: input.now,
        expires_at,
        lease_active: true,
        revoked: false,
    })
}

/// Re-checks every binding against the current instance (design §5.2).
///
/// Returns the first failing check with a stable code, in the order: schema,
/// revocation, lease, expiry, ownership, settings revision, policy, candidate,
/// rule, conditions, content, facts.
pub fn validate(authorization: &Authorization, current: &CurrentState) -> AuthResult<()> {
    validate_stored(authorization)
        .map_err(|error| Invalidation::new("authorization_invalid", error.to_string()))?;

    if authorization.revoked || current.revoked {
        return Err(Invalidation::code("authorization_revoked"));
    }
    if !authorization.lease_active || !current.lease_active {
        return Err(Invalidation::code("authorization_lease_inactive"));
    }
    if current.now > authorization.expires_at {
        return Err(Invalidation::new(
            "authorization_expired",
            format!("expired at {}", authorization.expires_at),
        ));
    }
    if authorization.plan_id != current.plan_id || authorization.instance_id != current.instance_id
    {
        return Err(Invalidation::code("authorization_ownership_changed"));
    }
    if authorization.settings_revision != current.settings_revision {
        return Err(Invalidation::new(
            "authorization_settings_revision_changed",
            format!(
                "{} -> {}",
                authorization.settings_revision, current.settings_revision
            ),
        ));
    }
    if authorization.policy != current.policy {
        return Err(Invalidation::code("authorization_policy_changed"));
    }
    if authorization.candidate_sha256 != current.candidate_sha256 {
        return Err(Invalidation::code("authorization_candidate_changed"));
    }
    if authorization.rule_fingerprint != current.rule_fingerprint {
        return Err(Invalidation::code("authorization_rule_changed"));
    }
    if authorization.conditions_sha256 != current.conditions_sha256 {
        return Err(Invalidation::code("authorization_conditions_changed"));
    }
    if authorization.content != current.content {
        return Err(Invalidation::code("authorization_content_changed"));
    }
    if authorization.facts != current.facts {
        let failure = current
            .facts
            .failures()
            .first()
            .map(|failure| failure.code.clone());
        return Err(match failure {
            Some(code) => Invalidation::new("authorization_facts_changed", code),
            None => Invalidation::code("authorization_facts_changed"),
        });
    }
    Ok(())
}

/// Structural validation used before persist/load (schema/path/TTL bounds).
pub fn validate_stored(authorization: &Authorization) -> Result<()> {
    if authorization.schema_version != AUTHORIZATION_SCHEMA_VERSION {
        return Err(invalid("authorization_schema_unsupported"));
    }
    if authorization.installer_schema_version != super::STORE_SCHEMA_VERSION {
        return Err(invalid("installer_schema_unsupported"));
    }
    if Uuid::parse_str(&authorization.plan_id).is_err() {
        return Err(invalid("authorization_invalid"));
    }
    if authorization.instance_id.is_empty()
        || authorization.release_id.is_empty()
        || authorization.source_id.is_empty()
    {
        return Err(invalid("authorization_invalid"));
    }
    if authorization.expires_at < authorization.created_at
        || authorization.expires_at - authorization.created_at > PLAN_TTL_SECONDS
    {
        return Err(invalid("authorization_invalid"));
    }
    Ok(())
}

/// `jobs/<plan_id>/authorization.json` (protected).
pub fn authorization_path(paths: &InstallerPaths, plan_id: &str) -> Result<PathBuf> {
    paths.authorization(plan_id)
}

/// Atomically persists the authorization into its protected job directory.
pub fn write(paths: &InstallerPaths, authorization: &Authorization) -> Result<()> {
    validate_stored(authorization)?;
    let path = authorization_path(paths, &authorization.plan_id)?;
    let bytes = serde_json::to_vec(authorization).map_err(|error| invalid(error.to_string()))?;
    super::store::write_atomic(&path, &bytes, MAX_AUTHORIZATION_BYTES)
}

/// Loads the authorization for `plan_id`, if present.
pub fn load(paths: &InstallerPaths, plan_id: &str) -> Result<Option<Authorization>> {
    let path = authorization_path(paths, plan_id)?;
    if !path.exists() {
        return Ok(None);
    }
    if !super::zip_recipe::no_links(&path) {
        return Err(invalid("linked_installer_state"));
    }
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.is_file() {
        return Err(invalid("linked_installer_state"));
    }
    if metadata.len() > MAX_AUTHORIZATION_BYTES {
        return Err(invalid("authorization_too_large"));
    }
    let bytes = fs::read(&path)?;
    let authorization: Authorization =
        serde_json::from_slice(&bytes).map_err(|_| invalid("authorization_invalid"))?;
    validate_stored(&authorization)?;
    Ok(Some(authorization))
}

/// Whether the persistent revocation marker exists for `plan_id`
/// (p0-contracts §5.1; written by the orchestrator/transaction layer).
pub fn is_revoked(paths: &InstallerPaths, plan_id: &str) -> Result<bool> {
    Ok(paths.job(plan_id)?.join("revoked").exists())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::installer::facts::{
        BaselineFact, BuildFacts, DlcFacts, Fact, GameFacts, InstanceFacts, LanguageFacts,
        ManifestFacts,
    };

    const PLAN: &str = "11111111-1111-1111-1111-111111111111";

    fn facts() -> GameFacts {
        GameFacts {
            instance: InstanceFacts {
                instance_id: format!("2303800|{}|{}", "/lib", "/lib/steamapps/common/Test"),
                appid: 2_303_800,
                library_path: PathBuf::from("/lib"),
                install_path: PathBuf::from("/lib/steamapps/common/Test"),
            },
            manifest: Fact::evidence(ManifestFacts {
                sha256: "a".repeat(64),
                installdir: "Test".into(),
                stateflags: Some("4".into()),
                buildid: Some("12345".into()),
            }),
            build: Fact::evidence(BuildFacts {
                buildid: "12345".into(),
                verified: true,
            }),
            language: Fact::evidence(LanguageFacts {
                user: Some("schinese".into()),
                mounted: Some("schinese".into()),
                selected: Some("schinese".into()),
            }),
            dlc: Fact::evidence(DlcFacts { installed: vec![] }),
            baselines: vec![BaselineFact {
                path: "data.xp3".into(),
                expected_sha256: "b".repeat(64),
                expected_crc32: None,
                observed: None,
                failure: None,
            }],
        }
    }

    fn mappings() -> Vec<FileMapping> {
        vec![FileMapping {
            source: "data.xp3".into(),
            destination: "data.xp3".into(),
            original_sha256: Some("c".repeat(64)),
            installed_sha256: "d".repeat(64),
        }]
    }

    fn content() -> ContentBinding {
        ContentBinding::publisher_zip(&"e".repeat(64), 42, &mappings())
    }

    fn authorize_ok() -> Authorization {
        authorize(AuthorizeInput {
            release_id: "exe-content-abc-2303800",
            selection_id: Some("exe-selection-abc-2303800"),
            source_id: "src",
            source_record_id: Some("https://example.invalid/page"),
            candidate_sha256: &"f".repeat(64),
            rule_fingerprint: &"1".repeat(64),
            settings_revision: 7,
            conditions_sha256: &"2".repeat(64),
            content: content(),
            policy: PolicyVersions::default(),
            plan_id: PLAN,
            task_id: Some("task-1"),
            request_id: Some("req-1"),
            facts: facts(),
            now: 1_000,
            ttl_seconds: 900,
        })
        .unwrap()
    }

    fn current() -> CurrentState {
        CurrentState {
            now: 1_500,
            plan_id: PLAN.into(),
            instance_id: format!("2303800|{}|{}", "/lib", "/lib/steamapps/common/Test"),
            revoked: false,
            lease_active: true,
            facts: facts(),
            settings_revision: 7,
            policy: PolicyVersions::default(),
            candidate_sha256: "f".repeat(64),
            rule_fingerprint: "1".repeat(64),
            conditions_sha256: "2".repeat(64),
            content: content(),
        }
    }

    #[test]
    fn authorize_and_validate_round_trip() {
        let authorization = authorize_ok();
        assert_eq!(authorization.expires_at, 1_900);
        validate(&authorization, &current()).unwrap();
    }

    #[test]
    fn recheck_matrix_is_diagnosable() {
        let authorization = authorize_ok();

        let mut state = current();
        state.facts.build = Fact::evidence(BuildFacts {
            buildid: "99999".into(),
            verified: true,
        });
        assert_eq!(
            validate(&authorization, &state).unwrap_err().code,
            "authorization_facts_changed"
        );

        let mut state = current();
        state.content = ContentBinding::publisher_zip(&"0".repeat(64), 42, &mappings());
        assert_eq!(
            validate(&authorization, &state).unwrap_err().code,
            "authorization_content_changed"
        );

        let mut state = current();
        state.settings_revision += 1;
        assert_eq!(
            validate(&authorization, &state).unwrap_err().code,
            "authorization_settings_revision_changed"
        );

        let mut state = current();
        state.policy = PolicyVersions {
            report_schema_version: 2,
            ..PolicyVersions::default()
        };
        assert_eq!(
            validate(&authorization, &state).unwrap_err().code,
            "authorization_policy_changed"
        );

        let mut state = current();
        state.candidate_sha256 = "9".repeat(64);
        assert_eq!(
            validate(&authorization, &state).unwrap_err().code,
            "authorization_candidate_changed"
        );

        let mut state = current();
        state.rule_fingerprint = "9".repeat(64);
        assert_eq!(
            validate(&authorization, &state).unwrap_err().code,
            "authorization_rule_changed"
        );

        let mut state = current();
        state.conditions_sha256 = "9".repeat(64);
        assert_eq!(
            validate(&authorization, &state).unwrap_err().code,
            "authorization_conditions_changed"
        );

        let mut state = current();
        state.now = 1_901;
        assert_eq!(
            validate(&authorization, &state).unwrap_err().code,
            "authorization_expired"
        );

        let mut state = current();
        state.revoked = true;
        assert_eq!(
            validate(&authorization, &state).unwrap_err().code,
            "authorization_revoked"
        );

        let mut state = current();
        state.lease_active = false;
        assert_eq!(
            validate(&authorization, &state).unwrap_err().code,
            "authorization_lease_inactive"
        );

        let mut state = current();
        state.instance_id = "other".into();
        assert_eq!(
            validate(&authorization, &state).unwrap_err().code,
            "authorization_ownership_changed"
        );
    }

    #[test]
    fn authorize_rejects_bad_inputs() {
        fn input<'a>(
            ttl: u64,
            facts: GameFacts,
            plan: &'a str,
            candidate: &'a str,
            rule: &'a str,
            conditions: &'a str,
        ) -> AuthorizeInput<'a> {
            AuthorizeInput {
                release_id: "rel",
                selection_id: None,
                source_id: "src",
                source_record_id: None,
                candidate_sha256: candidate,
                rule_fingerprint: rule,
                settings_revision: 1,
                conditions_sha256: conditions,
                content: content(),
                policy: PolicyVersions::default(),
                plan_id: plan,
                task_id: None,
                request_id: None,
                facts,
                now: 10,
                ttl_seconds: ttl,
            }
        }

        let base = authorize_ok();
        assert_eq!(base.schema_version, AUTHORIZATION_SCHEMA_VERSION);

        let candidate = "f".repeat(64);
        let rule = "1".repeat(64);
        let conditions = "2".repeat(64);
        assert_eq!(
            authorize(input(0, facts(), PLAN, &candidate, &rule, &conditions))
                .unwrap_err()
                .code,
            "authorization_ttl_invalid"
        );
        assert_eq!(
            authorize(input(901, facts(), PLAN, &candidate, &rule, &conditions))
                .unwrap_err()
                .code,
            "authorization_ttl_invalid"
        );
        assert_eq!(
            authorize(input(
                900,
                facts(),
                "not-a-uuid",
                &candidate,
                &rule,
                &conditions
            ))
            .unwrap_err()
            .code,
            "authorization_plan_invalid"
        );

        let mut invalid_facts = facts();
        invalid_facts.build = Fact::failed_code("build_not_verified", "x");
        assert_eq!(
            authorize(input(
                900,
                invalid_facts,
                PLAN,
                &candidate,
                &rule,
                &conditions
            ))
            .unwrap_err()
            .code,
            "build_not_verified"
        );
    }

    #[test]
    fn persistence_round_trips_in_the_protected_job_directory() {
        let temp = tempfile::tempdir().unwrap();
        let paths = InstallerPaths::new(temp.path());
        crate::installer::init_atomic(&paths).unwrap();

        let authorization = authorize_ok();
        write(&paths, &authorization).unwrap();
        let path = authorization_path(&paths, PLAN).unwrap();
        assert_eq!(path, paths.jobs_dir().join(PLAN).join("authorization.json"));
        assert!(paths.is_protected(&path));

        let loaded = load(&paths, PLAN).unwrap().unwrap();
        assert_eq!(loaded, authorization);
        assert_eq!(
            load(&paths, "22222222-2222-2222-2222-222222222222").unwrap(),
            None
        );
        assert!(!is_revoked(&paths, PLAN).unwrap());

        fs::write(paths.job(PLAN).unwrap().join("revoked"), b"revoked\n").unwrap();
        assert!(is_revoked(&paths, PLAN).unwrap());
    }
}
