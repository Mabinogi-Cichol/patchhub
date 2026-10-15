//! Fixed cross-language acceptance corpus generated from the Rust contract.
use crate::*;
use serde_json::{json, Value};
pub fn capabilities() -> Value {
    json!({"protocol_version":4,"methods":METHODS,"contracts":ContractVersions::default(),"installer":{"schema_version":1,"selection_version":1,"content_version":1,"adapter_version":1,"policy_schema_version":1,"report_schema_version":1,"formats":{"zip":"supported","exe":"supported"},"strategies":["managed_zip","publisher_zip","publisher_exe"],"links":{"zip":{"available":true,"blockers":[]},"exe":{"available":false,"blockers":[{"code":"exe_sandbox_unavailable"}]}}},"sources":{"discovery_schema_version":1,"adapter_version":1,"parser_version":steam_patch_core::sources::PARSER_VERSION,"query_version":steam_patch_core::sources::QUERY_VERSION,"methods":SOURCE_METHODS}})
}
pub fn host() -> Value {
    json!({"roots":[],"catalog":null,"signed_catalog":null,"trust_keys":null,"state_dir":null,"platform":"windows"})
}
pub fn candidate() -> Value {
    json!({"candidate_id":"candidate-0123456789abcdef01234567","source_id":"fixture","source_record_id":"https://example.com/game","game_appid":42,"parent_appid":null,"aliases":[],"game_title":"Fixture","patch_title":"Patch","variant":"base","requires_dlc":null,"excludes_dlc":null,"platforms":["windows"],"patch_kind":"adult_content","languages":["zh_cn"],"version_raw":null,"version_evidence":[],"version_identity":"fixture","published_at":null,"patch_updated_at":null,"source_page":"https://example.com/game","upstream_page":null,"mirrors":[],"format_reported":"zip","format_observed":"unknown","capability":"discover_only","identity_evidence":[],"provenance":[],"match_basis":"appid","observed_at":"2026-10-11T00:00:00Z","cache_status":"fresh","warnings":[],"fingerprint":"fixture"})
}
pub fn discovery() -> Value {
    json!({"status":"complete","candidates":[candidate()],"diagnostics":[]})
}
pub fn source_summary() -> Value {
    let mut v = rule();
    v["origin"] = json!("builtin");
    v["status"] = json!("source_refreshed");
    v["entry_count"] = json!(1);
    v["cached_at"] = Value::Null;
    v["checked_at"] = json!(1);
    v
}
pub fn scan() -> Value {
    let available = json!({"available":true,"blockers":[]});
    let unavailable =
        json!({"available":false,"blockers":[{"code":"reinstall_unowned_or_incomplete"}]});
    json!({"catalog_revision":1,"catalog_trust":"signed","catalog_error":null,"platform":"windows","roots":[],"installs":[{"game":{"instance_id":"instance","appid":42,"name":"Fixture","localized_name":null,"name_language":null,"library_path":"fixture/library","install_path":"fixture/library/steamapps/common/game","buildid":null,"availability":"installed","kind":"game"},"match_status":"matched","patches":[{"release_id":"release","name":"Patch","source_page":"https://example.com/game","support_level":"managed_zip","actions":{"automatic":available,"local_archive":available,"reinstall":unavailable,"overwrite":available,"primary":{"operation":"overwrite","label":"install","input":"automatic"}},"compatibility":"verified","blockers":[],"detection":{"status":"not_detected","files":[{"path":"patch.dat","status":"missing","actual_sha256":null}]}}],"discovery":discovery(),"diagnostics":[]}],"diagnostics":[]})
}
pub fn transaction(state: &str) -> Value {
    json!({"transaction_id":"transaction","plan_id":"plan","instance_id":"instance","release_id":"release","state":state,"error":null,"created_at":1,"updated_at":1,"has_replacements":false,"selection_id":null,"content_id":null})
}
pub fn outcome(state: &str, disposition: &str) -> Value {
    let mut v = transaction(state);
    v["disposition"] = json!(disposition);
    v["cause"] = if disposition == "cancelled" {
        json!({"kind":"user_cancelled","code":"cancelled"})
    } else if disposition == "failed" {
        json!({"kind":"domain_failure","code":"apply_failed"})
    } else {
        Value::Null
    };
    v["recovery"] = if state == "needs_recovery" {
        json!({"required":true,"code":"needs_recovery"})
    } else {
        Value::Null
    };
    v["cleanup"] = json!(steam_patch_core::installer::outcome::CleanupResult::default());
    v
}
pub fn rule() -> Value {
    json!({"id":"fixture","name":"Fixture","adapter":"table_html","enabled":true,"pages":["https://example.com/"],"page_hosts":["example.com"],"download_hosts":[],"role":"publisher","discovery_only":false,"priority":100,"root_layout":null})
}
pub fn success_result(method: Method) -> Value {
    match method {
        Method::GetCapabilities => capabilities(),
        Method::Scan => scan(),
        Method::Preview => {
            json!({"instance_id":"instance","release_id":"release","selection_id":null,"catalog_revision":1,"executable":false,"blockers":[],"changes":[{"path":"patch.dat","action":"replace","baseline_matches":null,"actual_sha256":null}],"diagnostics":[]})
        }
        Method::InstallPatch => outcome("committed", "completed"),
        Method::ListTransactions | Method::Recover => json!([transaction("committed")]),
        Method::DiscardPlan => transaction("rolled_back"),
        Method::BackupStatus => json!({"total_bytes":0,"files":0,"failures":[]}),
        Method::CleanupBackups => {
            json!({"status":"completed","removed_bytes":0,"removed_files":0,"failures":[]})
        }
        Method::CleanupCache => {
            json!(steam_patch_core::installer::outcome::CleanupResult::default())
        }
        Method::ListSources => json!([source_summary()]),
        Method::ListGameSources => {
            let mut v = source_summary();
            v["subscribed"] = json!(true);
            v["matched"] = json!(true);
            json!([v])
        }
        Method::SetSourceEnabled => {
            json!({"settings_revision":1,"source_id":"fixture","enabled":true})
        }
        Method::SetGameSource => {
            json!({"settings_revision":1,"source_id":"fixture","subscribed":true})
        }
        Method::AddSource => json!({"settings_revision":1,"rule":rule()}),
        Method::RefreshSources => {
            json!({"settings_revision":1,"statuses":[{"checked_at":1,"cached_at":null,"entry_count":0,"code":"source_refreshed"}]})
        }
        Method::ScanSources => {
            json!({"settings_revision":1,"results":[{"instance_id":"instance","appid":42,"status":"complete","candidates":[candidate()],"diagnostics":[]}]})
        }
        Method::DiscoverGame => {
            json!({"instance_id":"instance","appid":42,"generation":1,"settings_revision":1,"status":"complete","candidates":[candidate()],"diagnostics":[]})
        }
    }
}
pub fn request(method: Method) -> Value {
    let mut p = json!({"host":host()});
    match method {
        Method::Preview => {
            p["instance_id"] = json!("instance");
            p["release_id"] = json!("release");
        }
        Method::InstallPatch => {
            p["instance_id"] = json!("instance");
            p["release_id"] = json!("release");
            p["operation"] = json!("overwrite");
            p["cleanup_policy"] = json!("disabled");
        }
        Method::DiscardPlan => p["plan_id"] = json!("plan"),
        Method::ListGameSources => p["appid"] = json!(42),
        Method::SetSourceEnabled => {
            p["source_id"] = json!("fixture");
            p["enabled"] = json!(true);
        }
        Method::SetGameSource => {
            p["source_id"] = json!("fixture");
            p["subscribed"] = json!(true);
            p["appid"] = json!(42);
        }
        Method::AddSource => p["rule"] = rule(),
        Method::DiscoverGame => p["instance_id"] = json!("instance"),
        _ => {}
    }
    json!({"protocol_version":4,"request_id":"fixture","method":method,"params":p})
}
pub fn install_event() -> Value {
    json!({"event":"install_progress","protocol_version":4,"request_id":"fixture","sequence":0,"instance_id":"instance","selection_id":null,"stage":"applying","substage":"writing","bytes_done":1,"total_bytes":null,"files_done":0,"total_files":null,"current_file":null,"archive_bytes":1})
}
pub fn cases() -> Value {
    let mut requests = Vec::new();
    let mut responses = Vec::new();
    let mut events = Vec::new();
    for method in ALL_METHODS {
        requests.push(json!({"name":format!("{}-request",method.as_str()),"request":request(method),"task_channel":method.is_task()}));
        let success = json!({"protocol_version":4,"request_id":"fixture","ok":true,"result":success_result(method)});
        responses.push(json!({"name":format!("{}-success",method.as_str()),"method":method,"response":success}));
        responses.push(json!({"name":format!("{}-failure",method.as_str()),"method":method,"response":{"protocol_version":4,"request_id":"fixture","ok":false,"error":{"code":"io_error","detail":"固定诊断"}}}));
    }
    let mut bad = |name: &str, mut r: Value, error: &str| {
        requests.push(json!({"name":name,"request":r.take(),"task_channel":true,"error":error}));
    };
    let mut r = request(Method::Scan);
    r["protocol_version"] = json!(3);
    r["params"] = json!(false);
    bad("v3-before-params", r, "unsupported_protocol");
    let mut r = request(Method::Scan);
    r["method"] = json!("plan_install");
    r["params"] = json!(false);
    bad("unknown-method-before-params", r, "unsupported_method");
    let mut r = request(Method::Scan);
    r["params"]["operation"] = json!("overwrite");
    bad("method-specific-unknown", r, "invalid_request");
    let mut r = request(Method::Scan);
    r["params"]["host"]
        .as_object_mut()
        .unwrap()
        .remove("catalog");
    bad("required-null-host", r, "invalid_request");
    let mut r = request(Method::InstallPatch);
    r["params"].as_object_mut().unwrap().remove("operation");
    bad("required-operation", r, "invalid_request");
    let mut r = request(Method::InstallPatch);
    r["params"]
        .as_object_mut()
        .unwrap()
        .remove("cleanup_policy");
    bad("required-policy", r, "invalid_request");
    let mut r = request(Method::InstallPatch);
    r["params"]["request_id"] = json!("inner");
    bad("outer-id-only", r, "invalid_request");
    for (name, id, error) in [
        (
            "id-unicode-limit",
            "界".repeat(43),
            Some("invalid_request_id"),
        ),
        ("id-unicode-exact", "界".repeat(42) + "ab", None),
        (
            "id-control",
            "id\u{0085}".to_owned(),
            Some("invalid_request_id"),
        ),
        ("id-empty", "".to_owned(), Some("invalid_request_id")),
    ] {
        let mut r = request(Method::Scan);
        r["request_id"] = json!(id);
        requests.push(json!({"name":name,"request":r,"task_channel":false,"error":error}));
    }
    let mut r = request(Method::Preview);
    r["params"]["selection_id"] = json!("zip-selection-0123456789abcdef01234567-42");
    requests.push(
        json!({"name":"target-xor","request":r,"task_channel":false,"error":"invalid_request"}),
    );
    let mut r = request(Method::InstallPatch);
    r["params"].as_object_mut().unwrap().remove("release_id");
    r["params"]["selection_id"] = json!("zip-selection-0123456789abcdef01234567-42");
    r["params"]["archive"] = json!("local.zip");
    requests.push(json!({"name":"selection-archive","request":r,"task_channel":true,"error":"archive_not_allowed_for_selection"}));
    requests.push(json!({"name":"task-required","request":request(Method::InstallPatch),"task_channel":false,"error":"task_required"}));
    let event_values = [
        ("install_progress", install_event()),
        (
            "source_progress",
            json!({"event":"source_progress","protocol_version":4,"request_id":"fixture","sequence":0,"completed":0,"total":1,"running":true,"settings_revision":1}),
        ),
        (
            "source_update-game",
            json!({"event":"source_update","protocol_version":4,"request_id":"fixture","sequence":0,"appid":42,"index":0,"instance_id":"instance","settings_revision":1,"discovery":discovery()}),
        ),
        (
            "source_update-refresh",
            json!({"event":"source_update","protocol_version":4,"request_id":"fixture","sequence":0,"source_id":"fixture","settings_revision":1,"status":{"checked_at":1,"cached_at":null,"entry_count":0,"code":"source_refreshed"}}),
        ),
    ];
    for (name, event) in event_values {
        events.push(json!({"name":name,"event":event}));
    }
    for (name, field, value, error) in [
        (
            "v3-event",
            "protocol_version",
            json!(3),
            "unsupported_protocol",
        ),
        (
            "unknown-stage",
            "stage",
            json!("done"),
            "invalid_core_response",
        ),
        (
            "bool-sequence",
            "sequence",
            json!(true),
            "invalid_core_response",
        ),
        (
            "unsafe-integer",
            "sequence",
            json!(MAX_SAFE_INTEGER + 1),
            "invalid_core_response",
        ),
        (
            "request-mismatch",
            "request_id",
            json!("other"),
            "invalid_core_response",
        ),
    ] {
        let mut e = install_event();
        e[field] = value;
        events.push(json!({"name":name,"event":e,"error":error}));
    }
    let mut e = install_event();
    e["stage"] = json!("downloading");
    events.push(json!({"name":"substage-only-applying","event":e,"error":"invalid_core_response"}));
    for (state, disposition) in [
        ("rolled_back", "cancelled"),
        ("rolled_back", "failed"),
        ("needs_recovery", "failed"),
    ] {
        responses.push(json!({"name":format!("outcome-{state}-{disposition}"),"method":"install_patch","response":{"protocol_version":4,"request_id":"fixture","ok":true,"result":outcome(state,disposition)}}));
    }
    let mut v = json!({"protocol_version":4,"request_id":"fixture","ok":false,"error":{"code":"io_error","detail":"界".repeat(683)}});
    responses.push(json!({"name":"detail-over-limit","method":"scan","response":v,"error":"invalid_core_response"}));
    v = json!({"protocol_version":4,"request_id":"fixture","ok":false,"error":{"code":"io_error","detail":"界".repeat(682)+"ab"}});
    responses.push(json!({"name":"detail-exact-limit","method":"scan","response":v}));
    for version in [1, 2, 5, 999] {
        let mut r = request(Method::Scan);
        r["protocol_version"] = json!(version);
        requests.push(json!({"name":format!("unsupported-version-{version}"),"request":r,"task_channel":false,"error":"unsupported_protocol"}));
    }
    for id in ["界".repeat(85) + "a", "a".repeat(256)] {
        let mut r = request(Method::Preview);
        r["params"]["instance_id"] = json!(id);
        requests.push(json!({"name":"business-id-exact","request":r,"task_channel":false}));
    }
    let mut r = request(Method::Preview);
    r["params"]["release_id"] = json!("界".repeat(86));
    requests.push(
        json!({"name":"business-id-over","request":r,"task_channel":false,"error":"invalid_id"}),
    );
    for (name, field, value) in [
        ("v3-response", "protocol_version", json!(3)),
        ("wrong-response-id", "request_id", json!("other")),
    ] {
        let mut v = json!({"protocol_version":4,"request_id":"fixture","ok":true,"result":scan()});
        v[field] = value;
        responses.push(json!({"name":name,"method":"scan","response":v,"error":if field=="protocol_version"{"unsupported_protocol"}else{"invalid_core_response"}}));
    }
    for name in ["cause", "recovery", "cleanup", "selection_id"] {
        let mut v = outcome("committed", "completed");
        v.as_object_mut().unwrap().remove(name);
        responses.push(json!({"name":format!("missing-outcome-{name}"),"method":"install_patch","response":{"protocol_version":4,"request_id":"fixture","ok":true,"result":v},"error":"invalid_core_response"}));
    }
    let mut v = scan();
    v["installs"][0]["patches"][0]
        .as_object_mut()
        .unwrap()
        .remove("actions");
    responses.push(json!({"name":"missing-actions","method":"scan","response":{"protocol_version":4,"request_id":"fixture","ok":true,"result":v},"error":"invalid_core_response"}));
    for (name, state, disposition) in [
        ("commit-cannot-cancel", "committed", "cancelled"),
        ("recovery-cannot-complete", "needs_recovery", "completed"),
    ] {
        responses.push(json!({"name":name,"method":"install_patch","response":{"protocol_version":4,"request_id":"fixture","ok":true,"result":outcome(state,disposition)},"error":"invalid_core_response"}));
    }
    for key in [
        "instance_id",
        "selection_id",
        "substage",
        "total_bytes",
        "total_files",
        "current_file",
        "archive_bytes",
    ] {
        let mut e = install_event();
        e.as_object_mut().unwrap().remove(key);
        events.push(json!({"name":format!("missing-event-{key}"),"event":e,"error":"invalid_core_response"}));
    }
    let mut r = request(Method::DiscoverGame);
    r["params"]["generation"] = json!(MAX_SAFE_INTEGER + 1);
    requests.push(json!({"name":"request-generation-unsafe","request":r,"task_channel":false,"error":"invalid_request"}));
    let mut r = request(Method::DiscoverGame);
    r["params"]["generation"] = json!(MAX_SAFE_INTEGER);
    requests.push(json!({"name":"request-generation-safe","request":r,"task_channel":false}));
    for status in [
        "complete",
        "partial",
        "failed",
        "skipped",
        "stale",
        "cache_complete",
        "cache_partial",
        "not_queried",
        "source_refresh_failed",
        "source_offline_cached",
        "empty",
        "queued",
    ] {
        let mut result = success_result(Method::DiscoverGame);
        result["status"] = json!(status);
        responses.push(json!({"name":format!("discovery-open-status-{status}"),"method":"discover_game","response":{"protocol_version":4,"request_id":"fixture","ok":true,"result":result}}));
    }
    for status in [
        "cached",
        "not_refreshed",
        "source_refreshed",
        "source_not_modified",
        "source_partial",
        "source_offline_cached",
        "source_refresh_failed",
    ] {
        let mut source = source_summary();
        source["status"] = json!(status);
        responses.push(json!({"name":format!("source-open-status-{status}"),"method":"list_sources","response":{"protocol_version":4,"request_id":"fixture","ok":true,"result":[source]}}));
    }
    for (name, code, field) in [
        (
            "capabilities-missing-contracts",
            "capability_mismatch",
            "contracts",
        ),
        (
            "capabilities-missing-methods",
            "capability_mismatch",
            "methods",
        ),
    ] {
        let mut v = capabilities();
        v.as_object_mut().unwrap().remove(field);
        responses.push(json!({"name":name,"method":"get_capabilities","response":{"protocol_version":4,"request_id":"fixture","ok":true,"result":v},"error":code}));
    }
    let mut v = capabilities();
    v["installer"]["schema_version"] = json!(2);
    responses.push(json!({"name":"capabilities-installer-schema","method":"get_capabilities","response":{"protocol_version":4,"request_id":"fixture","ok":true,"result":v},"error":"installer_schema_unsupported"}));
    for name in [
        "capabilities-format-unsupported",
        "capabilities-strategies-empty",
        "capabilities-link-inconsistent",
    ] {
        let mut v = capabilities();
        match name {
            "capabilities-format-unsupported" => {
                v["installer"]["formats"]["exe"] = json!("unsupported")
            }
            "capabilities-strategies-empty" => v["installer"]["strategies"] = json!([]),
            _ => v["installer"]["links"]["zip"]["blockers"] = json!([{"code":"installer_busy"}]),
        }
        responses.push(json!({"name":name,"method":"get_capabilities","response":{"protocol_version":4,"request_id":"fixture","ok":true,"result":v},"error":"capability_mismatch"}));
    }
    let library = format!("E:/{}", "a".repeat(100));
    let prefix = "E:/steamapps/common/";
    let install = format!("{prefix}{}", "g".repeat(171 - prefix.len()));
    let identity = format!("42|{library}|{install}");
    assert_eq!(identity.len(), 278);
    let mut r = request(Method::Preview);
    r["params"]["instance_id"] = json!(identity);
    requests.push(json!({"name":"long-steam-instance-preview","request":r,"task_channel":false}));
    let mut r = request(Method::Preview);
    r["params"]["instance_id"] = json!("i".repeat(MAX_INSTANCE_ID_BYTES));
    requests.push(json!({"name":"instance-id-exact-16k","request":r,"task_channel":false}));
    let mut r = request(Method::Preview);
    r["params"]["instance_id"] = json!("i".repeat(MAX_INSTANCE_ID_BYTES + 1));
    requests.push(json!({"name":"instance-id-over-16k","request":r,"task_channel":false,"error":"invalid_id"}));
    let mut v = scan();
    v["installs"][0]["game"]["instance_id"] = json!(identity);
    v["installs"][0]["game"]["library_path"] = json!(library);
    v["installs"][0]["game"]["install_path"] = json!(install);
    responses.push(json!({"name":"long-steam-instance-scan","method":"scan","response":{"protocol_version":4,"request_id":"fixture","ok":true,"result":v}}));
    let mut v = transaction("committed");
    v["instance_id"] = json!(identity);
    responses.push(json!({"name":"long-steam-instance-history","method":"list_transactions","response":{"protocol_version":4,"request_id":"fixture","ok":true,"result":[v]}}));
    let mut v = outcome("needs_recovery", "failed");
    v["recovery"] = json!({"required":true,"code":null,"detail":null});
    responses.push(json!({"name":"recovery-null-code","method":"install_patch","response":{"protocol_version":4,"request_id":"fixture","ok":true,"result":v}}));
    let mut v = success_result(Method::Preview);
    v["instance_id"] = json!(identity);
    responses.push(json!({"name":"long-steam-instance-preview-result","method":"preview","response":{"protocol_version":4,"request_id":"fixture","ok":true,"result":v}}));
    let mut v = scan();
    v["installs"][0]["game"]["instance_id"] = json!("i".repeat(MAX_INSTANCE_ID_BYTES + 1));
    responses.push(json!({"name":"instance-output-over-16k","method":"scan","response":{"protocol_version":4,"request_id":"fixture","ok":true,"result":v},"error":"invalid_core_response"}));
    let mut e = install_event();
    e["instance_id"] = json!(identity);
    events.push(json!({"name":"long-steam-instance-event","event":e}));
    let mut e = install_event();
    e["instance_id"] = json!("i".repeat(MAX_INSTANCE_ID_BYTES + 1));
    events
        .push(json!({"name":"instance-event-over-16k","event":e,"error":"invalid_core_response"}));
    let mut v = success_result(Method::Preview);
    v["executable"] = json!(true);
    responses.push(json!({"name":"preview-never-executable","method":"preview","response":{"protocol_version":4,"request_id":"fixture","ok":true,"result":v},"error":"invalid_core_response"}));
    for (name, error, expected) in [
        (
            "outcome-error-exact-2048",
            json!("界".repeat(682) + "ab"),
            None,
        ),
        (
            "outcome-error-over-2048",
            json!("界".repeat(683)),
            Some("invalid_core_response"),
        ),
        ("outcome-error-null", serde_json::Value::Null, None),
    ] {
        let mut result = outcome("rolled_back", "failed");
        result["error"] = error;
        responses.push(json!({"name":name,"method":"install_patch","response":{"protocol_version":4,"request_id":"fixture","ok":true,"result":result},"error":expected}));
    }
    let mut history = transaction("rolled_back");
    history["error"] = json!(format!("journal_error:{}", "界".repeat(1000)));
    responses.push(json!({"name":"history-error-preserved-over-2048","method":"list_transactions","response":{"protocol_version":4,"request_id":"fixture","ok":true,"result":[history]}}));
    responses.push(json!({"name":"early-error-null-request-id","method":"scan","response":{"protocol_version":4,"request_id":null,"ok":false,"error":{"code":"invalid_request"}},"schema_valid":true,"error":"invalid_core_response"}));
    let mut early_errors = Vec::new();
    for (name, version, id, code) in [
        (
            "old-version-preserves-valid-id",
            3,
            json!("fixture"),
            "unsupported_protocol",
        ),
        (
            "old-version-null-id",
            3,
            Value::Null,
            "unsupported_protocol",
        ),
        (
            "old-version-invalid-id",
            3,
            json!("bad\u{0085}id"),
            "unsupported_protocol",
        ),
        (
            "current-version-null-id",
            4,
            Value::Null,
            "invalid_request_id",
        ),
    ] {
        let request =
            json!({"protocol_version":version,"request_id":id,"method":"scan","params":false});
        let response_id = if name == "old-version-preserves-valid-id" {
            json!("fixture")
        } else {
            Value::Null
        };
        early_errors.push(json!({"name":name,"json":request.to_string(),"task_channel":false,"response":{"protocol_version":4,"request_id":response_id,"ok":false,"error":{"code":code}}}));
    }
    for (name, raw) in [
        ("malformed-json", "{"),
        (
            "duplicate-key",
            "{\"protocol_version\":4,\"protocol_version\":3}",
        ),
        (
            "current-version-missing-id",
            "{\"protocol_version\":4,\"method\":\"scan\",\"params\":false}",
        ),
    ] {
        let code = if name == "current-version-missing-id" {
            "invalid_request_id"
        } else {
            "invalid_request"
        };
        early_errors.push(json!({"name":name,"json":raw,"task_channel":false,"response":{"protocol_version":4,"request_id":null,"ok":false,"error":{"code":code}}}));
    }
    early_errors.push(json!({"name":"invalid-utf8","hex":"ff","task_channel":false,"response":{"protocol_version":4,"request_id":null,"ok":false,"error":{"code":"invalid_request"}}}));
    let mut e1 = install_event();
    e1["sequence"] = json!(1);
    let mut e2 = install_event();
    e2["sequence"] = json!(2);
    json!({"early_errors":early_errors,"raw_requests":[{"name":"duplicate-key","json":"{\"protocol_version\":4,\"protocol_version\":3}","error":"invalid_request"},{"name":"invalid-utf8","hex":"ff","error":"invalid_request"}],"requests":requests,"responses":responses,"events":events,"sequences":[{"name":"shared-monotonic","events":[install_event(),e1.clone(),install_event(),e2],"expected":["accept","accept","drop","accept"]},{"name":"illegal-first","events":[e1],"expected":["error"]},{"name":"after-terminal","events":[install_event(),install_event()],"terminal_after":1,"expected":["accept","drop"]}],"boundaries":[{"name":"request-exact","kind":"request","bytes":MAX_REQUEST_BYTES,"accepted":true},{"name":"request-plus-one","kind":"request","bytes":MAX_REQUEST_BYTES+1,"accepted":false},{"name":"response-exact","kind":"response","bytes":MAX_RESPONSE_BYTES,"accepted":true},{"name":"response-plus-one","kind":"response","bytes":MAX_RESPONSE_BYTES+1,"accepted":false}],"details":[{"input":"界".repeat(683),"expected":"界".repeat(682)}]})
}
