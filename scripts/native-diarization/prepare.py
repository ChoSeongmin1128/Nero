#!/usr/bin/env python3
"""Build-time only: verify and stage the pinned native diarizer, without weights."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import tarfile
import tempfile
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[2]
SPEC = json.loads(Path(__file__).with_name("runtime.json").read_text())


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def archive_path(name, prefix):
    path = PurePosixPath(name)
    if path.is_absolute() or ".." in path.parts or not path.parts or path.parts[0] != prefix:
        raise ValueError("Unsafe runtime archive path")
    relative = PurePosixPath(*path.parts[1:])
    if not relative.parts:
        return None
    if relative.parts[0] == "share" and relative.parts[1:2] == ("licenses",):
        return relative
    if relative.parts[0] == "bin" and relative.name in ("nemo-speech", "nemo-speech.exe"):
        return relative
    if relative.parts[0] in ("bin", "lib") and relative.suffix in (".dylib", ".dll", ".metal", ".metallib"):
        return relative
    return None


def extract(archive, destination, item):
    prefix = item["name"].removesuffix(".tar.gz").removesuffix(".zip")

    def write(name, stream):
        relative = archive_path(name, prefix)
        if relative is None:
            return
        path = destination.joinpath(*relative.parts)
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open("xb") as output:
            shutil.copyfileobj(stream, output)
        if relative.parts[0] == "bin" or relative.suffix == ".dylib":
            path.chmod(0o755)

    if archive.suffix == ".zip":
        with zipfile.ZipFile(archive) as bundle:
            for item in bundle.infolist():
                if item.is_dir():
                    continue
                if (item.external_attr >> 16) & 0o170000 == 0o120000:
                    raise ValueError("Linked ZIP archive entry")
                with bundle.open(item) as stream:
                    write(item.filename, stream)
    else:
        with tarfile.open(archive) as bundle:
            for item in bundle:
                if item.isfile() or item.issym() or item.islnk():
                    # Materialize official dylib aliases, retaining the original bin/lib layout.
                    if archive_path(item.name, prefix) is not None:
                        with bundle.extractfile(item) as stream:
                            write(item.name, stream)


def prepare(target, cache, output, redist=None):
    crt = {}
    if target.startswith("windows"):
        # The existing summary packager owns the app-local MSVC redistributable contract.
        spec = importlib.util.spec_from_file_location("nero_summary_build", ROOT / "scripts/native-summary/prepare.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        crt = module.windows_crt(redist)
    if output.exists():
        raise ValueError("Destination exists; preserve it and use a new destination")
    cache.mkdir(parents=True, exist_ok=True)
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="diarization-", dir=output.parent) as temporary:
        stage = Path(temporary) / "diarization"
        stage.mkdir()
        for item in SPEC["assets"][target]:
            archive = cache / item["name"]
            if not archive.is_file() or archive.stat().st_size != item["bytes"] or sha(archive) != item["sha256"]:
                partial = cache / (item["name"] + ".part")
                url = f"https://github.com/{SPEC['repository']}/releases/download/{SPEC['version']}/{item['name']}"
                request = urllib.request.Request(url, headers={"User-Agent": "Nero-build"})
                with urllib.request.urlopen(request, timeout=60) as response, partial.open("wb") as stream:
                    size = 0
                    while data := response.read(1024 * 1024):
                        size += len(data)
                        if size > item["bytes"]:
                            raise ValueError("Runtime archive exceeded pinned size")
                        stream.write(data)
                if partial.stat().st_size != item["bytes"] or sha(partial) != item["sha256"]:
                    raise ValueError("Runtime archive checksum mismatch")
                partial.replace(archive)
            destination = stage / item["backend"]
            destination.mkdir()
            extract(archive, destination, item)
            binary = destination / "bin" / ("nemo-speech.exe" if target.startswith("windows") else "nemo-speech")
            if not binary.is_file():
                raise ValueError("Missing diarization executable")
            if not (destination / "share/licenses/nemo-speech/LICENSE").is_file():
                raise ValueError("Missing runtime license")
            for name, source in crt.items():
                shutil.copyfile(source, destination / "bin" / name)
        receipt = {
            "target": target,
            "runtime": SPEC,
            "sourceArchiveHashesVerified": True,
            "fileHashScope": "before Developer ID code signing",
            "files": [
                {"path": str(path.relative_to(stage)), "bytes": path.stat().st_size, "sha256": sha(path)}
                for path in sorted(stage.rglob("*")) if path.is_file()
            ],
        }
        (stage / "runtime.json").write_text(json.dumps(receipt, indent=2) + "\n")
        os.rename(stage, output)
    print("Prepared native diarization runtime:", target)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--target", required=True, choices=SPEC["assets"])
    parser.add_argument("--cache", type=Path, default=ROOT / ".work/diarization-runtime-downloads")
    parser.add_argument("--output", type=Path, default=ROOT / "apps/desktop/.generated/native/diarization")
    parser.add_argument("--msvc-redist", type=Path)
    args = parser.parse_args()
    prepare(args.target, args.cache, args.output, args.msvc_redist)
