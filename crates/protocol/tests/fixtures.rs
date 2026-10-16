use serde_json::{json, Value};
use steam_patch_protocol::*;
fn corpus() -> Value {
    serde_json::from_str(include_str!(
        "../../../packages/contracts/fixtures/v4/cases.json"
    ))
    .unwrap()
}
#[test]
fn fixed_cross_language_corpus() {
    let cases = corpus();
    for c in cases["raw_requests"].as_array().unwrap() {
        let b = if let Some(s) = c["json"].as_str() {
            s.as_bytes().to_vec()
        } else {
            vec![255]
        };
        assert_eq!(
            decode_request(&b, false).unwrap_err().code,
            c["error"].as_str().unwrap()
        );
    }
    for c in cases["requests"].as_array().unwrap() {
        let result = decode_request(
            &serde_json::to_vec(&c["request"]).unwrap(),
            c["task_channel"].as_bool().unwrap(),
        );
        let expected = c["error"].as_str();
        assert_eq!(
            result.err().map(|e| e.code),
            expected.map(str::to_owned),
            "request {}",
            c["name"]
        );
    }
    for c in cases["responses"].as_array().unwrap() {
        let method = serde_json::from_value(c["method"].clone()).unwrap();
        let result = validate_response(&c["response"], "fixture", Some(method));
        assert_eq!(
            result.err().map(|e| e.code),
            c["error"].as_str().map(str::to_owned),
            "response {}",
            c["name"]
        );
    }
    for c in cases["events"].as_array().unwrap() {
        let result = validate_event(&c["event"], "fixture");
        assert_eq!(
            result.err().map(|e| e.code),
            c["error"].as_str().map(str::to_owned),
            "event {}",
            c["name"]
        );
    }
    for c in cases["sequences"].as_array().unwrap() {
        let mut cursor = EventSequence::new("fixture".into());
        for (i, e) in c["events"].as_array().unwrap().iter().enumerate() {
            if c["terminal_after"].as_u64() == Some(i as u64) {
                cursor.finish();
            }
            let got = match cursor.accept(e) {
                Ok(true) => "accept",
                Ok(false) => "drop",
                Err(_) => "error",
            };
            assert_eq!(
                got,
                c["expected"][i].as_str().unwrap(),
                "sequence {}",
                c["name"]
            );
        }
    }
}
#[test]
fn large_request_and_response_payloads_are_not_size_rejected() {
    // A valid request beyond the removed 64 KiB line cap still parses.
    let mut request = fixtures::request(Method::DiscoverGame);
    request["params"]["name_hints"] = json!((0..40)
        .map(|i| format!("{i}-{}", "hint".repeat(2000)))
        .collect::<Vec<_>>());
    let bytes = serde_json::to_vec(&request).unwrap();
    assert!(bytes.len() > 64 * 1024);
    assert!(decode_request(&bytes, false).is_ok());
    // A long error detail is preserved instead of rejected or truncated.
    let detail = "界".repeat(5000);
    let response = json!({"protocol_version":4,"request_id":"fixture","ok":false,"error":{"code":"io_error","detail":detail}});
    validate_response(&response, "fixture", Some(Method::Scan)).unwrap();
    // A wrong field type still fails.
    let wrong_type = json!({"protocol_version":4,"request_id":"fixture","ok":false,"error":{"code":"io_error","detail":7}});
    assert_eq!(
        validate_response(&wrong_type, "fixture", Some(Method::Scan))
            .unwrap_err()
            .code,
        "invalid_core_response"
    );
}
#[test]
fn strict_method_params_and_nullable_facts() {
    for method in ALL_METHODS {
        let mut r = fixtures::request(method);
        r["params"]["injected"] = json!(true);
        assert_eq!(
            validate_request(&r, true).unwrap_err().code,
            "invalid_request"
        );
    }
    for key in [
        "roots",
        "catalog",
        "signed_catalog",
        "trust_keys",
        "state_dir",
        "platform",
    ] {
        let mut r = fixtures::request(Method::Scan);
        r["params"]["host"].as_object_mut().unwrap().remove(key);
        assert_eq!(
            validate_request(&r, false).unwrap_err().code,
            "invalid_request"
        );
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
        let mut e = fixtures::install_event();
        e.as_object_mut().unwrap().remove(key);
        assert_eq!(
            validate_event(&e, "fixture").unwrap_err().code,
            "invalid_core_response"
        );
    }
    let mut c = fixtures::capabilities();
    c.as_object_mut().unwrap().remove("contracts");
    assert_eq!(
        validate_capabilities(&c).unwrap_err().code,
        "capability_mismatch"
    );
    let mut c = fixtures::capabilities();
    c["installer"]["schema_version"] = json!(2);
    assert_eq!(
        validate_capabilities(&c).unwrap_err().code,
        "installer_schema_unsupported"
    );
}
