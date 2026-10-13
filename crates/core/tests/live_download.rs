//! Opt-in production download acceptance; never writes game files or executes EXEs.
use std::{fs, net::ToSocketAddrs, path::PathBuf, sync::atomic::AtomicBool};
use steam_patch_core::installer::{self, Acquisition, DownloadRequest, InstallerPaths};

#[test]
#[ignore = "requires explicit official patch URL, evidence directory, and network access"]
fn production_download_uses_system_dns() {
    let url = std::env::var("STEAM_PATCH_TEST_DOWNLOAD_URL").expect("explicit download URL");
    let evidence = PathBuf::from(
        std::env::var("STEAM_PATCH_TEST_DOWNLOAD_EVIDENCE").expect("isolated evidence directory"),
    );
    let parsed = reqwest::Url::parse(&url).unwrap();
    let host = parsed.host_str().unwrap();
    let hosts: Vec<String> = [
        "download.patch.moe",
        "kaguraserver.com",
        "kaguragamespatch.com",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert!(hosts.iter().any(|approved| approved == host));
    let addresses: Vec<String> = (host, 443)
        .to_socket_addrs()
        .unwrap()
        .map(|address| address.ip().to_string())
        .collect();
    fs::create_dir_all(&evidence).unwrap();
    let request = DownloadRequest {
        url: &url,
        download_hosts: &hosts,
        expected_sha256: None,
        expected_size: None,
    };
    // Always use a fresh cache so repeated acceptance runs prove network retrieval.
    let paths = InstallerPaths::new(&evidence.join(uuid::Uuid::new_v4().to_string()));
    let transport = installer::https_transport().unwrap();
    let cancel = AtomicBool::new(false);
    let acquired = if parsed.path().to_ascii_lowercase().ends_with(".exe") {
        installer::acquire::acquire_exe_with(
            transport.as_ref(),
            &request,
            &paths,
            &cancel,
            |_, _| {},
        )
    } else {
        installer::acquire_with(transport.as_ref(), &request, &paths, &cancel, |_, _| {})
    }
    .expect("production acquisition succeeds using the current network");
    assert_eq!(acquired.source, Acquisition::Downloaded);
    assert!(acquired.size > 0);
    let file = fs::File::open(&acquired.path).unwrap();
    let digest = installer::digest_file(&acquired.path, acquired.size, &cancel).unwrap();
    assert_eq!(digest.sha256, acquired.sha256);
    let zip_entries = if parsed.path().to_ascii_lowercase().ends_with(".zip") {
        Some(
            zip::ZipArchive::new(file)
                .expect("valid ZIP directory")
                .len(),
        )
    } else {
        None
    };
    let result = serde_json::json!({
        "url": url,
        "dns_addresses": addresses,
        "final_url": acquired.final_url,
        "path": acquired.path,
        "size": acquired.size,
        "sha256": acquired.sha256,
        "zip_entries": zip_entries,
        "source": "downloaded",
        "game_files_written": false,
    });
    fs::write(
        evidence.join("download-evidence.json"),
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .unwrap();
    println!("{result}");
}
