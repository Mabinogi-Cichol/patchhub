use patchhub_installer_format::graph::{decode, inventory};
use serde_json::json;

fn text(s: &str) -> Vec<u8> {
    let mut v = vec![s.len() as u8];
    v.extend_from_slice(s.as_bytes());
    v
}
fn declaration(name: &str) -> Vec<u8> {
    let mut v = vec![255, 255, 1, 0];
    v.extend_from_slice(&(name.len() as u16).to_le_bytes());
    v.extend_from_slice(name.as_bytes());
    v
}
fn event(name: &str, script: &str, parameters: &str) -> Vec<u8> {
    let mut v = 1u32.to_le_bytes().to_vec();
    for s in [parameters, name, script] {
        v.extend(text(s));
    }
    v
}
fn includes() -> Vec<u8> {
    let mut v = 2u16.to_le_bytes().to_vec();
    v.extend(declaration("CGlobalIncludeLuaFile"));
    for (index, name, script) in [
        (0u32, "Constants", "IDOK=1;"),
        (1, "helpers.lua", "function helper() end"),
    ] {
        if index > 0 {
            v.extend_from_slice(&0x8001u16.to_le_bytes());
        }
        v.extend_from_slice(&1u32.to_le_bytes());
        v.extend_from_slice(&index.to_le_bytes());
        v.extend(text(name));
        v.extend(text(script));
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend(text("All"));
    }
    v
}

#[test]
fn framed_parameterized_events_and_include_selectors_are_diagnostic_only() {
    let mut raw = includes();
    let include_end = raw.len();
    raw.extend(declaration("CIREvent"));
    raw.extend(event("On Next", "Screen.Next();", ""));
    raw.extend_from_slice(&0x8010u16.to_le_bytes());
    raw.extend(event(
        "On Install Error",
        "",
        "string e_ErrorMsg, number e_Code",
    ));
    let result = inventory(&raw).unwrap();
    assert_eq!(result["include_end"], include_end);
    assert_eq!(result["includes"][0]["source_kind"], 0);
    assert_eq!(result["includes"][1]["source_kind"], 1);
    assert_eq!(result["includes"][1]["script"], "function helper() end");
    assert_eq!(result["includes"][1]["selectors"], json!(["All"]));
    assert_eq!(result["events"][0]["name"], "On Next");
    assert_eq!(result["events"][1]["name"], "On Install Error");
    assert_eq!(
        result["events"][1]["parameters"],
        json!(["string e_ErrorMsg", "number e_Code"])
    );
    assert_eq!(
        result["coverage"],
        json!({"complete": false, "blockers": ["exe_serialized_object_graph_incomplete"]})
    );
    assert_eq!(result["installable"], false);
    assert_eq!(result["metadata"]["size"], raw.len());
    assert_eq!(result["diagnostics"], json!([]));
}

#[test]
fn malformed_references_keep_offsets_and_cannot_become_complete_coverage() {
    let mut raw = vec![0, 0];
    raw.extend(declaration("CIREvent"));
    raw.extend(event("On Next", "", ""));
    let offset = raw.len();
    raw.extend_from_slice(&[0x10, 0x80, 1, 0, 0, 0, 255]);
    let result = inventory(&raw).unwrap();
    assert_eq!(result["events"].as_array().unwrap().len(), 1);
    assert_eq!(
        result["diagnostics"],
        json!([{"code": "exe_event_reference_unresolved", "offset": offset}])
    );
    assert_eq!(result["coverage"]["complete"], false);
}

#[test]
fn false_class_markers_and_duplicate_classes_do_not_authorize_inventory() {
    let mut raw = vec![0, 0];
    raw.extend_from_slice(b"CIREvent marker text\xff\xff\x01\0\xff\xffCUnknown");
    let result = inventory(&raw).unwrap();
    assert_eq!(result["classes"], json!([]));
    assert_eq!(
        result["diagnostics"],
        json!([{"code": "exe_event_graph_unresolved"}])
    );
    raw.extend(declaration("CIREvent"));
    raw.extend(declaration("CIREvent"));
    let result = inventory(&raw).unwrap();
    assert_eq!(
        result["diagnostics"],
        json!([{"code": "exe_event_class_ambiguous"}])
    );
    assert_eq!(result["events"], json!([]));
}

#[test]
fn malformed_inventory_headers_and_event_parameters_fail_closed() {
    let mut raw = includes();
    let position = raw.windows(2).position(|s| s == [1, 128]).unwrap();
    raw[position] = 2;
    assert_eq!(
        inventory(&raw).unwrap_err().0,
        "exe_include_reference_unsupported"
    );
    for parameters in [
        "string x,",
        "string x,   ",
        "string x y",
        "number 1bad",
        "table a.b",
    ] {
        let mut raw = vec![0, 0];
        raw.extend(declaration("CIREvent"));
        raw.extend(event("On Next", "", parameters));
        assert_eq!(
            inventory(&raw).unwrap_err().0,
            "exe_event_parameters_unsupported"
        );
    }
    let mut raw = vec![0, 0];
    raw.extend(declaration("CIREvent"));
    raw.extend(event("On Next", "hello", ""));
    raw.pop();
    assert_eq!(inventory(&raw).unwrap_err().0, "truncated_record");
}

#[test]
fn class_counts_and_metadata_limits_are_bounded() {
    let mut raw = vec![0, 0];
    for _ in 0..257 {
        raw.extend(declaration("CTest"));
    }
    assert_eq!(inventory(&raw).unwrap_err().0, "exe_class_count_limit");
    assert_eq!(inventory(&[]).unwrap_err().0, "exe_metadata_limit");
    assert_eq!(decode(&[], "9.5").unwrap_err().0, "exe_metadata_limit");
    assert_eq!(
        decode(&[0, 0], "10.0").unwrap_err().0,
        "unsupported_installer_version"
    );
}

#[test]
fn unresolved_diagnostic_output_is_bounded() {
    let mut raw = vec![0, 0];
    raw.extend(declaration("CIREvent"));
    raw.extend(event("On Next", "", ""));
    for _ in 0..1025 {
        raw.extend_from_slice(&[0x10, 0x80, 1, 0, 0, 0, 255]);
    }
    assert_eq!(inventory(&raw).unwrap_err().0, "exe_event_count_limit");
}

#[test]
fn declaration_scans_do_not_reinterpret_overlapping_markers() {
    // Python finditer consumes the first marker even when its schema is invalid.
    let mut raw = vec![0, 0, 255];
    raw.extend(declaration("CIREvent"));
    let result = inventory(&raw).unwrap();
    assert_eq!(result["classes"], json!([]));
    assert_eq!(
        result["diagnostics"],
        json!([{"code": "exe_event_graph_unresolved"}])
    );
}
