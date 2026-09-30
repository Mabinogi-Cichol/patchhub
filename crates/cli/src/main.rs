use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    env,
    io::{self, BufRead, Write},
    path::PathBuf,
    process::ExitCode,
};
use steam_patch_core::{
    catalog::{self, Catalog, Platform},
    steam,
};

const MAX_REQUEST_BYTES: usize = 64 * 1024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    protocol_version: u32,
    request_id: String,
    method: String,
    params: Params,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Params {
    #[serde(default)]
    roots: Vec<PathBuf>,
    catalog: Option<PathBuf>,
    platform: Option<Platform>,
    instance_id: Option<String>,
    release_id: Option<String>,
}

fn host_platform() -> Platform {
    if cfg!(windows) {
        Platform::Windows
    } else {
        Platform::Steamos
    }
}

fn execute(method: &str, params: Params) -> Result<Value, String> {
    if params.roots.len() > 32 {
        return Err("too_many_roots".into());
    }
    if !["scan", "preview"].contains(&method) {
        return Err("unsupported_method".into());
    }
    if method == "scan" && (params.instance_id.is_some() || params.release_id.is_some()) {
        return Err("unexpected_preview_arguments".into());
    }
    let roots = if params.roots.is_empty() {
        steam::discover_roots()
    } else {
        params.roots
    };
    let report = steam::scan(&roots);
    let catalog = match params.catalog {
        Some(path) => catalog::load(&path).map_err(|e| format!("catalog_invalid: {e}"))?,
        None => Catalog {
            schema_version: 1,
            revision: 0,
            releases: Vec::new(),
        },
    };
    let platform = params.platform.unwrap_or_else(host_platform);
    if method == "preview" {
        let instance_id = params.instance_id.ok_or("instance_required")?;
        let release_id = params.release_id.ok_or("release_required")?;
        let install = report
            .installs
            .iter()
            .find(|i| i.instance_id == instance_id)
            .ok_or("instance_not_found")?;
        let preview = catalog::preview(install, &catalog, &release_id, platform)
            .map_err(|e| e.to_string())?;
        return serde_json::to_value(preview).map_err(|e| e.to_string());
    }
    let rows: Vec<_> = report.installs.iter().map(|install| {
        let matches = catalog::matches(install, &catalog, platform);
        json!({ "game": install, "match_status": if matches.is_empty() { "no_catalog_match" } else { "matched" }, "patches": matches })
    }).collect();
    Ok(
        json!({ "catalog_revision": catalog.revision, "catalog_trust": "unsigned_development",
        "platform": platform, "roots": roots, "installs": rows, "diagnostics": report.diagnostics }),
    )
}

fn error(id: Option<&str>, code: &str) -> Value {
    json!({ "protocol_version": 1, "request_id": id, "ok": false, "error": { "code": code } })
}

fn respond(bytes: &[u8]) -> Value {
    let request: Request = match serde_json::from_slice(bytes) {
        Ok(r) => r,
        Err(_) => return error(None, "invalid_request"),
    };
    if request.protocol_version != 1 {
        return error(Some(&request.request_id), "unsupported_protocol");
    }
    if request.request_id.is_empty() || request.request_id.len() > 128 {
        return error(None, "invalid_request_id");
    }
    match execute(&request.method, request.params) {
        Ok(result) => {
            json!({ "protocol_version": 1, "request_id": request.request_id, "ok": true, "result": result })
        }
        Err(code) => error(Some(&request.request_id), &code),
    }
}

/// Read one bounded JSON line without allocating for a hostile unbounded stream.
fn read_request(reader: &mut impl BufRead) -> io::Result<Option<(Vec<u8>, bool)>> {
    let mut bytes = Vec::new();
    let mut oversized = false;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if bytes.is_empty() && !oversized {
                Ok(None)
            } else {
                Ok(Some((bytes, oversized)))
            };
        }
        let newline = available.iter().position(|b| *b == b'\n');
        let take = newline.map_or(available.len(), |p| p + 1);
        if bytes.len() + take <= MAX_REQUEST_BYTES && !oversized {
            bytes.extend_from_slice(&available[..take]);
        } else {
            oversized = true;
        }
        reader.consume(take);
        if newline.is_some() {
            return Ok(Some((bytes, oversized)));
        }
    }
}

fn serve(reader: &mut impl BufRead, writer: &mut impl Write) -> io::Result<()> {
    while let Some((bytes, oversized)) = read_request(reader)? {
        let response = if oversized {
            error(None, "request_too_large")
        } else {
            respond(&bytes)
        };
        serde_json::to_writer(&mut *writer, &response)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
    }
    Ok(())
}

const HELP: &str = "Steam patch manager P0 (read-only)\n\
Usage: steam-patch scan [--steam-root PATH]... [--catalog PATH] [--platform windows|steamos]\n\
       steam-patch preview --instance ID --release ID [scan options]\n\
       steam-patch serve  (versioned JSON-lines on stdin/stdout)\n\
No command downloads or modifies games. Unsigned catalogs never authorize installation.";

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let Some(command) = args.next() else {
        println!("{HELP}");
        return Ok(());
    };
    if ["--help", "-h", "help"].contains(&command.as_str()) {
        println!("{HELP}");
        return Ok(());
    }
    if command == "serve" {
        if args.next().is_some() {
            return Err("serve_accepts_no_arguments".into());
        }
        return serve(&mut io::stdin().lock(), &mut io::stdout().lock()).map_err(|e| e.to_string());
    }
    let mut params = Params::default();
    while let Some(flag) = args.next() {
        let value = args.next().ok_or_else(|| format!("missing_value:{flag}"))?;
        match flag.as_str() {
            "--steam-root" => params.roots.push(PathBuf::from(value)),
            "--catalog" if params.catalog.is_none() => params.catalog = Some(PathBuf::from(value)),
            "--instance" if params.instance_id.is_none() => params.instance_id = Some(value),
            "--release" if params.release_id.is_none() => params.release_id = Some(value),
            "--platform" if params.platform.is_none() => {
                params.platform = Some(match value.as_str() {
                    "windows" => Platform::Windows,
                    "steamos" => Platform::Steamos,
                    _ => return Err("invalid_platform".into()),
                })
            }
            _ => return Err(format!("unknown_or_duplicate_option:{flag}")),
        }
    }
    let result = execute(&command, params)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&result).map_err(|e| e.to_string())?
    );
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn protocol_recovers_after_bad_or_oversized_line() {
        let root = tempfile::tempdir().unwrap();
        let valid = json!({"protocol_version":1,"request_id":"r1","method":"scan","params":{"roots":[root.path()]}}).to_string();
        let input = format!("broken\n{}\n{valid}\n", "x".repeat(MAX_REQUEST_BYTES + 1));
        let mut output = Vec::new();
        serve(&mut Cursor::new(input), &mut output).unwrap();
        let rows: Vec<Value> = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0]["error"]["code"], "invalid_request");
        assert_eq!(rows[1]["error"]["code"], "request_too_large");
        assert_eq!(rows[2]["ok"], true);
        assert_eq!(rows[2]["request_id"], "r1");
    }

    #[test]
    fn protocol_rejects_unknown_fields_versions_and_mutations() {
        let request = |version, method| {
            json!({"protocol_version":version,"request_id":"r","method":method,"params":{}})
                .to_string()
        };
        assert_eq!(
            respond(request(2, "scan").as_bytes())["error"]["code"],
            "unsupported_protocol"
        );
        assert_eq!(
            respond(request(1, "start_install").as_bytes())["error"]["code"],
            "unsupported_method"
        );
        assert_eq!(respond(br#"{"protocol_version":1,"request_id":"r","method":"scan","params":{"shell":"evil"}}"#)["error"]["code"], "invalid_request");
    }

    #[test]
    fn scan_to_preview_preserves_instance_and_is_read_only() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("Steam");
        std::fs::create_dir_all(root.join("steamapps/common/Test/game")).unwrap();
        std::fs::write(root.join("steamapps/appmanifest_42.acf"),
            "\"AppState\" { \"appid\" \"42\" \"name\" \"Synthetic\" \"installdir\" \"Test\" \"buildid\" \"100\" \"StateFlags\" \"4\" }").unwrap();
        let hash = "0".repeat(64);
        let catalog_path = temp.path().join("catalog.json");
        let data = json!({"schema_version":1,"revision":1,"releases":[{
            "patch_id":"test","release_id":"test-v1","appid":42,"name":"Synthetic patch",
            "version":"1","language":"zh-CN","category":"content_restore",
            "source_page":"https://example.invalid/patch","support_level":"managed_zip",
            "platforms":["windows"],"verified_buildids":["100"],
            "detection":[{"path":"game/patch.rpa","sha256":hash}],
            "recipe":{"artifact_sha256":hash,"files":[{"source":"patch.rpa","destination":"game/patch.rpa",
                "original_sha256":null,"installed_sha256":hash}]}
        }]});
        std::fs::write(&catalog_path, data.to_string()).unwrap();
        let params = |instance| Params {
            roots: vec![root.clone()],
            catalog: Some(catalog_path.clone()),
            platform: Some(Platform::Windows),
            instance_id: instance,
            release_id: None,
        };
        let scan = execute("scan", params(None)).unwrap();
        assert_eq!(scan["installs"][0]["match_status"], "matched");
        assert_eq!(
            scan["installs"][0]["patches"][0]["detection"]["status"],
            "not_detected"
        );
        let id = scan["installs"][0]["game"]["instance_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut preview_params = params(Some(id));
        preview_params.release_id = Some("test-v1".into());
        let result = execute("preview", preview_params).unwrap();
        assert_eq!(result["executable"], false);
        assert_eq!(result["changes"][0]["baseline_matches"], true);
        assert!(!root.join("steamapps/common/Test/game/patch.rpa").exists());
    }
}
