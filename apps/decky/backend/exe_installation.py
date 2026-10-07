"""Prepare reviewed EXE payloads; the Rust core alone writes game files.

The official EXE is an immutable acquisition input. A locally signed catalog
authorizes only the independently checked normalized ZIP, with the original
EXE and source/profile evidence kept in an offline per-plan policy receipt.
"""
import asyncio
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
from threading import Event
import time
import uuid

from acquisition import no_links
from discovery import safe_url
from exe_facts import game_facts as legacy_game_facts, read_file
from exe_profiles import candidate_binding, digest
from exe_worker import extract
from publisher import _download, _hash_file, _json_new, _recipe, _safe_dir
from tasks import current_task

MAX_POLICY = 4 * 1024 * 1024
UUID_PATTERN = r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}"
CORE_PLAN_MAX_TTL = 900  # core install::validate_job enforces this upper bound


def game_facts(game, profile, cancel=None):
    if profile.get("generic") and "behavior" in profile:
        from exe_generic import facts
        return facts(game, profile, cancel)
    return legacy_game_facts(game, profile, cancel)


def _need(condition, code):
    if not condition:
        raise ValueError(code)


def _check(cancel):
    if cancel.is_set():
        raise RuntimeError("download_cancelled")


def _read_json(path, limit=MAX_POLICY):
    _need(no_links(path) and path.is_file(), "exe_policy_unavailable")
    _, raw = read_file(path, limit, contents=True)
    try:
        value = json.loads(raw)
    except (ValueError, UnicodeDecodeError) as error:
        raise ValueError("exe_policy_invalid") from error
    _need(isinstance(value, dict), "exe_policy_invalid")
    return value


def revoke_plan(state, plan_id):
    """Persist revocation before the fallible discard RPC."""
    _need(isinstance(plan_id, str) and re.fullmatch(UUID_PATTERN, plan_id), "invalid_id")
    job = Path(state) / "jobs" / plan_id
    _need(no_links(job) and job.is_dir(), "exe_core_job_invalid")
    marker = job / "revoked"
    _need(no_links(marker), "linked_state_path")
    if not marker.exists():
        with os.fdopen(os.open(marker, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "wb") as stream:
            stream.write(b"revoked\n")
            stream.flush()
            os.fsync(stream.fileno())
        if os.name != "nt":
            fd = os.open(job, os.O_RDONLY | os.O_DIRECTORY)
            try:
                os.fsync(fd)
            finally:
                os.close(fd)


def _overlap(state, root):
    state, root = state.absolute(), root.absolute()
    _need(state != root and not state.is_relative_to(root) and not root.is_relative_to(state), "state_overlaps_game")


def _space(state, root, profile):
    """Budget each volume before acquisition, including temporary failure-recovery copies.

    ZIP_STORED needs payload + bounded directory/header overhead. State holds
    EXE, worker ZIP, staging, and original backups; target holds temporary files.
    On one volume both budgets must fit together, rather than separately.
    """
    total = sum(row["size"] for row in profile["payload"])
    original = 0
    for row in profile["payload"]:
        path = root.joinpath(*row["path"].split("/"))
        _need(no_links(path), "linked_target")
        if path.exists():
            _need(path.is_file(), "target_not_file")
            original += path.stat().st_size
            _need(original <= 16 * 1024 ** 3, "exe_original_size_limit")
    overhead = 16 * 1024 * 1024
    state_bytes = profile["exe_size"] + 2 * total + original + overhead
    target_bytes = total + original + overhead
    same_device = state.stat().st_dev == root.stat().st_dev
    state_need = state_bytes + target_bytes if same_device else state_bytes
    _need(shutil.disk_usage(state).free >= state_need, "exe_state_insufficient_space")
    _need(shutil.disk_usage(root).free >= target_bytes, "exe_target_insufficient_space")
    return {"state_bytes": state_bytes, "target_bytes": target_bytes, "same_device": same_device}


def _remove_work(state, work):
    """Only remove our UUID work tree after checking every descendant for links."""
    state, work = Path(state).absolute(), Path(work).absolute()
    if work.parent != state / "exe-work" or not re.fullmatch(UUID_PATTERN, work.name):
        return
    try:
        if not no_links(work) or not work.is_dir():
            return
        owner = _read_json(work / "owner.json", 8192)
        if owner != {"schema_version": 1, "job_id": work.name}:
            return
        for directory, dirs, files in os.walk(work, followlinks=False):
            if any(not no_links(Path(directory) / n) for n in (*dirs, *files)):
                return
        shutil.rmtree(work)
    except (OSError, ValueError):
        pass


class ExeInstallation:
    def __init__(self, publisher, profiles, tool_dir):
        self.publisher = publisher
        self.profiles = profiles
        self.tool_dir = Path(tool_dir)
        from exe_generic import routing
        self.profiles._routing_profile = lambda game, content: routing(self, game, content)

    def _source_name(self, profile):
        return next((rule.get("name") or rule["id"] for rule in self.profiles.sources._rules
                     if rule["id"] == profile["source_id"]), profile["source_id"])

    def matches(self, game):
        result = []
        for review in self.profiles.reviews(game):
            if review.get("status") not in ("installable", "check_required") or not review.get("release_id"):
                continue
            try:
                profile, candidate = self.profiles.resolve(game, review["release_id"])
            except ValueError:
                continue
            language_name = {"zh-CN": "简体中文", "zh-TW": "繁体中文", "en": "英文", "ja": "日文"}.get(profile["language"], profile["language"])
            display_version = candidate.get("version_raw") or profile["version_identity"].split(":", 1)[0]
            selected = review["release_id"]
            if profile.get("generic"):
                # Reuse the installed content's identity, rather than showing
                # a second selection blocked by that same installation.
                for instance, content, _ in self.publisher.bridges():
                    if instance != game["instance_id"] or not content.startswith("exe-content-"): continue
                    try: routed, _ = self.profiles.resolve(game, content)
                    except (ValueError, OSError): continue
                    if routed.get("selection_id") == selected:
                        selected = content
                        break
            result.append({"release_id": selected, "profile_id": profile["profile_id"], "candidate_id": candidate["candidate_id"],
                "name": f"{candidate.get('title') or game.get('name', '')} · {self._source_name(profile)} · EXE · {language_name}",
                "version": display_version, "language": profile["language"],
                "source_id": profile["source_id"], "source_name": self._source_name(profile), "source_page": profile["source_page"],
                "support_level": "publisher_exe", "compatibility": "check_required" if profile.get("generic") else "verified", "blockers": [],
                "download_available": True, "detection": {"status": "unknown", "files": []},
                "exe": {"profile_id": profile["profile_id"], "language": profile["language"],
                    "version": display_version, "variant": profile["variant"],
                    "requires_dlc": profile["requires_dlc"],
                    "excludes_dlc": profile["excludes_dlc"]}})
        return result

    async def _idle(self, state, operation="install"):
        if (state / "journal.sqlite").exists() and not (state / "catalog.json").is_file():
            raise ValueError("exe_existing_state_invalid")
        if (state / "catalog.json").exists():
            _need(no_links(state / "catalog.json"), "linked_state_path")
            bridge = self.publisher._bridge(state)
            transactions = await bridge.list_transactions()
            swept = set()
            for transaction in transactions:
                if transaction.get("state") != "planned":
                    continue
                plan_id = transaction.get("plan_id")
                if (not isinstance(plan_id, str) or not re.fullmatch(UUID_PATTERN, plan_id)
                        or transaction.get("transaction_id") != plan_id
                        or transaction.get("release_id") != state.parent.name
                        or not isinstance(transaction.get("instance_id"), str)
                        or self.publisher._state(transaction["instance_id"], transaction["release_id"]) != state
                        or not no_links(state / "jobs" / plan_id)):
                    continue
                now = int(time.time())
                receipt = None
                try:
                    receipt = self.plan_evidence(state, plan_id)
                except (OSError, ValueError):
                    pass
                expired = False
                if (receipt and receipt.get("instance_id") == transaction["instance_id"]
                        and receipt.get("release_id") == transaction["release_id"]):
                    expired = receipt["lease_active"] is False or receipt["expires_at"] < now
                # Rust owns/validates the SQLite job, including the TTL bound;
                # a crash before writing our receipt can only pin it for 900s.
                created = transaction.get("created_at")
                if type(created) is int and 0 <= created < now - CORE_PLAN_MAX_TTL:
                    expired = True
                if expired:
                    result = await bridge.discard_plan(plan_id)
                    if result.get("state") == "rolled_back":
                        swept.add(plan_id)
                        try:
                            self.release_lease(state, plan_id)
                        except (OSError, ValueError):
                            pass
            transactions = [t for t in transactions if t.get("plan_id") not in swept]
            if any(t.get("state") not in (("uninstalled", "rolled_back", "superseded", "committed") if operation == "reinstall" else ("uninstalled", "rolled_back", "superseded")) for t in transactions):
                raise RuntimeError("publisher_existing_transaction")

    def _recheck(self, game, release_id, profile, candidate, facts, cancel):
        current_profile, current_candidate = self.profiles.resolve(game, release_id)
        _need(digest(current_profile) == digest(profile), "exe_profile_changed")
        _need(candidate_binding(current_candidate) == candidate_binding(candidate), "exe_candidate_changed")
        _need(game_facts(game, current_profile, cancel) == facts, "exe_game_facts_changed")

    @staticmethod
    def plan_evidence(state, plan_id):
        if not isinstance(plan_id, str) or not re.fullmatch(UUID_PATTERN, plan_id):
            raise ValueError("invalid_id")
        path = Path(state) / "jobs" / plan_id / "exe-policy.json"
        if not path.exists():
            return None
        receipt = _read_json(path)
        _need(receipt.get("schema_version") == 2 and receipt.get("plan_id") == plan_id
              and type(receipt.get("lease_active")) is bool
              and type(receipt.get("expires_at")) is int, "exe_policy_invalid")
        return receipt

    @classmethod
    def release_lease(cls, state, plan_id):
        receipt = cls.plan_evidence(state, plan_id)
        if receipt:
            receipt["lease_active"] = False
            _json_new(Path(state) / "jobs" / plan_id / "exe-policy.json", receipt)

    def check_game(self, game, release_id):
        profile, _ = self.profiles.resolve(game, release_id)
        return game_facts(game, profile)

    async def prepare(self, game, release_id, cancel, operation="install", *, _analyzed=False):
        _check(cancel)
        from exe_generic import SELECTION, prepare_analysis
        if re.fullmatch(SELECTION, release_id):
            _need(operation == "install", "exe_analysis_required")
            content = await prepare_analysis(self, game, release_id, cancel)
            return await self.prepare(game, content, cancel, operation, _analyzed=True)
        profile, candidate = self.profiles.resolve(game, release_id)
        if profile.get("generic") and not _analyzed:
            await prepare_analysis(self, game, profile["selection_id"], cancel, expected_content=release_id)
            return await self.prepare(game, release_id, cancel, operation, _analyzed=True)
        _need(operation in ("install", "reinstall"), "invalid_install_operation")
        task = current_task.get()
        if task:
            task.stage("checking_game")
        facts = await asyncio.to_thread(game_facts, game, profile, cancel)
        instance_id, root = game["instance_id"], Path(game["install_path"]).absolute()
        state = self.publisher._state(instance_id, release_id)
        _overlap(state, root)
        _safe_dir(state)
        if (state / "instance.json").exists():
            old = _read_json(state / "instance.json")
            _need((old.get("instance_id"), old.get("release_id"), old.get("install_path")) ==
                  (instance_id, release_id, str(root)), "publisher_state_identity_changed")
        await self._idle(state, operation)
        estimate = await asyncio.to_thread(_space, state, root, profile)
        _check(cancel)
        cache = _safe_dir(state / "cache")
        work = _safe_dir(state / "exe-work" / str(uuid.uuid4()))
        _json_new(work / "owner.json", {"schema_version": 1, "job_id": work.name})
        archive = cache / "normalized.zip"
        bridge, plan, published = None, None, None
        try:
            allowed = set(self.profiles.rule(profile["source_id"], profile["appid"])["download_hosts"]) & set(profile["download_hosts"])

            def validate(url):
                checked = safe_url(url, allowed)
                # Accept only a mirror of this reviewed candidate, including
                # redirects restricted to its official EXE download hosts.
                from urllib.parse import urlsplit
                parsed = urlsplit(checked)
                _need(not parsed.query and not parsed.fragment and parsed.path.lower().endswith(".exe"), "exe_download_url_invalid")
                return checked

            last_error = None
            urls = self.profiles.mirrors(profile, candidate)
            if profile.get("generic"):
                acquired = profile["analysis_acquisition"]
                url = validate(acquired["url"])
                _need(url in urls, "exe_candidate_changed")
                original = cache / "archive.exe"
                original_sha, original_size = profile["exe_sha256"], profile["exe_size"]
                acquisition = acquired["mode"]
            else:
                for url in urls:
                    try:
                        original, original_sha, original_size, acquisition = await asyncio.to_thread(_download,
                            url, cache, cancel, validate, archive_name="archive.exe", manifest_name="exe-download.json",
                            max_size=2 * 1024 ** 3, expected_sha256=profile["exe_sha256"], expected_size=profile["exe_size"])
                        break
                    except (ValueError, OSError) as error:
                        last_error = error
                        _check(cancel)
                else:
                    raise last_error or ValueError("exe_no_direct_download")
            _need(original_sha == profile["exe_sha256"] and original_size == profile["exe_size"], "exe_payload_changed")
            download = _read_json(cache / "exe-download.json", 32768)
            final_url = validate(download.get("final_url", url))
            # The downloader and worker are separate trust checks; rehash even
            # when a synthetic/future downloader returns inconsistent metadata.
            _need(await asyncio.to_thread(_hash_file, original, 2 * 1024 ** 3, cancel, False) ==
                  (profile["exe_sha256"], profile["exe_size"]), "exe_payload_changed")
            converted = await extract(self.publisher.bridge, self.tool_dir, original, work, profile, cancel)
            output = work / "normalized.zip"
            zip_sha, zip_size = await asyncio.to_thread(_hash_file, output, 2 * 1024 ** 3 + 16 * 1024 ** 2, cancel, False)
            _need(converted.get("exe_sha256") == original_sha and converted.get("metadata_sha256") == profile["metadata_sha256"]
                  and converted.get("artifact_sha256") == zip_sha and converted.get("artifact_size") == zip_size,
                  "exe_worker_evidence_changed")
            _need(sorted(converted.get("files", []), key=lambda r: r["path"]) ==
                  sorted(profile["payload"], key=lambda r: r["path"]), "exe_manifest_changed")
            files = await asyncio.to_thread(_recipe, output, root, cancel)
            actual = sorted((f["source"], f["installed_sha256"]) for f in files)
            expected = sorted((f["path"], f["sha256"]) for f in profile["payload"])
            _need(actual == expected, "exe_manifest_changed")
            await asyncio.to_thread(self._recheck, game, release_id, profile, candidate, facts, cancel)
            await self._idle(state, operation)
            _check(cancel)
            catalog_path = state / "catalog.json"
            revision = _read_json(catalog_path)["revision"] + 1 if catalog_path.exists() else 1
            release = {"patch_id": release_id, "release_id": release_id, "appid": profile["appid"],
                "name": f"{game.get('name', '')} · {self._source_name(profile)} · EXE · {profile['language']}", "version": profile["version_identity"],
                "language": profile["language"], "category": "publisher_exe", "source_page": profile["source_page"],
                "support_level": "managed_zip", "platforms": ["steamos"],
                "verified_buildids": profile["verified_buildids"],
                "detection": [{"path": f["destination"], "sha256": f["installed_sha256"]} for f in files],
                "recipe": {"artifact_sha256": zip_sha, "files": files},
                "artifact": {"url": url, "size": zip_size, "sha256": zip_sha, "allowed_redirect_hosts": []}}
            _json_new(work / "catalog.json", {"schema_version": 1, "revision": revision, "releases": [release]})
            private, public = state / "private.key", state / "public.json"
            key_id = "local-" + hashlib.sha256(instance_id.encode()).hexdigest()[:16]
            if not private.exists() and not public.exists():
                await self.publisher._native("keygen", work / "private.key", work / "public.json", key_id)
                _check(cancel)
                _need(all(no_links(work / n) and (work / n).is_file() for n in ("private.key", "public.json")), "invalid_local_keys")
                private, public = work / "private.key", work / "public.json"
            else:
                _need(all(no_links(p) and p.is_file() for p in (private, public)), "invalid_local_keys")
            if task:
                task.stage("preparing_plan")
            await self.publisher._native("sign-catalog", work / "catalog.json", private, key_id, work / "signed.json", "7776000")
            _check(cancel)
            await self._idle(state, operation)
            await asyncio.to_thread(self._recheck, game, release_id, profile, candidate, facts, cancel)
            for path in (archive, state / "catalog.json", state / "signed.json", state / "instance.json"):
                _need(no_links(path), "linked_state_path")
            if private.parent == work:
                os.replace(private, state / "private.key")
                os.replace(public, state / "public.json")
            os.replace(output, archive)
            published = (zip_sha, zip_size)
            os.replace(work / "catalog.json", catalog_path)
            os.replace(work / "signed.json", state / "signed.json")
            _json_new(state / "instance.json", {"instance_id": instance_id, "release_id": release_id,
                "install_path": str(root), "url": url, "kind": "publisher_exe"})
            bridge = self.publisher._bridge(state)
            await bridge.migrate_state()
            _check(cancel)
            # Rust starts the plan TTL only here, after EXE conversion completes.
            plan = await bridge.plan_install(instance_id, release_id, archive, operation)
            _check(cancel)
            plan_id = plan.get("plan_id")
            _need(isinstance(plan_id, str) and re.fullmatch(UUID_PATTERN, plan_id), "exe_core_plan_invalid")
            _need(type(plan.get("expires_at")) is int and plan["expires_at"] >= int(time.time()), "exe_core_plan_invalid")
            job = state / "jobs" / plan_id
            _need(no_links(job) and job.is_dir(), "exe_core_job_invalid")
            receipt = {"schema_version": 2, "kind": "publisher_exe", "plan_id": plan_id,
                "instance_id": instance_id, "release_id": release_id, "expires_at": plan["expires_at"],
                "lease_active": True, "operation": operation,
                "terms_sha256": profile["terms"]["sha256"],
                "profile_id": profile["profile_id"], "profile_revision": profile["revision"],
                "profile_sha256": digest(profile), "candidate_sha256": candidate_binding(candidate),
                "facts": facts, "source_page": profile["source_page"], "source_id": profile["source_id"],
                "download_url": url, "final_url": final_url, "exe_sha256": original_sha, "exe_size": original_size,
                "metadata_sha256": profile["metadata_sha256"], "payload_sha256": digest(profile["payload"]),
                "normalized_sha256": zip_sha, "normalized_size": zip_size, "extractor": converted.get("extractor"),
                "catalog_sha256": read_file(catalog_path, MAX_POLICY)["sha256"],
                "public_key_sha256": read_file(state / "public.json", 32768)["sha256"], "space_estimate": estimate}
            _json_new(job / "exe-policy.json", receipt)
            await asyncio.to_thread(self._recheck, game, release_id, profile, candidate, facts, cancel)
            _check(cancel)
            return bridge, str(archive), plan, acquisition
        except BaseException as error:
            if isinstance(error, asyncio.CancelledError):
                cancel.set()
            if plan and bridge and isinstance(plan.get("plan_id"), str):
                revoke_plan(state, plan["plan_id"])
                self.release_lease(state, plan["plan_id"])
                try:
                    await bridge.discard_plan(plan["plan_id"])
                except Exception:
                    pass
            if published:
                try:
                    # Owned cache publication only: never remove a replaced file.
                    if _hash_file(archive, 2 * 1024 ** 3 + 16 * 1024 ** 2, Event(), False) == published:
                        archive.unlink()
                except (OSError, ValueError):
                    pass
            raise
        finally:
            await asyncio.to_thread(_remove_work, state, work)

    async def validate_apply(self, game, release_id, core_plan_id):
        task = current_task.get()
        cancel = task.cancel if task else Event()
        _check(cancel)
        state = self.publisher._state(game["instance_id"], release_id)
        _need(not (state / "jobs" / core_plan_id / "revoked").exists(), "plan_revoked")
        receipt = self.plan_evidence(state, core_plan_id)
        _need(receipt is not None, "exe_policy_unavailable")
        _need(receipt.get("instance_id") == game["instance_id"] and receipt.get("release_id") == release_id,
              "exe_policy_identity_changed")
        _need(receipt["lease_active"] and int(time.time()) <= receipt["expires_at"], "plan_expired")
        profile, candidate = self.profiles.resolve(game, release_id)
        _need(receipt.get("profile_sha256") == digest(profile), "exe_profile_changed")
        _need(receipt.get("candidate_sha256") == candidate_binding(candidate), "exe_candidate_changed")
        _need(receipt.get("terms_sha256") == profile["terms"]["sha256"], "exe_terms_changed")
        facts = await asyncio.to_thread(game_facts, game, profile, cancel)
        _check(cancel)
        _need(facts == receipt.get("facts"), "exe_game_facts_changed")
        _need(read_file(state / "catalog.json", MAX_POLICY)["sha256"] == receipt.get("catalog_sha256")
              and read_file(state / "public.json", 32768)["sha256"] == receipt.get("public_key_sha256"), "exe_local_authorization_changed")
        _need(receipt.get("exe_sha256") == profile["exe_sha256"] and receipt.get("exe_size") == profile["exe_size"]
              and receipt.get("metadata_sha256") == profile["metadata_sha256"]
              and receipt.get("payload_sha256") == digest(profile["payload"]), "exe_policy_invalid")
        current_profile, current_candidate = self.profiles.resolve(game, release_id)
        _need(digest(current_profile) == digest(profile), "exe_profile_changed")
        _need(candidate_binding(current_candidate) == candidate_binding(candidate), "exe_candidate_changed")
        _check(cancel)
        return receipt
