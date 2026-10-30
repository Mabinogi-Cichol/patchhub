//! Fixed cross-language acceptance corpus generated from the Rust contract.
use crate::*;
use serde_json::{json, Value};
pub fn capabilities() -> Value {
    json!({"protocol_version":5,"methods":METHODS,"contracts":ContractVersions::default(),"installer":{"schema_version":1,"selection_version":1,"content_version":1,"adapter_version":1,"policy_schema_version":1,"report_schema_version":1,"formats":{"zip":"supported","exe":"supported"},"strategies":["managed_zip","publisher_zip","publisher_exe"],"links":{"zip":{"available":true,"blockers":[]},"exe":{"available":true,"blockers":[]}}},"sources":{"index_schema_version":1,"detail_schema_version":1,"match_schema_version":1,"schedule_schema_version":1,"adapter_version":1,"parser_version":patchhub_core::sources::PARSER_VERSION,"encoding_version":1,"script_api_version":1,"methods":SOURCE_METHODS}})
}
pub fn host() -> Value {
    json!({"roots":[],"catalog":null,"signed_catalog":null,"trust_keys":null,"state_dir":null,"platform":"windows"})
}
pub fn candidate() -> Value {
    json!({"candidate_id":"candidate-0123456789abcdef01234567","source_id":"fixture","source_record_id":"https://example.com/game","game_appid":42,"parent_appid":null,"aliases":[],"game_title":"Fixture","patch_title":"Patch","variant":"base","requires_dlc":null,"excludes_dlc":null,"platforms":["windows"],"patch_kind":"adult_content","languages":["zh_cn"],"version_raw":null,"version_evidence":[],"version_identity":"fixture","published_at":null,"patch_updated_at":null,"source_page":"https://example.com/game","upstream_page":null,"mirrors":[],"format_reported":"zip","format_observed":"unknown","capability":"discover_only","identity_evidence":[],"provenance":[],"match_basis":"appid","observed_at":"2026-10-11T00:00:00Z","cache_status":"fresh","warnings":[],"fingerprint":"fixture"})
}
pub fn discovery() -> Value {
    json!({"status":"pending","candidates":[],"diagnostics":[]})
}
pub fn game_match() -> Value {
    json!({"appid":42,"instance_id":"instance","status":"publisher_match","candidates":[candidate()],"diagnostics":[],"source_revisions":{"fixture":1}})
}
pub fn scan() -> Value {
    let available = json!({"available":true,"blockers":[]});
    let unavailable =
        json!({"available":false,"blockers":[{"code":"reinstall_unowned_or_incomplete"}]});
    json!({"catalog_revision":1,"catalog_trust":"signed","catalog_error":null,"platform":"windows","roots":[],"steam_languages":{},"installs":[{"game":{"instance_id":"instance","appid":42,"name":"Fixture","localized_name":null,"name_language":null,"library_path":"fixture/library","install_path":"fixture/library/steamapps/common/game","buildid":null,"availability":"installed","kind":"game"},"match_status":"matched","patches":[{"release_id":"release","name":"Patch","source_page":"https://example.com/game","support_level":"managed_zip","actions":{"automatic":available,"local_archive":available,"reinstall":unavailable,"overwrite":available,"primary":{"operation":"overwrite","label":"install","input":"automatic"}},"compatibility":"verified","blockers":[],"detection":{"status":"not_detected","files":[{"path":"patch.dat","status":"missing","actual_sha256":null}]}}],"discovery":discovery(),"diagnostics":[]}],"diagnostics":[]})
}
pub fn transaction(state: &str) -> Value {
    json!({"transaction_id":"transaction","plan_id":"plan","instance_id":"instance","release_id":"release","state":state,"error":null,"created_at":1,"updated_at":1,"has_replacements":false,"selection_id":null,"content_id":null,"direct":false})
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
    v["cleanup"] = json!(patchhub_core::installer::outcome::CleanupResult::default());
    v
}
pub fn rule() -> Value {
    json!({"id":"fixture","name":"Fixture","adapter":"table_html","enabled":true,"pages":["https://example.com/"],"page_hosts":["example.com"],"download_hosts":[],"role":"publisher","discovery_only":false,"priority":100,"root_layout":null})
}
pub fn source_status() -> Value {
    json!({"checked_at":1,"cached_at":null,"entry_count":0,"code":"source_refreshed"})
}
pub fn source_summary() -> Value {
    let mut v = rule();
    v["origin"] = json!("builtin");
    v["status"] = json!("source_refreshed");
    v["entry_count"] = json!(1);
    v["cached_at"] = Value::Null;
    v["checked_at"] = json!(1);
    v["revision"] = json!(1);
    v["complete"] = json!(true);
    v["interval_hours"] = json!(24);
    v["next_due_at"] = Value::Null;
    v
}
fn source_update_result() -> Value {
    json!({"source_id":"fixture","settings_revision":1,"status":source_status(),"revision":1,"complete":true,"affected_appids":[42]})
}
pub fn success_result(method: Method) -> Value {
    match method {
        Method::GetCapabilities => capabilities(),
        Method::Scan => scan(),
        Method::ProjectInstances => {
            json!({"settings_revision":1,"instances":[scan()["installs"][0].clone()]})
        }
        Method::Preview => {
            json!({"instance_id":"instance","release_id":"release","selection_id":null,"catalog_revision":1,"executable":false,"blockers":[],"changes":[{"path":"patch.dat","action":"replace","baseline_matches":null,"actual_sha256":null}],"diagnostics":[]})
        }
        Method::InstallPatch => outcome("committed", "completed"),
        Method::ListTransactions | Method::Recover => json!([transaction("committed")]),
        Method::DiscardPlan => transaction("rolled_back"),
        Method::CacheStatus => json!({"total_bytes":0,"entries":0,"diagnostics":[]}),
        Method::BackupStatus => json!({"total_bytes":0,"files":0,"failures":[]}),
        Method::CleanupBackups => {
            json!({"status":"completed","removed_bytes":0,"removed_files":0,"failures":[]})
        }
        Method::CleanupCache => {
            json!(patchhub_core::installer::outcome::CleanupResult::default())
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
        Method::SetSourceName => {
            json!({"settings_revision":1,"source_id":"fixture","name":"发行商名称"})
        }
        Method::SetGameSource => {
            json!({"settings_revision":1,"source_id":"fixture","subscribed":true})
        }
        Method::AddSource | Method::ImportScriptSource => {
            json!({"settings_revision":1,"rule":rule()})
        }
        Method::GetScriptSource => {
            json!({"rule":rule(),"api_version":1,"code":"return {}","sha256":"a".repeat(64)})
        }
        Method::SetSourceUpdateInterval => {
            json!({"settings_revision":1,"source_id":"fixture","interval_hours":24})
        }
        Method::UpdateSource => source_update_result(),
        Method::MatchGames | Method::RematchGame => {
            json!({"settings_revision":1,"source_revisions":{"fixture":1},"results":[game_match()]})
        }
    }
}
pub fn request(method: Method) -> Value {
    let mut p = json!({"host":host()});
    match method {
        Method::ProjectInstances => p["instance_ids"] = json!(["instance"]),
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
        Method::SetSourceName => {
            p["source_id"] = json!("fixture");
            p["name"] = json!("发行商名称");
        }
        Method::SetGameSource => {
            p["source_id"] = json!("fixture");
            p["subscribed"] = json!(true);
            p["appid"] = json!(42);
        }
        Method::AddSource => p["rule"] = rule(),
        Method::ImportScriptSource => {
            p["rule"] = rule();
            p["api_version"] = json!(1);
            p["code"] = json!("return {}");
        }
        Method::GetScriptSource => p["source_id"] = json!("fixture"),
        Method::SetSourceUpdateInterval => {
            p["source_id"] = json!("fixture");
            p["interval_hours"] = json!(24);
        }
        Method::UpdateSource => {
            p["source_id"] = json!("fixture");
            p["force"] = json!(true);
        }
        Method::MatchGames => {
            p["instances"] = json!(["instance"]);
            p["name_hints"] = json!(["Hint"]);

            p["force_details"] = json!(true);
        }
        Method::RematchGame => p["instance_id"] = json!("instance"),
        _ => {}
    }
    json!({"protocol_version":5,"request_id":"fixture","method":method,"params":p})
}
pub fn install_event() -> Value {
    json!({"event":"install_progress","protocol_version":5,"request_id":"fixture","sequence":0,"instance_id":"instance","selection_id":null,"stage":"applying","substage":"writing","bytes_done":1,"total_bytes":null,"files_done":0,"total_files":null,"current_file":null,"archive_bytes":1})
}
pub fn source_progress_event() -> Value {
    json!({"event":"source_progress","protocol_version":5,"request_id":"fixture","sequence":0,"source_id":"fixture","phase":"index","completed":0,"total":1,"running":true,"settings_revision":1})
}
pub fn source_update_event() -> Value {
    json!({"event":"source_update","protocol_version":5,"request_id":"fixture","sequence":0,"source_id":"fixture","settings_revision":1,"status":source_status()})
}
pub fn match_progress_event() -> Value {
    json!({"event":"match_progress","protocol_version":5,"request_id":"fixture","sequence":0,"phase":"inventory","completed":0,"total":1,"running":true,"settings_revision":1})
}
pub fn match_update_event() -> Value {
    json!({"event":"match_update","protocol_version":5,"request_id":"fixture","sequence":0,"instance_id":"instance","appid":42,"status":"publisher_match","candidates":[candidate()],"diagnostics":[],"settings_revision":1})
}
pub fn cases() -> Value {
    let mut requests = Vec::new();
    let mut responses = Vec::new();
    let mut events = Vec::new();
    for method in ALL_METHODS {
        requests.push(json!({"name":format!("{}-request",method.as_str()),"request":request(method),"task_channel":method.is_task()}));
        let success = json!({"protocol_version":5,"request_id":"fixture","ok":true,"result":success_result(method)});
        responses.push(json!({"name":format!("{}-success",method.as_str()),"method":method,"response":success}));
        responses.push(json!({"name":format!("{}-failure",method.as_str()),"method":method,"response":{"protocol_version":5,"request_id":"fixture","ok":false,"error":{"code":"io_error","detail":"固定诊断"}}}));
    }
    fn bad(requests: &mut Vec<Value>, name: &str, mut r: Value, error: &str) {
        requests.push(json!({"name":name,"request":r.take(),"task_channel":true,"error":error}));
    }
    let large_code = format!("-- {}\nreturn {{}}", "来源脚本 ".repeat(6000));
    for (index, code) in [
        "".to_string(),
        "Upper".to_string(),
        "a".repeat(129),
        "invalid\n".to_string(),
    ]
    .into_iter()
    .enumerate()
    {
        responses.push(json!({"name":format!("error-code-invalid-{index}"),"method":"get_script_source","response":{"protocol_version":5,"request_id":"fixture","ok":false,"error":{"code":code}},"error":"invalid_core_response"}));
    }
    let mut r = request(Method::ImportScriptSource);
    r["params"]["code"] = json!(large_code);
    r["params"]["replace"] = json!(true);
    requests.push(json!({"name":"script-import-large-replace","request":r,"task_channel":false}));
    let mut document = success_result(Method::GetScriptSource);
    document["code"] = json!(large_code);
    responses.push(json!({"name":"script-document-large","method":"get_script_source","response":{"protocol_version":5,"request_id":"fixture","ok":true,"result":document}}));
    for (field, value) in [
        ("code", json!(false)),
        ("api_version", json!(true)),
        ("replace", json!("true")),
        ("path", json!("adapter.lua")),
    ] {
        let mut r = request(Method::ImportScriptSource);
        r["params"][field] = value;
        bad(
            &mut requests,
            &format!("script-import-invalid-{field}"),
            r,
            "invalid_request",
        );
    }
    for version in [None, Some(2)] {
        let mut v = capabilities();
        if let Some(version) = version {
            v["sources"]["script_api_version"] = json!(version);
        } else {
            v["sources"]
                .as_object_mut()
                .unwrap()
                .remove("script_api_version");
        }
        responses.push(json!({"name":format!("capabilities-script-api-{version:?}"),"method":"get_capabilities","response":{"protocol_version":5,"request_id":"fixture","ok":true,"result":v},"error":if version.is_none() {"capability_mismatch"} else {"installer_schema_unsupported"}}));
    }
    let mut r = request(Method::Scan);
    r["protocol_version"] = json!(4);
    r["params"] = json!(false);
    bad(&mut requests, "v4-before-params", r, "unsupported_protocol");
    let mut r = request(Method::Scan);
    r["method"] = json!("plan_install");
    r["params"] = json!(false);
    bad(
        &mut requests,
        "unknown-method-before-params",
        r,
        "unsupported_method",
    );
    for deleted in ["refresh_sources", "scan_sources", "discover_game"] {
        requests.push(json!({"name":format!("deleted-{deleted}"),"request":{"protocol_version":5,"request_id":"fixture","method":deleted,"params":{"host":host()}},"task_channel":true,"error":"unsupported_method"}));
    }
    let mut r = request(Method::Scan);
    r["params"]["operation"] = json!("overwrite");
    bad(
        &mut requests,
        "method-specific-unknown",
        r,
        "invalid_request",
    );
    let mut r = request(Method::Scan);
    r["params"]["host"]
        .as_object_mut()
        .unwrap()
        .remove("catalog");
    bad(&mut requests, "required-null-host", r, "invalid_request");
    let mut r = request(Method::InstallPatch);
    r["params"].as_object_mut().unwrap().remove("operation");
    bad(&mut requests, "required-operation", r, "invalid_request");
    let mut r = request(Method::InstallPatch);
    r["params"]
        .as_object_mut()
        .unwrap()
        .remove("cleanup_policy");
    bad(&mut requests, "required-policy", r, "invalid_request");
    let mut r = request(Method::InstallPatch);
    r["params"]["request_id"] = json!("inner");
    bad(&mut requests, "outer-id-only", r, "invalid_request");
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
    let mut r = request(Method::RematchGame);
    r["params"].as_object_mut().unwrap().remove("instance_id");
    bad(
        &mut requests,
        "rematch-game-target-required",
        r,
        "instance_required",
    );
    let mut r = request(Method::ProjectInstances);
    r["params"]["instance_ids"] = json!(["bad\u{0085}id"]);
    bad(
        &mut requests,
        "project-instances-invalid-id",
        r,
        "invalid_id",
    );
    let mut r = request(Method::ProjectInstances);
    r["params"]["instance_ids"] = json!([MAX_SAFE_INTEGER + 1]);
    bad(
        &mut requests,
        "project-instances-non-string",
        r,
        "invalid_request",
    );
    let mut r = request(Method::MatchGames);
    r["params"]["instances"] = json!(["i".repeat(MAX_INSTANCE_ID_BYTES + 1)]);
    bad(
        &mut requests,
        "match-games-instance-over-16k",
        r,
        "invalid_id",
    );
    for method in [Method::MatchGames, Method::RematchGame] {
        let mut r = request(method);
        r["params"]["preferred_language"] = json!("zh-CN");
        bad(
            &mut requests,
            &format!("{}-removed-language-field", method.as_str()),
            r,
            "invalid_request",
        );
    }
    let mut r = request(Method::SetSourceUpdateInterval);
    r["params"]["interval_hours"] = json!(MAX_SAFE_INTEGER + 1);
    bad(&mut requests, "interval-unsafe", r, "invalid_request");
    let mut r = request(Method::SetSourceUpdateInterval);
    r["params"]["interval_hours"] = json!(MAX_SAFE_INTEGER);
    requests.push(json!({"name":"interval-safe","request":r,"task_channel":false}));
    let event_values = [
        ("install_progress", install_event()),
        ("source_progress", source_progress_event()),
        ("source_update", source_update_event()),
        ("match_progress", match_progress_event()),
        ("match_update", match_update_event()),
    ];
    for (name, event) in event_values {
        events.push(json!({"name":name,"event":event}));
    }
    for (name, field, value, error) in [
        (
            "v4-event",
            "protocol_version",
            json!(4),
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
    for (name, event) in [
        (
            "unknown-event",
            json!({"event":"graph_progress","protocol_version":5,"request_id":"fixture","sequence":0}),
        ),
        ("source-progress-missing-source", {
            let mut e = source_progress_event();
            e.as_object_mut().unwrap().remove("source_id");
            e
        }),
        ("source-progress-missing-phase", {
            let mut e = source_progress_event();
            e.as_object_mut().unwrap().remove("phase");
            e
        }),
        ("match-update-missing-instance", {
            let mut e = match_update_event();
            e.as_object_mut().unwrap().remove("instance_id");
            e
        }),
    ] {
        events.push(json!({"name":name,"event":event,"error":"invalid_core_response"}));
    }
    for (state, disposition) in [
        ("rolled_back", "cancelled"),
        ("rolled_back", "failed"),
        ("needs_recovery", "failed"),
        ("failed", "cancelled"),
        ("failed", "failed"),
    ] {
        responses.push(json!({"name":format!("outcome-{state}-{disposition}"),"method":"install_patch","response":{"protocol_version":5,"request_id":"fixture","ok":true,"result":outcome(state,disposition)}}));
    }
    let v = json!({"protocol_version":5,"request_id":"fixture","ok":false,"error":{"code":"io_error","detail":"界".repeat(683)}});
    responses.push(json!({"name":"detail-beyond-2048-bytes","method":"scan","response":v}));
    for version in [1, 2, 4, 999] {
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
        ("v4-response", "protocol_version", json!(4)),
        ("wrong-response-id", "request_id", json!("other")),
    ] {
        let mut v = json!({"protocol_version":5,"request_id":"fixture","ok":true,"result":scan()});
        v[field] = value;
        responses.push(json!({"name":name,"method":"scan","response":v,"error":if field=="protocol_version"{"unsupported_protocol"}else{"invalid_core_response"}}));
    }
    for name in ["cause", "recovery", "cleanup", "selection_id"] {
        let mut v = outcome("committed", "completed");
        v.as_object_mut().unwrap().remove(name);
        responses.push(json!({"name":format!("missing-outcome-{name}"),"method":"install_patch","response":{"protocol_version":5,"request_id":"fixture","ok":true,"result":v},"error":"invalid_core_response"}));
    }
    let mut v = scan();
    v["installs"][0]["patches"][0]
        .as_object_mut()
        .unwrap()
        .remove("actions");
    responses.push(json!({"name":"missing-actions","method":"scan","response":{"protocol_version":5,"request_id":"fixture","ok":true,"result":v},"error":"invalid_core_response"}));
    for (name, state, disposition) in [
        ("commit-cannot-cancel", "committed", "cancelled"),
        ("recovery-cannot-complete", "needs_recovery", "completed"),
    ] {
        responses.push(json!({"name":name,"method":"install_patch","response":{"protocol_version":5,"request_id":"fixture","ok":true,"result":outcome(state,disposition)},"error":"invalid_core_response"}));
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
    for status in [
        "pending",
        "publisher_match",
        "fallback_match",
        "no_match",
        "incomplete",
        "failed",
    ] {
        let mut result = success_result(Method::MatchGames);
        result["results"][0]["status"] = json!(status);
        responses.push(json!({"name":format!("match-open-status-{status}"),"method":"match_games","response":{"protocol_version":5,"request_id":"fixture","ok":true,"result":result}}));
    }
    let mut result = success_result(Method::MatchGames);
    result["results"][0]["status"] = json!("complete");
    responses.push(json!({"name":"match-status-unknown","method":"match_games","response":{"protocol_version":5,"request_id":"fixture","ok":true,"result":result},"error":"invalid_core_response"}));
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
        responses.push(json!({"name":format!("source-open-status-{status}"),"method":"list_sources","response":{"protocol_version":5,"request_id":"fixture","ok":true,"result":[source]}}));
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
        responses.push(json!({"name":name,"method":"get_capabilities","response":{"protocol_version":5,"request_id":"fixture","ok":true,"result":v},"error":code}));
    }
    let mut v = capabilities();
    v["installer"]["schema_version"] = json!(2);
    responses.push(json!({"name":"capabilities-installer-schema","method":"get_capabilities","response":{"protocol_version":5,"request_id":"fixture","ok":true,"result":v},"error":"installer_schema_unsupported"}));
    let mut v = capabilities();
    v["sources"]["index_schema_version"] = json!(2);
    responses.push(json!({"name":"capabilities-source-schema","method":"get_capabilities","response":{"protocol_version":5,"request_id":"fixture","ok":true,"result":v},"error":"installer_schema_unsupported"}));
    let mut v = capabilities();
    v["sources"]["parser_version"] = json!(1);
    responses.push(json!({"name":"capabilities-source-parser","method":"get_capabilities","response":{"protocol_version":5,"request_id":"fixture","ok":true,"result":v},"error":"installer_schema_unsupported"}));
    let mut v = capabilities();
    v["sources"]["methods"] = json!(SOURCE_METHODS[..8]);
    responses.push(json!({"name":"capabilities-source-methods","method":"get_capabilities","response":{"protocol_version":5,"request_id":"fixture","ok":true,"result":v},"error":"capability_mismatch"}));
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
        responses.push(json!({"name":name,"method":"get_capabilities","response":{"protocol_version":5,"request_id":"fixture","ok":true,"result":v},"error":"capability_mismatch"}));
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
    responses.push(json!({"name":"long-steam-instance-scan","method":"scan","response":{"protocol_version":5,"request_id":"fixture","ok":true,"result":v}}));
    let mut v = transaction("committed");
    v["instance_id"] = json!(identity);
    responses.push(json!({"name":"long-steam-instance-history","method":"list_transactions","response":{"protocol_version":5,"request_id":"fixture","ok":true,"result":[v]}}));
    let mut v = outcome("needs_recovery", "failed");
    v["recovery"] = json!({"required":true,"code":null,"detail":null});
    responses.push(json!({"name":"recovery-null-code","method":"install_patch","response":{"protocol_version":5,"request_id":"fixture","ok":true,"result":v}}));
    let mut v = success_result(Method::Preview);
    v["instance_id"] = json!(identity);
    responses.push(json!({"name":"long-steam-instance-preview-result","method":"preview","response":{"protocol_version":5,"request_id":"fixture","ok":true,"result":v}}));
    let mut v = scan();
    v["installs"][0]["game"]["instance_id"] = json!("i".repeat(MAX_INSTANCE_ID_BYTES + 1));
    responses.push(json!({"name":"instance-output-over-16k","method":"scan","response":{"protocol_version":5,"request_id":"fixture","ok":true,"result":v},"error":"invalid_core_response"}));
    let mut e = install_event();
    e["instance_id"] = json!(identity);
    events.push(json!({"name":"long-steam-instance-event","event":e}));
    let mut e = install_event();
    e["instance_id"] = json!("i".repeat(MAX_INSTANCE_ID_BYTES + 1));
    events
        .push(json!({"name":"instance-event-over-16k","event":e,"error":"invalid_core_response"}));
    let mut e = match_update_event();
    e["instance_id"] = json!(identity);
    events.push(json!({"name":"long-steam-instance-match-event","event":e}));
    let mut v = success_result(Method::Preview);
    v["executable"] = json!(true);
    responses.push(json!({"name":"preview-never-executable","method":"preview","response":{"protocol_version":5,"request_id":"fixture","ok":true,"result":v},"error":"invalid_core_response"}));
    let result = outcome("rolled_back", "failed");
    responses.push(json!({"name":"outcome-error-null","method":"install_patch","response":{"protocol_version":5,"request_id":"fixture","ok":true,"result":result}}));
    let mut history = transaction("rolled_back");
    history["error"] = json!(format!("journal_error:{}", "界".repeat(1000)));
    responses.push(json!({"name":"history-error-preserved-over-2048","method":"list_transactions","response":{"protocol_version":5,"request_id":"fixture","ok":true,"result":[history]}}));
    responses.push(json!({"name":"early-error-null-request-id","method":"scan","response":{"protocol_version":5,"request_id":null,"ok":false,"error":{"code":"invalid_request"}},"schema_valid":true,"error":"invalid_core_response"}));
    let mut early_errors = Vec::new();
    for (name, version, id, code) in [
        (
            "old-version-preserves-valid-id",
            4,
            json!("fixture"),
            "unsupported_protocol",
        ),
        (
            "old-version-null-id",
            4,
            Value::Null,
            "unsupported_protocol",
        ),
        (
            "old-version-invalid-id",
            4,
            json!("bad\u{0085}id"),
            "unsupported_protocol",
        ),
        (
            "current-version-null-id",
            5,
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
        early_errors.push(json!({"name":name,"json":request.to_string(),"task_channel":false,"response":{"protocol_version":5,"request_id":response_id,"ok":false,"error":{"code":code}}}));
    }
    for (name, raw) in [
        ("malformed-json", "{"),
        (
            "duplicate-key",
            "{\"protocol_version\":5,\"protocol_version\":4}",
        ),
        (
            "current-version-missing-id",
            "{\"protocol_version\":5,\"method\":\"scan\",\"params\":false}",
        ),
    ] {
        let code = if name == "current-version-missing-id" {
            "invalid_request_id"
        } else {
            "invalid_request"
        };
        early_errors.push(json!({"name":name,"json":raw,"task_channel":false,"response":{"protocol_version":5,"request_id":null,"ok":false,"error":{"code":code}}}));
    }
    early_errors.push(json!({"name":"invalid-utf8","hex":"ff","task_channel":false,"response":{"protocol_version":5,"request_id":null,"ok":false,"error":{"code":"invalid_request"}}}));
    let mut manual = request(Method::InstallPatch);
    manual["params"]
        .as_object_mut()
        .unwrap()
        .remove("release_id");
    manual["params"]["manual_download"] = json!({"source_id":"fixture","candidate_id":"candidate-0123456789abcdef01234567","url":"https://user-download.example/download?id=42"});
    requests.push(json!({"name":"manual-download","request":manual.clone(),"task_channel":true}));
    for (field, value) in [
        ("release_id", json!("release")),
        (
            "selection_id",
            json!("zip-selection-0123456789abcdef01234567-42"),
        ),
        ("archive", json!("local.zip")),
    ] {
        let mut invalid = manual.clone();
        invalid["params"][field] = value;
        requests.push(json!({"name":format!("manual-download-conflict-{field}"),"request":invalid,"task_channel":true,"error":"invalid_request"}));
    }
    let mut invalid = manual.clone();
    invalid["params"]["manual_download"]["url"] = json!("file:///patch.zip");
    requests.push(json!({"name":"manual-download-non-network","request":invalid,"task_channel":true,"error":"invalid_download_url"}));
    let mut e1 = install_event();
    e1["sequence"] = json!(1);
    let mut e2 = install_event();
    e2["sequence"] = json!(2);
    json!({"early_errors":early_errors,"raw_requests":[{"name":"duplicate-key","json":"{\"protocol_version\":5,\"protocol_version\":4}","error":"invalid_request"},{"name":"invalid-utf8","hex":"ff","error":"invalid_request"}],"requests":requests,"responses":responses,"events":events,"sequences":[{"name":"shared-monotonic","events":[install_event(),e1.clone(),install_event(),e2],"expected":["accept","accept","drop","accept"]},{"name":"illegal-first","events":[e1],"expected":["error"]},{"name":"after-terminal","events":[install_event(),install_event()],"terminal_after":1,"expected":["accept","drop"]}]})
}
