"""Sequential subprocess runner; ground truth never enters worker requests."""
from datetime import datetime, timezone
import math
import os
from pathlib import Path
import platform
import re
import signal
import subprocess
import sys
import tempfile
import time
import uuid
import wave

from metrics import correction_metrics, diarization_metrics, entity_metrics, text_metrics
from protocol import HERE, ROOT, absolute, candidates, contained, file_hash, load_context, load_reference, private_path, read_json, write_json
from report import write_report

ADAPTER = HERE / "adapters" / "adapter.py"


def cancel_from_signal(signum, frame):
    raise KeyboardInterrupt("evaluation terminated by signal " + str(signum))


def hardware():
    return {"os": platform.system(), "release": platform.release(), "machine": platform.machine(), "cpu": platform.processor(), "gpu": "not_measured", "memory": "not_measured"}


def source_snapshot():
    def git(*args):
        result = subprocess.run(["git", "-C", str(ROOT), *args], capture_output=True, text=True)
        return result.stdout.strip() if result.returncode == 0 else None
    return {"commit": git("rev-parse", "HEAD"), "dirty": bool(git("status", "--porcelain")), "tool_sha256": {str(p.relative_to(HERE)): file_hash(p) for p in HERE.rglob("*") if p.suffix in (".py", ".json") and "tests" not in p.parts}}


def audio_metadata(path):
    with wave.open(str(path), "rb") as audio:
        if (audio.getnchannels(), audio.getframerate(), audio.getsampwidth(), audio.getcomptype()) != (1, 16000, 2, "NONE") or not audio.getnframes():
            raise ValueError("evaluation input must be nonempty mono 16 kHz PCM16 WAV; prepare one shared input explicitly")
        return {"frames": audio.getnframes(), "sample_rate": 16000, "duration_seconds": audio.getnframes() / 16000, "sha256": file_hash(path)}


def preflight(case, experiment, model):
    issues = []
    for field in ("audio", "reference") + (("context",) if experiment.get("context") == "dictionary" else ()):
        value = case.get(field)
        if not value or not Path(value).is_file():
            issues.append("missing local " + field)
        elif not private_path(value):
            issues.append(field + " must be outside Git or explicitly ignored")
    if not Path(model["path"]).exists():
        issues.append("missing local model")
    elif model["adapter"] != "correction" and not (Path(model["path"]) / "config.json").is_file():
        issues.append("speech model directory requires a local config.json")
    for field in ("python",) + (("executable",) if model["adapter"] != "qwen" else ()):
        executable = model.get("runtime", {}).get(field)
        if not executable or not Path(executable).is_file() or not os.access(executable, os.X_OK):
            issues.append("missing executable runtime." + field)
    if model["adapter"] == "moss" and (platform.system() != "Darwin" or platform.machine() not in ("arm64", "aarch64")):
        issues.append("MOSS native harness requires Apple silicon macOS")
    if model["adapter"] == "nemotron" and platform.system() != "Linux":
        issues.append("Mini W4A16 adapter requires Linux/CUDA; Mac route unverified")
    source = model["source"]
    if not isinstance(source.get("revision"), str) or not re.fullmatch(r"[0-9a-f]{40,64}", source["revision"]):
        issues.append("model source requires an immutable revision")
    if model["format"] == "gguf":
        if not any(isinstance(source.get(key), str) and source[key] and "REPLACE_WITH" not in source[key] for key in ("repository", "conversion_repository")):
            issues.append("GGUF requires its actual conversion repository")
        if not isinstance(source.get("file"), str) or not source["file"] or "REPLACE_WITH" in source["file"]:
            issues.append("GGUF requires its original source filename; local aliases are allowed and hash-checked")
        if not isinstance(source.get("sha256"), str) or not re.fullmatch(r"[0-9a-f]{64}", source["sha256"]):
            issues.append("GGUF requires its actual SHA-256")
    return issues


def request_for(model, experiment, output_dir, case=None, baseline=None, context=None):
    request = {"schema_version": 1, "adapter": model["adapter"], "task": experiment["track"], "model": {key: model[key] for key in ("id", "path", "format", "variant")}, "runtime": model.get("runtime", {}), "output_dir": str(output_dir), "decoding": model.get("decoding", {}), "context": {"terms": (context or {}).get("terms", [])}}
    if experiment["track"] == "asr":
        if case:
            request["audio_path"] = str(absolute(case["audio"]))
    else:
        request["text"] = baseline or ""
        request["candidates"] = candidates(baseline or "", context or {"corrections": []})
    return request


def stop_group(proc):
    """Only the isolated evaluation process group, never an installed app."""
    try:
        os.killpg(proc.pid, signal.SIGTERM)
    except ProcessLookupError:
        return
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        pass
    # A server grandchild may survive its parent. Kill the original group too.
    try:
        os.killpg(proc.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    proc.wait()


def execute(command, cwd, log, timeout):
    if os.name != "posix":
        raise ValueError("evaluation runner currently requires POSIX process groups; Windows execution is unsupported")
    env = os.environ.copy()
    env.update({"HF_HUB_OFFLINE": "1", "TRANSFORMERS_OFFLINE": "1", "HF_DATASETS_OFFLINE": "1", "DO_NOT_TRACK": "1", "VLLM_NO_USAGE_STATS": "1", "PYTHONDONTWRITEBYTECODE": "1", "OMP_NUM_THREADS": "2", "MKL_NUM_THREADS": "2", "TOKENIZERS_PARALLELISM": "false"})
    started = time.monotonic()
    with open(log, "xb") as handle:
        proc = subprocess.Popen(command, cwd=cwd, env=env, stdout=handle, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            code = proc.wait(timeout=timeout)
            status = "returned"
        except subprocess.TimeoutExpired:
            status, code = "timeout", None
        except BaseException:
            stop_group(proc)
            raise
        finally:
            stop_group(proc)
    return {"status": status, "exit_code": code, "wall_seconds": time.monotonic() - started, "memory": {"cpu_peak_bytes": None, "gpu_peak_bytes": None, "scope": "not measured"}, "timing_scope": "cold subprocess, model load/server startup, inference and shutdown; excludes input/model hashing and scoring"}


def doctor_model(model, experiment):
    with tempfile.TemporaryDirectory(prefix="nero-eval-doctor-") as temporary:
        directory = Path(temporary)
        request = directory / "request.json"
        write_json(request, request_for(model, experiment, directory))
        receipt = execute([model["runtime"]["python"], "-B", str(ADAPTER), "--doctor", str(request)], directory, directory / "doctor.log", 15)
        if receipt["status"] != "returned" or receipt["exit_code"] != 0:
            raise ValueError("adapter doctor failed: " + (directory / "doctor.log").read_text(encoding="utf-8", errors="replace")[-1000:])
        checks = read_json(directory / "doctor.log")
        if any(value is False for value in checks.values()):
            raise ValueError("adapter prerequisites failed: " + str(checks))
        return checks


def plan(config, profiles, doctor=False):
    rows = []
    checked = {}
    for case in config["cases"]:
        for experiment in config["experiments"]:
            model = profiles[experiment["model"]]
            issues = preflight(case, experiment, model)
            if doctor and not issues:
                try:
                    audio_metadata(case["audio"])
                    reference = load_reference(case["reference"])
                    diarization_metrics(reference.get("segments", []), [])
                    if experiment.get("context") == "dictionary":
                        context = load_context(case["context"])
                        if not context["terms"] and not context["corrections"]:
                            raise ValueError("dictionary context is empty")
                    if model["id"] not in checked:
                        checked[model["id"]] = doctor_model(model, experiment)
                except (OSError, ValueError, KeyError, TypeError, wave.Error) as error:
                    issues.append(str(error))
            rows.append({"case_id": case["id"], "experiment": experiment["id"], "model": model["id"], "track": experiment["track"], "context": experiment.get("context", "none"), "status": "blocked" if issues else "ready_for_explicit_run", "issues": issues, "checks": checked.get(model["id"]), "runtime_validation": "not executed; doctor only checks prerequisites", "source": model["source"]})
    return rows


def model_fingerprint(model):
    root = Path(model["path"])
    files = sorted(p for p in root.rglob("*") if p.is_file()) if root.is_dir() else [root]
    if not files:
        raise ValueError("model directory contains no files")
    entries = []
    for p in files:
        before = p.stat()
        digest = file_hash(p)
        after = p.stat()
        if (before.st_size, before.st_mtime_ns) != (after.st_size, after.st_mtime_ns):
            raise ValueError("model changed while fingerprinting")
        entries.append({"file": str(p.relative_to(root)) if root.is_dir() else p.name, "bytes": before.st_size, "sha256": digest})
    expected = model["source"].get("sha256")
    if root.is_file() and expected and entries[0]["sha256"] != expected:
        raise ValueError("model SHA-256 differs from declared source")
    return {"source_declared": model["source"], "files_measured": entries, "source_revision_content_match": "single-file SHA verified" if root.is_file() and expected else "not independently verified against remote revision"}


def validate_result(result, duration):
    if not isinstance(result, dict) or not isinstance(result.get("text"), str) or not isinstance(result.get("segments", []), list) or not isinstance(result.get("completion"), dict):
        raise ValueError("invalid adapter result")
    for segment in result.get("segments", []):
        if not isinstance(segment, dict) or any(type(segment.get(k)) not in (float, int) or not math.isfinite(segment[k]) for k in ("start", "end")) or not 0 <= segment["start"] <= segment["end"] <= duration + .5:
            raise ValueError("invalid native segment bounds")


def run(config, profiles):
    os.umask(0o077)
    root = Path(config["output_root"])
    root.mkdir(parents=True, exist_ok=True, mode=0o700)
    directory = root / (datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ-") + uuid.uuid4().hex[:8])
    directory.mkdir(mode=0o700)
    write_json(directory / "config.json", config)
    write_json(directory / "provenance.json", {"schema_version": 1, "hardware": hardware(), "tool": source_snapshot(), "policy": "one cold process per cell, sequential; no warm timing or GPU/RAM measurement; model bytes hashed once per profile"})
    rows, fingerprints, checks, completed = [], {}, {}, {}
    ordered = sorted(config["experiments"], key=lambda e: e["track"] != "asr")
    try:
        for case in config["cases"]:
            for experiment in ordered:
                model = profiles[experiment["model"]]
                cell = directory / case["id"] / experiment["id"]
                cell.mkdir(parents=True, mode=0o700)
                row = {"case_id": case["id"], "experiment": experiment["id"], "track": experiment["track"], "model": model["id"], "hardware": hardware(), "decoding": model.get("decoding", {}), "context": experiment.get("context", "none"), "status": "failed", "metrics": {}, "full_input_completed": None, "timed_words": None}
                try:
                    issues = preflight(case, experiment, model)
                    if issues:
                        row["status"] = "unsupported"
                        raise ValueError("; ".join(issues))
                    audio = audio_metadata(case["audio"])
                    reference = load_reference(case["reference"])
                    diarization_metrics(reference.get("segments", []), [])
                    context = load_context(case.get("context")) if experiment.get("context") == "dictionary" else {"terms": [], "corrections": []}
                    if experiment.get("context") == "dictionary" and not context["terms"] and not context["corrections"]:
                        raise ValueError("dictionary context is empty")
                    if experiment["track"] == "asr" and experiment.get("context") == "dictionary" and not context["terms"]:
                        raise ValueError("ASR dictionary needs terms/attendees, not only correction rules")
                    row["inputs"] = {"audio": audio, "reference_sha256": file_hash(case["reference"]), "context_sha256": file_hash(case["context"]) if experiment.get("context") == "dictionary" else None}
                    baseline = None
                    if experiment["track"] == "postprocess":
                        baseline = completed.get((case["id"], experiment["baseline"]))
                        if baseline is None:
                            row["status"] = "not_scored"
                            raise ValueError("baseline failed or missing; postprocess skipped")
                    if model["id"] not in checks:
                        checks[model["id"]] = doctor_model(model, experiment)
                    if model["id"] not in fingerprints:
                        fingerprints[model["id"]] = model_fingerprint(model)
                        write_json(directory / "models.json", fingerprints)
                    runtime = model["runtime"]
                    row["runtime_files"] = {key: {"path": value, "sha256": file_hash(value)} for key, value in runtime.items() if key in ("python", "executable")}
                    worker = cell / "worker"
                    worker.mkdir(mode=0o700)
                    request = request_for(model, experiment, worker, case, baseline["text"] if baseline else None, context)
                    if experiment["track"] == "postprocess" and not request["candidates"]:
                        row["status"] = "not_scored"
                        raise ValueError("no explicit correction candidates matched baseline; skipped model execution")
                    write_json(cell / "request.json", request)
                    row["execution"] = execute([runtime["python"], "-B", str(ADAPTER), str(cell / "request.json")], cell, cell / "adapter.log", config.get("timeout_seconds", 600))
                    row["execution"]["rtf_end_to_end"] = row["execution"]["wall_seconds"] / audio["duration_seconds"]
                    if row["execution"]["status"] != "returned" or row["execution"]["exit_code"] != 0:
                        raise ValueError("adapter failed or timed out; preserved adapter.log and partial worker output")
                    result = read_json(worker / "result.json")
                    validate_result(result, audio["duration_seconds"])
                    row["completion"] = result["completion"]
                    complete = result["completion"].get("status") == "complete"
                    if baseline is not None:
                        complete = complete and baseline["full_input_completed"] is True
                    row["full_input_completed"] = complete
                    row["status"] = "scored" if complete else "unqualified"
                    row["decoding"] = result.get("metadata", {}).get("decoding", row["decoding"])
                    row["adapter_metadata"] = result.get("metadata", {})
                    scoring_supported = result.get("metadata", {}).get("text_scoring_supported", True)
                    if scoring_supported:
                        row["metrics"]["text"] = text_metrics(reference["text"], result["text"])
                        row["metrics"]["entities"] = entity_metrics(reference["text"], result["text"], reference.get("entities", []))
                    else:
                        row["status"] = "unqualified"
                        row["metrics"]["text"] = {"status": "not_scored", "reason": "native format markers need a reviewed parser; raw output preserved"}
                    segments = result.get("segments", [])
                    speakers = [s for s in segments if s.get("speaker") and s["end"] > s["start"]]
                    settings = config.get("metrics", {})
                    if model["adapter"] == "moss" and any(s["end"] == s["start"] or s["end"] > audio["duration_seconds"] for s in segments):
                        row["metrics"]["diarization"] = {"status": "not_scored", "reason": "native zero-duration or beyond-EOF segments cannot be projected into DER; original output preserved"}
                    else:
                        row["metrics"]["diarization"] = diarization_metrics(reference.get("segments", []), speakers, collar=settings.get("collar", 0), overlap_policy=settings.get("overlap_policy", "include"), output_available=model["adapter"] == "moss")
                    if baseline is not None:
                        row["metrics"]["correction"] = correction_metrics(reference["text"], baseline["text"], result["text"])
                        row["baseline"] = experiment["baseline"]
                    if row["inputs"]["audio"]["sha256"] != file_hash(case["audio"]) or row["inputs"]["reference_sha256"] != file_hash(case["reference"]) or (row["inputs"]["context_sha256"] and row["inputs"]["context_sha256"] != file_hash(case["context"])):
                        raise ValueError("evaluation input changed during execution; result disqualified")
                    if scoring_supported:
                        completed[(case["id"], experiment["id"])] = {"text": result["text"], "full_input_completed": complete}
                except (OSError, ValueError, KeyError, TypeError, wave.Error) as error:
                    if row["status"] not in ("unsupported", "not_scored"):
                        row["status"] = "failed"
                    row["error"] = str(error)
                finally:
                    write_json(cell / "receipt.json", row)
                    rows.append(row)
                    write_json(directory / "results.json", rows)
    except KeyboardInterrupt:
        write_json(directory / "interrupted.json", {"status": "cancelled", "finished_cells": len(rows)})
        raise
    finally:
        write_report(rows, directory / "report")
    return directory, rows
