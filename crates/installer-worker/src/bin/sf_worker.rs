//! Fixed SF worker entry point. The trusted host supplies the archive path.
use patchhub_installer_worker::sf;
use std::io::{Read, Write};
use std::path::PathBuf;

fn main() {
    let mut extracting = false;
    let result = (|| {
        let mut args = std::env::args_os().skip(1);
        let input = args.next().map(PathBuf::from);
        if input.is_none() || args.next().is_some() {
            return Err(patchhub_installer_format::Error(
                "invalid_worker_arguments".into(),
            ));
        }
        let mut raw = Vec::new();
        std::io::stdin()
            .take((sf::MAX_MESSAGE + 1) as u64)
            .read_to_end(&mut raw)?;
        let descriptor = sf::parse_descriptor(&raw)?;
        extracting = matches!(descriptor, sf::Descriptor::ExtractBehavior { .. });
        let file = sf::open_input(&input.unwrap())?;
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
        Ok::<(), patchhub_installer_format::Error>(())
    })();
    if let Err(error) = result {
        let detail = error.0;
        if extracting {
            let _ = writeln!(std::io::stderr(), "{detail}");
        } else {
            let _ = writeln!(std::io::stdout(), "{}", serde_json::json!({"error":detail}));
        }
        std::process::exit(1);
    }
}
