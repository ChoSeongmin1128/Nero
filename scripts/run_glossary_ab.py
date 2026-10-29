#!/usr/bin/env python3
"""Run baseline/domain/domain+decoy conditioning through both local engines."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import re
import signal
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from glossary_context import CONDITIONS, build_condition_terms, build_whisper_prompt, load_decoy_terms


ROOT = Path(__file__).parents[1]
WHISPERKIT_EFFECTIVE_PROMPT_TOKEN_LIMIT = 223


def run(command: list[str]) -> None:
    print("+", " ".join(command), flush=True)
    subprocess.run(command, cwd=ROOT, check=True)


def port_ready(host: str, port: int) -> bool:
    try:
        with socket.create_connection((host, port), timeout=0.25):
            return True
    except OSError:
        return False


def export_conditions(
    output_dir: Path,
    profile: str,
    glossary: Path,
    speakers: Path,
    decoy_path: Path,
) -> None:
    for condition in CONDITIONS:
        terms = build_condition_terms(
            condition,
            glossary_path=glossary,
            speaker_path=speakers,
            decoy_path=decoy_path,
            profile_name=profile,
        )
        decoys = load_decoy_terms(decoy_path) if condition == "domain-decoy" else []
        prompt, whisper_terms = build_whisper_prompt(terms, required_terms=decoys)
        (output_dir / f"{condition}-apple-context.json").write_text(
            json.dumps(terms, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
        )
        (output_dir / f"{condition}-whisper-prompt.txt").write_text(
            prompt + ("\n" if prompt else ""), encoding="utf-8"
        )
        (output_dir / f"{condition}-manifest.json").write_text(
            json.dumps(
                {
                    "condition": condition,
                    "profile": profile,
                    "termCount": len(terms),
                    "decoyCount": len(decoys),
                    "terms": terms,
                    "whisperTerms": whisper_terms,
                    "decoys": decoys,
                    "whisperPrompt": prompt,
                },
                ensure_ascii=False,
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )


def validate_whisper_prompt_tokens(
    *,
    audio_dir: Path,
    context_dir: Path,
    server_log_path: Path,
    host: str,
    port: int,
    model: str,
) -> None:
    """Abort before the full run if WhisperKit would suffix-trim a prompt."""
    for condition in ("domain", "domain-decoy"):
        with tempfile.TemporaryDirectory(prefix="grove-prompt-preflight-") as directory:
            run(
                [
                    sys.executable,
                    str(ROOT / "scripts/run_whisper.py"),
                    "--audio-dir",
                    str(audio_dir),
                    "--output",
                    str(Path(directory) / f"{condition}.jsonl"),
                    "--server",
                    f"http://{host}:{port}",
                    "--model",
                    model,
                    "--prompt-file",
                    str(context_dir / f"{condition}-whisper-prompt.txt"),
                    "--conditioning-mode",
                    f"preflight-{condition}",
                    "--limit",
                    "1",
                ]
            )
        encoded = re.findall(
            r"Encoded prompt tokens: \[([^\]]*)\]",
            server_log_path.read_text(encoding="utf-8", errors="replace"),
        )
        if not encoded:
            raise SystemExit(
                "WhisperKit prompt token preflight produced no verbose token log"
            )
        token_count = len([token for token in encoded[-1].split(",") if token.strip()])
        manifest_path = context_dir / f"{condition}-manifest.json"
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        manifest["whisperPromptTokenCount"] = token_count
        manifest["whisperEffectivePromptTokenLimit"] = (
            WHISPERKIT_EFFECTIVE_PROMPT_TOKEN_LIMIT
        )
        manifest_path.write_text(
            json.dumps(manifest, ensure_ascii=False, indent=2) + "\n",
            encoding="utf-8",
        )
        print(
            f"Whisper prompt preflight: {condition} "
            f"{token_count}/{WHISPERKIT_EFFECTIVE_PROMPT_TOKEN_LIMIT} tokens"
        )
        if token_count > WHISPERKIT_EFFECTIVE_PROMPT_TOKEN_LIMIT:
            raise SystemExit(
                f"{condition} prompt has {token_count} tokens; WhisperKit would keep only "
                f"the final {WHISPERKIT_EFFECTIVE_PROMPT_TOKEN_LIMIT}. Reduce the glossary."
            )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--audio-dir", type=Path, required=True)
    parser.add_argument("--labels-dir", type=Path)
    parser.add_argument("--profile", default="example")
    parser.add_argument(
        "--glossary", type=Path, default=ROOT / "tests/fixtures/domain-glossary.json"
    )
    parser.add_argument(
        "--speakers", type=Path, default=ROOT / "tests/fixtures/speaker-hints.json"
    )
    parser.add_argument(
        "--decoys", type=Path, default=ROOT / "tests/fixtures/glossary-decoys.json"
    )
    parser.add_argument("--output-dir", type=Path)
    parser.add_argument("--limit", type=int)
    parser.add_argument("--skip-apple", action="store_true")
    parser.add_argument("--skip-whisper", action="store_true")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=50060)
    parser.add_argument("--model", default="large-v3-v20240930_626MB")
    args = parser.parse_args()

    audio_dir = args.audio_dir.resolve()
    if not list(audio_dir.glob("*.wav")):
        raise SystemExit(f"No WAV files found at {audio_dir}")
    timestamp = dt.datetime.now().astimezone().strftime("%Y%m%d-%H%M%S")
    output_dir = (args.output_dir or ROOT / "results/glossary-ab" / timestamp).resolve()
    context_dir = output_dir / "contexts"
    raw_dir = output_dir / "raw"
    context_dir.mkdir(parents=True, exist_ok=True)
    raw_dir.mkdir(parents=True, exist_ok=True)
    export_conditions(
        context_dir,
        args.profile,
        args.glossary.resolve(),
        args.speakers.resolve(),
        args.decoys.resolve(),
    )

    run_manifest = {
        "createdAt": dt.datetime.now().astimezone().isoformat(timespec="seconds"),
        "audioDirectory": str(audio_dir),
        "profile": args.profile,
        "model": args.model,
        "conditions": list(CONDITIONS),
        "labelsAvailable": bool(args.labels_dir),
        "note": "Domain-decoy includes approved domain and speaker hints plus ten unrelated decoy terms.",
    }
    (output_dir / "run-manifest.json").write_text(
        json.dumps(run_manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )

    run_arguments: list[tuple[str, str, Path]] = []
    if not args.skip_apple:
        run(["swift", "build", "--package-path", str(ROOT / "legacy/swift"), "--scratch-path", str(ROOT / ".work/legacy-swift/swift-build"), "-c", "release"])
        for condition in CONDITIONS:
            output = raw_dir / f"apple-{condition}.jsonl"
            command = [
                str(ROOT / ".work/legacy-swift/swift-build/release/grove-apple-benchmark"),
                "--audio-dir",
                str(audio_dir),
                "--output",
                str(output),
                "--locale",
                "ko-KR",
                "--context-file",
                str(context_dir / f"{condition}-apple-context.json"),
                "--conditioning-mode",
                condition,
            ]
            if args.limit is not None:
                command.extend(["--limit", str(args.limit)])
            run(command)
            run_arguments.append(("apple", condition, output))

    server: subprocess.Popen | None = None
    server_log = None
    try:
        if not args.skip_whisper:
            if port_ready(args.host, args.port):
                raise SystemExit(
                    f"{args.host}:{args.port} is already in use; stop the existing server or choose another port"
                )
            server_log = (output_dir / "whisperkit-server.log").open("w", encoding="utf-8")
            server = subprocess.Popen(
                [
                    "whisperkit-cli",
                    "serve",
                    "--model",
                    args.model,
                    "--language",
                    "ko",
                    "--host",
                    args.host,
                    "--port",
                    str(args.port),
                    "--concurrent-worker-count",
                    "1",
                    "--chunking-strategy",
                    "none",
                    "--verbose",
                ],
                cwd=ROOT,
                stdout=server_log,
                stderr=subprocess.STDOUT,
            )
            for _ in range(240):
                if port_ready(args.host, args.port):
                    break
                if server.poll() is not None:
                    raise SystemExit(f"WhisperKit server failed; see {output_dir / 'whisperkit-server.log'}")
                time.sleep(1)
            else:
                raise SystemExit("WhisperKit server did not become ready within 240 seconds")

            server_log.flush()
            validate_whisper_prompt_tokens(
                audio_dir=audio_dir,
                context_dir=context_dir,
                server_log_path=output_dir / "whisperkit-server.log",
                host=args.host,
                port=args.port,
                model=args.model,
            )

            for condition in CONDITIONS:
                output = raw_dir / f"whisperkit-{condition}.jsonl"
                command = [
                    sys.executable,
                    str(ROOT / "scripts/run_whisper.py"),
                    "--audio-dir",
                    str(audio_dir),
                    "--output",
                    str(output),
                    "--server",
                    f"http://{args.host}:{args.port}",
                    "--model",
                    args.model,
                    "--prompt-file",
                    str(context_dir / f"{condition}-whisper-prompt.txt"),
                    "--conditioning-mode",
                    condition,
                ]
                if args.limit is not None:
                    command.extend(["--limit", str(args.limit)])
                run(command)
                run_arguments.append(("whisperkit", condition, output))
    finally:
        if server is not None and server.poll() is None:
            server.send_signal(signal.SIGINT)
            try:
                server.wait(timeout=30)
            except subprocess.TimeoutExpired:
                server.terminate()
                server.wait(timeout=10)
        if server_log is not None:
            server_log.close()

    analysis_command = [
        sys.executable,
        str(ROOT / "scripts/analyze_glossary_ab.py"),
        "--context-dir",
        str(context_dir),
        "--output-dir",
        str(output_dir / "analysis"),
    ]
    for engine, condition, path in run_arguments:
        analysis_command.extend(["--run", engine, condition, str(path)])
    if args.labels_dir:
        analysis_command.extend(["--labels-dir", str(args.labels_dir.resolve())])
    run(analysis_command)
    print(f"Glossary A/B complete: {output_dir}")


if __name__ == "__main__":
    main()
