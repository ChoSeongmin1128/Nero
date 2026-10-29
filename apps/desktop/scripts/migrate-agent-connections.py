import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import tempfile

CLIENTS = ('claude', 'codex')
MAX_STATE_BYTES = 4096


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def regular(path):
    metadata = path.lstat()
    require(stat.S_ISREG(metadata.st_mode) and not path.is_symlink(),
            'Connection migration requires a plain file')
    return metadata


def directory(path):
    require(path.is_dir() and not path.is_symlink(),
            'Connection migration requires a plain directory')


def unique_object(pairs):
    value = {}
    for key, item in pairs:
        require(key not in value, 'Connection state has duplicate fields')
        value[key] = item
    return value


def state_bytes(path):
    require(regular(path).st_size <= MAX_STATE_BYTES, 'Connection state exceeds the read limit')
    with path.open('rb') as stream:
        value = stream.read(MAX_STATE_BYTES + 1)
    require(len(value) <= MAX_STATE_BYTES, 'Connection state exceeds the read limit')
    return value


def replace_data_dir(raw, source, destination, client):
    text = raw.decode('utf-8')
    value = json.loads(text, object_pairs_hook=unique_object)
    require(isinstance(value, dict) and type(value.get('schemaVersion')) is int
            and value.get('schemaVersion') == 1
            and value.get('client') == client
            and isinstance(value.get('token'), str)
            and re.fullmatch(r'[0-9a-fA-F]{8}(?:-[0-9a-fA-F]{4}){3}-[0-9a-fA-F]{12}', value['token'])
            and isinstance(value.get('generation'), str)
            and re.fullmatch(r'[0-9a-fA-F]{64}', value['generation'])
            and value.get('dataDir') == str(source), 'Connection state does not match the migration source')
    decoder = json.JSONDecoder()
    offset = text.index('{') + 1
    while True:
        offset = skip_space(text, offset)
        key, offset = decoder.raw_decode(text, offset)
        offset = skip_space(text, offset)
        require(text[offset] == ':', 'Connection state is invalid')
        start = skip_space(text, offset + 1)
        _, end = decoder.raw_decode(text, start)
        if key == 'dataDir':
            replacement = json.dumps(str(destination), ensure_ascii=False)
            return (text[:start] + replacement + text[end:]).encode('utf-8')
        offset = skip_space(text, end)
        require(text[offset] == ',', 'Connection state is missing its data directory')
        offset += 1


def skip_space(text, offset):
    while offset < len(text) and text[offset] in ' \t\r\n':
        offset += 1
    return offset


def lock_file(path, create=False):
    if path.exists() or path.is_symlink():
        regular(path)
    flags = os.O_RDWR | getattr(os, 'O_NOFOLLOW', 0)
    if create:
        flags |= os.O_CREAT
    descriptor = os.open(path, flags, 0o600)
    stream = os.fdopen(descriptor, 'r+b')
    try:
        require(stat.S_ISREG(os.fstat(descriptor).st_mode), 'Connection lock must be a plain file')
        fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BaseException:
        stream.close()
        raise RuntimeError('Close active Nero agent sessions before migrating their data path') from None
    return stream


def require_sessions_idle(client):
    sessions = client / 'sessions'
    if not sessions.exists():
        require(not sessions.is_symlink(), 'Connection sessions must be a plain directory')
        return
    directory(sessions)
    for path in sessions.iterdir():
        if path.suffix == '.lock':
            with lock_file(path):
                pass


def prepare(connections, source_data, destination_data):
    connections, source_data, destination_data = map(Path, (connections, source_data, destination_data))
    require(source_data.is_absolute() and destination_data.is_absolute()
            and source_data != destination_data, 'Connection migration paths are invalid')
    directory(connections)
    plan = {'entries': [], 'locks': [], 'applied': []}
    try:
        for client in CLIENTS:
            folder = connections / client
            if not folder.exists():
                require(not folder.is_symlink(), 'Connection directory must not be linked')
                continue
            directory(folder)
            path = folder / 'state.json'
            if not path.exists():
                require(not path.is_symlink(), 'Connection state must not be linked')
                continue
            plan['locks'].append(lock_file(folder / 'prepare.lock', create=True))
            require_sessions_idle(folder)
            original = state_bytes(path)
            migrated = replace_data_dir(original, source_data, destination_data, client)
            require(len(migrated) <= MAX_STATE_BYTES, 'Migrated connection state exceeds the read limit')
            plan['entries'].append({'client': client, 'path': path, 'original': original,
                                    'migrated': migrated, 'mode': stat.S_IMODE(regular(path).st_mode)})
        return plan
    except BaseException:
        release(plan)
        raise


def atomic_write(path, value, mode):
    descriptor, name = tempfile.mkstemp(prefix='.state-migration-', dir=path.parent)
    temporary = Path(name)
    try:
        with os.fdopen(descriptor, 'wb') as stream:
            os.fchmod(stream.fileno(), mode)
            stream.write(value)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        if temporary.exists():
            temporary.unlink()


def apply(plan):
    require(not plan['applied'], 'Connection migration has already been applied')
    for entry in plan['entries']:
        require(state_bytes(entry['path']) == entry['original'], 'Connection state changed before migration')
        require_sessions_idle(entry['path'].parent)
    for entry in plan['entries']:
        atomic_write(entry['path'], entry['migrated'], entry['mode'])
        plan['applied'].append(entry)


def verify(plan):
    for entry in plan['entries']:
        require(state_bytes(entry['path']) == entry['migrated'], 'Migrated connection state changed')
    return public_receipt(plan)


def rollback(plan):
    while plan['applied']:
        entry = plan['applied'][-1]
        current = state_bytes(entry['path'])
        if current != entry['original']:
            require(current == entry['migrated'], 'Connection state changed; preserve it for recovery')
            atomic_write(entry['path'], entry['original'], entry['mode'])
        plan['applied'].pop()


def release(plan):
    for stream in plan['locks']:
        stream.close()
    plan['locks'].clear()


def public_receipt(plan):
    return {entry['client']: {
        'path': entry['client'] + '/state.json',
        'before': fingerprint(entry['original']), 'after': fingerprint(entry['migrated']),
    } for entry in plan['entries']}


def fingerprint(value):
    return {'bytes': len(value), 'sha256': hashlib.sha256(value).hexdigest()}
