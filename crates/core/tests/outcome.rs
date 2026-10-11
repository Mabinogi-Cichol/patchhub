use steam_patch_core::{install::Transaction, installer::outcome::*};

fn transaction(state: &str, error: Option<&str>) -> Transaction {
    Transaction {
        transaction_id: "t".into(),
        plan_id: "p".into(),
        instance_id: "i".into(),
        release_id: "exe-content-legacy".into(),
        state: state.into(),
        error: error.map(str::to_owned),
        created_at: 10,
        updated_at: 20,
        has_replacements: true,
        selection_id: None,
        content_id: None,
    }
}
#[test]
fn durable_terminal_truth_table() {
    let cases = [
        ("committed", None, Disposition::Completed, None, false),
        (
            "committed",
            Some("cancelled"),
            Disposition::Completed,
            None,
            false,
        ),
        (
            "committed",
            Some("lease_expired"),
            Disposition::Completed,
            None,
            false,
        ),
        (
            "rolled_back",
            Some("cancelled"),
            Disposition::Cancelled,
            Some(FailureKind::UserCancelled),
            false,
        ),
        (
            "rolled_back",
            Some("task_cancelled"),
            Disposition::Cancelled,
            Some(FailureKind::UserCancelled),
            false,
        ),
        (
            "rolled_back",
            Some("lease_expired"),
            Disposition::Cancelled,
            Some(FailureKind::LeaseExpired),
            false,
        ),
        (
            "rolled_back",
            None,
            Disposition::Failed,
            Some(FailureKind::DomainFailure),
            false,
        ),
        (
            "rolled_back",
            Some("cannot_cancel_write: denied"),
            Disposition::Failed,
            Some(FailureKind::DomainFailure),
            false,
        ),
        (
            "rolled_back",
            Some("cancelled: recovery_conflict"),
            Disposition::Failed,
            Some(FailureKind::DomainFailure),
            false,
        ),
        (
            "rolled_back",
            Some("installer_lease_expired"),
            Disposition::Failed,
            Some(FailureKind::DomainFailure),
            false,
        ),
        (
            "rolled_back",
            Some("authorization_revoked"),
            Disposition::Failed,
            Some(FailureKind::DomainFailure),
            false,
        ),
        (
            "needs_recovery",
            Some("cancelled"),
            Disposition::Failed,
            Some(FailureKind::UserCancelled),
            true,
        ),
        (
            "needs_recovery",
            Some("recovery_conflict"),
            Disposition::Failed,
            Some(FailureKind::DomainFailure),
            true,
        ),
        (
            "planned",
            Some("cancelled"),
            Disposition::Failed,
            Some(FailureKind::UserCancelled),
            false,
        ),
        (
            "superseded",
            None,
            Disposition::Failed,
            Some(FailureKind::DomainFailure),
            false,
        ),
    ];
    for (state, error, disposition, kind, required) in cases {
        let t = transaction(state, error);
        let out = InstallerOutcome::from_transaction(&t, None, None);
        assert_eq!(out.disposition, disposition, "{state}/{error:?}");
        assert_eq!(
            out.cause.as_ref().map(|c| c.kind),
            kind,
            "{state}/{error:?}"
        );
        assert_eq!(out.recovery.as_ref().is_some_and(|r| r.required), required);
        assert_eq!(out.error, t.error);
        assert_eq!(out.cleanup, CleanupResult::default());
    }
}
#[test]
fn identities_are_explicit_and_durable_values_take_precedence() {
    let mut t = transaction("committed", None);
    let legacy = InstallerOutcome::from_transaction(&t, None, None);
    assert!(legacy.selection_id.is_none());
    assert!(legacy.content_id.is_none());
    let route = InstallerOutcome::from_transaction(&t, Some("selection"), Some("content"));
    assert_eq!(route.selection_id.as_deref(), Some("selection"));
    assert_eq!(route.content_id.as_deref(), Some("content"));
    t.selection_id = Some("durable-selection".into());
    t.content_id = Some("durable-content".into());
    let durable = InstallerOutcome::from_transaction(&t, Some("other"), Some("other"));
    assert_eq!(durable.selection_id, t.selection_id);
    assert_eq!(durable.content_id, t.content_id);
}
#[test]
fn cleanup_failure_cannot_reinterpret_commit_and_replay_does_not_repeat_cleanup() {
    let t = transaction("committed", None);
    let mut out = InstallerOutcome::from_transaction(&t, None, None);
    out.cleanup = CleanupResult {
        status: CleanupStatus::Failed,
        performed: true,
        removed_bytes: 0,
        failures: vec![CleanupFailure {
            code: CleanupErrorCode::CleanupFailed,
            detail: Some("denied".into()),
        }],
    };
    assert_eq!(out.disposition, Disposition::Completed);
    assert!(out.cause.is_none());
    let replay = InstallerOutcome::from_transaction(&t, None, None);
    assert_eq!(replay.disposition, out.disposition);
    assert_eq!(replay.cleanup, CleanupResult::default());
}
#[test]
fn diagnostic_details_stay_separate_from_machine_codes() {
    let out = InstallerOutcome::from_transaction(
        &transaction(
            "rolled_back",
            Some("plan_has_blockers: baseline_conflict:a"),
        ),
        None,
        None,
    );
    let cause = out.cause.unwrap();
    assert_eq!(cause.kind, FailureKind::DomainFailure);
    assert_eq!(cause.code, "plan_has_blockers");
    assert_eq!(cause.detail.as_deref(), Some("baseline_conflict:a"));
    let out = InstallerOutcome::from_transaction(
        &transaction("rolled_back", Some("OS error while cancelling file")),
        None,
        None,
    );
    let cause = out.cause.unwrap();
    assert_eq!(cause.code, "installer_failed");
    assert_eq!(
        cause.detail.as_deref(),
        Some("OS error while cancelling file")
    );
}
#[test]
fn wire_shape_keeps_transaction_fields_and_required_cleanup() {
    let out = InstallerOutcome::from_transaction(
        &transaction("rolled_back", Some("lease_expired")),
        None,
        None,
    );
    let json = serde_json::to_value(out).unwrap();
    assert_eq!(json["disposition"], "cancelled");
    assert_eq!(json["cause"]["kind"], "lease_expired");
    assert_eq!(json["cleanup"]["status"], "not_requested");
    assert_eq!(json["cleanup"]["performed"], false);
    assert_eq!(json["cleanup"]["removed_bytes"], 0);
    assert!(json["selection_id"].is_null());
    assert!(json["content_id"].is_null());
    assert!(json["recovery"].is_null());
}

#[test]
fn structured_details_are_bounded_on_utf8_boundaries_while_error_fact_is_retained() {
    let error = format!("failure: {}", "中".repeat(1000));
    let out = InstallerOutcome::from_transaction(
        &transaction("needs_recovery", Some(&error)),
        None,
        None,
    );
    assert_eq!(out.error.as_deref(), Some(error.as_str()));
    assert_eq!(out.cause.unwrap().detail.unwrap().len(), 2046);
    let detail = out.recovery.unwrap().detail.unwrap();
    assert!(detail.len() <= 2048);
    assert!(error.starts_with(&detail));
}

#[test]
fn unbounded_free_error_head_is_not_a_machine_code() {
    let error = format!("{}: original diagnostic", "a".repeat(129));
    let out =
        InstallerOutcome::from_transaction(&transaction("rolled_back", Some(&error)), None, None);
    let cause = out.cause.unwrap();
    assert_eq!(cause.code, "installer_failed");
    assert_eq!(cause.detail.as_deref(), Some(error.as_str()));
    assert_eq!(out.error.as_deref(), Some(error.as_str()));
}

#[test]
fn writers_exact_cleanup_suffix_preserves_cancel_cause_but_free_text_does_not() {
    for (code, kind) in [
        ("cancelled", FailureKind::UserCancelled),
        ("task_cancelled", FailureKind::UserCancelled),
        ("lease_expired", FailureKind::LeaseExpired),
    ] {
        let raw = format!("{code}; cleanup_pending:unsafe_path");
        let result =
            InstallerOutcome::from_transaction(&transaction("rolled_back", Some(&raw)), None, None);
        assert_eq!(result.disposition, Disposition::Cancelled);
        let cause = result.cause.unwrap();
        assert_eq!(cause.code, code);
        assert_eq!(cause.kind, kind);
        assert_eq!(cause.detail.as_deref(), Some("cleanup_pending:unsafe_path"));
        assert_eq!(result.error.as_deref(), Some(raw.as_str()));
    }
    for raw in [
        "cancelled; arbitrary_detail",
        "cancelled: cleanup_pending:unsafe_path",
        "cannot_cancel; cleanup_pending:unsafe_path",
        "lease_expired; cleanup_pending:",
    ] {
        assert_eq!(
            InstallerOutcome::from_transaction(&transaction("rolled_back", Some(raw)), None, None)
                .disposition,
            Disposition::Failed
        );
    }
}
