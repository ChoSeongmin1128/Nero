"""Pinned build-only tools. No system SDK, registry, PATH or driver installation."""
import json
import os
from pathlib import Path, PurePosixPath
import subprocess
import zipfile

PIN = json.loads(Path(__file__).with_name('toolchain.json').read_text(encoding='utf-8'))


def inventory(root, sha):
    result = []
    for path in sorted(root.rglob('*')):
        if path.is_symlink() or (hasattr(path, 'is_junction') and path.is_junction()):
            raise ValueError('Linked build toolchain entry')
        if path.is_file() and path != root / 'nero-toolchain.json':
            result.append({'path': path.relative_to(root).as_posix(),
                           'bytes': path.stat().st_size, 'sha256': sha(path)})
    return result


def verify(root, sha):
    receipt = json.loads((root / 'nero-toolchain.json').read_text(encoding='utf-8'))
    if receipt['pin'] != PIN or receipt['files'] != inventory(root, sha):
        raise ValueError('Build toolchain differs from its pinned receipt')
    for name in ('sdk/Bin/glslc.exe', 'sdk/Include/vulkan/vulkan.hpp',
                 'sdk/Lib/vulkan-1.lib', 'sdk/Lib/cmake/SPIRV-Headers/SPIRV-HeadersConfig.cmake',
                 'cmake/cmake-4.1.2-windows-x86_64/bin/cmake.exe', 'ninja/ninja.exe'):
        if not (root / name).is_file():
            raise ValueError('Missing pinned build toolchain component: ' + name)
    return receipt


def prepare(root, cache, download, sha, download_only=False):
    root, cache = Path(root).absolute(), Path(cache).absolute()
    # Tool provisioning is intentionally restricted to an explicit ignored work tree.
    if '.work' not in root.parts or '.work' not in cache.parts:
        raise ValueError('Build toolchain and downloads must remain under .work')
    cache.mkdir(parents=True, exist_ok=True)
    archives = {key: download(cache, item['name'], item, item['url']) for key, item in PIN.items()
                if isinstance(item, dict)}
    if download_only:
        return
    if root.exists():
        return verify(root, sha)
    root.mkdir(parents=True)
    # Official LunarG copy_only contract: unpack files only, no registry/layers,
    # shortcuts, system runtime installer or environment modifications.
    env = dict(os.environ, QT_QPA_PLATFORM='offscreen')
    subprocess.run([str(archives['sdk']), '--root', str(root / 'sdk'),
                    *PIN['sdk']['arguments']], env=env, check=True, timeout=600)
    for key in ('cmake', 'ninja'):
        with zipfile.ZipFile(archives[key]) as bundle:
            for entry in bundle.infolist():
                path = PurePosixPath(entry.filename)
                if path.is_absolute() or '..' in path.parts or ':' in entry.filename or '\\' in entry.filename:
                    raise ValueError('Unsafe build tool archive path')
            bundle.extractall(root / key)
    receipt = {'pin': PIN, 'files': inventory(root, sha)}
    (root / 'nero-toolchain.json').write_text(json.dumps(receipt, indent=2) + '\n', encoding='utf-8')
    return verify(root, sha)
