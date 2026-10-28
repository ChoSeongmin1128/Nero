#!/usr/bin/env python3
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import stat
import subprocess
import time

ROOT = Path(__file__).resolve().parents[3]
TARGET = Path('/Applications/Nero.app')
DATA = Path.home() / 'Library/Application Support/io.github.choseongmin1128.grove.desktop'
CONNECTIONS = Path.home() / 'Library/Application Support/Nero/connections'
IDENTIFIER = 'io.github.choseongmin1128.grove.desktop'


def require(condition, reason):
    if not condition:
        raise RuntimeError(reason)


def run(*args):
    return subprocess.run(args, check=True, text=True, capture_output=True, timeout=120)


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def regular_file(path):
    metadata = path.lstat()
    require(stat.S_ISREG(metadata.st_mode) and not path.is_symlink(),
            'Expected a regular file: ' + str(path))
    return metadata


def load(path, maximum=16 * 1024 * 1024):
    require(regular_file(path).st_size <= maximum, 'JSON file exceeds the read limit')
    return json.loads(path.read_text())


def require_directory(path):
    require(path.is_dir() and not path.is_symlink(), 'Expected a plain directory: ' + str(path))


def walk_error(error):
    raise error


def app_tree(folder):
    require_directory(folder)
    result = {}
    for directory, children, names in os.walk(folder, onerror=walk_error):
        for name in sorted(children + names):
            path = Path(directory) / name
            relative = str(path.relative_to(folder))
            if path.is_symlink():
                require(path.resolve().is_relative_to(folder.resolve()), 'App link escapes its bundle')
                result[relative] = {'link': os.readlink(path)}
            elif path.is_file():
                result[relative] = {'bytes': path.stat().st_size, 'sha256': sha(path)}
            else:
                require(path.is_dir(), 'Unexpected app bundle entry')
    return result


def tree_hash(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def inventory(folder):
    require_directory(folder)
    result = {}
    for directory, children, names in os.walk(folder, onerror=walk_error):
        current = Path(directory)
        if current == folder:
            children[:] = [name for name in children if name != 'models']
        for name in children:
            require_directory(current / name)
        for name in names:
            path = current / name
            if path == folder / 'store.lock':
                continue
            metadata = regular_file(path)
            result[str(path.relative_to(folder))] = {'bytes': metadata.st_size, 'sha256': sha(path)}
    return result


def connection_states():
    require_directory(CONNECTIONS)
    result = {}
    for client in ('claude', 'codex'):
        folder = CONNECTIONS / client
        require_directory(folder)
        path = folder / 'state.json'
        metadata = regular_file(path)
        result[client] = {'bytes': metadata.st_size, 'sha256': sha(path)}
    return result


def model_metadata():
    folder = DATA / 'models'
    if not folder.exists():
        require(not folder.is_symlink(), 'Unexpected model directory link')
        return {}
    require_directory(folder)
    result = {}
    for directory, children, names in os.walk(folder, onerror=walk_error):
        current = Path(directory)
        for name in children:
            require_directory(current / name)
        for name in names:
            path = current / name
            metadata = regular_file(path)
            result[str(path.relative_to(folder))] = {
                'bytes': metadata.st_size, 'mtimeNs': metadata.st_mtime_ns,
                'inode': metadata.st_ino, 'device': metadata.st_dev,
            }
    return result


def installed_version(info):
    version = info.get('CFBundleShortVersionString', '')
    build = info.get('CFBundleVersion', '')
    require(isinstance(version, str) and re.fullmatch(r'\d+\.\d+\.\d+', version) is not None,
            'Unknown app version')
    require(isinstance(build, str) and build.isdigit(), 'Unknown app build')
    return tuple(int(part) for part in version.split('.')), int(build)


def require_stopped():
    processes = run('ps', '-axo', 'pid=,comm=').stdout.splitlines()
    require(not any(str(TARGET / 'Contents/MacOS/') in line for line in processes),
            'Close Nero through its normal application UI; no process will be stopped by this installer')


def idle_snapshot():
    cli = TARGET / 'Contents/Resources/agent/nero'
    offset = 0
    version = None
    meetings = []
    for _ in range(10000):
        page = json.loads(run(str(cli), '--data-dir', str(DATA), 'meetings', 'list',
                              '--offset', str(offset), '--limit', '200', '--json').stdout)
        require(isinstance(page.get('meetings'), list), 'Meeting list is not valid')
        page_version = page.get('sourceVersion')
        require(isinstance(page_version, str) and re.fullmatch(r'sha256:[0-9a-f]{64}', page_version) is not None,
                'Meeting snapshot version missing')
        if version is None:
            version = page_version
        require(version == page_version, 'Library changed during paging; no installation changes made')
        pagination = page.get('pagination', {})
        require(pagination.get('offset') == offset and pagination.get('limit') == 200,
                'Unexpected meeting pagination')
        for meeting in page['meetings']:
            require(meeting.get('status') not in ('recording', 'paused', 'finalizing'), 'A recording is active')
            meetings.append({'id': meeting['id'], 'status': meeting['status']})
        if pagination.get('hasMore') is False:
            break
        require(pagination.get('hasMore') is True and page['meetings'], 'Meeting pagination cannot advance')
        offset += pagination['limit']
    else:
        raise RuntimeError('Meeting pagination exceeded the review limit')
    processing = DATA / 'processing'
    jobs = {}
    if processing.exists():
        require_directory(processing)
        for job in sorted(processing.iterdir()):
            if not job.is_dir() or job.is_symlink():
                require(not job.is_symlink(), 'Linked processing job path')
                continue
            states = job / 'job-state'
            if not states.exists():
                require(not states.is_symlink(), 'Linked job-state path')
                continue
            require_directory(states)
            generations = []
            for path in states.iterdir():
                if not path.name.endswith('.json'):
                    continue
                require(re.fullmatch(r'[0-9]{20}\.json', path.name) is not None,
                        'Unrecognized job-state generation; preserve it for review')
                require(int(path.stem) > 0, 'Invalid job-state generation')
                generations.append(path)
            if not generations:
                continue
            latest = max(generations, key=lambda path: int(path.stem))
            state = load(latest, 16 * 1024)
            require(state.get('id') == job.name, 'Job identity differs from its directory')
            require(state.get('status') not in ('running', 'queued', 'cancelling'), 'A processing job is active')
            require(state.get('status') in ('completed', 'cancelled', 'failed', 'interrupted', 'needsReview'),
                    'Unrecognized job-state status')
            jobs[job.name] = {'generation': latest.name, 'sha256': sha(latest), 'status': state['status']}
    else:
        require(not processing.is_symlink(), 'Linked processing directory')
    return {'sourceVersion': version, 'meetings': meetings, 'jobs': jobs}


def write_new(path, value):
    with path.open('x') as destination:
        json.dump(value, destination, ensure_ascii=False, indent=2)
        destination.write('\n')
        destination.flush()
        os.fsync(destination.fileno())


def ledger(work, event, **fields):
    with (work / 'installation-events.jsonl').open('a') as stream:
        stream.write(json.dumps({'event': event, 'time': time.time(), **fields}) + '\n')
        stream.flush()
        os.fsync(stream.fileno())


def verify_signature(app, receipt):
    run('codesign', '--verify', '--deep', '--strict', str(app))
    inspected = run('codesign', '-d', '--verbose=4', str(app))
    description = inspected.stdout + inspected.stderr
    require('Authority=Developer ID Application:' in description and 'runtime' in description,
            'A Developer ID app with hardened runtime is required')
    identity = {}
    for name in ('Identifier', 'TeamIdentifier', 'CDHash'):
        matches = re.findall(r'^' + name + r'=(.+)$', description, re.MULTILINE)
        require(len(matches) == 1, 'App signing identity is missing or ambiguous')
        identity[name] = matches[0].strip()
    require(identity == receipt.get('app_signature') and identity['Identifier'] == IDENTIFIER,
            'App signing identity differs from its release receipt')
    run('xcrun', 'stapler', 'validate', str(app))
    run('spctl', '--assess', '--type', 'execute', '--verbose=4', str(app))


def verify_release(app, dmg, archive=None):
    regular_file(dmg)
    receipt = load(Path(str(dmg) + '.receipt.json'))
    receipts = [receipt]
    if archive is not None:
        regular_file(archive)
        receipts.append(load(Path(str(archive) + '.receipt.json')))
    info = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
    installed_version(info)
    require(info.get('CFBundleIdentifier') == IDENTIFIER and info.get('CFBundleName') == 'Nero',
            'Unexpected release bundle identity')
    build = load(app / 'Contents/Resources/build-info.json')
    source = build.get('sourceCommit')
    source_tree = build.get('sourceTree')
    require(isinstance(source, str) and re.fullmatch(r'[0-9a-f]{40}', source) is not None,
            'Release source commit missing')
    require(isinstance(source_tree, str) and re.fullmatch(r'[0-9a-f]{40}', source_tree) is not None,
            'Release source tree missing')
    require(run('git', '-C', str(ROOT), 'rev-parse', source + '^{tree}').stdout.strip() == source_tree,
            'Release source tree differs from its commit')
    expected_identity = (info['CFBundleShortVersionString'], info['CFBundleVersion'], IDENTIFIER)
    for value in receipts:
        require((value.get('version'), value.get('build'), value.get('bundle_id')) == expected_identity,
                'Release artifacts identify different builds')
        require(value.get('source_commit') == source and value.get('source_tree') == source_tree
                and value.get('source_dirty') is False, 'Release receipt does not identify the clean source')
        require(value.get('signed') is True and value.get('notarized') is True
                and value.get('notary', {}).get('status') == 'Accepted',
                'Signed artifacts with accepted notarization are required')
    require(build.get('sourceDirty') is False
            and (build.get('version'), build.get('build'), build.get('bundleIdentifier')) == expected_identity,
            'Build provenance differs from the release bundle')
    if archive is not None:
        require(sha(archive) == receipts[1].get('zip_sha256'), 'ZIP hash differs from its receipt')
    require(sha(dmg) == receipt.get('dmg_sha256'), 'DMG hash differs from its receipt')
    expected_tree = app_tree(app)
    require(tree_hash(expected_tree) == receipt.get('app_tree_sha256'), 'App tree differs from its release receipt')
    if archive is not None:
        require(receipts[1].get('app_tree_sha256') == receipt.get('app_tree_sha256')
                and receipts[1].get('app_signature') == receipt.get('app_signature'),
                'DMG and ZIP identify different apps')
    require(receipt.get('verification', {}).get('embedded_app_matches_source') is True,
            'DMG embedded-app verification is missing')
    require(not any(Path(name).suffix.lower() in ('.gguf', '.safetensors', '.p8', '.p12', '.wav', '.m4a', '.mp3')
                    for name in expected_tree), 'Private/model data found inside release app')
    verify_signature(app, receipt)
    run('codesign', '--verify', '--strict', str(dmg))
    run('xcrun', 'stapler', 'validate', str(dmg))
    run('spctl', '--assess', '--type', 'open', '--context', 'context:primary-signature', '--verbose=4', str(dmg))
    return info, receipt, expected_tree


def install(app, dmg, work, archive=None):
    require_directory(work)
    require_directory(DATA)
    require_directory(TARGET)
    for path in (app, dmg, work) + ((archive,) if archive is not None else ()):
        require(not path.resolve().is_relative_to(DATA.resolve())
                and not path.resolve().is_relative_to(CONNECTIONS.resolve()),
                'Installation artifacts must stay outside user data and connection state')
    require(app.suffix == '.app' and dmg.suffix == '.dmg'
            and (archive is None or archive.suffix == '.zip'), 'Expected an app, DMG and optional ZIP release')
    require(app != TARGET and not work.is_relative_to(app) and not work.is_relative_to(TARGET)
            and not app.is_relative_to(TARGET), 'App and installation paths overlap')
    stage = TARGET.with_name('.Nero-install.app')
    previous = work / 'previous-installed.app'
    installation_path = work / 'installation.json'
    pending_path = work / 'installation.pending.json'
    require(app != stage and not app.is_relative_to(previous), 'Candidate uses a reserved installation path')
    current_info = plistlib.loads((TARGET / 'Contents/Info.plist').read_bytes())
    require(current_info.get('CFBundleIdentifier') == IDENTIFIER, 'Unexpected installed app identity')
    current_version = installed_version(current_info)
    candidate = app if app.exists() or app.is_symlink() else TARGET
    info, receipt, expected_tree = verify_release(candidate, dmg, archive)
    release_version = installed_version(info)
    require(current_version <= release_version, 'A newer Nero is already installed; downgrade refused')
    source = receipt['source_commit']
    if current_version == release_version:
        require(installation_path.exists(), 'This build is already present without this installation receipt')
        installed = load(installation_path)
        require(installed.get('build') == info['CFBundleVersion'] and installed.get('sourceCommit') == source
                and installed.get('appTreeSha256') == receipt['app_tree_sha256'],
                'Existing installation receipt differs; preserve it for review')
        require(app_tree(TARGET) == expected_tree, 'Installed app differs from the recorded release')
        verify_signature(TARGET, receipt)
        return {'alreadyInstalled': True, 'build': info['CFBundleVersion'], 'sourceCommit': source, 'changesMade': False}
    require(candidate == app, 'The release candidate is missing')
    require_directory(app)
    device = TARGET.parent.stat().st_dev
    require(all(path.stat().st_dev == device for path in (TARGET, app, work)),
            'App, installation and rollback directory must share a device; copying is not permitted')
    for path in (stage, previous, work / 'data-before', work / 'data-before-manifest.json',
                 work / 'connection-state-before.json', work / 'model-metadata-before.json',
                 work / 'idle-before.json', installation_path, pending_path, work / 'installation-events.jsonl'):
        require(not path.exists() and not path.is_symlink(), 'Preserve an existing installation attempt: ' + str(path))
    require_stopped()
    first_idle = idle_snapshot()
    time.sleep(2)
    require_stopped()
    second_idle = idle_snapshot()
    require(first_idle == second_idle, 'Library/job state changed during idle confirmation')
    lock_path = DATA / 'store.lock'
    metadata = regular_file(lock_path)
    with os.fdopen(os.open(lock_path, os.O_RDWR | os.O_NOFOLLOW), 'r+') as lock:
        require(os.fstat(lock.fileno()).st_ino == metadata.st_ino, 'Writer-lock file changed before opening')
        for attempt in range(100):
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                if attempt == 99:
                    raise RuntimeError('Nero still holds the writer lock; no installation changes made')
                time.sleep(0.1)
        require_stopped()
        require(idle_snapshot() == second_idle, 'Recording or jobs changed before acquiring the writer lock')
        before = inventory(DATA)
        connections_before = connection_states()
        models_before = model_metadata()
        require(inventory(DATA) == before, 'Data changed while preparing its manifest')
        require(connection_states() == connections_before, 'Connection state changed while preparing installation')
        require(model_metadata() == models_before, 'Model files changed while preparing installation')
        write_new(work / 'data-before-manifest.json', before)
        write_new(work / 'connection-state-before.json', connections_before)
        write_new(work / 'model-metadata-before.json', models_before)
        write_new(work / 'idle-before.json', second_idle)
        staged = False
        moved_previous = False
        promoted = False
        try:
            require(not stage.exists() and not stage.is_symlink(), 'Installation stage already exists')
            os.rename(app, stage)
            staged = True
            require(app_tree(stage) == expected_tree, 'Staged app differs from the release')
            verify_signature(stage, receipt)
            require_stopped()
            require(inventory(DATA) == before and connection_states() == connections_before
                    and model_metadata() == models_before, 'Data, connection state or models changed before app replacement')
            ledger(work, 'prepared', sourceCommit=source, oldVersion=current_info['CFBundleShortVersionString'],
                   oldBuild=current_info['CFBundleVersion'], dataFiles=len(before))
            require(not previous.exists() and not previous.is_symlink(), 'A rollback app already exists')
            os.rename(TARGET, previous)
            moved_previous = True
            os.rename(stage, TARGET)
            promoted = True
            require(app_tree(TARGET) == expected_tree, 'Installed app differs from the release')
            verify_signature(TARGET, receipt)
            require_stopped()
            require(inventory(DATA) == before, 'Existing data changed during installation')
            connections_after = connection_states()
            require(connections_after == connections_before, 'Existing connection state changed during installation')
            require(model_metadata() == models_before, 'Model files changed during installation')
            result = {
                'build': info['CFBundleVersion'], 'version': info['CFBundleShortVersionString'], 'sourceCommit': source,
                'installedApp': str(TARGET), 'previousApp': str(previous),
                'appTreeSha256': receipt['app_tree_sha256'],
                'dataFilesPreserved': len(before), 'dataUnchanged': True,
                'dataUnchangedWhileAppStopped': True, 'dataManifestSha256': tree_hash(before),
                'userDataCopiedOrMoved': False, 'dataBackupCreated': False, 'writerLockHeld': True,
                'connectionStateHashesBefore': connections_before,
                'connectionStateHashesAfter': connections_after, 'connectionStatesUnchanged': True,
                'clientConfigurationsChanged': False,
                'modelMetadataUnchanged': True, 'modelRelocations': [], 'modelWeightsCopiedOrMoved': False,
                'candidateAppMoved': True, 'appCopied': False,
                'guiVerified': False, 'idleConfirmations': 3, 'appWasStoppedAtReplacement': True,
                'installedAt': time.time(),
            }
            write_new(pending_path, result)
            ledger(work, 'validated', sourceCommit=source, build=info['CFBundleVersion'],
                   dataUnchanged=True, connectionStatesUnchanged=True)
            os.rename(pending_path, installation_path)
        except BaseException as error:
            rollback = []
            try:
                require_stopped()
                if promoted:
                    os.rename(TARGET, stage)
                if moved_previous:
                    os.rename(previous, TARGET)
                if staged:
                    require(not app.exists() and not app.is_symlink(), 'Candidate path exists during rollback')
                    os.rename(stage, app)
            except BaseException as recovery:
                rollback.append(str(recovery))
            ledger(work, 'installationFailed', error=str(error), rollbackErrors=rollback)
            if rollback:
                raise RuntimeError('Installation failed; preserve the stage and rollback app for recovery: '
                                   + '; '.join(rollback)) from error
            raise
    return {key: result[key] for key in ('build', 'sourceCommit', 'dataFilesPreserved',
                                        'dataUnchanged', 'connectionStatesUnchanged', 'guiVerified')}


def main(argv=None):
    parser = argparse.ArgumentParser()
    for name in ('app', 'dmg', 'work'):
        parser.add_argument('--' + name, required=True, type=Path)
    parser.add_argument('--zip', type=Path)
    args = parser.parse_args(argv)
    paths = [Path(os.path.abspath(value.expanduser())) for value in (args.app, args.dmg, args.work)]
    archive = Path(os.path.abspath(args.zip.expanduser())) if args.zip is not None else None
    print(json.dumps(install(*paths, archive=archive)))


if __name__ == '__main__':
    main()
