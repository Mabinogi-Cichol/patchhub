"""Generate the cross-language identity golden fixture from the authoritative
Python algorithms in apps/decky/backend/{discovery,sources}.py.

Regenerate:  python crates/core/tests/fixtures/sources/gen_identity_golden.py
Writes identity-golden.json next to this file. The Rust `canonical.rs`
implementation MUST reproduce the `payload` strings byte-for-byte and the
digests exactly.
"""
import hashlib
import json
import os
from urllib.parse import urlsplit, urlunsplit


def canonical_url(url):
    p = urlsplit(url)
    return urlunsplit((p.scheme, p.netloc.lower(), p.path, "", ""))


def candidate_id(identity):
    blob = json.dumps(identity, ensure_ascii=False)
    return "candidate-" + hashlib.sha256(blob.encode()).hexdigest()[:24], blob


def query_key(data):
    blob = json.dumps(data, ensure_ascii=False, sort_keys=True)
    return hashlib.sha256(blob.encode()).hexdigest(), blob


def fingerprint(c):
    blob = json.dumps(
        {k: v for k, v in c.items() if k not in ("fingerprint", "observed_at", "cache_status")},
        sort_keys=True,
        ensure_ascii=False,
    )
    return hashlib.sha256(blob.encode()).hexdigest(), blob


cand_inputs = [
    ["official", canonical_url("https://publisher.example/game/"), "base", ["en"], "page-unknown"],
    ["kagura-cn", canonical_url("https://kagurafan.com/game/"), "dlc", ["zh-CN"], "v1.2.3:patch_v1.2.3.zip"],
]
cand_cases = []
for identity in cand_inputs:
    digest, blob = candidate_id(identity)
    cand_cases.append({"identity": identity, "expected": digest, "payload": blob})

# Real Decky device evidence for AppID 2303800 ("My Life as a Cult Leader" /
# 妃神会秘史), captured 2026-10-07 — see
# docs/features/shared-source-discovery/p0-baseline.md.
real_2303800 = [
    (
        ["kagura-cn", canonical_url("https://kagurafan.com/2023/03/24/%e5%a6%83%e7%a5%9e%e4%bc%9a%e7%a7%98%e5%8f%b2/"), "base", ["zh-CN"], "4:My Life as a Cult Leader CN Patch v4.exe"],
        "candidate-21d97c05e6cf1324b0cea5c7",
    ),
    (
        ["kagura-en", canonical_url("https://www.kaguragames.com/product/my-life-as-a-cult-leader-patch/"), "base", ["en"], "3:My Life as a Cult Leader EN Patch v3.exe"],
        "candidate-d04331eb1335da62bedec536",
    ),
    (
        ["hgamer", canonical_url("https://hgamer.tw/zh-TW/patches/my-life-as-a-cult-leader-patch"), "base", ["zh-CN"], "4"],
        "candidate-625cfade8b8786da9e58842c",
    ),
]
for identity, expected in real_2303800:
    digest, blob = candidate_id(identity)
    assert digest == expected, (digest, expected)
    cand_cases.append(
        {"identity": identity, "expected": digest, "payload": blob, "origin": "decky-2303800-real"}
    )

qk_data = {
    "appid": 42,
    "name": "A Game",
    "name_hints": [],
    "preferred_language": None,
    "legacy_match": False,
    "source_context": ["official"],
    "parser_version": 2,
    "query_version": 1,
}
qk_digest, qk_blob = query_key(qk_data)

fp_input = {
    "aliases": ["A Game"],
    "candidate_id": "candidate-49d4abc4342bbe0aa2491fcb",
    "capability": "discover_only",
    "excludes_dlc": None,
    "format_observed": "unknown",
    "format_reported": "zip",
    "game_appid": 42,
    "game_title": "A Game",
    "identity_evidence": [
        {"appid": 42, "kind": "steam_appid", "page": "https://hgamer.tw/zh-CN/patches/game"}
    ],
    "languages": [],
    "match_basis": "appid",
    "mirrors": [],
    "parent_appid": None,
    "patch_kind": "adult_content",
    "patch_title": "A Game",
    "patch_updated_at": None,
    "platforms": ["steam"],
    "provenance": [{"page": "https://hgamer.tw/zh-CN/patches/game", "source_id": "hgamer"}],
    "published_at": None,
    "requires_dlc": None,
    "source_id": "hgamer",
    "source_page": "https://hgamer.tw/zh-CN/patches/game",
    "source_record_id": "https://hgamer.tw/zh-CN/patches/game",
    "upstream_page": None,
    "variant": "base",
    "version_evidence": ["1"],
    "version_identity": "1",
    "version_raw": "1",
    "warnings": [],
}
fp_digest, fp_blob = fingerprint(fp_input)

fixture = {
    "schema_version": 1,
    "note": (
        "Golden values produced by the authoritative Python algorithms "
        "(discovery._candidate / sources._query_key / discovery.discover fingerprint resave). "
        "The Rust canonical.rs implementation MUST reproduce payload strings byte-for-byte and "
        "the digests exactly."
    ),
    "separators": {"item": ", ", "key": ": "},
    "canonical_urls": [
        {"input": "https://publisher.example/game/", "expected": canonical_url("https://publisher.example/game/")},
        {"input": "https://kagurafan.com/game/", "expected": canonical_url("https://kagurafan.com/game/")},
    ],
    "candidate_id": cand_cases,
    "query_key": [{"input": qk_data, "expected": qk_digest, "payload": qk_blob}],
    "fingerprint": [{"input": fp_input, "expected": fp_digest, "payload": fp_blob}],
}

out_dir = os.path.dirname(os.path.abspath(__file__))
out = os.path.join(out_dir, "identity-golden.json")
with open(out, "w", encoding="utf-8") as fh:
    json.dump(fixture, fh, ensure_ascii=False, indent=2, sort_keys=False)
    fh.write("\n")
print("wrote", out)
print("cand:", [c["expected"] for c in cand_cases])
print("query_key:", qk_digest)
print("fingerprint:", fp_digest)
