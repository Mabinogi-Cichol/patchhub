"""Immutable bundled EXE capability rules. Discovery is never installation authority.

Only maintainer-reviewed profiles enter this registry; it has no write/import
RPC. Empty production registry is intentional until official P0/P5 approval.
All mutable inputs are reread at prepare/apply; receipts remain local to restore.
"""
import hashlib
from datetime import datetime
import json
from pathlib import Path
import re
import time
from urllib.parse import urlsplit

from acquisition import no_links
from source_primitives import safe_url

MAX_PROFILES_BYTES = 16 * 1024 * 1024
MAX_ENTRIES = 8192
PROFILE_KEYS = {"profile_id", "revision", "source_id", "source_record_id", "source_page", "appid",
                "language", "variant", "version_identity", "exe_sha256", "exe_size", "metadata_sha256",
                "installer_version", "compression", "payload", "verified_buildids", "requires_dlc",
                "excludes_dlc", "baselines", "terms", "review", "download_hosts"}
OFFICIAL_SOURCES = {"kagura-cn", "kagura-en"}


def _need(condition, code="invalid_exe_profile"):
    if not condition:
        raise ValueError(code)


def digest(value):
    return hashlib.sha256(json.dumps(value, ensure_ascii=True, sort_keys=True,
                                     separators=(",", ":")).encode()).hexdigest()


def _sha(value):
    return isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value) is not None


def _path(value):
    _need(isinstance(value, str) and 0 < len(value) <= 1024 and "\\" not in value
          and ":" not in value and "%" not in value and all(ord(c) >= 32 for c in value))
    for part in value.split("/"):
        _need(part not in ("", ".", "..") and not part.endswith((".", " "))
              and not any(c in '<>"|?*' for c in part)
              and not part.casefold().startswith(".steam-patch-"))
        device = part.split(".", 1)[0].upper()
        _need(device not in ("CON", "PRN", "AUX", "NUL") and not re.fullmatch(r"(?:COM|LPT)[0-9¹²³]", device))
    return value


def validate_profile(profile):
    _need(isinstance(profile, dict) and set(profile) == PROFILE_KEYS)
    _need(isinstance(profile["profile_id"], str) and re.fullmatch(r"[a-z][a-z0-9-]{0,63}", profile["profile_id"]))
    _need(type(profile["revision"]) is int and 0 < profile["revision"] < 2 ** 31)
    _need(profile["source_id"] in OFFICIAL_SOURCES and type(profile["appid"]) is int and profile["appid"] > 0)
    for key in ("source_record_id", "version_identity", "language"):
        _need(isinstance(profile[key], str) and 0 < len(profile[key]) <= 2048)
    _need(profile["variant"] in ("base", "dlc"))
    safe_url(profile["source_page"])
    for key in ("exe_sha256", "metadata_sha256"):
        _need(_sha(profile[key]))
    _need(type(profile["exe_size"]) is int and 0 < profile["exe_size"] <= 2 * 1024 ** 3)
    _need(profile["installer_version"] == "9.5" and profile["compression"] == "lzma2")
    _need(isinstance(profile["download_hosts"], list) and 0 < len(profile["download_hosts"]) <= 8
          and all(h in ("kaguraserver.com", "kaguragamespatch.com") for h in profile["download_hosts"]))
    _need(isinstance(profile["verified_buildids"], list) and 0 < len(profile["verified_buildids"]) <= 64
          and all(isinstance(b, str) and re.fullmatch(r"[0-9]{1,20}", b) for b in profile["verified_buildids"]))
    for key in ("requires_dlc", "excludes_dlc"):
        _need(isinstance(profile[key], list) and len(profile[key]) <= 32
              and all(type(d) is int and d > 0 for d in profile[key]) and len(set(profile[key])) == len(profile[key]))
    _need(not set(profile["requires_dlc"]) & set(profile["excludes_dlc"]))
    _need(isinstance(profile["payload"], list) and 0 < len(profile["payload"]) <= MAX_ENTRIES)
    total = 0
    for row in profile["payload"]:
        _need(isinstance(row, dict) and set(row) == {"path", "size", "sha256"})
        _path(row["path"])
        _need(type(row["size"]) is int and 0 <= row["size"] <= 1024 ** 3 and _sha(row["sha256"]))
        total += row["size"]
    _need(total <= 2 * 1024 ** 3)
    # Same Unicode/case and prefix rules as the auditor, without importing lzma
    # into Decky's frozen Python runtime.
    import unicodedata
    paths = set()
    prefixes = {}
    for row in profile["payload"]:
        name = row["path"]
        key = unicodedata.normalize("NFC", name).casefold()
        _need(key not in paths)
        paths.add(key)
        for i in range(1, len(name.split("/")) + 1):
            part = "/".join(name.split("/")[:i])
            norm = unicodedata.normalize("NFC", part).casefold()
            _need(prefixes.setdefault(norm, part) == part)
    _need(not any("/".join(p.split("/")[:i]) in paths for p in paths for i in range(1, len(p.split("/")))))
    _need(isinstance(profile["baselines"], list) and 0 < len(profile["baselines"]) <= 64)
    baseline_paths = set()
    for row in profile["baselines"]:
        _need(isinstance(row, dict) and set(row) == {"path", "sha256", "crc32"})
        _path(row["path"])
        _need(row["path"] not in baseline_paths and _sha(row["sha256"]))
        baseline_paths.add(row["path"])
        _need(row["crc32"] is None or type(row["crc32"]) is int and 0 <= row["crc32"] <= 0xffffffff)
    # The reviewed profile declares every original file its installer conditions
    # on. Game.exe/Script.vdf were the verified samples' names; other engines use
    # their own (e.g. VBFI.exe), so the registry is the authority for the set.
    _need(len(baseline_paths) >= 1)
    terms = profile["terms"]
    _need(isinstance(terms, dict) and set(terms) == {"url", "text", "sha256"})
    safe_url(terms["url"])
    _need(isinstance(terms["text"], str) and 0 < len(terms["text"]) <= 32768
          and hashlib.sha256(terms["text"].encode()).hexdigest() == terms["sha256"])
    review = profile["review"]
    _need(isinstance(review, dict) and set(review) == {"semantics", "steamos", "evidence"})
    _need(type(review["semantics"]) is bool and type(review["steamos"]) is bool)
    _need(isinstance(review["evidence"], str) and 0 < len(review["evidence"]) <= 2048)
    return profile


def release_id(profile):
    return f"exe-{digest(profile)[:24]}-{profile['appid']}"


def candidate_binding(candidate):
    """Stable source identity and applicable conditions; no title/URL suffix matches."""
    keys = ("source_id", "source_record_id", "game_appid", "languages", "variant", "version_identity",
            "source_page", "requires_dlc", "excludes_dlc", "mirrors")
    return digest({key: candidate.get(key) for key in keys})


class ExeProfiles:
    def __init__(self, path, sources, *, pipeline_ready=False):
        path = Path(path)
        _need(no_links(path) and path.is_file() and path.stat().st_size <= MAX_PROFILES_BYTES)
        self.path = path
        self.sources = sources
        self.pipeline_ready = pipeline_ready
        self._generic_profiles = {}
        self.reload()

    def reload(self):
        _need(no_links(self.path) and self.path.stat().st_size <= MAX_PROFILES_BYTES)
        data = json.loads(self.path.read_text(encoding="utf-8"))
        _need(isinstance(data, dict) and set(data) == {"schema_version", "profiles"}
              and data["schema_version"] == 1 and isinstance(data["profiles"], list) and len(data["profiles"]) <= 256)
        profiles = [validate_profile(p) for p in data["profiles"]]
        _need(len({p["profile_id"] for p in profiles}) == len(profiles))
        self._profiles = {release_id(p): p for p in profiles}

    def rule(self, source_id, appid):
        # Do not let a newly added HGamer/custom source widen the official rules.
        rule = next((r for r in self.sources._rules if r["id"] == source_id), None)
        builtin = next((r for r in self.sources.builtins if r["id"] == source_id), None)
        if (rule is None or builtin is None or source_id not in OFFICIAL_SOURCES
                or rule.get("role") != "publisher" or not rule.get("enabled")
                or not self.sources.game_subscribed(appid, source_id)):
            raise ValueError("exe_source_disabled")
        _need(rule.get("page_hosts") == builtin.get("page_hosts")
              and rule.get("download_hosts") == builtin.get("download_hosts"), "exe_source_changed")
        return rule

    def _match(self, profile, candidate):
        return (candidate.get("source_id") == profile["source_id"]
                and candidate.get("source_record_id") == profile["source_record_id"]
                and candidate.get("game_appid") == profile["appid"]
                and candidate.get("languages") == [profile["language"]]
                and candidate.get("variant") == profile["variant"]
                and candidate.get("version_identity") == profile["version_identity"]
                and candidate.get("source_page") == profile["source_page"]
                and (candidate.get("requires_dlc") or []) == profile["requires_dlc"]
                and (candidate.get("excludes_dlc") or []) == profile["excludes_dlc"]
                and candidate.get("match_basis") in ("appid", "verified_alias", "official_title")
                and candidate.get("capability") == "discover_only")

    def mirrors(self, profile, candidate):
        rule = self.rule(profile["source_id"], profile["appid"])
        allowed = set(rule["download_hosts"]) & set(profile["download_hosts"])
        mirrors = []
        for mirror in candidate.get("mirrors", []):
            if mirror.get("format") != "exe":
                continue
            try:
                url = safe_url(mirror.get("url"), allowed)
                p = urlsplit(url)
                _need(not p.fragment and not p.query and p.path.lower().endswith(".exe"), "exe_download_url_invalid")
                mirrors.append(url)
            except ValueError:
                continue
        return list(dict.fromkeys(mirrors))

    def resolve(self, game, selection, *, now=None):
        """Fresh backend-owned lookup. Frontend submits only a release selection."""
        _need(isinstance(selection, str) and re.fullmatch(r"exe-(?:(?:selection|content)-)?[0-9a-f]{24}-[1-9][0-9]*", selection), "exe_release_invalid")
        from exe_generic import SELECTION, CONTENT, candidate, placeholder
        if re.fullmatch(SELECTION, selection):
            value = candidate(self, game, selection)
            _need(self.pipeline_ready, "exe_pipeline_unavailable")
            return placeholder(game, value), value
        if re.fullmatch(CONTENT, selection):
            profile = self._generic_profiles.get((game.get("instance_id"), selection))
            if profile is None and getattr(self, "_routing_profile", None):
                profile = self._routing_profile(game, selection)
            _need(profile is not None, "exe_analysis_required")
            value = candidate(self, game, profile["selection_id"])
            _need(candidate_binding(value) == profile["candidate_sha256"], "exe_candidate_changed")
            _need(game.get("buildid") in profile["verified_buildids"], "exe_game_facts_changed")
            return json.loads(json.dumps(profile)), value
        self.reload()  # profile disable/change also invalidates a persisted plan
        profile = self._profiles.get(selection)
        _need(profile is not None and profile["appid"] == game.get("appid"), "exe_profile_changed")
        self.rule(profile["source_id"], profile["appid"])
        candidates = self.sources.cached_candidates(game)
        matches = [c for c in candidates if self._match(profile, c)]
        _need(len(matches) == 1, "exe_candidate_changed")
        candidate = matches[0]
        observed = candidate.get("observed_at")
        if isinstance(observed, str) and len(observed) <= 64:
            try:
                parsed = datetime.fromisoformat(observed.replace("Z", "+00:00"))
                observed = parsed.timestamp() if parsed.tzinfo is not None else None
            except (ValueError, OverflowError):
                observed = None
        now = time.time() if now is None else now
        _need(type(observed) in (int, float) and now - 6 * 3600 <= observed <= now + 60
              and candidate.get("cache_status") in ("fresh", "cached"), "exe_candidate_stale")
        _need(profile["review"]["semantics"] and profile["review"]["steamos"], "exe_profile_not_reviewed")
        _need(game.get("availability") == "installed", "game_not_ready")
        _need(game.get("buildid") in profile["verified_buildids"], "build_not_verified")
        _need(self.mirrors(profile, candidate), "exe_no_direct_download")
        return json.loads(json.dumps(profile)), dict(candidate)

    def reviews(self, game):
        self.reload()
        reviews = []
        for candidate in self.sources.cached_candidates(game):
            reported = candidate.get("format_reported") or "unknown"
            observed = candidate.get("format_observed") or "unknown"
            # The official page may not label the artifact, while the linked
            # payload is an EXE. Accept an observed EXE, but never a zip/mixed
            # conflict or a candidate with no executable evidence at all.
            if (candidate.get("source_id") not in OFFICIAL_SOURCES
                    or reported not in ("exe", "unknown")
                    or observed not in ("exe", "unknown")
                    or "exe" not in (reported, observed)):
                continue
            profiles = [p for p in self._profiles.values() if self._match(p, candidate)]
            profile = profiles[0] if len(profiles) == 1 else None
            blockers = ["exe_profile_not_reviewed"]
            selection = None
            if profile:
                selection = release_id(profile)
                try:
                    self.resolve(game, selection)
                    blockers = [] if self.pipeline_ready else ["exe_pipeline_unavailable"]
                except ValueError as error:
                    blockers = [str(error)]
            # Legacy sample/build coverage must not block fresh behavior analysis.
            # Keep legacy receipt resolution strict; only new candidate selection
            # falls back, rechecking the current source through generic resolve.
            if self.pipeline_ready and (profile is None or blockers):
                from exe_generic import selection_id
                selection = selection_id(candidate) if candidate.get("game_appid") == game.get("appid") else None
                if selection:
                    try:
                        self.resolve(game, selection)
                        profile = None
                        blockers = []
                    except ValueError as error:
                        blockers = [str(error)]
            reviews.append({"candidate_id": candidate["candidate_id"], "release_id": selection,
                "profile_id": profile["profile_id"] if profile else None,
                "status": ("installable" if profile else "check_required") if not blockers else "guidance", "blockers": blockers})
        return reviews
