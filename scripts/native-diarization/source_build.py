import hashlib
import os
import platform
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import urllib.request


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def archive(item, cache):
    path = cache / (item["repository"].replace("/", "-") + "-" + item["commit"] + ".tar.gz")
    if path.is_file() and path.stat().st_size == item["bytes"] and digest(path) == item["sha256"]:
        return path
    request = urllib.request.Request(
        f"https://codeload.github.com/{item['repository']}/tar.gz/{item['commit']}",
        headers={"User-Agent": "Nero-build"})
    partial = path.with_suffix(".part")
    with urllib.request.urlopen(request, timeout=60) as response, partial.open("wb") as output:
        size = 0
        while data := response.read(1024 * 1024):
            size += len(data)
            if size > item["bytes"]:
                raise ValueError("Source archive exceeds its pinned size")
            output.write(data)
    if partial.stat().st_size != item["bytes"] or digest(partial) != item["sha256"]:
        raise ValueError("Source archive checksum mismatch")
    partial.replace(path)
    return path


def unpack(path, destination):
    destination.mkdir()
    with tarfile.open(path) as bundle:
        if sum(item.size for item in bundle) > 512 * 1024 * 1024:
            raise ValueError("Source archive is too large")
        bundle.extractall(destination, filter="data")
    children = list(destination.iterdir())
    if len(children) != 1 or children[0].is_symlink() or not children[0].is_dir():
        raise ValueError("Source archive must contain one directory")
    return children[0]


def patch_source(source, profile, root, env):
    patch = profile["patch"]
    path = root / patch["file"]
    if path.stat().st_size != patch["bytes"] or digest(path) != patch["sha256"]:
        raise ValueError("Runtime patch checksum mismatch")
    for item in patch["files"]:
        file = source / item["path"]
        before = digest(file) if file.exists() else None
        if before != item["baseSha256"]:
            raise ValueError("Runtime source differs before patch: " + item["path"])
    for option in (["--check"], []):
        subprocess.run(["git", "apply", *option, str(path)], cwd=source, env=env, check=True)
    for item in patch["files"]:
        if digest(source / item["path"]) != item["resultSha256"]:
            raise ValueError("Runtime source differs after patch: " + item["path"])


def build(profile, cache, destination, cmake, package_path):
    if platform.system() != "Darwin" or platform.machine() != "arm64":
        raise ValueError("The patched source runtime is qualified only for native macOS arm64")
    cache.mkdir(parents=True, exist_ok=True)
    archives = [archive(item, cache) for item in profile["sources"]]
    with tempfile.TemporaryDirectory(prefix="speech-source-", dir=cache.parent) as directory:
        work = Path(directory).resolve()
        env = dict(os.environ, GIT_CEILING_DIRECTORIES=str(work), CMAKE_BUILD_PARALLEL_LEVEL="2")
        source, llama, sentencepiece = [unpack(path, work / str(index)) for index, path in enumerate(archives)]
        patch_source(source, profile, Path(__file__).resolve().parent, env)
        (source / "llama.cpp").rmdir()
        llama.rename(source / "llama.cpp")
        dependency = work / "dependency-build"
        command = [*cmake, "-G", "Unix Makefiles", "-S", str(sentencepiece), "-B", str(dependency),
                   "-DCMAKE_BUILD_TYPE=Release", "-DCMAKE_OSX_ARCHITECTURES=arm64",
                   "-DSPM_BUILD_TEST=OFF", "-DSPM_ENABLE_SHARED=OFF", "-DSPM_ENABLE_TCMALLOC=OFF"]
        subprocess.run(command, env=env, check=True)
        subprocess.run([*cmake, "--build", str(dependency), "--target", "sentencepiece-static", "-j", "2"], env=env, check=True)
        prefix = source / ".deps/sentencepiece"
        (prefix / "lib").mkdir(parents=True)
        (prefix / "include").mkdir()
        shutil.copyfile(dependency / "src/libsentencepiece.a", prefix / "lib/libsentencepiece.a")
        shutil.copyfile(sentencepiece / "src/sentencepiece_processor.h", prefix / "include/sentencepiece_processor.h")
        licenses = prefix / "share/licenses/nemo-speech/third_party/sentencepiece"
        licenses.mkdir(parents=True)
        for name, original in {
            "LICENSE": "LICENSE", "absl-LICENSE": "third_party/absl/LICENSE",
            "darts-clone-LICENSE": "third_party/darts_clone/LICENSE",
            "protobuf-lite-LICENSE": "third_party/protobuf-lite/LICENSE",
        }.items():
            shutil.copyfile(sentencepiece / original, licenses / name)
        build_dir = work / "build"
        install = work / "install"
        subprocess.run([*cmake, "-S", str(source), "-B", str(build_dir), *profile["cmakeFlags"],
                        "-DCMAKE_INSTALL_PREFIX=" + str(install)], env=env, check=True)
        subprocess.run([*cmake, "--build", str(build_dir), "-j", "2"], env=env, check=True)
        subprocess.run([*cmake, "--install", str(build_dir)], env=env, check=True)
        destination.mkdir()
        for path in sorted(install.rglob("*")):
            if not path.is_file():
                continue
            relative = path.relative_to(install).as_posix()
            if package_path("package/" + relative, "package") is None:
                continue
            if not path.resolve().is_relative_to(install.resolve()):
                raise ValueError("External runtime install alias")
            target = destination / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, target)
            target.chmod(0o755 if relative.startswith("bin/") or path.suffix == ".dylib" else 0o644)
        if not (destination / "share/licenses/nemo-speech/LICENSE").is_file():
            raise ValueError("Runtime license is missing")
