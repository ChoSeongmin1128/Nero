#!/usr/bin/env python3
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[3]
APP = ROOT / 'apps/desktop'
TARGETS = ('aarch64-apple-darwin', 'x86_64-pc-windows-msvc')
ROOT_PACKAGES = ('nero-desktop', 'nero-cli')
NOTICE_NAME = re.compile(r'^(?:licen[cs]e|copying|notice|copyright|unlicense)(?:[._-].*)?$', re.I)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def load(path):
    return json.loads(Path(path).read_text(encoding='utf-8'))


def safe_file(root, relative):
    path = root / relative
    if Path(relative).is_absolute() or '..' in Path(relative).parts or not Path(relative).parts:
        raise ValueError('Invalid notice path')
    if not path.resolve().is_relative_to(root.resolve()) or path.is_symlink() or not path.is_file():
        raise ValueError('Missing or linked notice: ' + str(relative))
    return path


def metadata(target, root=ROOT):
    if target not in TARGETS:
        raise ValueError('Unsupported distribution target')
    result = subprocess.run(['cargo', 'metadata', '--locked', '--format-version', '1',
                             '--filter-platform', target], cwd=root, check=True,
                            stdout=subprocess.PIPE)
    return json.loads(result.stdout)


def runtime_packages(document):
    packages = {p['id']: p for p in document['packages']}
    nodes = {n['id']: n for n in document['resolve']['nodes']}
    pending = [p['id'] for p in document['packages']
               if p['id'] in document['workspace_members'] and p['name'] in ROOT_PACKAGES]
    if len(pending) != len(ROOT_PACKAGES):
        raise ValueError('Desktop and agent runtime roots are required')
    seen = set()
    while pending:
        identity = pending.pop()
        package = packages[identity]
        if identity in seen or any('proc-macro' in t['kind'] for t in package['targets']):
            continue
        seen.add(identity)
        pending.extend(dep['pkg'] for dep in nodes[identity]['deps']
                       if any(kind['kind'] is None for kind in dep['dep_kinds']))
    return sorted((packages[p] for p in seen if packages[p]['name'] not in ROOT_PACKAGES
                   and packages[p]['name'] != 'nero-core'), key=lambda p: (p['name'], p['version']))


def package_identity(package):
    return {'name': package['name'], 'version': package['version'],
            'license': package.get('license'), 'repository': package.get('repository')}


def source_notices(package, root=ROOT):
    folder = Path(package['manifest_path']).parent
    files = sorted(p for p in folder.rglob('*') if p.is_file() and NOTICE_NAME.fullmatch(p.name))
    declared = package.get('license_file')
    if declared:
        path = safe_file(folder, declared)
        if path not in files:
            files.append(path)
    result = [(p.relative_to(folder).as_posix(), safe_file(folder, p.relative_to(folder)).read_bytes())
              for p in files]
    if not result:
        overrides = root / 'apps/desktop/licenses/rust-supplements'
        entry = next((p for p in load(overrides / 'manifest.json')
                      if (p['name'], p['version']) == (package['name'], package['version'])), None)
        if entry is None or entry['license'] != package.get('license'):
            raise ValueError('Missing upstream license text: ' + package['name'] + '@' + package['version'])
        # Supplements bind the crate archive's own commit, rather than a moving upstream branch.
        vcs = load(folder / '.cargo_vcs_info.json')
        if vcs['git']['sha1'] != entry['crateSourceCommit']:
            raise ValueError('Supplement source commit differs: ' + package['name'])
        for item in entry['files']:
            data = safe_file(overrides, item['path']).read_bytes()
            if sha(data) != item['sha256']:
                raise ValueError('Modified upstream license supplement')
            result.append((Path(item['path']).name, data))
        result.append(('provenance.json', (json.dumps(entry, indent=2) + '\n').encode()))
    if not result or any(not data.strip() for _, data in result):
        raise ValueError('Empty upstream license text: ' + package['name'])
    return result


def model_notices(root=ROOT):
    folder = root / 'apps/desktop/licenses/models'
    entries = load(folder / 'manifest.json')
    if {e['manifest'] for e in entries} != {
        'config/models/nemotron-asr.json', 'config/models/whisper-turbo.json',
        'apps/desktop/models/diarization.json', 'apps/desktop/models/summary.json',
    }:
        raise ValueError('Model notice catalog is incomplete')
    result = [('models/manifest.json', (folder / 'manifest.json').read_bytes())]
    for entry in entries:
        pin = load(root / entry['manifest'])
        if {k: pin[k] for k in ('repository', 'revision', 'files')} != entry['model']:
            raise ValueError('Model license review differs from the pinned model: ' + entry['manifest'])
        for notice in entry['notices']:
            data = safe_file(folder, notice['path']).read_bytes()
            if not data.strip() or sha(data) != notice['sha256']:
                raise ValueError('Model license text differs from its reviewed source')
            result.append(('models/' + notice['path'], data))
    return list(dict(result).items())


def expected_manifest(target, document, root=ROOT):
    files, packages = {}, []
    for package in runtime_packages(document):
        identity = package_identity(package)
        identity['authors'] = package.get('authors', [])
        identity['source'] = package.get('source') or 'repository source'
        if (package.get('source') or '').startswith('registry+'):
            identity['sourceArchive'] = 'https://crates.io/api/v1/crates/' + package['name'] + '/' + package['version'] + '/download'
        elif package.get('source') is None:
            identity['sourcePath'] = Path(package['manifest_path']).parent.relative_to(root).as_posix()
        identity['files'] = []
        for relative, data in source_notices(package, root):
            path = 'rust/' + package['name'] + '-' + package['version'] + '/' + relative
            if path in files:
                raise ValueError('Duplicate notice path')
            files[path] = data
            identity['files'].append(path)
        packages.append(identity)
    files['THIRD_PARTY_NOTICES.md'] = (root / 'THIRD_PARTY_NOTICES.md').read_bytes()
    files.update(model_notices(root))
    manifest = {'schemaVersion': 1, 'target': target, 'roots': list(ROOT_PACKAGES),
                'cargoLockSha256': sha((root / 'Cargo.lock').read_bytes()),
                'scope': 'Normal target dependencies of desktop and agent; excludes build, dev and proc-macro dependencies.',
                'packages': packages,
                'files': [{'path': path, 'bytes': len(data), 'sha256': sha(data)}
                          for path, data in sorted(files.items())]}
    return manifest, files


def verify(directory, target, document=None, root=ROOT):
    if directory.is_symlink() or not directory.is_dir():
        raise ValueError('Distribution notices are missing')
    manifest = load(safe_file(directory, 'manifest.json'))
    expected, _ = expected_manifest(target, document or metadata(target, root), root)
    if manifest != expected:
        raise ValueError('Distribution notices differ from the current target runtime graph or model pins')
    expected_paths = {'manifest.json'}
    for item in manifest['files']:
        path = safe_file(directory, item['path'])
        data = path.read_bytes()
        if len(data) != item['bytes'] or sha(data) != item['sha256']:
            raise ValueError('Missing or changed packaged notice: ' + item['path'])
        expected_paths.add(item['path'])
    actual = {p.relative_to(directory).as_posix() for p in directory.rglob('*') if p.is_file()}
    if actual != expected_paths or any(p.is_symlink() for p in directory.rglob('*')):
        raise ValueError('Unexpected or linked distribution notice file')
    return manifest


def prepare(directory, target, document=None, root=ROOT):
    relative = directory.absolute().relative_to(root.absolute())
    cursor = root
    for part in relative.parts:
        cursor = cursor / part
        if cursor.is_symlink():
            raise ValueError('Invalid generated notice output: linked directory')
    document = document or metadata(target, root)
    manifest, files = expected_manifest(target, document, root)
    directory.parent.mkdir(parents=True, exist_ok=True)
    if directory.is_symlink() or (directory.exists() and not directory.is_dir()):
        raise ValueError('Invalid generated notice output')
    with tempfile.TemporaryDirectory(prefix='.nero-notices-', dir=directory.parent) as temporary:
        stage = Path(temporary) / 'distribution'
        stage.mkdir()
        for relative, data in files.items():
            path = stage / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        (stage / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n', encoding='utf-8')
        verify(stage, target, document, root)
        # Only this regenerable notice output is replaced; stale target/package notices must not survive.
        if directory.exists():
            previous = Path(temporary) / 'previous'
            directory.rename(previous)
        stage.rename(directory)
    return manifest


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--target', default=os.environ.get('TAURI_ENV_TARGET_TRIPLE')
                        or os.environ.get('CARGO_BUILD_TARGET'))
    parser.add_argument('--verify', type=Path)
    parser.add_argument('--resources', type=Path)
    args = parser.parse_args()
    target = args.target or subprocess.check_output(['rustc', '-vV'], text=True).split('host: ')[1].splitlines()[0]
    if args.verify and args.resources:
        parser.error('Select a staged directory or packaged resources')
    directory = (args.resources / 'native/licenses/distribution' if args.resources else args.verify)
    if directory:
        result = verify(directory, target)
    else:
        result = prepare(APP / '.generated/native/licenses/distribution', target)
    print('Verified distribution notices:', target, len(result['packages']), 'runtime packages')


if __name__ == '__main__':
    main()
