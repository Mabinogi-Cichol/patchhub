//! Public-HTML fetching: a [`Fetcher`] trait with a real
//! [`ReqwestFetcher`] plus the [`RequestContext`] throttle / retry / redirect /
//! cookie policy ported from `discovery.py` (P2 network foundation).
//!
//! This layer never downloads patches or executes page scripts: it only reads
//! public metadata, rejects unsafe hops, and decodes the body to text.

use super::validate;
use super::{invalid, Result};
use reqwest::blocking::Client;
use reqwest::redirect::Policy;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Value used by every bounded discovery request (discovery.py `min(15, ...)`).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// Client-level default timeout for [`ReqwestFetcher`].
const CLIENT_TIMEOUT: Duration = Duration::from_secs(20);
/// Default per-host interval throttle.
const DEFAULT_INTERVAL: Duration = Duration::from_secs(1);
/// Maximum redirect hops followed inside one logical fetch.
const MAX_REDIRECTS: usize = 5;

/// A bounded request handed to a [`Fetcher`].
#[derive(Debug, Clone)]
pub struct FetchRequest {
    pub url: String,
    /// HTTP method; `"GET"` or `"POST"`.
    pub method: String,
    pub headers: Vec<(String, String)>,
    /// Request body for a body-bearing method, already encoded by the caller.
    pub body: Option<Vec<u8>>,
    /// `Content-Type` value describing [`FetchRequest::body`].
    pub content_type: Option<String>,
    pub timeout: Duration,
}

/// Internal request description carried through redirect handling so a POST's
/// method/body can be preserved (307/308) or downgraded to GET (301/302/303).
#[derive(Debug, Clone)]
struct RequestOptions {
    method: String,
    headers: Vec<(String, String)>,
    body: Option<Vec<u8>>,
    content_type: Option<String>,
}

impl RequestOptions {
    fn get(headers: Vec<(String, String)>) -> Self {
        Self {
            method: "GET".to_string(),
            headers,
            body: None,
            content_type: None,
        }
    }
}

/// A bounded response returned by a [`Fetcher`].
#[derive(Debug, Clone, Default)]
pub struct FetchResponse {
    pub status: u16,
    pub body: Vec<u8>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub final_url: String,
    pub content_type: Option<String>,
    pub content_encoding: Option<String>,
    pub location: Option<String>,
    /// All response headers, lower/upper-case preserved, for `Retry-After` etc.
    pub headers: Vec<(String, String)>,
}

impl FetchResponse {
    /// Case-insensitive header lookup.
    pub fn header(&self, name: &str) -> Option<&str> {
        header_value(&self.headers, name)
    }

    /// Decodes the body to text honouring BOM, `Content-Type` charset and
    /// `<meta charset>` sniffing.
    pub fn text(&self) -> String {
        decode_body(&self.body, self.content_type.as_deref())
    }
}

/// Upper bound on cookies held by one [`RequestContext`].
const MAX_COOKIES: usize = 128;
/// Upper bound on a single cookie name/value pair.
const MAX_COOKIE_BYTES: usize = 4096;
/// Upper bound on the serialized `Cookie` header sent on one request.
const MAX_COOKIE_HEADER_BYTES: usize = 8192;

#[derive(Debug, Clone)]
struct Cookie {
    name: String,
    value: String,
    /// Lowercase host/domain without a leading dot.
    domain: String,
    /// `true` when no `Domain` attribute was supplied (exact-host only).
    host_only: bool,
    path: String,
    secure: bool,
    expires_at: Option<Instant>,
}

/// A small bounded cookie store: captures `Set-Cookie` from responses and
/// re-sends matching `Cookie` headers on later requests, mirroring the subset of
/// `http.cookiejar.CookieJar` the discovery / source fetchers rely on.
///
/// `Expires` is intentionally not parsed (patch.moe's gate issues a session
/// cookie); only `Max-Age` (including `0`/negative deletion) affects expiry.
/// The jar never sends a cookie to a non-matching host and is capped at
/// [`MAX_COOKIES`] entries.
#[derive(Debug, Default)]
pub struct CookieJar {
    cookies: Vec<Cookie>,
}

impl CookieJar {
    /// Captures every `Set-Cookie` header on `response` for the request `url`.
    fn store_from_response(&mut self, url: &str, response: &FetchResponse) {
        let Some(host) = validate::split_url(url).hostname() else {
            return;
        };
        let request_path = request_path(url);
        for (name, value) in &response.headers {
            if name.eq_ignore_ascii_case("set-cookie") {
                self.store_set_cookie(&host, &request_path, value);
            }
        }
    }

    fn store_set_cookie(&mut self, host: &str, request_path: &str, header: &str) {
        let mut attributes = header.split(';');
        let Some(pair) = attributes.next() else {
            return;
        };
        let Some((name, value)) = pair.split_once('=') else {
            return;
        };
        let name = name.trim();
        let value = value.trim();
        if name.is_empty() || name.len() + value.len() > MAX_COOKIE_BYTES {
            return;
        }

        let mut domain_attr: Option<String> = None;
        let mut path_attr: Option<String> = None;
        let mut secure = false;
        let mut expires_at: Option<Instant> = None;
        let mut delete = false;
        for attribute in attributes {
            let attribute = attribute.trim();
            let (key, value) = match attribute.split_once('=') {
                Some((key, value)) => (key.trim(), value.trim()),
                None => (attribute, ""),
            };
            if key.eq_ignore_ascii_case("domain") && !value.is_empty() {
                domain_attr = Some(value.trim_start_matches('.').to_ascii_lowercase());
            } else if key.eq_ignore_ascii_case("path") && value.starts_with('/') {
                path_attr = Some(value.to_string());
            } else if key.eq_ignore_ascii_case("secure") {
                secure = true;
            } else if key.eq_ignore_ascii_case("max-age") {
                match value.parse::<i64>() {
                    Ok(seconds) if seconds <= 0 => delete = true,
                    Ok(seconds) => {
                        expires_at =
                            Instant::now().checked_add(Duration::from_secs(seconds as u64));
                    }
                    Err(_) => {}
                }
            }
        }

        let host_only = domain_attr.is_none();
        let domain = match domain_attr {
            Some(domain) if domain_matches(host, &domain) => domain,
            Some(_) => return, // Domain attribute does not cover the origin host.
            None => host.to_string(),
        };
        let path = path_attr.unwrap_or_else(|| default_cookie_path(request_path));
        if delete {
            self.cookies
                .retain(|cookie| !cookie_matches(cookie, name, &domain, &path));
            return;
        }
        let cookie = Cookie {
            name: name.to_string(),
            value: value.to_string(),
            domain,
            host_only,
            path,
            secure,
            expires_at,
        };
        self.cookies.retain(|existing| {
            !cookie_matches(existing, &cookie.name, &cookie.domain, &cookie.path)
        });
        if self.cookies.len() >= MAX_COOKIES {
            self.cookies.remove(0);
        }
        self.cookies.push(cookie);
    }

    /// Builds the `Cookie` header for `url`, or `None` when nothing matches.
    fn cookie_header(&self, url: &str) -> Option<String> {
        let parsed = validate::split_url(url);
        let host = parsed.hostname().unwrap_or_default();
        let path = request_path(url);
        let now = Instant::now();
        let mut header = String::new();
        for cookie in &self.cookies {
            if cookie.expires_at.is_some_and(|expires| expires <= now) {
                continue;
            }
            if cookie.secure && parsed.scheme != "https" {
                continue;
            }
            if !cookie_host_matches(&host, cookie) || !path_matches(&path, &cookie.path) {
                continue;
            }
            let pair = format!("{}={}", cookie.name, cookie.value);
            if !header.is_empty() {
                header.push_str("; ");
            }
            header.push_str(&pair);
            if header.len() > MAX_COOKIE_HEADER_BYTES {
                break;
            }
        }
        if header.is_empty() {
            None
        } else {
            Some(header)
        }
    }
}

/// Identity of a stored cookie: name + domain + path.
fn cookie_matches(cookie: &Cookie, name: &str, domain: &str, path: &str) -> bool {
    cookie.name == name && cookie.domain == domain && cookie.path == path
}

/// `domain` is lowercased and dot-stripped by the caller; `host` is either an
/// exact match or a subdomain of it.
fn domain_matches(host: &str, domain: &str) -> bool {
    host == domain
        || (host.len() > domain.len()
            && host.ends_with(domain)
            && host.as_bytes()[host.len() - domain.len() - 1] == b'.')
}

fn cookie_host_matches(host: &str, cookie: &Cookie) -> bool {
    if cookie.host_only {
        host == cookie.domain
    } else {
        domain_matches(host, &cookie.domain)
    }
}

/// RFC 6265 default-path: the request path up to (not including) its rightmost
/// `/`, or `/` when there is nothing to strip.
fn default_cookie_path(request_path: &str) -> String {
    if !request_path.starts_with('/') || request_path.matches('/').count() <= 1 {
        return "/".to_string();
    }
    match request_path.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(index) => request_path[..index].to_string(),
    }
}

fn path_matches(request_path: &str, cookie_path: &str) -> bool {
    request_path == cookie_path
        || (request_path.starts_with(cookie_path)
            && (cookie_path.ends_with('/')
                || request_path.as_bytes().get(cookie_path.len()) == Some(&b'/')))
}

fn request_path(url: &str) -> String {
    let path = validate::split_url(url).path;
    if path.is_empty() {
        "/".to_string()
    } else {
        path
    }
}

/// `application/x-www-form-urlencoded` encoding matching Python `urlencode`
/// (`quote_plus` safe set `A-Za-z0-9_.-~`, space -> `+`).
fn encode_form(fields: &[(String, String)]) -> String {
    let mut encoded = String::new();
    for (index, (name, value)) in fields.iter().enumerate() {
        if index > 0 {
            encoded.push('&');
        }
        encode_form_component(&mut encoded, name);
        encoded.push('=');
        encode_form_component(&mut encoded, value);
    }
    encoded
}

fn encode_form_component(encoded: &mut String, value: &str) {
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' => {
                encoded.push(*byte as char);
            }
            b' ' => encoded.push('+'),
            other => {
                encoded.push('%');
                encoded.push_str(&format!("{:02X}", *other));
            }
        }
    }
}

/// A network transport. Implementations must not follow redirects; the
/// [`RequestContext`] validates and follows each hop itself.
pub trait Fetcher: Send + Sync {
    fn fetch(&self, request: &FetchRequest) -> Result<FetchResponse>;
}

/// Real HTTPS fetcher: no auto-redirect, bounded body, charset-aware decoding.
pub struct ReqwestFetcher {
    client: Client,
}

impl ReqwestFetcher {
    pub fn new() -> Result<Self> {
        let client = Client::builder()
            .redirect(Policy::none())
            .timeout(CLIENT_TIMEOUT)
            .user_agent("PatchHubDiscovery/1")
            .build()
            .map_err(|_| invalid("discovery_client_failed"))?;
        Ok(Self { client })
    }
}

impl Fetcher for ReqwestFetcher {
    fn fetch(&self, request: &FetchRequest) -> Result<FetchResponse> {
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|_| invalid("discovery_method_invalid"))?;
        let mut builder = self
            .client
            .request(method, &request.url)
            .timeout(request.timeout);
        for (name, value) in &request.headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
        if let Some(content_type) = &request.content_type {
            builder = builder.header(reqwest::header::CONTENT_TYPE, content_type.as_str());
        }
        if let Some(body) = &request.body {
            builder = builder.body(body.clone());
        }
        let mut response = builder
            .send()
            .map_err(|_| invalid("discovery_network_error"))?;
        let status = response.status().as_u16();
        let final_url = response.url().to_string();
        let headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_string(),
                    String::from_utf8_lossy(value.as_bytes()).into_owned(),
                )
            })
            .collect();
        let etag = header_value(&headers, "etag").map(str::to_string);
        let last_modified = header_value(&headers, "last-modified").map(str::to_string);
        let content_type = header_value(&headers, "content-type").map(str::to_string);
        let content_encoding = header_value(&headers, "content-encoding").map(str::to_string);
        let location = header_value(&headers, "location").map(str::to_string);

        // Full body read: no fixed byte cap, so a large legitimate page is
        // decoded and parsed completely instead of being rejected.
        let mut body = Vec::new();
        read_body_checked(&mut response, &mut body)?;
        Ok(FetchResponse {
            status,
            body,
            etag,
            last_modified,
            final_url,
            content_type,
            content_encoding,
            location,
            headers,
        })
    }
}

/// Reads the full body in fixed-size chunks, checking for cancellation before
/// each chunk so a large transfer stays interruptible. No size cap is applied.
fn read_body_checked(reader: &mut impl Read, body: &mut Vec<u8>) -> Result<()> {
    let mut buffer = [0u8; 65536];
    loop {
        crate::operation::check()?;
        let count = reader
            .read(&mut buffer)
            .map_err(|_| invalid("discovery_network_error"))?;
        if count == 0 {
            return Ok(());
        }
        body.extend_from_slice(&buffer[..count]);
    }
}

/// In-memory transport for offline tests: queues canned responses and records
/// every request. Wrap in `Arc` and pass `Box::new(Arc::clone(&fake))` to a
/// [`RequestContext`].
#[derive(Default)]
pub struct FakeFetcher {
    queue: Mutex<VecDeque<std::result::Result<FetchResponse, String>>>,
    calls: Mutex<Vec<FetchRequest>>,
}

impl FakeFetcher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&self, response: FetchResponse) {
        self.queue
            .lock()
            .expect("fake fetcher queue")
            .push_back(Ok(response));
    }

    pub fn push_error(&self, code: &str) {
        self.queue
            .lock()
            .expect("fake fetcher queue")
            .push_back(Err(code.to_string()));
    }

    pub fn calls(&self) -> Vec<FetchRequest> {
        self.calls.lock().expect("fake fetcher calls").clone()
    }

    pub fn call_count(&self) -> usize {
        self.calls.lock().expect("fake fetcher calls").len()
    }
}

impl Fetcher for Arc<FakeFetcher> {
    fn fetch(&self, request: &FetchRequest) -> Result<FetchResponse> {
        self.calls
            .lock()
            .expect("fake fetcher calls")
            .push(request.clone());
        match self.queue.lock().expect("fake fetcher queue").pop_front() {
            Some(Ok(response)) => Ok(response),
            Some(Err(code)) => Err(invalid(code)),
            None => Err(invalid("discovery_fake_empty")),
        }
    }
}

/// One game's bounded request context: per-host throttle, response cache,
/// visited-URL dedup, cookie jar and transient-retry policy (discovery.py
/// `RequestContext`; design §6.2/§9).
pub struct RequestContext {
    fetcher: Box<dyn Fetcher>,
    pub interval: Duration,
    pub visited: HashSet<String>,
    pub responses: HashMap<String, FetchResponse>,
    pub last_request: HashMap<String, Instant>,
    /// Cookies captured from responses and re-sent to matching hosts.
    pub cookies: CookieJar,
}

impl RequestContext {
    /// Builds a context with the real HTTPS fetcher (1s throttle).
    pub fn new() -> Result<Self> {
        let fetcher = Box::new(ReqwestFetcher::new()?);
        Ok(Self::with_settings(fetcher, DEFAULT_INTERVAL))
    }

    /// Builds a context with an injected fetcher (offline / fixture use).
    pub fn with_fetcher(fetcher: Box<dyn Fetcher>) -> Self {
        Self::with_settings(fetcher, DEFAULT_INTERVAL)
    }

    /// Full constructor controlling the per-host interval.
    pub fn with_settings(fetcher: Box<dyn Fetcher>, interval: Duration) -> Self {
        Self {
            fetcher,
            interval,
            visited: HashSet::new(),
            responses: HashMap::new(),
            last_request: HashMap::new(),
            cookies: CookieJar::default(),
        }
    }

    /// Fetches `url`, following validated redirect hops. Returns the final
    /// response for any non-transient status (e.g. 304) so adapters can handle
    /// conditional caching; only transport/transient failures error.
    pub fn fetch(&mut self, url: &str, hosts: &[String]) -> Result<FetchResponse> {
        self.fetch_with(url, hosts, RequestOptions::get(Vec::new()))
    }

    /// Conditional variant that forwards `If-None-Match` / `If-Modified-Since`.
    pub fn fetch_conditional(
        &mut self,
        url: &str,
        hosts: &[String],
        etag: Option<&str>,
        modified: Option<&str>,
    ) -> Result<FetchResponse> {
        let mut headers = Vec::new();
        if let Some(etag) = etag {
            headers.push(("If-None-Match".to_string(), etag.to_string()));
        }
        if let Some(modified) = modified {
            headers.push(("If-Modified-Since".to_string(), modified.to_string()));
        }
        self.fetch_with(url, hosts, RequestOptions::get(headers))
    }

    /// Issues a `POST` with `body`, following validated redirect hops and
    /// reusing the context cookie jar.
    pub fn fetch_post(
        &mut self,
        url: &str,
        hosts: &[String],
        content_type: Option<&str>,
        body: Vec<u8>,
    ) -> Result<FetchResponse> {
        self.fetch_with(
            url,
            hosts,
            RequestOptions {
                method: "POST".to_string(),
                headers: Vec::new(),
                body: Some(body),
                content_type: content_type.map(str::to_string),
            },
        )
    }

    /// Submits an `application/x-www-form-urlencoded` form body via `POST`.
    pub fn fetch_form(
        &mut self,
        url: &str,
        hosts: &[String],
        fields: &[(String, String)],
    ) -> Result<FetchResponse> {
        self.fetch_post(
            url,
            hosts,
            Some("application/x-www-form-urlencoded"),
            encode_form(fields).into_bytes(),
        )
    }

    /// Fetches and requires a 200 HTML body: challenge detection only.
    pub fn fetch_html(&mut self, url: &str, hosts: &[String]) -> Result<String> {
        let response = self.fetch(url, hosts)?;
        if response.status != 200 {
            return Err(invalid(format!("discovery_http_{}", response.status)));
        }
        let text = response.text();
        if is_challenge(&text) {
            return Err(invalid("discovery_challenge"));
        }
        Ok(text)
    }

    fn fetch_with(
        &mut self,
        url: &str,
        hosts: &[String],
        options: RequestOptions,
    ) -> Result<FetchResponse> {
        crate::operation::check()?;
        // Validate the initial URL before any cache lookup or network request
        // (discovery.py `RequestContext.fetch` -> `safe_url(url, hosts)`).
        discovery_safe_url(url, Some(hosts))?;

        let RequestOptions {
            mut method,
            headers,
            mut body,
            mut content_type,
        } = options;
        let is_get = method == "GET";
        if is_get {
            if let Some(cached) = self.responses.get(url) {
                return Ok(cached.clone());
            }
            if self.visited.contains(url) {
                return Err(invalid("discovery_request_cycle"));
            }
            self.visited.insert(url.to_string());
        }

        let mut current = url.to_string();
        for _ in 0..=MAX_REDIRECTS {
            let response = self.request_once(
                &current,
                &headers,
                &method,
                body.as_deref(),
                content_type.as_deref(),
            )?;
            if is_redirect(response.status) {
                let location = response
                    .location
                    .clone()
                    .ok_or_else(|| invalid("discovery_redirect_missing"))?;
                let next = join_url(&current, &location)?;
                // Reject every hop that leaves the rule hosts before issuing the
                // next request (discovery.py `_Redirect.redirect_request` ->
                // `safe_url(newurl, hosts)`). This also rejects the real
                // kaguragames.com -> kaguragamesjp.com cross-host redirect.
                discovery_safe_url(&next, Some(hosts))?;
                // 301/302/303 downgrade a POST to a GET and drop the body
                // (urllib `HTTPRedirectHandler`); 307/308 preserve them.
                if method != "GET" && matches!(response.status, 301..=303) {
                    method = "GET".to_string();
                    body = None;
                    content_type = None;
                }
                current = next;
                continue;
            }
            if is_get {
                self.responses.insert(url.to_string(), response.clone());
            }
            return Ok(response);
        }
        Err(invalid("discovery_too_many_redirects"))
    }

    /// One logical request (throttle + transport retry), no redirects.
    fn request_once(
        &mut self,
        url: &str,
        extra: &[(String, String)],
        method: &str,
        body: Option<&[u8]>,
        content_type: Option<&str>,
    ) -> Result<FetchResponse> {
        crate::operation::check()?;
        let host = validate::split_url(url).hostname().unwrap_or_default();
        let wait = self
            .last_request
            .get(&host)
            .map(|last| self.interval.saturating_sub(last.elapsed()))
            .unwrap_or(Duration::ZERO);
        if !wait.is_zero() {
            sleep_interruptible(wait)?;
        }
        self.last_request.insert(host.clone(), Instant::now());

        let mut headers: Vec<(String, String)> =
            vec![("Accept".to_string(), "text/html".to_string())];
        headers.extend_from_slice(extra);
        if let Some(cookie) = self.cookies.cookie_header(url) {
            headers.push(("Cookie".to_string(), cookie));
        }

        for attempt in 0..2 {
            crate::operation::check()?;
            let request = FetchRequest {
                url: url.to_string(),
                method: method.to_string(),
                headers: headers.clone(),
                body: body.map(|bytes| bytes.to_vec()),
                content_type: content_type.map(str::to_string),
                timeout: REQUEST_TIMEOUT,
            };
            let result = self.fetcher.fetch(&request);
            match result {
                Ok(response) if !is_transient(response.status) => {
                    self.cookies.store_from_response(url, &response);
                    check_encoding(&response)?;
                    return Ok(response);
                }
                Ok(response) => {
                    self.cookies.store_from_response(url, &response);
                    if attempt == 1 {
                        return Err(invalid("discovery_retry_exhausted"));
                    }
                    let retry_after = response.header("retry-after").unwrap_or("1");
                    let wait = self.interval.max(parse_retry_after(retry_after));
                    sleep_interruptible(wait)?;
                    self.last_request.insert(host.clone(), Instant::now());
                }
                Err(error) => {
                    if attempt == 1 {
                        return Err(error);
                    }
                    let wait = self.interval.max(Duration::from_secs(1));
                    sleep_interruptible(wait)?;
                    self.last_request.insert(host.clone(), Instant::now());
                }
            }
        }
        Err(invalid("discovery_retry_exhausted"))
    }
}

fn is_transient(status: u16) -> bool {
    matches!(status, 429 | 500 | 502 | 503 | 504)
}

fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

/// Port of `discovery.py::safe_url` for the discovery/source navigation layer.
///
/// Unlike `validate::allowed_url` (which returns `invalid_source_*` and backs
/// the ZIP/source validation path), this returns the discovery error codes
/// `invalid_discovery_url` / `invalid_discovery_host` and enforces the same
/// host policy as `_Redirect` when `hosts` is supplied.
fn discovery_safe_url(url: &str, hosts: Option<&[String]>) -> Result<String> {
    if url.is_empty() || url != url.trim() || url.chars().any(|c| (c as u32) < 33) {
        return Err(invalid("invalid_discovery_url"));
    }
    let parsed = validate::split_url(url);
    let host = parsed.hostname().unwrap_or_default();
    // `ipaddress.ip_address` on the parsed hostname; IPv4 dotted-quad and
    // bracketed IPv6 are both rejected as literal hosts.
    if host.contains(':') || is_ipv4_literal(&host) {
        return Err(invalid("invalid_discovery_host"));
    }
    let port = parsed
        .port()
        .map_err(|_| invalid("invalid_discovery_url"))?;
    let valid_host = !host.is_empty()
        && host
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
        && host.contains('.')
        && !host.ends_with(".local")
        && !host.ends_with(".localhost")
        && !host.ends_with('.')
        && !host.contains("..")
        && host.bytes().any(|b| b.is_ascii_lowercase());
    let credentials = parsed.username().is_some_and(|user| !user.is_empty())
        || parsed
            .password()
            .is_some_and(|password| !password.is_empty());
    let host_allowed = hosts.is_none_or(|list| list.iter().any(|allowed| allowed == &host));
    if parsed.scheme != "https"
        || !valid_host
        || credentials
        || !matches!(port, None | Some(443))
        || !host_allowed
    {
        return Err(invalid("invalid_discovery_url"));
    }
    Ok(url.to_string())
}

/// Mirrors `ipaddress.ip_address` for the strings that reach a dotted-quad
/// hostname: a leading-zero octet is not a valid IPv4 address.
fn is_ipv4_literal(host: &str) -> bool {
    let parts: Vec<&str> = host.split('.').collect();
    parts.len() == 4
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.len() <= 3
                && part.bytes().all(|b| b.is_ascii_digit())
                && (part.len() == 1 || !part.starts_with('0'))
                && part.parse::<u16>().is_ok_and(|octet| octet <= 255)
        })
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn check_encoding(response: &FetchResponse) -> Result<()> {
    if let Some(encoding) = &response.content_encoding {
        let normalized = encoding.trim().to_ascii_lowercase();
        if !matches!(normalized.as_str(), "" | "identity" | "gzip" | "deflate") {
            return Err(invalid("discovery_compressed_response"));
        }
    }
    Ok(())
}

fn parse_retry_after(value: &str) -> Duration {
    if let Ok(seconds) = value.trim().parse::<f64>() {
        if seconds.is_finite() && seconds >= 0.0 {
            // Cap to a value guaranteed representable.
            return Duration::from_secs_f64(seconds.min(1_000_000_000.0));
        }
    }
    // HTTP-date forms fall back to the discovery.py default of one second.
    Duration::from_secs(1)
}

fn sleep_interruptible(wait: Duration) -> Result<()> {
    crate::operation::check()?;
    std::thread::sleep(wait);
    crate::operation::check()
}

fn join_url(base: &str, reference: &str) -> Result<String> {
    let base = url::Url::parse(base).map_err(|_| invalid("discovery_redirect_url"))?;
    let joined = base
        .join(reference)
        .map_err(|_| invalid("discovery_redirect_url"))?;
    Ok(joined.to_string())
}

/// Case-insensitive challenge / interstitial title detection.
fn is_challenge(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    let mut search = 0;
    while let Some(offset) = lower[search..].find("<title") {
        let after_tag = search + offset + "<title".len();
        let Some(gt) = lower[after_tag..].find('>') else {
            return false;
        };
        let content = after_tag + gt + 1;
        let trimmed = lower[content..].trim_start();
        if trimmed.starts_with("just a moment")
            || trimmed.starts_with("access denied")
            || trimmed.starts_with("captcha")
        {
            return true;
        }
        search = content;
    }
    false
}

/// Decodes bytes to text using BOM, then `Content-Type` charset, then
/// `<meta charset>` sniffing; UTF-8 is the final fallback.
pub fn decode_body(body: &[u8], content_type: Option<&str>) -> String {
    if let Some((encoding, bom_len)) = encoding_rs::Encoding::for_bom(body) {
        return encoding.decode(&body[bom_len..]).0.into_owned();
    }
    let label = content_type
        .and_then(charset_from_content_type)
        .or_else(|| sniff_meta_charset(body));
    let encoding = label
        .as_deref()
        .and_then(|label| encoding_rs::Encoding::for_label(label.as_bytes()))
        .unwrap_or(encoding_rs::UTF_8);
    encoding.decode(body).0.into_owned()
}

fn charset_from_content_type(content_type: &str) -> Option<String> {
    let lower = content_type.to_ascii_lowercase();
    let index = lower.find("charset")?;
    let after = content_type[index + "charset".len()..].trim_start();
    let after = after.strip_prefix('=')?.trim_start();
    let after = after.trim_start_matches(['"', '\'']);
    let end = after
        .find([';', '"', '\'', ' ', '\t'])
        .unwrap_or(after.len());
    let label = after[..end].trim();
    if label.is_empty() {
        None
    } else {
        Some(label.to_string())
    }
}

fn sniff_meta_charset(body: &[u8]) -> Option<String> {
    let window = &body[..body.len().min(1024)];
    let lower = String::from_utf8_lossy(window).to_ascii_lowercase();
    for needle in ["<meta charset", "charset="] {
        let Some(index) = lower.find(needle) else {
            continue;
        };
        let after = &lower[index..];
        let Some(eq) = after.find('=') else {
            continue;
        };
        let value = after[eq + 1..].trim_start();
        let value = value.trim_start_matches(['"', '\'']);
        let end = value
            .find(['"', '\'', ';', '>', ' ', '\t', '\n', '\r'])
            .unwrap_or(value.len());
        let label = value[..end].trim();
        if !label.is_empty() {
            return Some(label.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn new_fake() -> Arc<FakeFetcher> {
        Arc::new(FakeFetcher::new())
    }

    fn hosts() -> Vec<String> {
        vec!["example.com".to_string()]
    }

    fn ok(status: u16, body: &[u8]) -> FetchResponse {
        FetchResponse {
            status,
            body: body.to_vec(),
            final_url: "https://example.com/".to_string(),
            ..FetchResponse::default()
        }
    }

    fn context(fake: &Arc<FakeFetcher>, interval: Duration) -> RequestContext {
        RequestContext::with_settings(Box::new(Arc::clone(fake)), interval)
    }

    #[test]
    fn sequential_requests_are_not_capped() {
        let fake = new_fake();
        fake.push(ok(200, b"a"));
        fake.push(ok(200, b"b"));
        let mut ctx = context(&fake, Duration::ZERO);
        assert_eq!(
            ctx.fetch("https://example.com/a", &hosts()).unwrap().status,
            200
        );
        assert_eq!(
            ctx.fetch("https://example.com/b", &hosts()).unwrap().status,
            200
        );
        assert_eq!(fake.call_count(), 2);
    }

    #[test]
    fn request_without_deadline_still_calls_network() {
        let fake = new_fake();
        fake.push(ok(200, b"a"));
        let mut ctx = context(&fake, Duration::ZERO);
        assert_eq!(
            ctx.fetch("https://example.com/a", &hosts()).unwrap().status,
            200
        );
        assert_eq!(fake.call_count(), 1);
    }

    #[test]
    fn per_host_throttle_delays_back_to_back_requests() {
        let fake = new_fake();
        fake.push(ok(200, b"a"));
        fake.push(ok(200, b"b"));
        let interval = Duration::from_millis(120);
        let mut ctx = context(&fake, interval);
        let start = Instant::now();
        ctx.fetch("https://example.com/a", &hosts()).unwrap();
        ctx.fetch("https://example.com/b", &hosts()).unwrap();
        assert!(start.elapsed() >= interval);
    }

    #[test]
    fn transient_retry_honours_retry_after_zero() {
        let fake = new_fake();
        let mut throttled = ok(503, b"");
        throttled
            .headers
            .push(("Retry-After".to_string(), "0".to_string()));
        fake.push(throttled);
        fake.push(ok(200, b"page"));
        let mut ctx = context(&fake, Duration::ZERO);
        let response = ctx.fetch("https://example.com/", &hosts()).unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(fake.call_count(), 2);
    }

    #[test]
    fn transient_retry_exhausts_after_two_attempts() {
        let fake = new_fake();
        for _ in 0..2 {
            let mut throttled = ok(503, b"");
            throttled
                .headers
                .push(("Retry-After".to_string(), "0".to_string()));
            fake.push(throttled);
        }
        let mut ctx = context(&fake, Duration::ZERO);
        assert_eq!(
            ctx.fetch("https://example.com/", &hosts())
                .unwrap_err()
                .to_string(),
            "discovery_retry_exhausted"
        );
        assert_eq!(fake.call_count(), 2);
    }

    #[test]
    fn retry_after_without_deadline_retries_instead_of_deferring() {
        let fake = new_fake();
        let mut throttled = ok(503, b"");
        throttled
            .headers
            .push(("Retry-After".to_string(), "0.02".to_string()));
        fake.push(throttled);
        fake.push(ok(200, b"page"));
        let mut ctx = context(&fake, Duration::ZERO);
        let response = ctx.fetch("https://example.com/", &hosts()).unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(fake.call_count(), 2);
    }

    #[test]
    fn conditional_304_passes_through_without_retry() {
        let fake = new_fake();
        fake.push(ok(304, b""));
        let mut ctx = context(&fake, Duration::ZERO);
        let response = ctx
            .fetch_conditional("https://example.com/", &hosts(), Some("\"etag\""), None)
            .unwrap();
        assert_eq!(response.status, 304);
        assert_eq!(fake.call_count(), 1);
        assert!(fake.calls()[0]
            .headers
            .iter()
            .any(|(name, value)| name == "If-None-Match" && value == "\"etag\""));
    }

    #[test]
    fn large_body_is_processed_in_full() {
        // Raw body and its decoded text are both above the old 4 MiB cap; the
        // full body must be read and decoded, not rejected.
        let size = 4 * 1024 * 1024 + 4096;
        assert!(size > 4 * 1024 * 1024);
        let fake = new_fake();
        fake.push(FetchResponse {
            status: 200,
            body: vec![b'a'; size],
            final_url: "https://example.com/".to_string(),
            ..FetchResponse::default()
        });
        let mut ctx = context(&fake, Duration::ZERO);
        let text = ctx.fetch_html("https://example.com/", &hosts()).unwrap();
        assert_eq!(text.len(), size);
        assert!(text.len() > 4 * 1024 * 1024);
        assert!(text.bytes().all(|byte| byte == b'a'));
    }

    #[test]
    fn unsupported_content_encoding_is_rejected() {
        let fake = new_fake();
        let mut compressed = ok(200, b"x");
        compressed.content_encoding = Some("br".to_string());
        fake.push(compressed);
        let mut ctx = context(&fake, Duration::ZERO);
        assert_eq!(
            ctx.fetch("https://example.com/", &hosts())
                .unwrap_err()
                .to_string(),
            "discovery_compressed_response"
        );
    }

    #[test]
    fn redirect_hops_are_revalidated() {
        let fake = new_fake();
        let mut redirect = ok(302, b"");
        redirect.location = Some("https://10.0.0.1/x".to_string());
        fake.push(redirect);
        let mut ctx = context(&fake, Duration::ZERO);
        assert_eq!(
            ctx.fetch("https://example.com/start", &hosts())
                .unwrap_err()
                .to_string(),
            "invalid_discovery_host"
        );

        let fake2 = new_fake();
        let mut redirect = ok(302, b"");
        redirect.location = Some("https://user@example.com/x".to_string());
        fake2.push(redirect);
        let mut ctx = context(&fake2, Duration::ZERO);
        assert_eq!(
            ctx.fetch("https://example.com/start", &hosts())
                .unwrap_err()
                .to_string(),
            "invalid_discovery_url"
        );
    }

    #[test]
    fn safe_redirect_is_followed_and_final_host_enforced() {
        let fake = new_fake();
        let mut redirect = ok(302, b"");
        redirect.location = Some("https://example.com/next".to_string());
        fake.push(redirect);
        fake.push(ok(200, b"final"));
        let mut ctx = context(&fake, Duration::ZERO);
        assert_eq!(
            ctx.fetch("https://example.com/start", &hosts())
                .unwrap()
                .status,
            200
        );
        assert_eq!(fake.call_count(), 2);

        // Final host outside page_hosts is rejected even though the hop was safe.
        let fake2 = new_fake();
        let mut redirect = ok(302, b"");
        redirect.location = Some("https://other.example/next".to_string());
        fake2.push(redirect);
        fake2.push(ok(200, b"final"));
        let mut ctx = context(&fake2, Duration::ZERO);
        assert!(ctx.fetch("https://example.com/start", &hosts()).is_err());
    }

    #[test]
    fn cancellation_aborts_before_any_request() {
        let fake = new_fake();
        let mut ctx = context(&fake, Duration::ZERO);
        let cancel = Arc::new(AtomicBool::new(true));
        let result = crate::operation::run(
            Arc::clone(&cancel),
            |_| {},
            || ctx.fetch("https://example.com/", &hosts()),
        );
        assert!(result.is_err());
        assert_eq!(fake.call_count(), 0);
    }

    #[test]
    fn cookie_is_captured_and_resent_across_a_redirect() {
        let fake = new_fake();
        let mut redirect = ok(302, b"");
        redirect.location = Some("https://example.com/gate".to_string());
        redirect.headers.push((
            "Set-Cookie".to_string(),
            "PHPSESSID=abc; Path=/".to_string(),
        ));
        fake.push(redirect);
        fake.push(ok(200, b"gate"));

        let mut ctx = context(&fake, Duration::ZERO);
        ctx.fetch("https://example.com/start", &hosts()).unwrap();

        let calls = fake.calls();
        assert_eq!(calls.len(), 2);
        assert!(!calls[0].headers.iter().any(|(name, _)| name == "Cookie"));
        assert!(calls[1]
            .headers
            .iter()
            .any(|(name, value)| name == "Cookie" && value == "PHPSESSID=abc"));
    }

    #[test]
    fn cookie_persists_across_requests_and_is_host_scoped() {
        let fake = new_fake();
        let mut first = ok(200, b"a");
        first.headers.push((
            "Set-Cookie".to_string(),
            "PHPSESSID=abc; Path=/".to_string(),
        ));
        fake.push(first);
        fake.push(ok(200, b"b"));
        fake.push(ok(200, b"c"));

        let hosts = vec!["example.com".to_string(), "other.example".to_string()];
        let mut ctx = context(&fake, Duration::ZERO);
        ctx.fetch("https://example.com/a", &hosts).unwrap();
        ctx.fetch("https://example.com/b", &hosts).unwrap();
        ctx.fetch("https://other.example/c", &hosts).unwrap();

        let calls = fake.calls();
        assert!(calls[1]
            .headers
            .iter()
            .any(|(name, value)| name == "Cookie" && value == "PHPSESSID=abc"));
        assert!(!calls[2].headers.iter().any(|(name, _)| name == "Cookie"));
    }

    #[test]
    fn max_age_zero_deletes_cookie() {
        let fake = new_fake();
        let mut first = ok(200, b"a");
        first.headers.push((
            "Set-Cookie".to_string(),
            "PHPSESSID=abc; Path=/".to_string(),
        ));
        fake.push(first);
        let mut second = ok(200, b"b");
        second.headers.push((
            "Set-Cookie".to_string(),
            "PHPSESSID=; Max-Age=0; Path=/".to_string(),
        ));
        fake.push(second);
        fake.push(ok(200, b"c"));

        let mut ctx = context(&fake, Duration::ZERO);
        ctx.fetch("https://example.com/a", &hosts()).unwrap();
        ctx.fetch("https://example.com/b", &hosts()).unwrap();
        ctx.fetch("https://example.com/c", &hosts()).unwrap();

        let calls = fake.calls();
        assert!(calls[1]
            .headers
            .iter()
            .any(|(name, value)| name == "Cookie" && value == "PHPSESSID=abc"));
        assert!(!calls[2].headers.iter().any(|(name, _)| name == "Cookie"));
    }

    #[test]
    fn post_request_carries_method_body_and_content_type() {
        let fake = new_fake();
        fake.push(ok(200, b"localized"));
        let mut ctx = context(&fake, Duration::ZERO);
        ctx.fetch_form(
            "https://example.com/zh",
            &hosts(),
            &[("lang".to_string(), "zh-CN".to_string())],
        )
        .unwrap();

        let call = &fake.calls()[0];
        assert_eq!(call.method, "POST");
        assert_eq!(
            call.content_type.as_deref(),
            Some("application/x-www-form-urlencoded")
        );
        assert_eq!(call.body.as_deref(), Some(&b"lang=zh-CN"[..]));
    }

    #[test]
    fn post_redirect_downgrades_to_get() {
        let fake = new_fake();
        let mut redirect = ok(302, b"");
        redirect.location = Some("https://example.com/after".to_string());
        fake.push(redirect);
        fake.push(ok(200, b"done"));

        let mut ctx = context(&fake, Duration::ZERO);
        ctx.fetch_form(
            "https://example.com/zh",
            &hosts(),
            &[("lang".to_string(), "zh-CN".to_string())],
        )
        .unwrap();

        let calls = fake.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].method, "POST");
        assert_eq!(calls[1].method, "GET");
        assert!(calls[1].body.is_none());
    }

    #[test]
    fn cross_host_redirect_is_rejected_before_request() {
        let fake = new_fake();
        let mut redirect = ok(302, b"");
        redirect.location = Some("https://kaguragamesjp.com/product/x/".to_string());
        fake.push(redirect);
        fake.push(ok(200, b"should not be fetched"));

        let hosts = vec!["kaguragames.com".to_string()];
        let mut ctx = context(&fake, Duration::ZERO);
        assert_eq!(
            ctx.fetch("https://kaguragames.com/product/x/", &hosts)
                .unwrap_err()
                .to_string(),
            "invalid_discovery_url"
        );
        assert_eq!(fake.call_count(), 1);
    }

    #[test]
    fn initial_url_is_validated_before_request() {
        let fake = new_fake();
        let mut ctx = context(&fake, Duration::ZERO);
        assert_eq!(
            ctx.fetch("https://evil.example/x", &hosts())
                .unwrap_err()
                .to_string(),
            "invalid_discovery_url"
        );
        assert_eq!(
            ctx.fetch("http://example.com/x", &hosts())
                .unwrap_err()
                .to_string(),
            "invalid_discovery_url"
        );
        assert_eq!(
            ctx.fetch("https://10.0.0.1/x", &hosts())
                .unwrap_err()
                .to_string(),
            "invalid_discovery_host"
        );
        assert_eq!(fake.call_count(), 0);
    }

    #[test]
    fn challenge_titles_are_rejected() {
        for body in [
            "<html><head><title>Just a moment...</title></head></html>",
            "<html><head><title>Access denied</title></head></html>",
            "<html><head><title>Captcha</title></head></html>",
        ] {
            let fake = new_fake();
            fake.push(ok(200, body.as_bytes()));
            let mut ctx = context(&fake, Duration::ZERO);
            assert_eq!(
                ctx.fetch_html("https://example.com/", &hosts())
                    .unwrap_err()
                    .to_string(),
                "discovery_challenge"
            );
        }
    }

    #[test]
    fn charset_decoding_honours_content_type_meta_and_bom() {
        assert_eq!(
            decode_body(&[0xD6, 0xD0], Some("text/html; charset=gbk")),
            "中"
        );
        assert_eq!(decode_body(&[0xEF, 0xBB, 0xBF, b'h', b'i'], None), "hi");
        assert_eq!(decode_body(b"plain", None), "plain");
        let mut body = br#"<meta charset="gbk">"#.to_vec();
        body.extend_from_slice(&[0xD6, 0xD0]);
        assert_eq!(decode_body(&body, None), "<meta charset=\"gbk\">中");
    }

    #[test]
    fn cached_response_is_reused_without_new_request() {
        let fake = new_fake();
        fake.push(ok(200, b"page"));
        let mut ctx = context(&fake, Duration::ZERO);
        ctx.fetch("https://example.com/", &hosts()).unwrap();
        ctx.fetch("https://example.com/", &hosts()).unwrap();
        assert_eq!(fake.call_count(), 1);
    }

    #[test]
    fn request_errors_propagate_after_one_retry() {
        let fake = new_fake();
        fake.push_error("discovery_network_error");
        fake.push_error("discovery_network_error");
        let mut ctx = context(&fake, Duration::ZERO);
        assert_eq!(
            ctx.fetch("https://example.com/", &hosts())
                .unwrap_err()
                .to_string(),
            "discovery_network_error"
        );
        assert_eq!(fake.call_count(), 2);
    }

    #[test]
    fn operation_check_reports_cancellation() {
        let cancelled = Arc::new(AtomicBool::new(true));
        let result = crate::operation::run(Arc::clone(&cancelled), |_| {}, crate::operation::check);
        assert!(result.is_err());
        let result = crate::operation::run(
            Arc::new(AtomicBool::new(false)),
            |_| {},
            crate::operation::check,
        );
        assert!(result.is_ok());
    }

    /// `Read` source that yields fixed data and can flip a cancel flag on a
    /// chosen read number.
    struct ChunkedReader {
        data: Vec<u8>,
        position: usize,
        reads: usize,
        cancel_at: Option<(usize, Arc<AtomicBool>)>,
    }

    impl ChunkedReader {
        fn new(data: Vec<u8>) -> Self {
            Self {
                data,
                position: 0,
                reads: 0,
                cancel_at: None,
            }
        }

        fn cancelling(mut self, at: usize, flag: Arc<AtomicBool>) -> Self {
            self.cancel_at = Some((at, flag));
            self
        }
    }

    impl Read for ChunkedReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.reads += 1;
            if let Some((at, flag)) = &self.cancel_at {
                if self.reads == *at {
                    flag.store(true, Ordering::Relaxed);
                }
            }
            let remaining = &self.data[self.position..];
            let count = remaining.len().min(buf.len());
            buf[..count].copy_from_slice(&remaining[..count]);
            self.position += count;
            Ok(count)
        }
    }

    #[test]
    fn body_read_returns_full_body_without_operation_scope() {
        let data: Vec<u8> = (0..9 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
        let mut reader = ChunkedReader::new(data.clone());
        let mut body = Vec::new();
        read_body_checked(&mut reader, &mut body).unwrap();
        assert_eq!(body.len(), data.len());
        assert_eq!(body, data);
    }

    #[test]
    fn body_read_completes_under_active_uncancelled_scope() {
        let flag = Arc::new(AtomicBool::new(false));
        let data = vec![3u8; 200_000];
        let mut reader = ChunkedReader::new(data.clone());
        let mut body = Vec::new();
        crate::operation::with_cancel(Arc::clone(&flag), || {
            read_body_checked(&mut reader, &mut body)
        })
        .unwrap();
        assert_eq!(body, data);
    }

    #[test]
    fn body_read_stops_at_cancellation_without_partial_body() {
        let flag = Arc::new(AtomicBool::new(false));
        let data = vec![7u8; 4 * 1024 * 1024];
        let mut reader = ChunkedReader::new(data.clone()).cancelling(2, Arc::clone(&flag));
        let mut body = Vec::new();
        let error = crate::operation::with_cancel(Arc::clone(&flag), || {
            read_body_checked(&mut reader, &mut body)
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "task_cancelled");
        assert!(reader.position < data.len());
        assert!(body.len() < data.len());
    }

    #[test]
    fn retry_after_parses_seconds_and_falls_back() {
        assert_eq!(parse_retry_after("0"), Duration::ZERO);
        assert_eq!(parse_retry_after("2.5"), Duration::from_secs_f64(2.5));
        assert_eq!(
            parse_retry_after("Wed, 21 Oct 2015 07:28:00 GMT"),
            Duration::from_secs(1)
        );
        assert_eq!(parse_retry_after("-1"), Duration::from_secs(1));
    }

    #[test]
    fn transient_and_redirect_classification() {
        for status in [429, 500, 502, 503, 504] {
            assert!(is_transient(status), "{status}");
        }
        assert!(!is_transient(200));
        assert!(!is_transient(404));
        for status in [301, 302, 303, 307, 308] {
            assert!(is_redirect(status), "{status}");
        }
        assert!(!is_redirect(200));
    }
}
