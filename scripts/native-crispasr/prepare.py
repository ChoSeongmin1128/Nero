#!/usr/bin/env python3
"""Build-time only: stage the pinned CrispASR CLI and its licenses for macOS arm64; no weights."""
import argparse
import hashlib
import json
import os
import platform
import posixpath
import re
import shlex
import shutil
import subprocess
import tarfile
import tempfile
import urllib.request
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parents[2]
SPEC = json.loads(Path(__file__).with_name('runtime.json').read_text(encoding='utf-8'))
TARGET = 'macos-arm64'
EXECUTABLE = 'bin/crispasr'
LICENSES = {
    'share/licenses/CrispASR/LICENSE': ('source', 'LICENSE'),
    'share/licenses/CrispASR/THIRD_PARTY_NOTICES.txt': ('source', 'THIRD_PARTY_NOTICES.txt'),
    'share/licenses/ggml/LICENSE': ('ggml', 'LICENSE'),
    'share/licenses/c2pa-audio/LICENSE': ('c2paAudio', 'LICENSE'),
}
REQUIRED_CACHE = {
    'CMAKE_BUILD_TYPE': 'Release', 'CMAKE_OSX_ARCHITECTURES': 'arm64',
    'CMAKE_OSX_DEPLOYMENT_TARGET': '13.0', 'BUILD_SHARED_LIBS': 'OFF', 'GGML_NATIVE': 'OFF',
    'GGML_CPU_ARM_ARCH': 'armv8.2-a+dotprod+fp16', 'GGML_METAL': 'ON',
    'GGML_METAL_EMBED_LIBRARY': 'ON', 'GGML_BLAS': 'ON', 'GGML_BLAS_VENDOR': 'Apple',
    'CRISPASR_BUILD_SERVER': 'OFF', 'CRISPASR_C2PA_FETCH': 'OFF', 'CRISPASR_CURL': 'OFF',
    'CRISPASR_FFMPEG': 'OFF', 'CRISPASR_COREML': 'OFF',
}
MACHO = {b'\xcf\xfa\xed\xfe'}


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def allowed(relative):
    return relative == EXECUTABLE or relative in LICENSES


def package_files(root):
    entries = []
    for path in sorted(root.rglob('*')):
        if path.is_symlink() or not (path.is_file() or path.is_dir()):
            raise ValueError('Linked or special CrispASR package entry')
        if path.is_file() and path != root / 'runtime.json':
            relative = path.relative_to(root).as_posix()
            if not allowed(relative):
                raise ValueError('Unexpected CrispASR package file: ' + relative)
            entries.append({'path': relative, 'bytes': path.stat().st_size, 'sha256': sha(path)})
    return entries


def archive(cache, item):
    name = f"{item['repository'].split('/')[-1]}-{item['commit']}.tar.gz"
    path = cache / name
    if path.exists() or path.is_symlink():
        if path.is_symlink() or path.stat().st_size != item['bytes'] or sha(path) != item['sha256']:
            raise ValueError('Existing CrispASR source archive differs from its pin: ' + name)
        return path
    with tempfile.TemporaryDirectory(prefix='crispasr-download-', dir=cache) as folder:
        partial = Path(folder) / name
        request = urllib.request.Request(
            f"https://codeload.github.com/{item['repository']}/tar.gz/{item['commit']}",
            headers={'User-Agent': 'Nero-build'})
        with urllib.request.urlopen(request, timeout=60) as response, partial.open('xb') as output:
            size = 0
            while data := response.read(1024 * 1024):
                size += len(data)
                if size > item['bytes']:
                    raise ValueError('CrispASR source archive exceeds its pinned size')
                output.write(data)
        if partial.stat().st_size != item['bytes'] or sha(partial) != item['sha256']:
            raise ValueError('CrispASR source archive checksum mismatch')
        os.link(partial, path)
    return path


def unpack(path, destination, item):
    destination.mkdir()
    with tarfile.open(path) as bundle:
        if sum(member.size for member in bundle) > 1024 * 1024 * 1024:
            raise ValueError('CrispASR source archive is too large')
        bundle.extractall(destination, filter='data')
    children = list(destination.iterdir())
    expected = f"{item['repository'].split('/')[-1]}-{item['commit']}"
    if len(children) != 1 or children[0].name != expected or children[0].is_symlink() or not children[0].is_dir():
        raise ValueError('CrispASR source archive must contain its commit directory')
    return children[0]


def dependencies(path):
    lines = subprocess.check_output(['otool', '-L', str(path)], text=True).splitlines()[1:]
    return [line.strip().split(' (', 1)[0] for line in lines]


def verify_links(root):
    # Static build: only canonical system libraries and frameworks may be loaded.
    for dependency in dependencies(root / EXECUTABLE):
        if (not dependency.startswith(('/usr/lib/', '/System/Library/'))
                or posixpath.normpath(dependency) != dependency):
            raise ValueError('CrispASR has a non-system runtime dependency: ' + dependency)
    commands = subprocess.check_output(['otool', '-l', str(root / EXECUTABLE)], text=True)
    if 'cmd LC_RPATH' in commands:
        raise ValueError('CrispASR runtime has a loader search path')


def build(cache, stage, cmake):
    if platform.system() != 'Darwin' or platform.machine() != 'arm64':
        raise ValueError('CrispASR Metal source build requires native macOS arm64')
    source_archive = archive(cache, SPEC['source'])
    ggml_archive = archive(cache, SPEC['ggml'])
    c2pa_archive = archive(cache, SPEC['c2paAudio'])
    with tempfile.TemporaryDirectory(prefix='crispasr-build-', dir=cache.parent) as folder:
        folder = Path(folder)
        source = unpack(source_archive, folder / 'source', SPEC['source'])
        trees = {}
        # The CLI compiles only c2pa-audio's SHA-256 header; C2PA signing stays disabled.
        for key, relative in (('ggml', 'ggml'), ('c2paAudio', 'third_party/c2pa-audio')):
            unpacked = unpack(ggml_archive if key == 'ggml' else c2pa_archive, folder / key, SPEC[key])
            target = source / relative
            if target.exists() and any(target.iterdir()):
                raise ValueError('CrispASR source already contains a submodule tree: ' + relative)
            if target.exists():
                target.rmdir()
            os.rename(unpacked, target)
            trees[key] = target
        output = folder / 'build'
        env = {key: value for key, value in os.environ.items() if not key.startswith(('GGML_', 'CRISPASR_'))}
        env['CMAKE_BUILD_PARALLEL_LEVEL'] = '2'
        subprocess.run([*cmake, '-S', str(source), '-B', str(output), *SPEC['targets'][TARGET]['cmakeFlags']],
                       env=env, check=True)
        actual = {}
        for line in (output / 'CMakeCache.txt').read_text().splitlines():
            if '=' in line and ':' in line.split('=', 1)[0] and not line.startswith(('#', '//')):
                key, value = line.split('=', 1)
                actual[key.split(':', 1)[0]] = value
        for name, expected in REQUIRED_CACHE.items():
            if actual.get(name) != expected:
                raise ValueError(f'CrispASR source build option differs: {name}={actual.get(name)}')
        subprocess.run([*cmake, '--build', str(output), '--target', 'crispasr-cli', '-j', '2'],
                       env=env, check=True)
        stage.mkdir(parents=True)
        (stage / 'bin').mkdir()
        built = output / 'bin/crispasr'
        if built.is_symlink() or not built.is_file():
            raise ValueError('CrispASR build did not produce its CLI')
        shutil.copyfile(built, stage / EXECUTABLE)
        (stage / EXECUTABLE).chmod(0o755)
        roots = {'source': source, **trees}
        for relative, (origin, name) in LICENSES.items():
            original = roots[origin] / name
            if original.is_symlink() or not original.is_file() or not 0 < original.stat().st_size <= 1024 * 1024:
                raise ValueError('Missing pinned CrispASR license: ' + relative)
            destination = stage / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(original, destination)
            destination.chmod(0o644)
        return {'cmakeCacheSha256': sha(output / 'CMakeCache.txt')}


def package_receipt(root):
    root = Path(root)
    path = root / 'runtime.json'
    if root.is_symlink() or not root.is_dir() or path.is_symlink() or not path.is_file():
        raise ValueError('Missing CrispASR runtime receipt')
    receipt = json.loads(path.read_text(encoding='utf-8'))
    if (receipt.get('runtime') != SPEC or receipt.get('target') != TARGET
            or receipt.get('sourceArchiveHashesVerified') is not True
            or re.fullmatch(r'[0-9a-f]{64}', receipt.get('build', {}).get('cmakeCacheSha256', '')) is None):
        raise ValueError('CrispASR runtime differs from its pinned source and target')
    entries = receipt.get('files')
    names = [item.get('path') for item in entries] if isinstance(entries, list) else []
    if sorted(names) != sorted([EXECUTABLE, *LICENSES]) or any(
            type(item.get('bytes')) is not int
            or re.fullmatch(r'[0-9a-f]{64}', item.get('sha256', '')) is None for item in entries):
        raise ValueError('Invalid CrispASR runtime file receipt')
    return receipt


def verify_staged(root, execute_runtime=False):
    root = Path(root)
    receipt = package_receipt(root)
    if package_files(root) != receipt['files']:
        raise ValueError('CrispASR runtime files differ from their receipt')
    verify_links(root)
    if execute_runtime:
        result = subprocess.run([str(root / EXECUTABLE), '--help'], capture_output=True, timeout=30,
                                env={'GGML_METAL_PIPELINE_CACHE_DISABLE': '1', 'PATH': '/usr/bin:/bin'})
        text = result.stdout + result.stderr
        if b'qwen3' not in text or b'-am' not in text:
            raise ValueError('CrispASR CLI lacks the pinned Qwen3 aligner interface')
    return receipt


def signed_transition(root, before):
    root = Path(root)
    if package_receipt(root) != before:
        raise ValueError('CrispASR runtime receipt changed during signing')
    after = package_files(root)
    if [item['path'] for item in after] != [item['path'] for item in before['files']]:
        raise ValueError('CrispASR file inventory changed during signing')
    for old, new in zip(before['files'], after):
        with (root / new['path']).open('rb') as stream:
            binary = stream.read(4) in MACHO
        if old != new and not (binary and new['path'] == EXECUTABLE):
            raise ValueError('CrispASR resource changed during signing')
    verify_links(root)
    return {'beforeSigning': before, 'signedFiles': after}


def verify_signed(root, receipt):
    root = Path(root)
    if (not isinstance(receipt, dict) or package_receipt(root) != receipt.get('beforeSigning')
            or package_files(root) != receipt.get('signedFiles')):
        raise ValueError('Signed CrispASR package differs from its preserved receipt')
    verify_links(root)


def prepare(cache, output, cmake):
    if output.exists() or output.is_symlink():
        raise ValueError('Destination exists; preserve it and use a new destination')
    cache.mkdir(parents=True, exist_ok=True)
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='crispasr-stage-', dir=output.parent) as folder:
        stage = Path(folder) / 'crispasr'
        evidence = build(cache, stage, cmake)
        receipt = {'runtime': SPEC, 'target': TARGET, 'sourceArchiveHashesVerified': True,
                   'fileHashScope': 'before Developer ID code signing', 'build': evidence,
                   'files': package_files(stage)}
        (stage / 'runtime.json').write_text(json.dumps(receipt, indent=2) + '\n', encoding='utf-8')
        verify_staged(stage, execute_runtime=True)
        os.rename(stage, output)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--target', choices=[TARGET])
    parser.add_argument('--cache', type=Path, default=ROOT / '.work/crispasr-runtime-downloads')
    parser.add_argument('--output', type=Path, default=ROOT / 'apps/desktop/.generated/native/crispasr')
    parser.add_argument('--cmake', default='cmake')
    parser.add_argument('--verify-staged', type=Path)
    parser.add_argument('--optional', action='store_true')
    parser.add_argument('--execute-runtime', action='store_true')
    args = parser.parse_args()
    if args.verify_staged is not None:
        if args.optional and not args.verify_staged.exists() and not args.verify_staged.is_symlink():
            print('CrispASR runtime absent: Qwen recognition unavailable.')
        else:
            verify_staged(args.verify_staged, args.execute_runtime)
            print('Verified native CrispASR runtime.')
    elif args.target is None:
        parser.error('--target is required to prepare a runtime')
    else:
        prepare(args.cache, args.output, shlex.split(args.cmake))
