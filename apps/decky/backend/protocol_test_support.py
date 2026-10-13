"""Shared generated fixtures for host transport tests; never packaged."""
import copy
import json
from pathlib import Path

CASES = json.loads((Path(__file__).resolve().parents[3] / "packages/contracts/fixtures/v4/cases.json").read_text(encoding="utf-8"))

def response(method, request_id="fixture", updates=None):
    row = copy.deepcopy(next(case["response"] for case in CASES["responses"] if case["name"] == method + "-success"))
    row["request_id"] = request_id
    if updates is not None:
        row["result"].update(updates)
    return row


def event(name, request_id="fixture", **updates):
    row = copy.deepcopy(next(case["event"] for case in CASES["events"] if case["name"] == name))
    row.update(request_id=request_id, **updates)
    return row


def outcome(state="committed", error=None, **updates):
    row = response("install_patch")["result"]
    row.update(state=state, error=error, **updates)
    if state == "committed":
        row.update(disposition="completed", cause=None, recovery=None)
    elif state == "needs_recovery":
        row.update(disposition="failed", cause={"kind": "domain_failure", "code": error or "restore_conflict", "detail": None},
                   recovery={"required": True, "code": error or "restore_conflict", "detail": None})
    else:
        cancelled = error in ("cancelled", "task_cancelled", "lease_expired")
        row.update(disposition="cancelled" if cancelled else "failed", recovery=None,
                   cause={"kind": "user_cancelled" if cancelled else "domain_failure", "code": error or "file_changed", "detail": None})
    return row
