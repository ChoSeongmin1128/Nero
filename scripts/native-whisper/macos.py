import hashlib
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tarfile
import tempfile
import urllib.request


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def source_archive(item, cache):
    path = cache / f"whisper.cpp-{item['commit']}.tar.gz"
    if path.exists():
        if path.is_symlink() or path.stat().st_size != item['bytes'] or digest(path) != item['sha256']:
            raise ValueError('Existing Whisper source archive differs from its pin')
        return path
    with tempfile.TemporaryDirectory(prefix='whisper-download-', dir=cache) as folder:
        partial = Path(folder) / path.name
        request = urllib.request.Request(
            f"https://codeload.github.com/{item['repository']}/tar.gz/{item['commit']}",
            headers={'User-Agent': 'Nero-build'})
        with urllib.request.urlopen(request, timeout=60) as response, partial.open('xb') as output:
            size = 0
            while data := response.read(1024 * 1024):
                size += len(data)
                if size > item['bytes']:
                    raise ValueError('Whisper source archive exceeds its pinned size')
                output.write(data)
        if partial.stat().st_size != item['bytes'] or digest(partial) != item['sha256']:
            raise ValueError('Whisper source archive checksum mismatch')
        os.link(partial, path)
    return path


def unpack(archive, destination):
    destination.mkdir()
    with tarfile.open(archive) as bundle:
        if sum(member.size for member in bundle) > 512 * 1024 * 1024:
            raise ValueError('Whisper source archive is too large')
        bundle.extractall(destination, filter='data')
    children = list(destination.iterdir())
    if len(children) != 1 or children[0].is_symlink() or not children[0].is_dir():
        raise ValueError('Whisper source must contain one directory')
    return children[0]


def copy_runtime(build, source, destination, package_path):
    files = {p.name: p for p in sorted(build.rglob('*.dylib'))}
    pending = [(build / 'bin/whisper-cli', 'bin/whisper-cli')]
    copied = set()
    while pending:
        original, relative = pending.pop()
        if relative in copied:
            continue
        if not original.resolve().is_relative_to(build.resolve()):
            raise ValueError('Whisper library alias escapes its build')
        if package_path('package/' + relative, 'package') is None:
            raise ValueError('Whisper runtime path is outside the package contract')
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(original, target)
        target.chmod(0o755)
        copied.add(relative)
        linked = subprocess.check_output(['otool', '-L', str(original)], text=True).splitlines()[1:]
        for line in linked:
            dependency = line.strip().split(' (', 1)[0]
            if dependency.startswith(('/usr/lib/', '/System/Library/')):
                continue
            if not dependency.startswith(('@rpath/', '@loader_path/', '@executable_path/')):
                raise ValueError('Whisper has a non-system absolute runtime dependency')
            name = Path(dependency).name
            if name not in files:
                raise ValueError('Whisper runtime dependency is missing: ' + name)
            pending.append((files[name], 'lib/' + name))
    license_path = 'share/licenses/whisper.cpp/LICENSE'
    if package_path('package/' + license_path, 'package') is None:
        raise ValueError('Whisper license path is outside the package contract')
    target = destination / license_path
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source / 'LICENSE', target)
    target.chmod(0o644)


def build(profile, cache, destination, cmake, package_path):
    if platform.system() != 'Darwin' or platform.machine() != 'arm64':
        raise ValueError('Whisper Metal source build requires native macOS arm64')
    cache, destination = Path(cache), Path(destination)
    if destination.exists() or destination.is_symlink():
        raise ValueError('Whisper package destination already exists')
    cache.mkdir(parents=True, exist_ok=True)
    archive = source_archive(profile['source'], cache)
    with tempfile.TemporaryDirectory(prefix='whisper-build-', dir=cache.parent) as folder:
        folder = Path(folder)
        source = unpack(archive, folder / 'source')
        output = folder / 'build'
        env = dict(os.environ, CMAKE_BUILD_PARALLEL_LEVEL='2')
        subprocess.run([*cmake, '-S', str(source), '-B', str(output), *profile['cmakeFlags']],
                       env=env, check=True)
        actual = dict(line.split('=', 1) for line in (output / 'CMakeCache.txt').read_text().splitlines()
                      if '=' in line and not line.startswith(('#', '//')))
        for name, expected in {'CMAKE_BUILD_TYPE:STRING': 'Release', 'CMAKE_OSX_ARCHITECTURES:STRING': 'arm64',
                               'CMAKE_OSX_DEPLOYMENT_TARGET:STRING': '13.0', 'BUILD_SHARED_LIBS:BOOL': 'ON',
                               'GGML_NATIVE:BOOL': 'OFF', 'GGML_CPU_ARM_ARCH:STRING': 'armv8.2-a+dotprod+fp16',
                               'GGML_METAL:BOOL': 'ON', 'GGML_METAL_EMBED_LIBRARY:BOOL': 'ON',
                               'WHISPER_COMMON_FFMPEG:BOOL': 'OFF', 'WHISPER_BUILD_SERVER:BOOL': 'OFF'}.items():
            if actual.get(name) != expected:
                raise ValueError('Whisper source build option differs: ' + name)
        subprocess.run([*cmake, '--build', str(output), '--target', 'whisper-cli', '-j', '2'],
                       env=env, check=True)
        destination.mkdir(parents=True)
        copy_runtime(output, source, destination, package_path)
