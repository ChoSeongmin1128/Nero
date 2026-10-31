#!/usr/bin/env python3
import argparse
import importlib.util
import json
import os
from pathlib import Path, PurePosixPath
import plistlib
import re
import stat
import sys
import tarfile
import tempfile

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('nero_installer', Path(__file__).with_name('install-macos.py'))
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)
IDENTIFIER = 'io.github.choseongmin1128.nero.desktop'


def validate_macos_trust(app):
    installer.run('codesign', '--verify', '--deep', '--strict', str(app))
    installer.run('xcrun', 'stapler', 'validate', str(app))
    installer.run('spctl', '--assess', '--type', 'execute', str(app))


def bundle_metadata(app, receipt):
    installer.require_directory(app)
    installer.require(app.name == 'Nero.app', 'Expected Nero.app')
    info = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
    build = installer.load(app / 'Contents/Resources/build-info.json')
    installer.require(
        build.get('formatVersion') == 1 and build.get('bundleIdentifier') == IDENTIFIER
        and build.get('hostPlatform') == 'darwin' and build.get('hostArchitecture') == 'arm64'
        and build.get('sourceDirty') is False
        and re.fullmatch(r'[0-9a-f]{40}', build.get('sourceCommit', '')) is not None
        and re.fullmatch(r'[0-9a-f]{40}', build.get('sourceTree', '')) is not None,
        'Clean macOS arm64 build provenance is required')
    installer.installed_version({'CFBundleShortVersionString': build.get('version'),
                                 'CFBundleVersion': build.get('build')})
    installer.require(re.fullmatch(r'(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)', build['version'])
                      and re.fullmatch(r'[1-9]\d*', build['build']), 'Version/build must be canonical')
    installer.require(0 < int(build['build']) <= 2**64 - 1, 'Build exceeds the supported range')
    installer.require(
        (info.get('CFBundleIdentifier'), info.get('CFBundleName'),
         info.get('CFBundleShortVersionString'), info.get('CFBundleVersion'))
        == (IDENTIFIER, 'Nero', build['version'], build['build']), 'Bundle metadata differs')
    installer.require(
        (receipt.get('bundle_id'), receipt.get('version'), receipt.get('build'),
         receipt.get('source_commit'), receipt.get('source_tree'), receipt.get('architecture'))
        == (IDENTIFIER, build['version'], build['build'], build['sourceCommit'],
            build['sourceTree'], 'arm64')
        and receipt.get('signed') is True and receipt.get('notarized') is True
        and receipt.get('source_dirty') is False, 'Notarized release receipt differs')
    before = installer.app_tree(app)
    installer.require(installer.tree_hash(before) == receipt.get('app_tree_sha256'),
                      'Bundle changed after signing/notarization')
    return build, before


def modes(app):
    return {str(p.relative_to(app)): stat.S_IMODE(p.lstat().st_mode)
            for p in app.rglob('*') if not p.is_symlink()}


def extract_archive(archive, destination):
    with tarfile.open(archive, 'r:gz') as stream:
        entries = stream.getmembers()
        installer.require(bool(entries), 'Empty updater archive')
        for member in entries:
            name = PurePosixPath(member.name)
            installer.require(not name.is_absolute() and name.parts[0] == 'Nero.app'
                              and '..' not in name.parts
                              and (member.isfile() or member.isdir() or member.issym()),
                              'Invalid updater archive entry')
        stream.extractall(destination, members=entries, filter='data')
    app = destination / 'Nero.app'
    installer.require_directory(app)
    return app


def package(app, output, receipt_path, trust=validate_macos_trust):
    app, output = Path(app).absolute(), Path(output).absolute()
    installer.require(not app.is_symlink(), 'App path must not be a symlink')
    installer.require(re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9._-]+\.app\.tar\.gz', output.name)
                      is not None, 'Use a URL-safe .app.tar.gz filename')
    paths = [output, Path(str(output) + '.receipt.json'), Path(str(output) + '.build-info.json')]
    installer.require(not any(p.exists() or p.is_symlink() for p in paths),
                      'An output or receipt already exists')
    receipt = installer.load(Path(receipt_path))
    build, before = bundle_metadata(app, receipt)
    original_modes = modes(app)
    trust(app)
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='.nero-update-', dir=output.parent) as temporary:
        temporary = Path(temporary)
        archive = temporary / output.name
        with tarfile.open(archive, 'w:gz', format=tarfile.PAX_FORMAT) as stream:
            stream.add(app, arcname='Nero.app')
        installer.require(0 < archive.stat().st_size <= 512 * 1024 * 1024,
                          'Updater archive exceeds the size limit')
        extracted = extract_archive(archive, temporary / 'extracted')
        installer.require(installer.app_tree(extracted) == before and modes(extracted) == original_modes,
                          'Archived app content or executable permissions changed')
        installer.require(installer.app_tree(app) == before and modes(app) == original_modes,
                          'Source app changed during packaging')
        trust(extracted)
        identity = {key: build[key] for key in ('version', 'build', 'sourceCommit', 'sourceTree', 'sourceDirty')}
        result = dict(schemaVersion=1, platform='darwin-aarch64', **identity,
                      artifact={'filename': output.name, 'bytes': archive.stat().st_size,
                                'sha256': installer.sha(archive)},
                      verification={'signedAndStapledSource': True, 'extractedAppVerified': True,
                                    'appTreeSha256': installer.tree_hash(before),
                                    'notary': receipt.get('app_notary', receipt.get('notary'))})
        os.link(archive, output)
        installer.write_new(paths[1], result)
        installer.write_new(paths[2], build)
    return result


def main():
    parser = argparse.ArgumentParser()
    for name in ('app', 'release-receipt', 'out'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    result = package(args.app, args.out, args.release_receipt)
    print(json.dumps(result))


if __name__ == '__main__':
    main()
