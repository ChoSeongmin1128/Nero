"""Local-only evaluation configuration and worker boundary. No model imports."""
import hashlib
import json
import math
import os
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
ID = re.compile(r"[a-zA-Z0-9][a-zA-Z0-9_.-]{0,79}\Z")


def read_json(path):
    def duplicate_pairs(pairs):
        value = {}
        for key, item in pairs:
            if key in value:
                raise ValueError("duplicate JSON key: " + key)
            value[key] = item
        return value
    def invalid_constant(value):
        raise ValueError("nonfinite JSON number: " + value)
    return json.loads(Path(path).read_text(encoding="utf-8"), object_pairs_hook=duplicate_pairs, parse_constant=invalid_constant)


def write_json(path, value):
    path = Path(path)
    temporary = path.with_name(path.name + ".tmp")
    fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as handle:
        json.dump(value, handle, ensure_ascii=False, indent=2, allow_nan=False)
        handle.write("\n")
    os.replace(temporary, path)


def absolute(value):
    if not isinstance(value, str) or not Path(value).is_absolute():
        raise ValueError("input, model, runtime and output paths must be absolute")
    return Path(value).resolve()


def identifier(value):
    if not isinstance(value, str) or not ID.fullmatch(value):
        raise ValueError("invalid identifier: " + repr(value))
    return value


def contained(path, parent):
    return path == parent or parent in path.parents


def private_path(path):
    """Reject tracked/trackable paths in any enclosing Git worktree."""
    path = Path(path).resolve()
    existing = path if path.is_dir() else path.parent
    while not existing.exists():
        existing = existing.parent
    git = subprocess.run(["git", "-C", str(existing), "rev-parse", "--show-toplevel"], capture_output=True, text=True)
    if git.returncode:
        return True
    root = Path(git.stdout.strip()).resolve()
    relative = str(path.relative_to(root))
    tracked = subprocess.run(["git", "-C", str(root), "ls-files", "--error-unmatch", "--", relative], capture_output=True)
    if tracked.returncode == 0:
        return False
    ignored = subprocess.run(["git", "-C", str(root), "check-ignore", "--no-index", "--quiet", "--", relative], capture_output=True)
    return ignored.returncode == 0


def load_config(path):
    config = read_json(path)
    if not isinstance(config, dict) or config.get("schema_version") != 1:
        raise ValueError("unsupported config schema")
    unknown = set(config) - {"schema_version", "output_root", "timeout_seconds", "models", "cases", "experiments", "metrics"}
    if unknown:
        raise ValueError("unknown config fields: " + str(sorted(unknown)))
    output = absolute(config["output_root"])
    if not private_path(output):
        raise ValueError("output_root must be outside Git or explicitly ignored")
    timeout = config.get("timeout_seconds", 600)
    if type(timeout) not in (int, float) or not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("timeout_seconds must be positive and finite")
    registry = {entry["id"]: entry for entry in read_json(HERE / "models.json")["models"]}
    if not isinstance(config.get("models"), dict) or not config["models"]:
        raise ValueError("models must be a nonempty object")
    profiles = {}
    for key, entry in config["models"].items():
        identifier(key)
        if not isinstance(entry, dict) or set(entry) - {"extends", "path", "runtime", "format", "variant", "decoding", "source"}:
            raise ValueError("invalid model profile: " + key)
        parent = entry.get("extends", key)
        if parent not in registry:
            raise ValueError("unknown model profile: " + str(parent))
        if not isinstance(entry.get("source", {}), dict):
            raise ValueError("source must be an object")
        if "variant" in entry and entry["variant"] != registry[parent]["variant"]:
            source = entry.get("source", {})
            if not all(field in source for field in ("revision", "file", "sha256")) or not any(field in source for field in ("repository", "conversion_repository")):
                raise ValueError("custom variant requires explicit source revision, file and SHA-256")
        definition = {**registry[parent], **entry, "id": key}
        if definition["format"] != registry[parent]["format"] or not isinstance(definition["variant"], str) or not definition["variant"]:
            raise ValueError("unsupported model format or empty variant")
        definition["source"] = {**registry[parent]["source"], **entry.get("source", {})}
        definition["path"] = str(absolute(entry["path"]))
        if contained(output, absolute(entry["path"])) or contained(absolute(entry["path"]), output):
            raise ValueError("model and output paths must be separate")
        runtime = entry.get("runtime", {})
        if not isinstance(runtime, dict) or set(runtime) - {"python", "executable", "device_map", "startup_timeout_seconds"}:
            raise ValueError("invalid runtime options")
        for field in ("python", "executable"):
            if field in runtime:
                absolute(runtime[field])
        startup = runtime.get("startup_timeout_seconds", 600)
        if type(startup) not in (int, float) or not math.isfinite(startup) or startup <= 0:
            raise ValueError("invalid startup timeout")
        decoding = entry.get("decoding", {})
        allowed = {"moss": set(), "qwen": {"max_new_tokens"}, "nemotron": {"max_new_tokens"}, "correction": {"temperature", "max_new_tokens"}}[definition["adapter"]]
        if not isinstance(decoding, dict) or set(decoding) - allowed:
            raise ValueError("unsupported decoding option: " + key)
        if "max_new_tokens" in decoding and (type(decoding["max_new_tokens"]) is not int or decoding["max_new_tokens"] <= 0):
            raise ValueError("max_new_tokens must be a positive integer")
        if definition["adapter"] == "nemotron" and decoding.get("max_new_tokens", 1024) > 8192:
            raise ValueError("Mini adapter max_new_tokens cannot exceed its 8192 context limit")
        if "temperature" in decoding and (type(decoding["temperature"]) not in (int, float) or not math.isfinite(decoding["temperature"]) or decoding["temperature"] < 0):
            raise ValueError("invalid temperature")
        profiles[key] = definition
    cases = config.get("cases")
    experiments = config.get("experiments")
    if not isinstance(cases, list) or not cases or not isinstance(experiments, list) or not experiments:
        raise ValueError("cases and experiments must be nonempty arrays")
    seen = set()
    for case in cases:
        if not isinstance(case, dict) or set(case) - {"id", "audio", "reference", "context"}:
            raise ValueError("invalid case fields")
        key = identifier(case["id"])
        if key in seen:
            raise ValueError("duplicate case ID")
        seen.add(key)
        for field in ("audio", "reference", "context"):
            if field in case:
                p = absolute(case[field])
                if contained(p, output) or contained(output, p):
                    raise ValueError("input and output paths must be separate")
        if "audio" not in case or "reference" not in case:
            raise ValueError("each case requires audio and reference")
    seen = {}
    for experiment in experiments:
        if not isinstance(experiment, dict) or set(experiment) - {"id", "model", "track", "context", "baseline"}:
            raise ValueError("invalid experiment fields")
        key = identifier(experiment["id"])
        if key in seen or experiment.get("model") not in profiles:
            raise ValueError("duplicate experiment ID or missing model")
        model = profiles[experiment["model"]]
        expected_track = "postprocess" if model["adapter"] == "correction" else "asr"
        if experiment.get("track") != expected_track or experiment.get("context", "none") not in ("none", "dictionary"):
            raise ValueError("invalid experiment track or context")
        if expected_track == "asr" and "baseline" in experiment:
            raise ValueError("ASR cannot depend on a baseline")
        if experiment.get("context") == "dictionary" and model["adapter"] in ("moss", "nemotron"):
            raise ValueError("dictionary context is not implemented for " + model["adapter"])
        seen[key] = experiment
    for experiment in experiments:
        if experiment["track"] == "postprocess":
            baseline = seen.get(experiment.get("baseline"))
            if not baseline or baseline["track"] != "asr":
                raise ValueError("postprocess requires an ASR experiment baseline")
    metrics = config.get("metrics", {})
    if not isinstance(metrics, dict) or set(metrics) - {"collar", "overlap_policy"}:
        raise ValueError("invalid metrics options")
    collar = metrics.get("collar", 0)
    if type(collar) not in (float, int) or not math.isfinite(collar) or collar < 0 or metrics.get("overlap_policy", "include") not in ("include", "exclude"):
        raise ValueError("invalid DER policy")
    return config, profiles


def load_reference(path):
    value = read_json(path)
    if not isinstance(value, dict) or set(value) - {"text", "segments", "entities"} or not isinstance(value.get("text"), str):
        raise ValueError("reference requires text, optional segments and entities")
    if not isinstance(value.get("entities", []), list) or any(not isinstance(v, str) or not v for v in value.get("entities", [])):
        raise ValueError("entities must be nonempty strings")
    if not isinstance(value.get("segments", []), list):
        raise ValueError("segments must be an array")
    return value


def load_context(path):
    value = read_json(path) if path else {}
    if not isinstance(value, dict) or set(value) - {"terms", "attendees", "corrections"}:
        raise ValueError("invalid context")
    terms = []
    for field in ("terms", "attendees"):
        items = value.get(field, [])
        if not isinstance(items, list) or any(not isinstance(term, str) or not term for term in items):
            raise ValueError("context terms and attendees must be nonempty strings")
        terms.extend(items)
    rules = value.get("corrections", [])
    if not isinstance(rules, list):
        raise ValueError("corrections must be an array")
    for rule in rules:
        if not isinstance(rule, dict) or set(rule) - {"source", "replacement", "kind", "context_cues"} or any(not isinstance(rule.get(f), str) or not rule[f] for f in ("source", "replacement")):
            raise ValueError("correction requires explicit source and replacement")
        cues = rule.get("context_cues", [])
        if not isinstance(cues, list) or any(not isinstance(cue, str) or not cue for cue in cues):
            raise ValueError("invalid correction context cues")
    return {"terms": list(dict.fromkeys(terms)), "corrections": rules}


def candidates(text, context):
    result = []
    for rule in context["corrections"]:
        if rule["source"] == rule["replacement"]:
            continue
        start = 0
        while (start := text.find(rule["source"], start)) >= 0:
            end = start + len(rule["source"])
            cues = rule.get("context_cues", [])
            if not cues or any(cue in text[max(0, start - 100):end + 100] for cue in cues):
                result.append({"id": "candidate-" + str(len(result)), "start": start, "end": end, "original": rule["source"], "replacement": rule["replacement"]})
            start = end
    ordered = sorted(result, key=lambda item: item["start"])
    if any(a["end"] > b["start"] for a, b in zip(ordered, ordered[1:])):
        raise ValueError("correction rules produce overlapping candidates; refine context rules")
    return ordered


def file_hash(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()
