//! Fixed SF worker entry point. Paths come only from the sandbox mount layout.
use std::io::{Read, Write};
use std::path::Path;
use steam_patch_installer_worker::sf;

fn main() {
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
        #[cfg(target_os = "linux")]
        let (input, output) = (
            Path::new("/input/archive.exe"),
            Path::new("/output/normalized.zip"),
        );
        #[cfg(windows)]
        let (input, output) = (
            Path::new("input/archive.exe"),
            Path::new("output/normalized.zip"),
        );
        #[cfg(not(any(windows, target_os = "linux")))]
        return Err(steam_patch_installer_format::Error(
            "worker_platform_unsupported".into(),
        ));
        let file = sf::open_input(input)?;
        let response = sf::process(&file, &descriptor, output)?;
        let encoded = sf::encode_response(&response)?;
        if let Err(e) = std::io::stdout().lock().write_all(&encoded) {
            if matches!(descriptor, sf::Descriptor::ConvertBehavior { .. }) {
                let _ = std::fs::remove_file(output);
            }
            return Err(e.into());
        }
        Ok::<(), steam_patch_installer_format::Error>(())
    })();
    if let Err(error) = result {
        let detail: String = error.0.chars().take(2048).collect();
        let _ = writeln!(std::io::stdout(), "{}", serde_json::json!({"error":detail}));
        std::process::exit(1);
    }
}
