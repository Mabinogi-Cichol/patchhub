"""Small URL and name primitives kept after the source engine moved to Rust.

Source policies, rule validation, subscriptions, caching and HTML discovery now
live in the shared Rust core. Only these pure helpers are still used by the
retained EXE platform bridge, install consumers and their tests.
"""
import ipaddress
import re
import unicodedata
from urllib.parse import urlsplit


def safe_url(url, hosts=None):
    """Validate a public HTTPS discovery URL, optionally restricted to hosts."""
    if not isinstance(url, str) or url != url.strip() or any(ord(c) < 33 for c in url):
        raise ValueError("invalid_discovery_url")
    p = urlsplit(url)
    try:
        port = p.port
        ipaddress.ip_address(p.hostname or "")
    except ValueError:
        port = p.port
    else:
        raise ValueError("invalid_discovery_host")
    host = p.hostname or ""
    if (p.scheme != "https" or not re.fullmatch(r"[a-z0-9.-]+", host)
            or "." not in host or host.endswith((".local", ".localhost", "."))
            or ".." in host or not re.search(r"[a-z]", host) or p.username or p.password or port not in (None, 443)
            or (hosts is not None and host not in hosts)):
        raise ValueError("invalid_discovery_url")
    return url


def allowed_url(url, hosts, *, download=False):
    """Validate an archive/page URL against a source's allowed hosts."""
    if not isinstance(url, str) or not url or url != url.strip() or any(ord(c) < 33 for c in url):
        raise ValueError("invalid_source_url")
    parsed = urlsplit(url)
    try:
        port = parsed.port
    except ValueError as error:
        raise ValueError("invalid_source_url") from error
    if (parsed.scheme != "https" or parsed.hostname not in hosts or parsed.username or parsed.password
            or port not in (None, 443) or not parsed.path.startswith("/")
            or (download and (not parsed.path.lower().endswith(".zip") or parsed.fragment))):
        raise ValueError("invalid_source_url")
    return url.strip()


def _normal(text):
    text = unicodedata.normalize("NFKC", text).casefold().replace("+", "plus")
    return "".join(c for c in text if unicodedata.category(c)[0] in {"L", "N"})


def _split_names(title):
    values = [title]
    if " / " in title:
        values.extend(x.strip() for x in title.split(" / "))
    return [x for x in values if x]
