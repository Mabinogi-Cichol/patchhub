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
fn list_members(file: &fs::File, sha: &str, size: u64) -> Value {
    let raw = json!({"action":"list_members","exe_sha256":sha,"exe_size":size});
    let descriptor = sf::parse_descriptor(&serde_json::to_vec(&raw).unwrap()).unwrap();
    sf::process(file, &descriptor).unwrap()
}
fn extraction_descriptor(_report: &Value, sha: &str, size: u64) -> Value {
    json!({"action":"extract_members","exe_sha256":sha,"exe_size":size})
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
fn listing_exposes_only_member_paths_sizes_and_checksums() {
    for case in cases() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("archive.exe");
        fs::write(&path, unhex(case["data_hex"].as_str().unwrap())).unwrap();
        let report = list_members(
            &sf::open_input(&path).unwrap(),
            case["descriptor"]["exe_sha256"].as_str().unwrap(),
            case["descriptor"]["exe_size"].as_u64().unwrap(),
        );
        let expected = &case["members"];
        assert_eq!(report["members"], json!(expected));
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
        let report = list_members(&file, sha, size);
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
    let report = list_members(&file, sha, size);
    for (key, value, code) in [
        ("exe_sha256", json!("0".repeat(64)), "exe_payload_changed"),
        ("exe_size", json!(1), "exe_size_mismatch"),
    ] {
        let mut raw = extraction_descriptor(&report, sha, size);
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
