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
    for c in cases["details"].as_array().unwrap() {
        assert_eq!(
            truncate_detail(c["input"].as_str().unwrap()),
            c["expected"].as_str().unwrap()
        );
    }
}
#[test]
fn complete_line_limits_and_recoverability() {
    let value = fixtures::request(Method::Scan);
    let mut request = serde_json::to_vec(&value).unwrap();
    request.resize(MAX_REQUEST_BYTES - 1, b' ');
    request.push(b'\n');
    assert!(decode_request(&request, false).is_ok());
    request.insert(request.len() - 1, b' ');
    assert_eq!(
        decode_request(&request, false).unwrap_err().code,
        "request_too_large"
    );
    assert!(decode_request(&serde_json::to_vec(&value).unwrap(), false).is_ok());
    let overhead = serde_json::to_vec(&json!({"padding":""})).unwrap().len() + 1;
    let exact = json!({"padding":"x".repeat(MAX_RESPONSE_BYTES-overhead)});
    assert_eq!(bounded_json_line(&exact).unwrap().len(), MAX_RESPONSE_BYTES);
    let over = json!({"padding":"x".repeat(MAX_RESPONSE_BYTES-overhead+1)});
    assert_eq!(
        bounded_json_line(&over).unwrap_err().code,
        "response_too_large"
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
