from pathlib import Path
import argparse
import hashlib
import json
import os
import subprocess


def build(output, target):
    root = Path(__file__).resolve().parents[2]
    source = root / 'scripts/apple-speech/AppleSpeech.swift'
    swift_target = {'aarch64-apple-darwin': 'arm64-apple-macos26.0', 'x86_64-apple-darwin': 'x86_64-apple-macos26.0'}[target]
    output.mkdir(parents=True, exist_ok=True)
    cache = Path(os.environ.get("CLANG_MODULE_CACHE_PATH") or Path(os.environ.get("CARGO_TARGET_DIR", root / ".work/rust-target")) / "swift-module-cache")
    cache.mkdir(parents=True, exist_ok=True)
    executable = output / 'nero-apple-speech'
    subprocess.run(['xcrun', 'swiftc', '-swift-version', '5', '-O', '-parse-as-library', '-target', swift_target, '-module-cache-path', str(cache),
                    str(source), '-o', str(executable)], check=True)
    receipt = {'schemaVersion': 1, 'helper': 'apple-speech-analyzer-v1', 'target': target,
               'sourceSha256': hashlib.sha256(source.read_bytes()).hexdigest(),
               'executableSha256': hashlib.sha256(executable.read_bytes()).hexdigest()}
    (output / 'build.json').write_text(json.dumps(receipt, indent=2) + '\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--target', required=True)
    args = parser.parse_args()
    build(args.out, args.target)
