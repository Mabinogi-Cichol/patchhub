use patchhub_installer_worker::production::WorkerRuntime;
use serde_json::{json, Value};
use std::{fs, path::PathBuf, sync::atomic::AtomicBool};
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
fn new_runtime(root: &std::path::Path) -> WorkerRuntime {
    let _ = root;
    WorkerRuntime::new(PathBuf::from(env!("CARGO_BIN_EXE_patchhub-worker"))).unwrap()
}
#[test]
fn production_analyzes_and_streams_all_synthetic_behavior_extractions() {
    let stage = tempfile::tempdir().unwrap();
    let runtime = new_runtime(stage.path());
    assert_eq!(fs::read_dir(stage.path()).unwrap().count(), 0);
    for case in cases() {
        let input = tempfile::tempdir().unwrap();
        let archive = input.path().join("archive.exe");
        fs::write(&archive, unhex(case["data_hex"].as_str().unwrap())).unwrap();
        let cancel = AtomicBool::new(false);
        let sha = case["descriptor"]["exe_sha256"].as_str().unwrap();
        let size = case["descriptor"]["exe_size"].as_u64().unwrap();
        let session = runtime.open_session(&archive, sha, size, &cancel).unwrap();
        let analyze = json!({"action":"analyze_behavior","exe_sha256":sha,"exe_size":size});
        let result = session
            .analyze(&serde_json::to_vec(&analyze).unwrap(), &cancel)
            .unwrap();
        assert_eq!(
            result.response["report"], case["report"],
            "{}",
            case["name"]
        );
        let report = &result.response["report"];
        let members: Vec<_> = report["files"]
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
        let extract =
            json!({"action":"extract_behavior","exe_sha256":sha,"exe_size":size,"payload":members});
        let mut collected = Vec::new();
        session
            .extract(
                &serde_json::to_vec(&extract).unwrap(),
                &cancel,
                &mut |chunk| {
                    collected.extend_from_slice(chunk);
                    Ok(())
                },
            )
            .unwrap();
        let expected = case["conversion"]["files"].as_array().unwrap();
        let members = parse_frames(&collected);
        assert_eq!(members.len(), expected.len(), "{}", case["name"]);
        for (member, expected) in members.iter().zip(expected) {
            assert_eq!(member.0, expected["path"].as_str().unwrap());
            assert_eq!(member.2, expected["sha256"].as_str().unwrap());
            assert_eq!(member.1.len() as u64, expected["size"].as_u64().unwrap());
        }
        drop(session);
        assert_eq!(
            fs::read_dir(stage.path()).unwrap().count(),
            0,
            "direct parser creates no staging copies"
        );
    }
}
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
        members.push((
            frame["path"].as_str().unwrap().into(),
            raw,
            trailer["sha256"].as_str().unwrap().into(),
        ));
    }
    members
}

#[test]
fn production_rejects_binding_paths_and_precancel_without_output() {
    let stage = tempfile::tempdir().unwrap();
    let runtime = new_runtime(stage.path());
    let input = tempfile::tempdir().unwrap();
    let archive = input.path().join("archive.exe");
    fs::write(&archive, b"x").unwrap();
    for (raw, stage_name) in [
        (
            json!({"action":"analyze","exe_size":1,"exe_sha256":"0".repeat(64)}),
            "worker",
        ),
        (
            json!({"action":"analyze","exe_size":1,"exe_sha256":"0".repeat(64),"path":"C:/game"}),
            "descriptor",
        ),
    ] {
        let e = runtime
            .execute(
                &archive,
                &serde_json::to_vec(&raw).unwrap(),
                &AtomicBool::new(false),
            )
            .unwrap_err();
        assert_eq!(e.stage, stage_name);
        assert_eq!(fs::read_dir(stage.path()).unwrap().count(), 0);
    }
    let e = runtime
        .execute(&archive, b"", &AtomicBool::new(true))
        .unwrap_err();
    assert_eq!(e.stage, "cancelled");
    assert_eq!(fs::read_dir(stage.path()).unwrap().count(), 0);
}
#[test]
fn production_missing_worker_reports_actual_spawn_error() {
    let root = tempfile::tempdir().unwrap();
    let runtime = WorkerRuntime::new(root.path().join("missing-worker")).unwrap();
    let archive = root.path().join("input.exe");
    fs::write(&archive, b"x").unwrap();
    let raw = json!({"action":"analyze","exe_size":1,"exe_sha256":"0".repeat(64)});
    let error = runtime
        .execute(
            &archive,
            &serde_json::to_vec(&raw).unwrap(),
            &AtomicBool::new(false),
        )
        .unwrap_err();
    assert_eq!(error.stage, "spawn");
    assert!(error.code.is_some());
    assert!(WorkerRuntime::new("relative-worker").is_err());
}

#[test]
fn production_parser_error_is_reported_without_executing_the_input() {
    use sha2::{Digest, Sha256};
    let root = tempfile::tempdir().unwrap();
    let runtime = new_runtime(root.path());
    let archive = root.path().join("not a program 中文.exe");
    fs::write(&archive, b"not an executable").unwrap();
    let raw = json!({"action":"analyze","exe_size":17,"exe_sha256":format!("{:x}", Sha256::digest(b"not an executable"))});
    let error = runtime
        .execute(
            &archive,
            &serde_json::to_vec(&raw).unwrap(),
            &AtomicBool::new(false),
        )
        .unwrap_err();
    assert_eq!(error.stage, "worker");
    assert_eq!(error.message, "not_pe");
}

#[test]
fn production_stream_consumer_error_preserves_the_error() {
    let root = tempfile::tempdir().unwrap();
    let runtime = new_runtime(root.path());
    let case = &cases()[0];
    let archive = root.path().join("archive.exe");
    fs::write(&archive, unhex(case["data_hex"].as_str().unwrap())).unwrap();
    let sha = case["descriptor"]["exe_sha256"].as_str().unwrap();
    let size = case["descriptor"]["exe_size"].as_u64().unwrap();
    let cancel = AtomicBool::new(false);
    let session = runtime.open_session(&archive, sha, size, &cancel).unwrap();
    let members: Vec<_> = case["report"]["files"].as_array().unwrap().iter().map(|f| json!({
        "path":f["destination"],"size":f["unpacked_size"],"crc32":f["crc32"],
        "payload_offset":f["payload_offset"],"packed_size":f["packed_size"],"compressed":f["compressed"]
    })).collect();
    let raw =
        json!({"action":"extract_behavior","exe_sha256":sha,"exe_size":size,"payload":members});
    let error = session
        .extract(&serde_json::to_vec(&raw).unwrap(), &cancel, &mut |_| {
            Err(patchhub_installer_worker::WorkerError::new(
                "destination",
                None,
                "write failed",
            ))
        })
        .unwrap_err();
    assert_eq!(error.stage, "destination");
    assert_eq!(error.message, "write failed");
    // The subprocess has exited and released its input before returning.
    drop(session);
    fs::remove_file(archive).unwrap();
}
