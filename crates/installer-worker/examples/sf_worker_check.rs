//! Development-only parity runner using the bundled parser process.
//! Every output stays below the caller's disposable .artifacts/p2 case.
use patchhub_installer_worker::{production::WorkerRuntime, sf::Descriptor, WorkerError};
use std::{fs, io::Write, path::PathBuf, sync::atomic::AtomicBool, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let repo = std::env::current_dir()?.canonicalize()?;
    let boundary = repo.join(".artifacts/p2").canonicalize()?;
    let root =
        PathBuf::from(std::env::args_os().nth(1).ok_or("missing case root")?).canonicalize()?;
    if !root.starts_with(&boundary) || root == boundary {
        return Err("case must be below .artifacts/p2".into());
    }
    let descriptor = fs::read(root.join("descriptor.json"))?;
    let parsed = patchhub_installer_worker::sf::parse_descriptor(&descriptor)?;
    let (hash, size) = match &parsed {
        Descriptor::Analyze {
            exe_sha256,
            exe_size,
        }
        | Descriptor::AnalyzeBehavior {
            exe_sha256,
            exe_size,
        }
        | Descriptor::ExtractBehavior {
            exe_sha256,
            exe_size,
            ..
        } => (exe_sha256, *exe_size),
    };
    let worker = std::env::current_exe()?
        .parent()
        .and_then(|p| p.parent())
        .ok_or("runner is outside build examples")?
        .join(if cfg!(windows) {
            "patchhub-worker.exe"
        } else {
            "patchhub-worker"
        });
    let runtime = WorkerRuntime::new(worker)?;
    let cancel = AtomicBool::new(false);
    let start = Instant::now();
    let session = runtime.open_session(&root.join("input/archive.exe"), hash, size, &cancel)?;
    let response = if matches!(parsed, Descriptor::ExtractBehavior { .. }) {
        let mut sink = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(root.join("members.stream"))?;
        session.extract(&descriptor, &cancel, &mut |chunk| {
            sink.write_all(chunk)
                .map_err(|e| WorkerError::new("parity-output", None, e.to_string()))
        })?;
        serde_json::Value::Null
    } else {
        session.analyze(&descriptor, &cancel)?.response
    };
    let evidence = serde_json::json!({"platform":std::env::consts::OS,"exit_code":0,
        "wall_ms":start.elapsed().as_millis(),"response":response});
    fs::write(
        root.join("result.json"),
        serde_json::to_vec_pretty(&evidence)?,
    )?;
    println!(
        "{}",
        serde_json::json!({"evidence":root.join("result.json"),"exit_code":0})
    );
    Ok(())
}
