#!/usr/bin/env python3
"""Build-time only: stage pinned whisper.cpp CLI, dependencies and license; no weights."""
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
SPEC = json.loads(Path(__file__).with_name('runtime.json').read_text(encoding='utf-8'))
CRT = {'concrt140.dll', 'msvcp140.dll', 'msvcp140_1.dll', 'msvcp140_2.dll',
       'msvcp140_atomic_wait.dll', 'msvcp140_codecvt_ids.dll', 'vcomp140.dll',
       'vcruntime140.dll', 'vcruntime140_1.dll'}
MACHO = {b'\xcf\xfa\xed\xfe', b'\xce\xfa\xed\xfe', b'\xfe\xed\xfa\xcf', b'\xfe\xed\xfa\xce'}


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def package_path(name, prefix):
    path = PurePosixPath(name)
    if (path.is_absolute() or '\\' in name or ':' in name or '..' in path.parts
            or path.as_posix() != name or not path.parts or path.parts[0] != prefix):
        raise ValueError('Unsafe Whisper runtime path')
    relative = PurePosixPath(*path.parts[1:])
    if relative.as_posix() == 'share/licenses/whisper.cpp/LICENSE':
        return relative
    if relative.parts[:1] == ('bin',) and len(relative.parts) == 2:
        if relative.name in {'whisper-cli', 'whisper-cli.exe', 'whisper.dll',
                              'ggml.dll', 'ggml-base.dll'} | CRT:
            return relative
        if re.fullmatch(r'ggml-cpu-[a-z0-9]+\.dll', relative.name):
            return relative
    if relative.parts[:1] == ('lib',) and len(relative.parts) == 2:
        if re.fullmatch(r'lib[a-zA-Z0-9_.-]+\.dylib', relative.name):
            return relative
    return None


def package_files(root):
    entries = []
    for path in sorted(root.rglob('*')):
        if path.is_symlink() or not (path.is_file() or path.is_dir()):
            raise ValueError('Linked or special Whisper package entry')
        if path.is_file() and path != root / 'runtime.json':
            relative = path.relative_to(root).as_posix()
            if package_path('package/' + relative, 'package') is None:
                raise ValueError('Unexpected Whisper package file')
            entries.append({'path': relative, 'bytes': path.stat().st_size, 'sha256': sha(path)})
    return entries


def package_receipt(root, target=None):
    root = Path(root)
    if root.is_symlink() or not root.is_dir():
        raise ValueError('Missing Whisper runtime directory')
    path = root / 'runtime.json'
    if path.is_symlink() or not path.is_file() or path.stat().st_size > 1024 * 1024:
        raise ValueError('Missing Whisper runtime receipt')
    receipt = json.loads(path.read_text(encoding='utf-8'))
    if (receipt.get('runtime') != SPEC or receipt.get('sourceArchiveHashesVerified') is not True
            or receipt.get('target') not in SPEC['targets']
            or target is not None and receipt['target'] != target):
        raise ValueError('Whisper runtime differs from its pinned source and target')
    entries = receipt.get('files')
    if not isinstance(entries, list) or not entries:
        raise ValueError('Empty Whisper runtime receipt')
    names = set()
    for item in entries:
        if not isinstance(item, dict):
            raise ValueError('Invalid Whisper runtime file entry')
        name = item.get('path', '')
        if (not isinstance(name, str) or package_path('package/' + name, 'package') is None
                or name in names or type(item.get('bytes')) is not int or item['bytes'] < 0
                or re.fullmatch(r'[0-9a-f]{64}', item.get('sha256', '')) is None):
            raise ValueError('Invalid Whisper runtime file receipt')
        names.add(name)
    required = {'share/licenses/whisper.cpp/LICENSE'}
    if receipt['target'] == 'windows-x64':
        required |= {'bin/' + name for name in ('whisper-cli.exe', 'whisper.dll', 'ggml.dll',
                    'ggml-base.dll', 'ggml-cpu-x64.dll', 'msvcp140.dll',
                    'vcruntime140.dll', 'vcruntime140_1.dll')}
        if any(name.startswith('lib/') or name == 'bin/whisper-cli' for name in names):
            raise ValueError('Unexpected macOS file in Windows runtime')
    else:
        required |= {'bin/whisper-cli', 'lib/libwhisper.1.dylib', 'lib/libggml-metal.0.dylib'}
        if any(name.endswith(('.exe', '.dll')) for name in names):
            raise ValueError('Unexpected Windows file in macOS runtime')
    if not required <= names:
        raise ValueError('Whisper executable, dependency or license is missing')
    return receipt


def verify_staged(root, target=None, execute_runtime=False):
    root = Path(root)
    receipt = package_receipt(root, target)
    if package_files(root) != receipt['files']:
        raise ValueError('Whisper runtime files differ from their receipt')
    if receipt['target'] == 'macos-arm64':
        verify_macos_links(root)
    if execute_runtime:
        binary = root / 'bin' / ('whisper-cli.exe' if receipt['target'] == 'windows-x64' else 'whisper-cli')
        result = subprocess.run([str(binary), '--help'], capture_output=True, timeout=15, check=True)
        if b'--output-json-full' not in result.stdout + result.stderr:
            raise ValueError('Whisper CLI lacks the pinned native token-output interface')
    return receipt


def signed_transition(root, before):
    root = Path(root)
    if package_receipt(root) != before or before['target'] != 'macos-arm64':
        raise ValueError('Whisper runtime receipt changed during signing')
    after = package_files(root)
    if [item['path'] for item in after] != [item['path'] for item in before['files']]:
        raise ValueError('Whisper file inventory changed during signing')
    for old, new in zip(before['files'], after):
        with (root / new['path']).open('rb') as stream:
            binary = stream.read(4) in MACHO
        if old != new and not binary:
            raise ValueError('Whisper resource changed during signing')
    verify_macos_links(root)
    return {'beforeSigning': before, 'signedFiles': after}


def verify_signed(root, receipt):
    root = Path(root)
    if (not isinstance(receipt, dict) or package_receipt(root) != receipt.get('beforeSigning')
            or package_files(root) != receipt.get('signedFiles')):
        raise ValueError('Signed Whisper package differs from its preserved receipt')
    verify_macos_links(root)


def download(cache, name, item, url):
    path = cache / name
    if path.exists() or path.is_symlink():
        if path.is_symlink() or not path.is_file() or path.stat().st_size != item['bytes'] or sha(path) != item['sha256']:
            raise ValueError('Existing Whisper archive differs from its pin')
        return path
    with tempfile.TemporaryDirectory(prefix='whisper-download-', dir=cache) as folder:
        partial = Path(folder) / name
        request = urllib.request.Request(url, headers={'User-Agent': 'Nero-build'})
        with urllib.request.urlopen(request, timeout=60) as response, partial.open('xb') as output:
            size = 0
            while data := response.read(1024 * 1024):
                size += len(data)
                if size > item['bytes']:
                    raise ValueError('Whisper archive exceeds its pinned size')
                output.write(data)
        if partial.stat().st_size != item['bytes'] or sha(partial) != item['sha256']:
            raise ValueError('Whisper archive checksum mismatch')
        os.link(partial, path)
    return path


def macos_files(root):
    return [root / 'bin/whisper-cli', *sorted((root / 'lib').glob('*.dylib'))]


def macos_dependencies(path):
    lines = subprocess.check_output(['otool', '-L', str(path)], text=True).splitlines()[1:]
    return [line.strip().split(' (', 1)[0] for line in lines]


def macos_rpaths(path):
    lines = subprocess.check_output(['otool', '-l', str(path)], text=True).splitlines()
    return [lines[i + 2].strip().split('path ', 1)[1].split(' (offset', 1)[0]
            for i, line in enumerate(lines) if line.strip() == 'cmd LC_RPATH']


def relocate_macos(root):
    for path in macos_files(root):
        dependencies = macos_dependencies(path)
        args = ['install_name_tool']
        if path.suffix == '.dylib':
            args += ['-id', '@rpath/' + path.name]
            dependencies = dependencies[1:]  # otool's first dylib entry is its install ID.
        for dependency in dependencies:
            if dependency.startswith(('/usr/lib/', '/System/Library/')):
                continue
            if not (root / 'lib' / Path(dependency).name).is_file():
                raise ValueError('Unpackaged Whisper dynamic dependency')
            args += ['-change', dependency, '@rpath/' + Path(dependency).name]
        for old in dict.fromkeys(macos_rpaths(path)):
            args += ['-delete_rpath', old]
        args += ['-add_rpath', '@loader_path' if path.suffix == '.dylib' else '@executable_path/../lib', str(path)]
        subprocess.run(args, check=True)
    verify_macos_links(root)
    # install_name_tool invalidates the compiler's ad-hoc signature on arm64.
    # Developer ID distribution signing happens later and has its own receipt.
    for path in [*sorted((root / 'lib').glob('*.dylib')), root / 'bin/whisper-cli']:
        subprocess.run(['codesign', '--force', '--sign', '-', '--timestamp=none', str(path)], check=True)


def verify_macos_links(root):
    for path in macos_files(root):
        expected = '@loader_path' if path.suffix == '.dylib' else '@executable_path/../lib'
        if macos_rpaths(path) != [expected]:
            raise ValueError('Whisper runtime has an unexpected loader search path')
        dependencies = macos_dependencies(path)
        if path.suffix == '.dylib':
            if not dependencies or dependencies[0] != '@rpath/' + path.name:
                raise ValueError('Whisper dylib install ID differs from its package name')
            dependencies = dependencies[1:]
        for dependency in dependencies:
            if dependency.startswith(('/usr/lib/', '/System/Library/')):
                continue
            if (not dependency.startswith('@rpath/') or dependency != '@rpath/' + Path(dependency).name
                    or not (root / 'lib' / Path(dependency).name).is_file()):
                raise ValueError('Whisper dependency escapes its package')


def prepare(target, cache, output, redist=None, cmake=None):
    if output.exists() or output.is_symlink():
        raise ValueError('Destination exists; preserve it and use a new destination')
    cache.mkdir(parents=True, exist_ok=True)
    output.parent.mkdir(parents=True, exist_ok=True)
    profile = SPEC['targets'][target]
    with tempfile.TemporaryDirectory(prefix='whisper-stage-', dir=output.parent) as folder:
        stage = Path(folder) / 'whisper'
        if target == 'macos-arm64':
            module = load_module('nero_whisper_macos', Path(__file__).with_name('macos.py'))
            module.build(dict(profile, source=SPEC['source']), cache, stage, cmake or ['cmake'], package_path)
            relocate_macos(stage)
        else:
            item = profile['archive']
            archive = download(cache, item['name'], item,
                f"https://github.com/{SPEC['repository']}/releases/download/{SPEC['version']}/{item['name']}")
            stage.mkdir()
            with zipfile.ZipFile(archive) as bundle:
                for entry in bundle.infolist():
                    if entry.is_dir():
                        continue
                    original = PurePosixPath(entry.filename)
                    if (original.is_absolute() or '..' in original.parts or '\\' in entry.filename
                            or original.as_posix() != entry.filename
                            or (entry.external_attr >> 16) & 0o170000 == 0o120000):
                        raise ValueError('Unsafe Whisper archive entry')
                    if len(original.parts) != 2 or original.parts[0] != 'Release':
                        raise ValueError('Unexpected Whisper archive layout')
                    relative = package_path('package/bin/' + original.name, 'package')
                    if relative is None:
                        continue
                    if entry.file_size > 64 * 1024 * 1024:
                        raise ValueError('Whisper runtime archive entry is too large')
                    destination = stage / relative
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    with bundle.open(entry) as source, destination.open('xb') as out:
                        shutil.copyfileobj(source, out)
            summary = load_module('nero_whisper_crt', ROOT / 'scripts/native-summary/prepare.py')
            if tuple(profile['minimumMsvcCrt']) != tuple(summary.MIN_WINDOWS_CRT):
                raise ValueError('Whisper and shared C++ runtime minimums differ')
            for name, path in summary.windows_crt(redist).items():
                shutil.copyfile(path, stage / 'bin' / name)
            source_pin = SPEC['source']
            archive = download(cache, f"whisper.cpp-{source_pin['commit']}.tar.gz", source_pin,
                f"https://codeload.github.com/{source_pin['repository']}/tar.gz/{source_pin['commit']}")
            with tarfile.open(archive) as bundle:
                license = bundle.getmember(f"whisper.cpp-{source_pin['commit']}/LICENSE")
                if not license.isfile() or not 0 < license.size <= 65536:
                    raise ValueError('Missing pinned Whisper license')
                destination = stage / 'share/licenses/whisper.cpp/LICENSE'
                destination.parent.mkdir(parents=True)
                with bundle.extractfile(license) as source, destination.open('xb') as out:
                    shutil.copyfileobj(source, out)
        receipt = {'runtime': SPEC, 'target': target, 'sourceArchiveHashesVerified': True,
                   'fileHashScope': 'before Developer ID code signing', 'files': package_files(stage)}
        (stage / 'runtime.json').write_text(json.dumps(receipt, indent=2) + '\n', encoding='utf-8')
        verify_staged(stage, target, execute_runtime=True)
        os.rename(stage, output)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--target', choices=SPEC['targets'])
    parser.add_argument('--cache', type=Path, default=ROOT / '.work/whisper-runtime-downloads')
    parser.add_argument('--output', type=Path, default=ROOT / 'apps/desktop/.generated/native/whisper')
    parser.add_argument('--msvc-redist', type=Path)
    parser.add_argument('--cmake', default='cmake')
    parser.add_argument('--verify-staged', type=Path)
    parser.add_argument('--optional', action='store_true')
    parser.add_argument('--execute-runtime', action='store_true')
    args = parser.parse_args()
    if args.verify_staged is not None:
        if args.optional and not args.verify_staged.exists() and not args.verify_staged.is_symlink():
            print('Whisper runtime absent: quality recognition unavailable.')
        else:
            verify_staged(args.verify_staged, args.target, args.execute_runtime)
            print('Verified native Whisper runtime.')
    elif args.target is None:
        parser.error('--target is required to prepare a runtime')
    else:
        prepare(args.target, args.cache, args.output, args.msvc_redist, [args.cmake])
