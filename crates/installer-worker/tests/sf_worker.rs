use patchhub_installer_worker::sf::{self, Descriptor};
use serde_json::{json, Value};
use std::fs;

fn cases() -> Vec<Value> {
    let v: Value = serde_json::from_str(include_str!(
        "../../installer-format/tests/fixtures/sf/worker-golden.json"
    ))
    .unwrap();
    v["cases"].as_array().unwrap().clone()
}
fn unhex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect()
}
fn analyze(file: &fs::File, sha: &str, size: u64) -> Value {
    let raw = json!({"action":"analyze_behavior","exe_sha256":sha,"exe_size":size});
    let descriptor = sf::parse_descriptor(&serde_json::to_vec(&raw).unwrap()).unwrap();
    sf::process(file, &descriptor).unwrap()
}
fn extraction_descriptor(report: &Value, sha: &str, size: u64) -> Value {
    let members: Vec<_> = report["report"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            json!({
                "path": f["destination"],
                "size": f["unpacked_size"],
                "crc32": f["crc32"],
                "payload_offset": f["payload_offset"],
                "packed_size": f["packed_size"],
                "compressed": f["compressed"],
            })
        })
        .collect();
    json!({"action":"extract_behavior","exe_sha256":sha,"exe_size":size,"payload":members})
}
/// Parse the framed stream: header, then (member header, raw bytes, trailer)*, done.
fn parse_frames(bytes: &[u8]) -> Vec<(String, Vec<u8>, String)> {
    fn line(bytes: &[u8], at: &mut usize) -> Value {
        let end = *at + bytes[*at..].iter().position(|b| *b == b'\n').unwrap();
        let value = serde_json::from_slice(&bytes[*at..end]).unwrap();
        *at = end + 1;
        value
    }
    let mut at = 0;
    let header = line(bytes, &mut at);
    assert_eq!(header["extract"], 1);
    let mut members = Vec::new();
    loop {
        let frame = line(bytes, &mut at);
        if frame["done"].as_bool() == Some(true) {
            break;
        }
        let size = frame["size"].as_u64().unwrap() as usize;
        let raw = bytes[at..at + size].to_vec();
        at += size;
        let trailer = line(bytes, &mut at);
        assert_eq!(trailer["index"], frame["index"]);
        members.push((
            frame["path"].as_str().unwrap().into(),
            raw,
            trailer["sha256"].as_str().unwrap().into(),
        ));
    }
    assert_eq!(at, bytes.len());
    members
}
#[test]
fn analysis_matches_frozen_python_reports_without_payload_digests() {
    for case in cases() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("archive.exe");
        fs::write(&input, unhex(case["data_hex"].as_str().unwrap())).unwrap();
        let file = sf::open_input(&input).unwrap();
        let report = analyze(
            &file,
            case["descriptor"]["exe_sha256"].as_str().unwrap(),
            case["descriptor"]["exe_size"].as_u64().unwrap(),
        );
        assert_eq!(report["report"], case["report"], "{}", case["name"]);
        for row in report["report"]["files"].as_array().unwrap() {
            assert!(row.get("payload").is_none());
            assert!(row["payload_offset"].as_u64().is_some());
            assert!(row["packed_size"].as_u64().is_some());
        }
    }
}
#[test]
fn extraction_streams_exact_bytes_and_verifies_the_manifest() {
    for case in cases() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("archive.exe");
        fs::write(&input, unhex(case["data_hex"].as_str().unwrap())).unwrap();
        let file = sf::open_input(&input).unwrap();
        let sha = case["descriptor"]["exe_sha256"].as_str().unwrap();
        let size = case["descriptor"]["exe_size"].as_u64().unwrap();
        let report = analyze(&file, sha, size);
        let descriptor = extraction_descriptor(&report, sha, size);
        let parsed = sf::parse_descriptor(&serde_json::to_vec(&descriptor).unwrap()).unwrap();
        let mut out = Vec::new();
        sf::extract(&file, &parsed, &mut out).unwrap();
        let members = parse_frames(&out);
        let expected = case["conversion"]["files"].as_array().unwrap();
        assert_eq!(members.len(), expected.len());
        for (member, expected) in members.iter().zip(expected) {
            assert_eq!(member.0, expected["path"].as_str().unwrap());
            assert_eq!(member.2, expected["sha256"].as_str().unwrap());
            assert_eq!(member.1.len() as u64, expected["size"].as_u64().unwrap());
        }
    }
}
#[test]
fn every_extraction_binding_is_checked_before_member_bytes() {
    let case = &cases()[0];
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("archive.exe");
    fs::write(&input, unhex(case["data_hex"].as_str().unwrap())).unwrap();
    let file = sf::open_input(&input).unwrap();
    let sha = case["descriptor"]["exe_sha256"].as_str().unwrap();
    let size = case["descriptor"]["exe_size"].as_u64().unwrap();
    let report = analyze(&file, sha, size);
    for (key, value, code) in [
        ("exe_sha256", json!("0".repeat(64)), "exe_payload_changed"),
        ("exe_size", json!(1), "exe_size_mismatch"),
        (
            "payload",
            json!([{"path":"wrong.bin","size":1,"crc32":0,"payload_offset":0,"packed_size":1,"compressed":false}]),
            "payload_crc_mismatch",
        ),
    ] {
        let mut raw = extraction_descriptor(&report, sha, size);
        if key == "exe_size" {
            raw["payload"] = json!([{"path":"x","size":0,"crc32":0,"payload_offset":0,"packed_size":0,"compressed":false}]);
        }
        raw[key] = value;
        let descriptor = match sf::parse_descriptor(&serde_json::to_vec(&raw).unwrap()) {
            Ok(d) => d,
            Err(e) => panic!("parse {} raw {}", e.0, raw),
        };
        let mut out = Vec::new();
        assert_eq!(
            sf::extract(&file, &descriptor, &mut out).unwrap_err().0,
            code
        );
    }
}
#[test]
fn descriptor_rejects_out_of_range_member_ranges() {
    let sha = "a".repeat(64);
    let raw = json!({"action":"extract_behavior","exe_sha256":sha,"exe_size":10,
        "payload":[{"path":"a","size":5,"crc32":0,"payload_offset":8,"packed_size":5,"compressed":false}]});
    assert_eq!(
        sf::parse_descriptor(&serde_json::to_vec(&raw).unwrap())
            .unwrap_err()
            .0,
        "invalid_descriptor"
    );
}
#[test]
fn binary_rejects_extra_arguments() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_patchhub-worker"))
        .arg("--input")
        .arg("anything.exe")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["error"],
        "invalid_worker_arguments"
    );
    let _action: Option<Descriptor> = None;
}
