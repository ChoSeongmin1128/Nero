#!/usr/bin/env python3
"""Build-time only: stage the shared native ASR/diarization runtime, without weights."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import tarfile
import tempfile
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[2]
SPEC = json.loads(Path(__file__).with_name("runtime.json").read_text())
MACHO = {b'\xcf\xfa\xed\xfe', b'\xce\xfa\xed\xfe', b'\xfe\xed\xfa\xcf', b'\xfe\xed\xfa\xce', b'\xca\xfe\xba\xbe', b'\xbe\xba\xfe\xca'}


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def package_files(root):
    files = []
    for path in sorted(root.rglob("*")):
        if path.is_symlink() or not (path.is_file() or path.is_dir()):
            raise ValueError("Linked or special runtime package entry")
        if path.is_file() and path != root / "runtime.json":
            files.append({"path": path.relative_to(root).as_posix(), "bytes": path.stat().st_size, "sha256": sha(path)})
    return files


def package_receipt(root, target=None):
    if root.is_symlink() or not root.is_dir():
        raise ValueError("Missing native speech runtime directory")
    path = root / "runtime.json"
    if path.is_symlink() or not path.is_file() or path.stat().st_size > 1024 * 1024:
        raise ValueError("Missing or invalid speech runtime receipt")
    receipt = json.loads(path.read_text())
    if (receipt.get("runtime") != SPEC or receipt.get("sourceArchiveHashesVerified") is not True
            or receipt.get("target") not in SPEC["assets"] or target is not None and receipt["target"] != target):
        raise ValueError("Speech runtime does not match the pinned target/source")
    profile = SPEC.get("sourceBuilds", {}).get(receipt["target"])
    if receipt.get("sourceBuild") != profile:
        raise ValueError("Speech source-build provenance differs from its pin")
    entries = receipt.get("files")
    if not isinstance(entries, list) or not entries:
        raise ValueError("Speech runtime file receipt is empty")
    names = set()
    backends = {item["backend"] for item in SPEC["assets"][receipt["target"]]}
    for entry in entries:
        name = entry.get("path", "")
        relative = PurePosixPath(name)
        if (not name or relative.is_absolute() or ".." in relative.parts or "\\" in name
                or relative.as_posix() != name or name in names or name == "runtime.json"):
            raise ValueError("Unsafe or duplicate runtime receipt path")
        if (len(relative.parts) < 3 or relative.parts[0] not in backends
                or archive_path("package/" + PurePosixPath(*relative.parts[1:]).as_posix(), "package") is None
                or relative.suffix.lower() in (".gguf", ".safetensors", ".wav", ".m4a", ".mp3", ".p8", ".p12")
                or type(entry.get("bytes")) is not int or entry["bytes"] < 0
                or re.fullmatch(r"[0-9a-f]{64}", entry.get("sha256", "")) is None):
            raise ValueError("Unexpected speech runtime file")
        names.add(name)
    for item in SPEC["assets"][receipt["target"]]:
        backend = item["backend"]
        binary = "nemo-speech.exe" if receipt["target"].startswith("windows") else "nemo-speech"
        if f"{backend}/bin/{binary}" not in names or f"{backend}/share/licenses/nemo-speech/LICENSE" not in names:
            raise ValueError("Speech executable or license is missing")
        if receipt["target"].startswith("windows") and not all(
                f"{backend}/bin/{name}" in names for name in ("msvcp140.dll", "vcruntime140.dll", "vcruntime140_1.dll")):
            raise ValueError("Speech package is missing its pinned app-local C++ runtime")
    return receipt


def verify_staged(root, target=None, execute_runtime=False):
    root = Path(root)
    receipt = package_receipt(root, target)
    if package_files(root) != receipt["files"]:
        raise ValueError("Speech runtime files differ from the verified archive")
    if execute_runtime:
        for item in SPEC["assets"][receipt["target"]]:
            binary = root / item["backend"] / "bin" / ("nemo-speech.exe" if receipt["target"].startswith("windows") else "nemo-speech")
            result = subprocess.run([str(binary), "--version"], capture_output=True, text=True, timeout=15, check=True)
            if result.stdout.strip() != "nemo-speech 0.2.0":
                raise ValueError("Unexpected speech runtime version")
    return receipt


def signed_transition(root, before):
    root = Path(root)
    if package_receipt(root) != before:
        raise ValueError("Speech runtime receipt changed during signing")
    after = package_files(root)
    if [entry["path"] for entry in after] != [entry["path"] for entry in before["files"]]:
        raise ValueError("Speech runtime file inventory changed during signing")
    for old, new in zip(before["files"], after):
        with (root / new["path"]).open("rb") as stream:
            is_binary = stream.read(4) in MACHO
        if old != new and not is_binary:
            raise ValueError("Speech runtime resource changed during signing")
    return {"beforeSigning": before, "signedFiles": after}


def verify_signed(root, receipt):
    if not isinstance(receipt, dict) or package_receipt(Path(root)) != receipt.get("beforeSigning"):
        raise ValueError("Missing or changed signed speech runtime receipt")
    if package_files(Path(root)) != receipt.get("signedFiles"):
        raise ValueError("Signed speech runtime differs from its preserved receipt")


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


def prepare(target, cache, output, redist=None, cmake=None):
    cmake = cmake or ["cmake"]
    if target.startswith("windows") and redist is not None:
        print("Using the verified archives' app-local C++ runtime; external CRT files are not substituted.")
    if output.exists():
        raise ValueError("Destination exists; preserve it and use a new destination")
    cache.mkdir(parents=True, exist_ok=True)
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="diarization-", dir=output.parent) as temporary:
        stage = Path(temporary) / "diarization"
        stage.mkdir()
        profile = SPEC.get("sourceBuilds", {}).get(target)
        if profile:
            spec = importlib.util.spec_from_file_location("nero_speech_source", Path(__file__).with_name("source_build.py"))
            module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(module)
            module.build(profile, cache, stage / "gpu", cmake, archive_path)
        for item in ([] if profile else SPEC["assets"][target]):
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
            if target.startswith("windows"):
                required = {"msvcp140.dll", "vcruntime140.dll", "vcruntime140_1.dll"}
                if not all((destination / "bin" / name).is_file() for name in required):
                    raise ValueError("Pinned speech archive is missing its app-local C++ runtime")
        receipt = {
            "target": target,
            "runtime": SPEC,
            "sourceArchiveHashesVerified": True,
            "sourceBuild": profile,
            "fileHashScope": "before Developer ID code signing",
            "files": package_files(stage),
        }
        (stage / "runtime.json").write_text(json.dumps(receipt, indent=2) + "\n")
        os.rename(stage, output)
    print("Prepared native diarization runtime:", target)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--target", choices=SPEC["assets"])
    parser.add_argument("--cache", type=Path, default=ROOT / ".work/diarization-runtime-downloads")
    parser.add_argument("--output", type=Path, default=ROOT / "apps/desktop/.generated/native/diarization")
    parser.add_argument("--msvc-redist", type=Path, help="Compatibility argument; the verified speech archives own their CRT files")
    parser.add_argument("--cmake", default="cmake")
    parser.add_argument("--cmake-python", type=Path)
    parser.add_argument("--verify-staged", type=Path)
    parser.add_argument("--optional", action="store_true")
    parser.add_argument("--execute-runtime", action="store_true")
    args = parser.parse_args()
    if args.verify_staged is not None:
        if args.optional and not args.verify_staged.exists() and not args.verify_staged.is_symlink():
            print("Native speech runtime absent: recognition/diarization unavailable.")
        else:
            verify_staged(args.verify_staged, args.target, args.execute_runtime)
            print("Verified shared native speech runtime.")
    else:
        if args.target is None:
            parser.error("--target is required to prepare a runtime")
        prepare(args.target, args.cache, args.output, args.msvc_redist,
                [str(args.cmake_python), "-m", "cmake"] if args.cmake_python else [args.cmake])
