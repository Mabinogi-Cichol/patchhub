//! Fixed SF worker entry point. Paths come only from the sandbox mount layout.
use std::io::{Read, Write};
use std::path::Path;
use steam_patch_installer_worker::sf;

fn main() {
    let mut extracting = false;
    let result = (|| {
        if std::env::args_os().len() != 1 {
            return Err(steam_patch_installer_format::Error(
                "invalid_worker_arguments".into(),
            ));
        }
        let mut raw = Vec::new();
        std::io::stdin()
            .take((sf::MAX_MESSAGE + 1) as u64)
            .read_to_end(&mut raw)?;
        let descriptor = sf::parse_descriptor(&raw)?;
        extracting = matches!(descriptor, sf::Descriptor::ExtractBehavior { .. });
        #[cfg(target_os = "linux")]
        let input = Path::new("/input/archive.exe");
        #[cfg(windows)]
        let input = Path::new("input/archive.exe");
        #[cfg(not(any(windows, target_os = "linux")))]
        return Err(steam_patch_installer_format::Error(
            "worker_platform_unsupported".into(),
        ));
        let file = sf::open_input(input)?;
        if extracting {
            // The stream is framed by the host, so a failure must not append a
            // JSON error object to member bytes. Report it on stderr and exit 1.
            sf::extract(&file, &descriptor, &mut std::io::stdout().lock())?;
        } else {
            let response = sf::process(&file, &descriptor)?;
            std::io::stdout()
                .lock()
                .write_all(&sf::encode_response(&response)?)?;
        }
        Ok::<(), steam_patch_installer_format::Error>(())
    })();
    if let Err(error) = result {
        let detail: String = error.0.chars().take(2048).collect();
        if extracting {
            let _ = writeln!(std::io::stderr(), "{detail}");
        } else {
            let _ = writeln!(std::io::stdout(), "{}", serde_json::json!({"error":detail}));
        }
        std::process::exit(1);
    }
}
