"""Host-side validation of the Rust-generated v4 contract (no business rules)."""
import importlib.util
import json
from pathlib import Path
import re
import unicodedata

# Release packaging places the exact generated pure-data module beside us.
# Repository tests load that same artifact directly, never run the generator.
try:
    import protocol as generated
except ModuleNotFoundError:
    location = Path(__file__).resolve().parents[3] / "packages/contracts/generated/protocol.py"
    spec = importlib.util.spec_from_file_location("steam_patch_protocol", location)
    generated = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(generated)

PROTOCOL_VERSION = generated.PROTOCOL_VERSION
METHODS = frozenset(generated.METHODS)
SOURCE_METHODS = frozenset(generated.SOURCE_METHODS)
CONTROLLED_METHODS = frozenset(generated.CONTROLLED_METHODS)
SOURCE_EVENTS = frozenset(generated.SOURCE_EVENTS)
INSTALL_STAGES = frozenset(generated.INSTALL_STAGES)
MAX_INSTANCE_ID_BYTES = generated.MAX_INSTANCE_ID_BYTES
MAX_REQUEST_BYTES = generated.MAX_REQUEST_BYTES
MAX_RESPONSE_BYTES = generated.MAX_RESPONSE_BYTES
SCHEMA = generated.SCHEMA
JS_MAX_INTEGER = generated.MAX_SAFE_INTEGER


def validate_id(value, limit=generated.MAX_ID_BYTES, code="invalid_id"):
    if (not isinstance(value, str) or not value
            or any(unicodedata.category(char) == "Cc" for char in value)):
        raise ValueError(code)
    try:
        if len(value.encode("utf-8")) > limit:
            raise ValueError(code)
    except UnicodeEncodeError as exc:
        raise ValueError(code) from exc
    return value


def truncate_detail(value):
    return value.encode("utf-8")[:generated.MAX_DETAIL_BYTES].decode("utf-8", errors="ignore")


def _valid(value, rule):
    if rule is True:
        return True
    if rule is False:
        return False
    if "$ref" in rule:
        node = SCHEMA
        for part in rule["$ref"].removeprefix("#/").split("/"):
            node = node[part.replace("~1", "/").replace("~0", "~")]
        return _valid(value, node)
    if "const" in rule and (type(value) is not type(rule["const"]) or value != rule["const"]):
        return False
    if "enum" in rule and not any(type(value) is type(item) and value == item for item in rule["enum"]):
        return False
    if "anyOf" in rule and not any(_valid(value, item) for item in rule["anyOf"]):
        return False
    if "oneOf" in rule and sum(_valid(value, item) for item in rule["oneOf"]) != 1:
        return False
    if "allOf" in rule and not all(_valid(value, item) for item in rule["allOf"]):
        return False
    if "not" in rule and _valid(value, rule["not"]):
        return False
    kind = rule.get("type")
    kinds = kind if isinstance(kind, list) else [kind] if kind else []
    matches = {"object": type(value) is dict, "array": type(value) is list,
               "string": type(value) is str, "integer": type(value) is int,
               "number": type(value) in (int, float), "boolean": type(value) is bool,
               "null": value is None}
    if kinds and not any(matches.get(item, False) for item in kinds):
        return False
    if type(value) is dict:
        properties = rule.get("properties", {})
        if any(key not in value for key in rule.get("required", [])):
            return False
        extra = rule.get("additionalProperties", True)
        for key, item in value.items():
            if key in properties:
                if not _valid(item, properties[key]):
                    return False
            elif extra is False or isinstance(extra, dict) and not _valid(item, extra):
                return False
    if type(value) is list:
        if len(value) < rule.get("minItems", 0) or len(value) > rule.get("maxItems", float("inf")):
            return False
        if "items" in rule and not all(_valid(item, rule["items"]) for item in value):
            return False
        if rule.get("uniqueItems") and len({json.dumps(item, sort_keys=True) for item in value}) != len(value):
            return False
    if type(value) in (int, float):
        numeric_format = rule.get("format", "")
        numeric_kind = re.fullmatch(r"(u?)int(8|16|32|64)", numeric_format)
        if numeric_kind:
            unsigned, width = numeric_kind.groups()
            bits = int(width)
            lower = 0 if unsigned else -(1 << (bits - 1))
            upper = (1 << bits) - 1 if unsigned else (1 << (bits - 1)) - 1
            if not lower <= value <= upper:
                return False
        if not -JS_MAX_INTEGER <= value <= JS_MAX_INTEGER:
            return False
        if value < rule.get("minimum", float("-inf")) or value > rule.get("maximum", float("inf")):
            return False
        if "exclusiveMinimum" in rule and value <= rule["exclusiveMinimum"]:
            return False
        if "exclusiveMaximum" in rule and value >= rule["exclusiveMaximum"]:
            return False
    if type(value) is str:
        try:
            encoded = value.encode("utf-8")
        except UnicodeEncodeError:
            return False
        if len(value) < rule.get("minLength", 0) or len(value) > rule.get("maxLength", float("inf")):
            return False
        if "pattern" in rule and not re.search(rule["pattern"], value):
            return False
        if "x-max-utf8-bytes" in rule and len(encoded) > rule["x-max-utf8-bytes"]:
            return False
    return True


def _bytes_valid(value):
    if type(value) is float or type(value) is int and abs(value) > JS_MAX_INTEGER:
        return False
    if isinstance(value, dict):
        for key, item in value.items():
            if item is not None and key in {"request_id", "instance_id", "release_id", "selection_id", "plan_id", "source_id", "candidate_id", "content_id", "transaction_id"}:
                try:
                    validate_id(item, generated.MAX_REQUEST_ID_BYTES if key == "request_id" else generated.MAX_INSTANCE_ID_BYTES if key == "instance_id" else generated.MAX_ID_BYTES)
                except ValueError:
                    return False
            if key == "detail" and item is not None:
                if not isinstance(item, str) or len(item.encode("utf-8")) > generated.MAX_DETAIL_BYTES:
                    return False
            if key == "code" and item is not None and (not isinstance(item, str) or not re.fullmatch(r"[a-z0-9_]{1,128}", item)):
                return False
            if not _bytes_valid(item):
                return False
    elif isinstance(value, list):
        return all(_bytes_valid(item) for item in value)
    return True


def validate_named(value, name, code="contract_mismatch"):
    name = name.rsplit("/", 1)[-1]
    definitions = SCHEMA.get("$defs", SCHEMA.get("definitions", {}))
    if name not in definitions or not _valid(value, definitions[name]) or not _bytes_valid(value):
        raise RuntimeError(code)
    return value


def validate_request(request, task_channel=None):
    if request.get("protocol_version") != PROTOCOL_VERSION or type(request.get("protocol_version")) is not int:
        raise ValueError("unsupported_protocol")
    if request.get("method") not in METHODS:
        raise ValueError("unsupported_method")
    validate_id(request.get("request_id"), generated.MAX_REQUEST_ID_BYTES, "invalid_request_id")
    if task_channel is False and request["method"] in CONTROLLED_METHODS:
        raise ValueError("task_required")
    validate_named(request, "Request", "invalid_request")


def validate_response(row, method, request_id):
    if row.get("protocol_version") != PROTOCOL_VERSION:
        raise RuntimeError("unsupported_protocol")
    if row.get("request_id") != request_id:
        raise RuntimeError("contract_mismatch")
    if method == "get_capabilities" and row.get("ok") is True:
        if set(row) != {"protocol_version", "request_id", "ok", "result"}:
            raise RuntimeError("contract_mismatch")
        validate_capabilities(row.get("result"))
    validate_named(row, "Response")
    if row["ok"]:
        validate_named(row["result"], generated.RESULT_TYPES[method])


def validate_event(row, request_id=None):
    if row.get("protocol_version") != PROTOCOL_VERSION:
        raise RuntimeError("unsupported_protocol")
    if request_id is not None and row.get("request_id") != request_id:
        raise RuntimeError("contract_mismatch")
    validate_named(row, "Event")
    if row.get("substage") is not None and row.get("stage") != "applying":
        raise RuntimeError("contract_mismatch")


def validate_capabilities(caps):
    if not isinstance(caps, dict) or caps.get("protocol_version") != PROTOCOL_VERSION:
        raise ValueError("unsupported_protocol")
    if (type(caps.get("methods")) is not list or len(caps["methods"]) != len(METHODS)
            or caps["methods"] != list(generated.METHODS) or caps.get("contracts") != generated.CONTRACTS):
        raise ValueError("capability_mismatch")
    installer, sources = caps.get("installer"), caps.get("sources")
    if (not isinstance(installer, dict) or not isinstance(sources, dict)
            or any(installer.get(key) != expected for key, expected in generated.INSTALLER_VERSIONS.items())
            or any(sources.get(key) != expected for key, expected in generated.SOURCE_VERSIONS.items())):
        raise ValueError("installer_schema_unsupported")
    if sources.get("methods") != list(generated.SOURCE_METHODS):
        raise ValueError("capability_mismatch")
    try:
        validate_named(caps, "Capabilities")
    except RuntimeError as exc:
        raise ValueError("capability_mismatch") from exc
