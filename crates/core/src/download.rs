use crate::catalog::Artifact;
use reqwest::{blocking::Client, redirect::Policy, Url};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

const DOWNLOAD_RESERVE_BYTES: u64 = 8 * 1024 * 1024;
fn invalid(code: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, code)
}

pub fn validate_artifact(artifact: &Artifact) -> io::Result<()> {
    approved_url(&artifact.url, artifact)?;
    if artifact.size == 0
        || artifact.sha256.len() != 64
        || !artifact
            .sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(invalid("invalid_artifact"));
    }
    for host in &artifact.allowed_redirect_hosts {
        let url = Url::parse(&format!("https://{host}/"))
            .map_err(|_| invalid("invalid_redirect_host"))?;
        if url.host_str() != Some(host.as_str()) || url.port().is_some() || !safe_host(host) {
            return Err(invalid("invalid_redirect_host"));
        }
    }
    Ok(())
}

fn safe_host(host: &str) -> bool {
    host.contains('.')
        && host != "localhost"
        && !host.ends_with(".localhost")
        && !host.ends_with(".local")
        && host.parse::<std::net::IpAddr>().is_err()
}

fn approved_url(value: &str, artifact: &Artifact) -> io::Result<Url> {
    let url = Url::parse(value).map_err(|_| invalid("invalid_download_url"))?;
    let host = url
        .host_str()
        .ok_or_else(|| invalid("invalid_download_host"))?;
    if url.scheme() != "https"
        || !safe_host(host)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some_and(|p| p != 443)
    {
        return Err(invalid("unsafe_download_url"));
    }
    let original = Url::parse(&artifact.url).map_err(|_| invalid("invalid_download_url"))?;
    if original.host_str() != Some(host)
        && !artifact.allowed_redirect_hosts.iter().any(|h| h == host)
    {
        return Err(invalid("redirect_host_not_approved"));
    }
    Ok(url)
}

fn no_link(path: &Path) -> io::Result<()> {
    for ancestor in path.ancestors() {
        if ancestor.as_os_str().is_empty() {
            continue;
        }
        let metadata = fs::symlink_metadata(ancestor)?;
        #[cfg(windows)]
        let linked = {
            use std::os::windows::fs::MetadataExt;
            metadata.file_attributes() & 0x400 != 0
        };
        #[cfg(not(windows))]
        let linked = metadata.file_type().is_symlink();
        if linked {
            return Err(invalid("linked_download_cache"));
        }
    }
    Ok(())
}

fn copy_verified(
    reader: &mut impl Read,
    writer: &mut impl Write,
    artifact: &Artifact,
    cancel: &AtomicBool,
    progress: &mut impl FnMut(u64, u64),
) -> io::Result<()> {
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 65536];
    let mut total = 0u64;
    let mut magic = Vec::new();
    loop {
        crate::operation::check()?;
        if cancel.load(Ordering::Relaxed) {
            return Err(invalid("download_cancelled"));
        }
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(count as u64)
            .ok_or_else(|| invalid("download_size_overflow"))?;
        if total > artifact.size {
            return Err(invalid("download_size_mismatch"));
        }
        if magic.len() < 4 {
            magic.extend_from_slice(&buffer[..count.min(4 - magic.len())]);
        }
        hasher.update(&buffer[..count]);
        writer.write_all(&buffer[..count])?;
        progress(total, artifact.size);
        crate::operation::advance(count as u64);
    }
    if total != artifact.size {
        return Err(invalid("download_size_mismatch"));
    }
    if magic.as_slice() != b"PK\x03\x04" && magic.as_slice() != b"PK\x05\x06" {
        return Err(invalid("download_not_zip"));
    }
    if format!("{:x}", hasher.finalize()) != artifact.sha256 {
        return Err(invalid("download_hash_mismatch"));
    }
    Ok(())
}

fn check_download_space(cache: &Path, bytes: u64) -> io::Result<()> {
    let available = fs2::available_space(cache)?;
    let required = bytes
        .checked_add(DOWNLOAD_RESERVE_BYTES)
        .ok_or_else(|| invalid("download_size_overflow"))?;
    if available < required {
        return Err(invalid(&format!(
            "insufficient_space:state:required={required}:available={available}"
        )));
    }
    Ok(())
}

/// The artifact URL and redirects must already come from a verified signed catalog.
pub fn fetch(
    artifact: &Artifact,
    cache: &Path,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, u64),
) -> io::Result<PathBuf> {
    crate::operation::archive(artifact.size);
    crate::operation::stage("connecting", None, None);
    validate_artifact(artifact)?;
    fs::create_dir_all(cache)?;
    no_link(cache)?;
    let cache = fs::canonicalize(cache)?;
    let partial = cache.join(format!("{}.partial", uuid::Uuid::new_v4()));
    let result = (|| {
        let client = Client::builder()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(15))
            // Blocking reqwest applies this timeout to each read operation,
            // allowing long downloads while bounding stalled cancellation.
            .timeout(Duration::from_secs(15))
            .user_agent("steam-patch-manager/0.1")
            // Artifact integrity checks compare Content-Length to the catalog
            // size, so force identity encoding even though the crate enables
            // gzip/deflate for the bounded metadata fetcher.
            .no_gzip()
            .no_deflate()
            .build()
            .map_err(|_| invalid("http_client_failed"))?;
        let mut url = approved_url(&artifact.url, artifact)?;
        let mut response = None;
        for _ in 0..=5 {
            crate::operation::check()?;
            if cancel.load(Ordering::Relaxed) {
                return Err(invalid("download_cancelled"));
            }
            let mut attempt = client.get(url.clone()).send();
            if attempt.is_err() {
                crate::operation::check()?;
                if cancel.load(Ordering::Relaxed) {
                    return Err(invalid("download_cancelled"));
                }
                attempt = client.get(url.clone()).send();
            }
            let current = attempt.map_err(|_| invalid("download_network_error"))?;
            if current.status().is_redirection() {
                let location = current
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|h| h.to_str().ok())
                    .ok_or_else(|| invalid("redirect_location_missing"))?;
                let next = url
                    .join(location)
                    .map_err(|_| invalid("invalid_redirect_url"))?;
                url = approved_url(next.as_str(), artifact)?;
                continue;
            }
            if !current.status().is_success() {
                return Err(invalid("download_http_error"));
            }
            if current.content_length().is_some_and(|n| n != artifact.size) {
                return Err(invalid("download_size_mismatch"));
            }
            if current
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|h| h.to_str().ok())
                .is_some_and(|mime| mime.contains("text/html") || mime.contains("application/json"))
            {
                return Err(invalid("download_not_zip"));
            }
            response = Some(current);
            break;
        }
        let mut response = response.ok_or_else(|| invalid("too_many_redirects"))?;
        check_download_space(&cache, artifact.size)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)?;
        crate::operation::stage("downloading", Some(artifact.size), None);
        copy_verified(&mut response, &mut file, artifact, cancel, &mut progress)?;
        file.sync_all()?;
        drop(file);
        let destination = cache.join(format!("{}.zip", artifact.sha256));
        if destination.exists() {
            no_link(&destination)?;
            let mut existing = File::open(&destination)?;
            crate::operation::stage("validating", Some(artifact.size), Some(1));
            copy_verified(
                &mut existing,
                &mut io::sink(),
                artifact,
                cancel,
                &mut |_, _| (),
            )?;
            fs::remove_file(&partial)?;
        } else {
            fs::rename(&partial, &destination)?;
        }
        Ok(destination)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&partial);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn artifact(bytes: &[u8]) -> Artifact {
        Artifact {
            url: "https://publisher.example/patch.zip".into(),
            size: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(bytes)),
            allowed_redirect_hosts: vec!["cdn.example".into()],
        }
    }
    #[test]
    fn large_signed_zip_is_valid_but_disk_space_is_checked() {
        let mut signed = artifact(b"PK\x03\x04data");
        signed.size = 8 * 1024 * 1024 * 1024 + 1;
        validate_artifact(&signed).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let available = fs2::available_space(temp.path()).unwrap();
        assert!(check_download_space(temp.path(), available)
            .unwrap_err()
            .to_string()
            .starts_with("insufficient_space:state:"));
    }

    #[test]
    fn https_and_redirect_policy_reject_credentials_local_and_unapproved_hosts() {
        let a = artifact(b"PK\x03\x04data");
        for url in [
            "http://publisher.example/a",
            "https://localhost/a",
            "https://127.0.0.1/a",
            "https://user:pass@publisher.example/a",
            "https://other.example/a",
        ] {
            assert!(approved_url(url, &a).is_err(), "{url}");
        }
        assert!(approved_url("https://cdn.example/a", &a).is_ok());
    }
    #[test]
    fn stream_rejects_html_hash_size_and_cancel() {
        let data = b"PK\x03\x04data";
        let a = artifact(data);
        let cancelled = AtomicBool::new(true);
        let active = AtomicBool::new(false);
        assert!(copy_verified(
            &mut &data[..],
            &mut Vec::new(),
            &a,
            &cancelled,
            &mut |_, _| ()
        )
        .is_err());
        copy_verified(&mut &data[..], &mut Vec::new(), &a, &active, &mut |_, _| ()).unwrap();
        assert!(copy_verified(
            &mut &b"<html>x!"[..],
            &mut Vec::new(),
            &a,
            &active,
            &mut |_, _| ()
        )
        .is_err());
        assert!(copy_verified(
            &mut &b"PK\x03\x04bad!"[..],
            &mut Vec::new(),
            &a,
            &active,
            &mut |_, _| ()
        )
        .is_err());
        assert!(copy_verified(
            &mut &data[..4],
            &mut Vec::new(),
            &a,
            &active,
            &mut |_, _| ()
        )
        .is_err());
    }
}
