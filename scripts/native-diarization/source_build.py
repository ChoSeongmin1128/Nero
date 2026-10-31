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
        subprocess.run(["git", "-c", "core.autocrlf=false", "apply", *option, str(path)], cwd=source, env=env, check=True)
    for item in patch["files"]:
        if digest(source / item["path"]) != item["resultSha256"]:
            raise ValueError("Runtime source differs after patch: " + item["path"])


def macos_settings(profile):
    required = {"CMAKE_OSX_ARCHITECTURES:STRING", "CMAKE_OSX_DEPLOYMENT_TARGET:STRING",
                "GGML_NATIVE:BOOL", "GGML_CPU_ARM_ARCH:STRING"}
    settings = {}
    for flag in profile.get("cmakeFlags", []):
        if not flag.startswith("-D") or "=" not in flag:
            continue
        key, value = flag[2:].split("=", 1)
        if key not in required:
            continue
        if key in settings or not value:
            raise ValueError("Duplicate or empty macOS source build option")
        settings[key] = value
    if settings.keys() != required or settings["GGML_NATIVE:BOOL"] != "OFF":
        raise ValueError("macOS runtime requires explicit portable CPU and deployment settings")
    return settings


def verify_cmake_settings(build, expected):
    actual = dict(line.split("=", 1) for line in (build / "CMakeCache.txt").read_text().splitlines()
                  if "=" in line and not line.startswith(("#", "//")))
    for key, value in expected.items():
        if actual.get(key) != value:
            raise ValueError("Native speech build option differs: " + key)


def build(profile, cache, destination, cmake, package_path):
    if profile.get("sourceBackends") == ["cpu"]:
        return build_windows_cpu(profile, cache, destination, cmake, package_path)
    if platform.system() != "Darwin" or platform.machine() != "arm64":
        raise ValueError("The patched source runtime is qualified only for native macOS arm64")
    settings = macos_settings(profile)
    dependency_settings = {key: settings[key] for key in
                           ("CMAKE_OSX_ARCHITECTURES:STRING", "CMAKE_OSX_DEPLOYMENT_TARGET:STRING")}
    cache.mkdir(parents=True, exist_ok=True)
    archives = [archive(item, cache) for item in profile["sources"]]
    with tempfile.TemporaryDirectory(prefix="speech-source-", dir=cache.parent) as directory:
        work = Path(directory).resolve()
        env = dict(os.environ, GIT_CEILING_DIRECTORIES=str(work), CMAKE_BUILD_PARALLEL_LEVEL="2",
                   GIT_CONFIG_COUNT="1", GIT_CONFIG_KEY_0="core.autocrlf", GIT_CONFIG_VALUE_0="false")
        source, llama, sentencepiece = [unpack(path, work / str(index)) for index, path in enumerate(archives)]
        patch_source(source, profile, Path(__file__).resolve().parent, env)
        (source / "llama.cpp").rmdir()
        llama.rename(source / "llama.cpp")
        dependency = work / "dependency-build"
        command = [*cmake, "-G", "Unix Makefiles", "-S", str(sentencepiece), "-B", str(dependency),
                   "-DCMAKE_BUILD_TYPE=Release", *["-D" + key + "=" + value for key, value in dependency_settings.items()],
                   "-DSPM_BUILD_TEST=OFF", "-DSPM_ENABLE_SHARED=OFF", "-DSPM_ENABLE_TCMALLOC=OFF"]
        subprocess.run(command, env=env, check=True)
        verify_cmake_settings(dependency, dependency_settings)
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
        verify_cmake_settings(build_dir, settings)
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


def build_windows_cpu(profile, cache, destination, cmake, package_path):
    """Pinned CPU mmap build; the separate Vulkan backend retains its official archive."""
    if platform.system() != "Windows" or platform.machine().lower() not in ("amd64", "x86_64"):
        raise ValueError("The CPU source runtime requires native Windows x64")
    cache.mkdir(parents=True, exist_ok=True)
    archives = [archive(item, cache) for item in profile["sources"]]
    with tempfile.TemporaryDirectory(prefix="speech-source-", dir=cache.parent) as directory:
        work = Path(directory).resolve()
        env = dict(os.environ, GIT_CEILING_DIRECTORIES=str(work), CMAKE_BUILD_PARALLEL_LEVEL="2",
                   GIT_CONFIG_COUNT="1", GIT_CONFIG_KEY_0="core.autocrlf", GIT_CONFIG_VALUE_0="false")
        source, llama, sentencepiece = [unpack(path, work / str(index)) for index, path in enumerate(archives)]
        patch_source(source, profile, Path(__file__).resolve().parent, env)
        patch_source(source, {"patch": profile["windowsPatch"]}, Path(__file__).resolve().parent, env)
        (source / "llama.cpp").rmdir()
        llama.rename(source / "llama.cpp")
        # Apply the base archive's own ggml series with explicit LF handling.
        # CMake's Windows staging copy otherwise depends on Git checkout settings.
        for line in (source / "patches/series").read_text().splitlines():
            name = line.split("#", 1)[0].strip()
            if not name:
                continue
            if Path(name).name != name or not name.endswith(".patch"):
                raise ValueError("Unsafe upstream dependency patch")
            subprocess.run(["git", "-c", "core.autocrlf=false", "apply", "--whitespace=nowarn",
                            str(source / "patches" / name)], cwd=source / "llama.cpp", env=env, check=True)
        dependency = work / "dependency-build"
        subprocess.run([*cmake, "-G", "Ninja", "-S", str(sentencepiece), "-B", str(dependency),
                        "-DCMAKE_BUILD_TYPE=Release", "-DSPM_BUILD_TEST=OFF", "-DSPM_ENABLE_SHARED=OFF",
                        "-DSPM_ENABLE_TCMALLOC=OFF", "-DSPM_USE_BUILTIN_PROTOBUF=ON"], env=env, check=True)
        subprocess.run([*cmake, "--build", str(dependency), "--target", "sentencepiece-static", "-j", "2"], env=env, check=True)
        prefix = source / ".deps/sentencepiece"
        (prefix / "lib").mkdir(parents=True)
        (prefix / "include").mkdir()
        shutil.copyfile(dependency / "src/sentencepiece.lib", prefix / "lib/sentencepiece.lib")
        shutil.copyfile(sentencepiece / "src/sentencepiece_processor.h", prefix / "include/sentencepiece_processor.h")
        licenses = prefix / "share/licenses/nemo-speech/third_party/sentencepiece"
        licenses.mkdir(parents=True)
        for name, original in {
            "LICENSE": "LICENSE", "absl-LICENSE": "third_party/absl/LICENSE",
            "darts-clone-LICENSE": "third_party/darts_clone/LICENSE",
            "protobuf-lite-LICENSE": "third_party/protobuf-lite/LICENSE",
        }.items():
            shutil.copyfile(sentencepiece / original, licenses / name)
        build_dir, install = work / "build", work / "install"
        subprocess.run([*cmake, "-S", str(source), "-B", str(build_dir), *profile["cmakeFlags"],
                        "-DNEMO_SPEECH_LLAMA_CPP_SOURCE_DIR=" + str(source / "llama.cpp"),
                        "-DCMAKE_PREFIX_PATH=" + str(prefix), "-DCMAKE_INSTALL_PREFIX=" + str(install)], env=env, check=True)
        subprocess.run([*cmake, "--build", str(build_dir), "-j", "2"], env=env, check=True)
        # The base archive's full test suite references a missing source file.
        # Compile the pinned independent scheduler-capacity check instead.
        unit = work / "scheduler-capacity-test.exe"
        subprocess.run(["cl", "/nologo", "/EHsc", "/std:c++17", "/utf-8",
                        "/I" + str(source / "src/runtime/ggml"),
                        str(source / "tests/cpp/asr/test_windows_scheduler_capacity.cpp"),
                        "/Fe:" + str(unit), "/Fo:" + str(work / "scheduler-capacity-test.obj")],
                       cwd=work, env=env, check=True)
        subprocess.run([str(unit)], env=env, check=True)
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
        if not (destination / "share/licenses/nemo-speech/LICENSE").is_file():
            raise ValueError("Runtime license is missing")
