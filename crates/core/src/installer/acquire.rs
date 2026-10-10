//! Dynamic URL acquisition and content-addressed download cache for the
//! installer (p0-contracts §2.4/§4.1).
//!
//! This generalizes [`crate::download`] rather than changing it: the signed
//! catalog path keeps its frozen expected-size/expected-hash contract, while
//! dynamic downloads validate every hop against the rule's `download_hosts`,
//! establish content identity from the first download and reuse the cache only
//! after re-hashing and revalidating it.
//!
//! Cache layout under `installer/rust-v1/downloads/`:
//!
//! ```text
//! <sha256>.zip          # content-addressed complete package
//! <sha256>.json         # small manifest (url/size/sha256/etag/last_modified)
//! download-<uuid>.partial  # in-flight download, never a success cache
//! ```
//!
//! The manifest is required to send `If-None-Match` / `If-Modified-Since`; the
//! package identity itself is always the SHA-256 of the bytes, never a URL or a
//! version string. Mirrors that serve different bytes land in different files.

use super::zip_recipe;
use super::{invalid, InstallerPaths, Result};
use crate::operation;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const MAX_EXE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const DOWNLOAD_RESERVE_BYTES: u64 = 8 * 1024 * 1024;
const SPACE_CHECK_INTERVAL_BYTES: u64 = 8 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;
const COPY_BUFFER: usize = 65536;
const ZIP_MAGIC_LOCAL: [u8; 4] = [0x50, 0x4b, 0x03, 0x04];
const ZIP_MAGIC_EMPTY: [u8; 4] = [0x50, 0x4b, 0x05, 0x06];

/// How an archive was obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Acquisition {
    /// Reused from the local cache (local re-hash and/or `304 Not Modified`).
    Cached,
    /// Freshly downloaded in this call.
    Downloaded,
}

/// A verified, content-addressed dynamic archive.
#[derive(Debug, Clone)]
pub struct AcquiredArchive {
    pub path: PathBuf,
    pub sha256: String,
    pub size: u64,
    pub final_url: String,
    pub source: Acquisition,
}

/// A dynamic download request validated against a source rule.
#[derive(Debug, Clone)]
pub struct DownloadRequest<'a> {
    pub url: &'a str,
    /// The rule's `download_hosts`; every redirect hop is revalidated against it.
    pub download_hosts: &'a [String],
    /// Optional digest published by a verified source.
    pub expected_sha256: Option<&'a str>,
    /// Optional size published by a verified source.
    pub expected_size: Option<u64>,
}

/// A single HTTP response, decoupled from the client so the algorithm is
/// testable without network access.
pub struct HttpResponse {
    pub status: u16,
    pub location: Option<String>,
    pub content_type: Option<String>,
    pub content_length: Option<u64>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub final_url: String,
    pub body: Box<dyn Read + Send>,
}

/// Minimal HTTP transport. Implementations must not follow redirects.
pub trait Transport {
    fn get(
        &self,
        url: &str,
        etag: Option<&str>,
        last_modified: Option<&str>,
    ) -> Result<HttpResponse>;
}

/// Validates a dynamic download URL against the rule's `download_hosts`.
///
/// Reuses the source engine primitives so the URL/host contract is identical to
/// discovery: HTTPS, approved host, no credentials, port `443`, rooted path and
/// a `.zip` suffix (p0-contracts §4.1).
pub fn validate_download(url: &str, download_hosts: &[String]) -> Result<String> {
    crate::sources::validate::allowed_url(url, download_hosts, true)
        .map_err(|_| invalid("invalid_download_url"))
}

/// Acquires `request` using the real HTTPS client (p0-contracts §4.1).
pub fn acquire(
    request: &DownloadRequest<'_>,
    paths: &InstallerPaths,
    cancel: &AtomicBool,
    progress: impl FnMut(u64, u64),
) -> Result<AcquiredArchive> {
    let transport = ReqwestTransport::new()?;
    acquire_with(&transport, request, paths, cancel, progress)
}

/// Production HTTPS transport, exposed so the orchestrator can be constructed
/// with a real transport while tests inject an offline [`Transport`].
pub fn https_transport() -> Result<Box<dyn Transport>> {
    Ok(Box::new(ReqwestTransport::new()?))
}

/// Acquires `request` using a caller-supplied transport (used by tests).
pub fn acquire_with(
    transport: &dyn Transport,
    request: &DownloadRequest<'_>,
    paths: &InstallerPaths,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, u64),
) -> Result<AcquiredArchive> {
    acquire_kind(
        transport,
        request,
        paths,
        cancel,
        &mut progress,
        ArchiveKind::Zip,
    )
}

#[derive(Clone, Copy)]
enum ArchiveKind {
    Zip,
    Exe,
}
impl ArchiveKind {
    fn extension(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::Exe => "exe",
        }
    }
    fn limit(self) -> Option<u64> {
        match self {
            Self::Zip => None,
            Self::Exe => Some(MAX_EXE_BYTES),
        }
    }
    fn url(self, url: &str, hosts: &[String]) -> Result<String> {
        let valid = crate::sources::validate::allowed_url(url, hosts, matches!(self, Self::Zip))
            .map_err(|_| invalid("invalid_download_url"))?;
        if matches!(self, Self::Exe)
            && !reqwest::Url::parse(&valid)
                .map_err(|_| invalid("invalid_download_url"))?
                .path()
                .to_ascii_lowercase()
                .ends_with(".exe")
        {
            return Err(invalid("invalid_download_url"));
        }
        Ok(valid)
    }
}
/// Only bytes with MZ magic are accepted here; all PE/SF parsing runs in the sandbox.
pub fn acquire_exe_with(
    transport: &dyn Transport,
    request: &DownloadRequest<'_>,
    paths: &InstallerPaths,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, u64),
) -> Result<AcquiredArchive> {
    acquire_kind(
        transport,
        request,
        paths,
        cancel,
        &mut progress,
        ArchiveKind::Exe,
    )
}
fn acquire_kind(
    transport: &dyn Transport,
    request: &DownloadRequest<'_>,
    paths: &InstallerPaths,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
    kind: ArchiveKind,
) -> Result<AcquiredArchive> {
    let start_url = kind.url(request.url, request.download_hosts)?;
    let dir = paths.downloads_dir();
    ensure_safe_dir(&dir)?;
    let dir = fs::canonicalize(&dir)?;

    let cached = load_cached(&dir, request, cancel, kind)?;
    let (etag, last_modified) = match &cached {
        Some((_, meta, _)) => (meta.etag.clone(), meta.last_modified.clone()),
        None => (None, None),
    };

    let partial = dir.join(format!(
        "download-{}.partial",
        uuid::Uuid::new_v4().simple()
    ));
    let result = download_and_persist(
        transport,
        request,
        &dir,
        &start_url,
        etag.as_deref(),
        last_modified.as_deref(),
        cached.as_ref(),
        &partial,
        cancel,
        progress,
        kind,
    );
    if result.is_err() {
        let _ = fs::remove_file(&partial);
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn download_and_persist(
    transport: &dyn Transport,
    request: &DownloadRequest<'_>,
    dir: &Path,
    start_url: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
    cached: Option<&(PathBuf, CacheEntry, String)>,
    partial: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
    kind: ArchiveKind,
) -> Result<AcquiredArchive> {
    let mut url = start_url.to_string();
    let mut response = None;
    for _ in 0..=MAX_REDIRECTS {
        operation::check()?;
        if cancel.load(Ordering::Relaxed) {
            return Err(invalid("download_cancelled"));
        }
        let current = transport.get(&url, etag, last_modified)?;
        if current.status == 304 {
            return match cached {
                Some((path, meta, digest)) => Ok(AcquiredArchive {
                    path: path.clone(),
                    sha256: digest.clone(),
                    size: meta.size,
                    final_url: meta
                        .final_url
                        .clone()
                        .unwrap_or_else(|| request.url.to_string()),
                    source: Acquisition::Cached,
                }),
                None => Err(invalid("download_not_modified_without_cache")),
            };
        }
        if (300..400).contains(&current.status) {
            let location = current
                .location
                .clone()
                .ok_or_else(|| invalid("redirect_location_missing"))?;
            let next = reqwest::Url::parse(&url)
                .and_then(|base| base.join(&location))
                .map_err(|_| invalid("invalid_redirect_url"))?;
            url = kind.url(next.as_str(), request.download_hosts)?;
            continue;
        }
        if !(200..300).contains(&current.status) {
            return Err(invalid("download_http_error"));
        }
        response = Some(current);
        break;
    }
    let response = response.ok_or_else(|| invalid("too_many_redirects"))?;
    let final_url = kind.url(&response.final_url, request.download_hosts)?;
    if final_url != url {
        return Err(invalid("download_response_url_changed"));
    }

    if response
        .content_type
        .as_deref()
        .is_some_and(|mime| mime.contains("text/html") || mime.contains("application/json"))
    {
        return Err(invalid("download_not_zip"));
    }
    if kind.limit().is_some_and(|max| {
        response.content_length.is_some_and(|length| length > max)
            || request.expected_size.is_some_and(|size| size > max)
    }) {
        return Err(invalid("download_size_limit"));
    }
    if let (Some(expected), Some(length)) = (request.expected_size, response.content_length) {
        if length != expected {
            return Err(invalid("download_size_mismatch"));
        }
    }

    let total_hint = response
        .content_length
        .or(request.expected_size)
        .unwrap_or(0);
    operation::stage(
        "downloading",
        if total_hint == 0 {
            None
        } else {
            Some(total_hint)
        },
        None,
    );

    check_download_space(dir, total_hint)?;
    let mut body = response.body;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(partial)?;
    let expected = Expected {
        sha256: request.expected_sha256,
        size: request.expected_size.or(response.content_length),
        max: kind.limit(),
        kind,
        total_hint,
        space_dir: dir,
    };
    let (digest, size) = copy_verified(&mut body, &mut file, &expected, cancel, progress)?;
    file.sync_all()?;
    drop(file);

    let destination = dir.join(format!("{digest}.{}", kind.extension()));
    if destination.exists() {
        let (existing, existing_size) = hash_file(&destination, kind.limit(), cancel)?;
        if existing != digest || existing_size != size {
            return Err(invalid("download_cache_conflict"));
        }
        let _ = fs::remove_file(partial);
    } else {
        fs::hard_link(partial, &destination)?;
        fs::remove_file(partial)?;
    }
    let meta = CacheEntry {
        format: kind.extension().to_string(),
        url: request.url.to_string(),
        sha256: digest.clone(),
        size,
        etag: response.etag,
        last_modified: response.last_modified,
        final_url: Some(final_url.clone()),
    };
    write_manifest(dir, &meta)?;
    operation::archive(size);
    Ok(AcquiredArchive {
        path: destination,
        sha256: digest,
        size,
        final_url,
        source: Acquisition::Downloaded,
    })
}

struct Expected<'a> {
    sha256: Option<&'a str>,
    size: Option<u64>,
    max: Option<u64>,
    kind: ArchiveKind,
    total_hint: u64,
    space_dir: &'a Path,
}

fn copy_verified(
    reader: &mut dyn Read,
    writer: &mut File,
    expected: &Expected<'_>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<(String, u64)> {
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut magic = [0u8; 4];
    let mut magic_len = 0usize;
    let mut buffer = vec![0u8; COPY_BUFFER];
    loop {
        operation::check()?;
        if cancel.load(Ordering::Relaxed) {
            return Err(invalid("download_cancelled"));
        }
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        if expected.total_hint == 0
            && total / SPACE_CHECK_INTERVAL_BYTES
                != (total.saturating_add(count as u64)) / SPACE_CHECK_INTERVAL_BYTES
        {
            check_download_space(expected.space_dir, COPY_BUFFER as u64)?;
        }
        total = total
            .checked_add(count as u64)
            .ok_or_else(|| invalid("download_size_overflow"))?;
        if expected.max.is_some_and(|max| total > max) {
            return Err(invalid("download_size_limit"));
        }
        if expected.size.is_some_and(|size| total > size) {
            return Err(invalid("download_size_mismatch"));
        }
        if magic_len < magic.len() {
            let take = count.min(magic.len() - magic_len);
            magic[magic_len..magic_len + take].copy_from_slice(&buffer[..take]);
            magic_len += take;
        }
        hasher.update(&buffer[..count]);
        writer.write_all(&buffer[..count])?;
        progress(total, expected.total_hint);
        operation::advance(count as u64);
    }
    if total == 0 {
        return Err(invalid("empty_archive"));
    }
    if let Some(size) = expected.size {
        if total != size {
            return Err(invalid("download_size_mismatch"));
        }
    }
    let valid = match expected.kind {
        ArchiveKind::Zip => {
            magic_len == 4 && (magic == ZIP_MAGIC_LOCAL || magic == ZIP_MAGIC_EMPTY)
        }
        ArchiveKind::Exe => magic_len >= 2 && &magic[..2] == b"MZ",
    };
    if !valid {
        return Err(invalid(format!(
            "download_not_{}",
            expected.kind.extension()
        )));
    }
    let digest = format!("{:x}", hasher.finalize());
    if let Some(sha256) = expected.sha256 {
        if digest != sha256 {
            return Err(invalid("download_hash_mismatch"));
        }
    }
    Ok((digest, total))
}

/// Content-addressed download manifest (one per `<sha256>.zip`).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheEntry {
    #[serde(default = "zip_format")]
    format: String,
    url: String,
    sha256: String,
    size: u64,
    #[serde(default)]
    etag: Option<String>,
    #[serde(default)]
    last_modified: Option<String>,
    #[serde(default)]
    final_url: Option<String>,
}

fn zip_format() -> String {
    "zip".into()
}

fn load_cached(
    dir: &Path,
    request: &DownloadRequest<'_>,
    cancel: &AtomicBool,
    kind: ArchiveKind,
) -> Result<Option<(PathBuf, CacheEntry, String)>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return Ok(None),
    };
    for entry in entries {
        let path = entry?.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        if !zip_recipe::no_links(&path) {
            continue;
        }
        if fs::metadata(&path).map(|m| m.len() > 65536).unwrap_or(true) {
            continue;
        }
        let Ok(bytes) = fs::read(&path) else {
            continue;
        };
        let Ok(meta) = serde_json::from_slice::<CacheEntry>(&bytes) else {
            continue;
        };
        if meta.format != kind.extension()
            || meta.url != request.url
            || !valid_sha256(&meta.sha256)
            || request
                .expected_sha256
                .is_some_and(|expected| expected != meta.sha256)
            || request
                .expected_size
                .is_some_and(|expected| expected != meta.size)
        {
            continue;
        }
        let archive = dir.join(format!("{}.{}", meta.sha256, kind.extension()));
        if !zip_recipe::no_links(&archive) || !archive.is_file() {
            continue;
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(invalid("download_cancelled"));
        }
        operation::stage("validating", None, None);
        let (digest, size) = hash_file(&archive, kind.limit(), cancel)?;
        if digest != meta.sha256 || size != meta.size {
            continue;
        }
        if request
            .expected_sha256
            .is_some_and(|expected| expected != digest)
        {
            continue;
        }
        return Ok(Some((archive, meta, digest)));
    }
    Ok(None)
}

fn write_manifest(dir: &Path, meta: &CacheEntry) -> Result<()> {
    let path = dir.join(format!("{}.json", meta.sha256));
    let tmp = dir.join(format!(
        ".{}.{}.tmp",
        meta.sha256,
        uuid::Uuid::new_v4().simple()
    ));
    let bytes = serde_json::to_vec(meta).map_err(|error| invalid(error.to_string()))?;
    {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    if fs::rename(&tmp, &path).is_err() {
        let _ = fs::remove_file(&path);
        fs::rename(&tmp, &path)?;
    }
    Ok(())
}

fn hash_file(path: &Path, limit: Option<u64>, cancel: &AtomicBool) -> Result<(String, u64)> {
    let size = fs::metadata(path)?.len();
    if limit.is_some_and(|max| size > max) {
        return Err(invalid("download_size_limit"));
    }
    let digest = super::facts::digest_file(path, size, cancel)?;
    Ok((digest.sha256, digest.size))
}

fn check_download_space(dir: &Path, bytes: u64) -> Result<()> {
    let available = fs2::available_space(dir)?;
    let required = bytes
        .checked_add(DOWNLOAD_RESERVE_BYTES)
        .ok_or_else(|| invalid("download_size_overflow"))?;
    if available < required {
        return Err(invalid(format!(
            "insufficient_space:state:required={required}:available={available}"
        )));
    }
    Ok(())
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn ensure_safe_dir(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir)?;
    if !zip_recipe::no_links(dir) {
        return Err(invalid("linked_download_cache"));
    }
    Ok(())
}

/// Production HTTPS transport. Never follows redirects: the algorithm
/// revalidates each hop against `download_hosts`.
struct ReqwestTransport;

impl ReqwestTransport {
    fn new() -> Result<Self> {
        Ok(Self)
    }
}

impl Transport for ReqwestTransport {
    fn get(
        &self,
        url: &str,
        etag: Option<&str>,
        last_modified: Option<&str>,
    ) -> Result<HttpResponse> {
        use std::net::ToSocketAddrs;
        let parsed = reqwest::Url::parse(url).map_err(|_| invalid("invalid_download_url"))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| invalid("invalid_download_url"))?;
        if parsed.scheme() != "https"
            || parsed.port_or_known_default() != Some(443)
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err(invalid("invalid_download_url"));
        }
        let addresses: Vec<_> = (host, 443)
            .to_socket_addrs()
            .map_err(|_| invalid("download_dns_failed"))?
            .collect();
        if addresses.is_empty() || addresses.iter().any(|a| !public_ip(a.ip())) {
            return Err(invalid("download_private_address"));
        }
        // Resolve exactly once per hop and pin every connection, preserving TLS hostname verification.
        let client = reqwest::blocking::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .resolve_to_addrs(host, &addresses)
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(15))
            .user_agent("steam-patch-manager/0.1")
            .no_gzip()
            .no_deflate()
            .build()
            .map_err(|_| invalid("http_client_failed"))?;
        let mut request = client.get(url);
        if let Some(value) = etag {
            request = request.header(reqwest::header::IF_NONE_MATCH, value);
        }
        if let Some(value) = last_modified {
            request = request.header(reqwest::header::IF_MODIFIED_SINCE, value);
        }
        let response = request
            .send()
            .map_err(|_| invalid("download_network_error"))?;
        let status = response.status().as_u16();
        let header = |name: reqwest::header::HeaderName| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string)
        };
        let location = header(reqwest::header::LOCATION);
        let content_type = header(reqwest::header::CONTENT_TYPE);
        let etag = header(reqwest::header::ETAG);
        let last_modified = header(reqwest::header::LAST_MODIFIED);
        let content_length = response.content_length();
        let final_url = response.url().to_string();
        Ok(HttpResponse {
            status,
            location,
            content_type,
            content_length,
            etag,
            last_modified,
            final_url,
            body: Box::new(response),
        })
    }
}

/// Conservative routable-address allowlist; reject private/reserved and transition networks.
fn public_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(a == 0
                || a == 10
                || a == 127
                || a >= 224
                || (a == 100 && (64..=127).contains(&b))
                || (a == 169 && b == 254)
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && (b == 168 || b == 0 || b == 2 || b == 88 && c == 99))
                || (a == 198 && (b == 18 || b == 19 || b == 51 && c == 100))
                || (a == 203 && b == 0 && c == 113))
        }
        std::net::IpAddr::V6(ip) => {
            let s = ip.segments();
            s[0] & 0xe000 == 0x2000
                && s[0] != 0x2002
                && s[0] != 0x3fff
                && !(s[0] == 0x2001 && (s[1] < 0x200 || s[1] == 0xdb8))
        }
    }
}

/// Test support: a deterministic in-memory response queue.
#[cfg(test)]
type RecordedCall = (String, Option<String>, Option<String>);

#[cfg(test)]
pub(crate) struct FakeTransport {
    responses: std::sync::Mutex<std::collections::VecDeque<HttpResponse>>,
    calls: std::sync::Mutex<Vec<RecordedCall>>,
}

#[cfg(test)]
impl FakeTransport {
    pub(crate) fn new(responses: Vec<HttpResponse>) -> Self {
        Self {
            responses: std::sync::Mutex::new(responses.into()),
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn calls(&self) -> Vec<RecordedCall> {
        self.calls.lock().unwrap().clone()
    }
}

#[cfg(test)]
impl Transport for FakeTransport {
    fn get(
        &self,
        url: &str,
        etag: Option<&str>,
        last_modified: Option<&str>,
    ) -> Result<HttpResponse> {
        self.calls.lock().unwrap().push((
            url.to_string(),
            etag.map(str::to_string),
            last_modified.map(str::to_string),
        ));
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| invalid("no_fake_response"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::installer::init_atomic;
    use std::io;
    use tempfile::TempDir;

    fn hosts() -> Vec<String> {
        vec!["download.example.com".to_string()]
    }

    fn data(bytes: &[u8]) -> Vec<u8> {
        [ZIP_MAGIC_LOCAL.as_slice(), bytes].concat()
    }

    fn response(status: u16, body: Vec<u8>, content_length: Option<u64>) -> HttpResponse {
        HttpResponse {
            status,
            location: None,
            content_type: Some("application/octet-stream".to_string()),
            content_length,
            etag: None,
            last_modified: None,
            final_url: "https://download.example.com/a.zip".to_string(),
            body: Box::new(io::Cursor::new(body)),
        }
    }

    fn setup() -> (TempDir, InstallerPaths) {
        let temp = tempfile::tempdir().unwrap();
        let paths = InstallerPaths::new(temp.path());
        init_atomic(&paths).unwrap();
        (temp, paths)
    }

    fn request<'a>(url: &'a str, hosts: &'a [String]) -> DownloadRequest<'a> {
        DownloadRequest {
            url,
            download_hosts: hosts,
            expected_sha256: None,
            expected_size: None,
        }
    }

    #[test]
    fn public_dns_policy_rejects_reserved_and_transition_ranges() {
        for address in [
            "0.0.0.0",
            "10.1.2.3",
            "127.0.0.1",
            "100.64.0.1",
            "169.254.1.2",
            "172.31.1.2",
            "192.168.1.1",
            "192.0.2.1",
            "198.18.1.1",
            "198.51.100.1",
            "203.0.113.1",
            "224.1.1.1",
            "::1",
            "::ffff:8.8.8.8",
            "fc00::1",
            "fe80::1",
            "2001:db8::1",
            "2002:0808:0808::1",
        ] {
            assert!(!public_ip(address.parse().unwrap()), "{address}");
        }
        for address in ["8.8.8.8", "1.1.1.1", "2606:4700:4700::1111"] {
            assert!(public_ip(address.parse().unwrap()));
        }
    }
    #[test]
    fn exe_magic_and_redirect_policy_are_bound() {
        let (_temp, paths) = setup();
        let hosts = hosts();
        let req = request("https://download.example.com/a.exe", &hosts);
        let fake = FakeTransport::new(vec![response(200, b"MZ inert SF fixture".to_vec(), None)]);
        // Response URL must equal the revalidated requested hop.
        assert!(acquire_exe_with(&fake, &req, &paths, &AtomicBool::new(false), |_, _| {}).is_err());
        let mut r = response(200, b"MZ inert SF fixture".to_vec(), None);
        r.final_url = req.url.into();
        let fake = FakeTransport::new(vec![r]);
        let archive =
            acquire_exe_with(&fake, &req, &paths, &AtomicBool::new(false), |_, _| {}).unwrap();
        assert_eq!(archive.path.extension().unwrap(), "exe");
        let mut r = response(200, b"PK\x03\x04fixture".to_vec(), None);
        r.final_url = req.url.into();
        let fake = FakeTransport::new(vec![r]);
        assert!(acquire_exe_with(&fake, &req, &paths, &AtomicBool::new(false), |_, _| {}).is_err());
    }
    #[test]
    fn validate_rejects_hostile_urls() {
        let hosts = hosts();
        for bad in [
            "http://download.example.com/a.zip",
            "https://other.example.com/a.zip",
            "https://user@download.example.com/a.zip",
            "https://download.example.com:8443/a.zip",
            "https://download.example.com/a.rar",
        ] {
            assert!(validate_download(bad, &hosts).is_err(), "{bad}");
        }
        assert!(validate_download("https://download.example.com/a.zip", &hosts).is_ok());
    }

    #[test]
    fn zip_uses_real_download_space_and_exe_keeps_format_limit() {
        let (_temp, paths) = setup();
        let hosts = hosts();
        let available = fs2::available_space(paths.downloads_dir()).unwrap();
        let mut zip = request("https://download.example.com/a.zip", &hosts);
        zip.expected_size = Some(available);
        let transport = FakeTransport::new(vec![response(200, data(b"small"), Some(available))]);
        let error =
            acquire_with(&transport, &zip, &paths, &AtomicBool::new(false), |_, _| {}).unwrap_err();
        assert!(error.to_string().starts_with("insufficient_space:state:"));
        assert!(!paths.downloads_dir().read_dir().unwrap().any(|entry| {
            entry
                .unwrap()
                .path()
                .extension()
                .is_some_and(|ext| ext == "partial")
        }));

        let mut exe = request("https://download.example.com/a.exe", &hosts);
        exe.expected_size = Some(MAX_EXE_BYTES + 1);
        let mut response = response(200, b"MZ".to_vec(), Some(MAX_EXE_BYTES + 1));
        response.final_url = exe.url.into();
        let transport = FakeTransport::new(vec![response]);
        assert_eq!(
            acquire_exe_with(&transport, &exe, &paths, &AtomicBool::new(false), |_, _| {})
                .unwrap_err()
                .to_string(),
            "download_size_limit"
        );
    }

    #[test]
    fn fresh_download_establishes_content_identity_and_cache() {
        let (_temp, paths) = setup();
        let hosts = hosts();
        let bytes = data(b"payload");
        let digest = format!("{:x}", Sha256::digest(&bytes));
        let mut first = response(200, bytes.clone(), Some(bytes.len() as u64));
        first.etag = Some("\"v1\"".to_string());
        let transport = FakeTransport::new(vec![first]);

        let acquired = acquire_with(
            &transport,
            &request("https://download.example.com/a.zip", &hosts),
            &paths,
            &AtomicBool::new(false),
            |_, _| {},
        )
        .unwrap();
        assert_eq!(acquired.source, Acquisition::Downloaded);
        assert_eq!(acquired.sha256, digest);
        assert!(acquired.path.is_file());
        assert_eq!(
            acquired.path.file_name().unwrap().to_string_lossy(),
            format!("{digest}.zip")
        );
        assert!(paths
            .downloads_dir()
            .join(format!("{digest}.json"))
            .is_file());
    }

    #[test]
    fn conditional_304_reuses_complete_cached_package() {
        let (_temp, paths) = setup();
        let hosts = hosts();
        let bytes = data(b"payload");
        let digest = format!("{:x}", Sha256::digest(&bytes));
        let mut first = response(200, bytes.clone(), Some(bytes.len() as u64));
        first.etag = Some("\"v1\"".to_string());
        let first_transport = FakeTransport::new(vec![first]);
        acquire_with(
            &first_transport,
            &request("https://download.example.com/a.zip", &hosts),
            &paths,
            &AtomicBool::new(false),
            |_, _| {},
        )
        .unwrap();

        // Second call: server answers 304 because of the stored validator.
        let mut not_modified = response(304, Vec::new(), None);
        not_modified.etag = Some("\"v1\"".to_string());
        let second_transport = FakeTransport::new(vec![not_modified]);
        let acquired = acquire_with(
            &second_transport,
            &request("https://download.example.com/a.zip", &hosts),
            &paths,
            &AtomicBool::new(false),
            |_, _| {},
        )
        .unwrap();
        assert_eq!(acquired.source, Acquisition::Cached);
        assert_eq!(acquired.sha256, digest);
        assert!(acquired.path.is_file());
        let calls = second_transport.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1.as_deref(), Some("\"v1\""));
    }

    #[test]
    fn changed_bytes_create_independent_content_identity() {
        let (_temp, paths) = setup();
        let hosts = hosts();
        let first_bytes = data(b"one");
        let first_digest = format!("{:x}", Sha256::digest(&first_bytes));
        let first_transport = FakeTransport::new(vec![response(
            200,
            first_bytes.clone(),
            Some(first_bytes.len() as u64),
        )]);
        acquire_with(
            &first_transport,
            &request("https://download.example.com/a.zip", &hosts),
            &paths,
            &AtomicBool::new(false),
            |_, _| {},
        )
        .unwrap();

        let second_bytes = data(b"two");
        let second_digest = format!("{:x}", Sha256::digest(&second_bytes));
        assert_ne!(first_digest, second_digest);
        let second_transport = FakeTransport::new(vec![response(
            200,
            second_bytes.clone(),
            Some(second_bytes.len() as u64),
        )]);
        let acquired = acquire_with(
            &second_transport,
            &request("https://download.example.com/a.zip", &hosts),
            &paths,
            &AtomicBool::new(false),
            |_, _| {},
        )
        .unwrap();
        assert_eq!(acquired.source, Acquisition::Downloaded);
        assert_eq!(acquired.sha256, second_digest);
        assert!(paths
            .downloads_dir()
            .join(format!("{first_digest}.zip"))
            .is_file());
        assert!(paths
            .downloads_dir()
            .join(format!("{second_digest}.zip"))
            .is_file());
    }

    #[test]
    fn redirects_are_revalidated_per_hop() {
        let (_temp, paths) = setup();
        let hosts = hosts();
        let bytes = data(b"payload");
        let mut redirect = response(302, Vec::new(), None);
        redirect.location = Some("https://evil.example.com/b.zip".to_string());
        let transport = FakeTransport::new(vec![redirect]);
        let error = acquire_with(
            &transport,
            &request("https://download.example.com/a.zip", &hosts),
            &paths,
            &AtomicBool::new(false),
            |_, _| {},
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "invalid_download_url");

        // An approved hop is followed and content identity is established.
        let mut redirect = response(302, Vec::new(), None);
        redirect.location = Some("/mirror/b.zip".to_string());
        let mut final_response = response(200, bytes.clone(), Some(bytes.len() as u64));
        final_response.final_url = "https://download.example.com/mirror/b.zip".into();
        let transport = FakeTransport::new(vec![redirect, final_response]);
        let acquired = acquire_with(
            &transport,
            &request("https://download.example.com/a.zip", &hosts),
            &paths,
            &AtomicBool::new(false),
            |_, _| {},
        )
        .unwrap();
        assert_eq!(acquired.source, Acquisition::Downloaded);
        assert_eq!(transport.calls().len(), 2);
    }

    #[test]
    fn rejects_non_zip_hash_and_size_mismatch() {
        let (_temp, paths) = setup();
        let hosts = hosts();
        let transport = FakeTransport::new(vec![response(
            200,
            b"<html>not a zip</html>".to_vec(),
            None,
        )]);
        assert_eq!(
            acquire_with(
                &transport,
                &request("https://download.example.com/a.zip", &hosts),
                &paths,
                &AtomicBool::new(false),
                |_, _| {},
            )
            .unwrap_err()
            .to_string(),
            "download_not_zip"
        );

        let bytes = data(b"payload");
        let wrong_hash = "0".repeat(64);
        let mut mismatched = request("https://download.example.com/a.zip", &hosts);
        mismatched.expected_sha256 = Some(&wrong_hash);
        let transport =
            FakeTransport::new(vec![response(200, bytes.clone(), Some(bytes.len() as u64))]);
        assert_eq!(
            acquire_with(
                &transport,
                &mismatched,
                &paths,
                &AtomicBool::new(false),
                |_, _| {},
            )
            .unwrap_err()
            .to_string(),
            "download_hash_mismatch"
        );
    }

    #[test]
    fn cancellation_leaves_no_success_cache() {
        let (_temp, paths) = setup();
        let hosts = hosts();
        let bytes = data(b"payload");
        let transport =
            FakeTransport::new(vec![response(200, bytes.clone(), Some(bytes.len() as u64))]);
        let cancel = AtomicBool::new(true);
        assert_eq!(
            acquire_with(
                &transport,
                &request("https://download.example.com/a.zip", &hosts),
                &paths,
                &cancel,
                |_, _| {},
            )
            .unwrap_err()
            .to_string(),
            "download_cancelled"
        );
        let leftovers: Vec<_> = fs::read_dir(paths.downloads_dir())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }
}
