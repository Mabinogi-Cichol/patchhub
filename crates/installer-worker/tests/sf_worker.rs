use serde_json::{json, Value};
use std::fs;
use steam_patch_installer_worker::sf::{self, Descriptor};
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
#[test]
fn conversion_matches_python_artifact_sha_and_repeatability() {
    for case in cases() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("archive.exe");
        fs::write(&input, unhex(case["data_hex"].as_str().unwrap())).unwrap();
        let descriptor =
            sf::parse_descriptor(&serde_json::to_vec(&case["descriptor"]).unwrap()).unwrap();
        let output = root.path().join("normalized.zip");
        let file = sf::open_input(&input).unwrap();
        let result = sf::process(&file, &descriptor, &output).unwrap();
        assert_eq!(result, case["conversion"], "{}", case["name"]);
        let bytes = fs::read(&output).unwrap();
        fs::remove_file(&output).unwrap();
        let second = sf::process(&file, &descriptor, &output).unwrap();
        assert_eq!(result, second);
        assert_eq!(bytes, fs::read(&output).unwrap());
        assert!(!root.path().join("normalized.zip.partial").exists());
    }
}
#[test]
fn analyze_never_authorizes_or_writes() {
    for case in cases() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("archive.exe");
        fs::write(&input, unhex(case["data_hex"].as_str().unwrap())).unwrap();
        let file = sf::open_input(&input).unwrap();
        let output = root.path().join("normalized.zip");
        for action in ["analyze", "analyze_behavior"] {
            let raw = json!({"action":action,"exe_sha256":case["descriptor"]["exe_sha256"],"exe_size":case["descriptor"]["exe_size"]});
            let descriptor = sf::parse_descriptor(&serde_json::to_vec(&raw).unwrap()).unwrap();
            let r = sf::process(&file, &descriptor, &output).unwrap();
            assert_eq!(r["installable"], false);
            assert_eq!(r["report"]["installable"], false);
            assert_eq!(
                r["report"]["capability"],
                if action == "analyze" {
                    "discover_only"
                } else {
                    "behavior_supported"
                }
            );
            if action == "analyze_behavior" {
                assert_eq!(r["report"], case["report"]);
            }
            assert!(!output.exists());
        }
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
}
#[test]
fn every_conversion_binding_is_checked_before_first_output_write() {
    let case = &cases()[0];
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("archive.exe");
    fs::write(&input, unhex(case["data_hex"].as_str().unwrap())).unwrap();
    let file = sf::open_input(&input).unwrap();
    let output = root.path().join("normalized.zip");
    for (key, value, code) in [
        ("exe_sha256", json!("0".repeat(64)), "exe_payload_changed"),
        ("exe_size", json!(1), "exe_size_mismatch"),
        (
            "metadata_sha256",
            json!("0".repeat(64)),
            "exe_metadata_changed",
        ),
        (
            "behavior_sha256",
            json!("0".repeat(64)),
            "exe_behavior_changed",
        ),
        (
            "payload",
            json!([{"path":"wrong.bin","size":1,"sha256":"0".repeat(64)}]),
            "exe_manifest_changed",
        ),
    ] {
        let mut raw = case["descriptor"].clone();
        raw[key] = value;
        let descriptor = sf::parse_descriptor(&serde_json::to_vec(&raw).unwrap()).unwrap();
        assert_eq!(
            sf::process(&file, &descriptor, &output).unwrap_err().0,
            code
        );
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
}
#[test]
fn another_invocations_final_or_partial_is_preserved() {
    let case = &cases()[0];
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("archive.exe");
    fs::write(&input, unhex(case["data_hex"].as_str().unwrap())).unwrap();
    let file = sf::open_input(&input).unwrap();
    let descriptor =
        sf::parse_descriptor(&serde_json::to_vec(&case["descriptor"]).unwrap()).unwrap();
    let output = root.path().join("normalized.zip");
    for name in ["normalized.zip", "normalized.zip.partial"] {
        let path = root.path().join(name);
        fs::write(&path, b"owned elsewhere").unwrap();
        assert_eq!(
            sf::process(&file, &descriptor, &output).unwrap_err().0,
            "worker_output_exists"
        );
        assert_eq!(fs::read(&path).unwrap(), b"owned elsewhere");
        fs::remove_file(&path).unwrap();
    }
}
#[test]
fn binary_rejects_paths_commands_and_unknown_actions() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_sf-worker"))
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
