use serde_json::{json, Value};
use std::{fs, path::PathBuf, sync::atomic::AtomicBool};
use steam_patch_installer_worker::production::WorkerRuntime;
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
    #[cfg(target_os = "linux")]
    let run_as = if steam_patch_installer_worker::linux::euid() == 0 {
        Some((1000, 1000))
    } else {
        None
    };
    #[cfg(not(target_os = "linux"))]
    let run_as = None;
    WorkerRuntime::new(PathBuf::from(env!("CARGO_BIN_EXE_sf-worker")), root, run_as).unwrap()
}
#[test]
fn production_probe_and_all_synthetic_behavior_conversion_goldens() {
    let stage = tempfile::tempdir().unwrap();
    let runtime = new_runtime(stage.path());
    let availability = runtime.probe();
    assert!(availability.available, "{}", availability.reason);
    assert_eq!(fs::read_dir(stage.path()).unwrap().count(), 0);
    for case in cases() {
        let input = tempfile::tempdir().unwrap();
        let archive = input.path().join("archive.exe");
        fs::write(&archive, unhex(case["data_hex"].as_str().unwrap())).unwrap();
        let analyze = json!({"action":"analyze_behavior","exe_sha256":case["descriptor"]["exe_sha256"],"exe_size":case["descriptor"]["exe_size"]});
        let result = runtime
            .execute(
                &archive,
                &serde_json::to_vec(&analyze).unwrap(),
                &AtomicBool::new(false),
            )
            .unwrap();
        assert_eq!(
            result.response["report"], case["report"],
            "{}",
            case["name"]
        );
        assert!(result.artifact_path().is_none());
        drop(result);
        let result = runtime
            .execute(
                &archive,
                &serde_json::to_vec(&case["descriptor"]).unwrap(),
                &AtomicBool::new(false),
            )
            .unwrap();
        assert_eq!(result.response, case["conversion"], "{}", case["name"]);
        let path = result.artifact_path().unwrap().to_path_buf();
        assert!(path.is_file());
        let _second_runtime = new_runtime(stage.path()); // live artifact lease must be skipped
        assert!(
            path.is_file(),
            "janitor must retain an artifact still held for apply"
        );
        drop(result);
        assert!(
            !path.exists(),
            "owned artifact directory must be reclaimed after apply/drop"
        );
        assert_eq!(
            fs::read_dir(stage.path()).unwrap().count(),
            0,
            "no staging/profile residue"
        );
    }
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
            "input",
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
fn production_missing_runtime_fails_closed() {
    let stage = tempfile::tempdir().unwrap();
    assert!(WorkerRuntime::new(stage.path().join("missing-worker"), stage.path(), None).is_err());
    assert!(WorkerRuntime::new("relative-worker", stage.path(), None).is_err());
    #[cfg(target_os = "linux")]
    if steam_patch_installer_worker::linux::euid() == 0 {
        assert!(WorkerRuntime::new(
            PathBuf::from(env!("CARGO_BIN_EXE_sf-worker")),
            stage.path(),
            None
        )
        .is_err());
    }
}
