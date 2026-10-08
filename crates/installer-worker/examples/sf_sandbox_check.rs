//! Development-only LPAC/bwrap parity runner restricted to .artifacts/p2/<case>.
use std::{path::PathBuf, time::Duration};
use steam_patch_installer_worker::{run_sandboxed, SandboxConfig};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let repo = std::env::current_dir()?.canonicalize()?;
    let boundary = repo.join(".artifacts/p2").canonicalize()?;
    let root =
        PathBuf::from(std::env::args_os().nth(1).ok_or("missing case root")?).canonicalize()?;
    if !root.starts_with(&boundary) || root == boundary {
        return Err("case must be below .artifacts/p2".into());
    }
    for p in root.ancestors().take_while(|p| p.starts_with(&boundary)) {
        let m = std::fs::symlink_metadata(p)?;
        if m.file_type().is_symlink() {
            return Err("linked case directory".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if m.file_attributes() & 0x400 != 0 {
                return Err("reparse case directory".into());
            }
        }
    }
    let descriptor = std::fs::read(root.join("descriptor.json"))?;
    steam_patch_installer_worker::sf::parse_descriptor(&descriptor)?;
    let runtime = root.join("runtime");
    std::fs::create_dir(&runtime)?;
    let input = root.join("input");
    let output = root.join("output");
    std::fs::create_dir(&output)?;
    #[cfg(windows)]
    let name = "sf-worker.exe";
    #[cfg(not(windows))]
    let name = "sf-worker";
    let worker = runtime.join(name);
    std::fs::copy(
        std::env::current_exe()?
            .parent()
            .and_then(|p| p.parent())
            .ok_or("runner is outside build examples")?
            .join(name),
        &worker,
    )?;
    let profile = format!("steam-patch-p2-{}", std::process::id());
    let mut cfg = SandboxConfig::new(&worker, &runtime, &profile);
    cfg.input_dir = Some(input.clone());
    cfg.output_dir = Some(output.clone());
    #[cfg(windows)]
    {
        cfg.working_dir = Some(root.clone());
    }
    cfg.stdin = descriptor;
    cfg.limits.process_memory_bytes = Some(3 * 1024 * 1024 * 1024);
    cfg.limits.job_memory_bytes = Some(3 * 1024 * 1024 * 1024);
    cfg.limits.active_process_limit = Some(1);
    cfg.limits.per_job_user_time_100ns = Some(300 * 10_000_000);
    cfg.limits.timeout = Some(Duration::from_secs(480));
    cfg.limits.file_size_bytes = Some(steam_patch_installer_worker::sf::MAX_ARTIFACT);
    #[cfg(windows)]
    {
        use steam_patch_installer_worker::win::{ensure_profile, grant_path_access, AccessKind};
        let handle = ensure_profile(&profile)?;
        // Only this freshly created, disposable verification case receives grants.
        grant_path_access(&root, handle.sid, AccessKind::ReadExecute)?;
        grant_path_access(&output, handle.sid, AccessKind::ReadWriteDelete)?;
        cfg.environment = Some(
            ["SystemRoot", "SystemDrive", "USERPROFILE", "LOCALAPPDATA"]
                .into_iter()
                .map(|k| (k.to_owned(), std::env::var(k).unwrap_or_default()))
                .chain([
                    ("TEMP".into(), output.to_string_lossy().into_owned()),
                    ("TMP".into(), output.to_string_lossy().into_owned()),
                ])
                .collect(),
        );
    }
    let result = run_sandboxed(&cfg);
    #[cfg(windows)]
    let cleanup = steam_patch_installer_worker::win::delete_profile(&profile);
    let outcome = result?;
    #[cfg(windows)]
    cleanup?;
    let evidence = serde_json::json!({"platform":std::env::consts::OS,"exit_code":outcome.exit_code,
        "termination":format!("{:?}",outcome.termination),"wall_ms":outcome.wall_time.as_millis(),
        "stderr":String::from_utf8_lossy(&outcome.stderr),"response":serde_json::from_slice::<serde_json::Value>(&outcome.stdout).unwrap_or(serde_json::Value::Null)});
    std::fs::write(
        root.join("result.json"),
        serde_json::to_vec_pretty(&evidence)?,
    )?;
    println!(
        "{}",
        serde_json::json!({"evidence":root.join("result.json"),"exit_code":outcome.exit_code,"wall_ms":outcome.wall_time.as_millis()})
    );
    if outcome.exit_code != 0 {
        return Err("worker failed (see evidence)".into());
    }
    Ok(())
}
