import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';

const source = await readFile(new URL('../src/main.js', import.meta.url), 'utf8');
const functions = source.slice(source.indexOf('function renderAgentConnection()'), source.indexOf('function renderSettings()'));
const deferred = () => { let resolve; const promise = new Promise(done => { resolve = done; }); return { promise, resolve }; };
const info = { available: true, executablePath: '/Example App/nero', dataDir: '/Example Library', config: { mcpServers: { nero: { command: '/Example App/nero', args: ['mcp', '--data-dir', '/Example Library'] } } }, cliExample: 'nero meetings list', clients: [{ id: 'claude', state: 'not_connected' }, { id: 'codex', state: 'not_connected' }] };

function harness({ load = async () => info, write = async () => {}, connect = async () => info } = {}) {
  const nodes = new Map();
  const byId = id => {
    if (!nodes.has(id)) {
      const classes = new Set();
      const attributes = new Map();
      nodes.set(id, { textContent: '', dataset: {}, hidden: false, disabled: false, open: id === 'settings-dialog',
        classList: { add: value => classes.add(value), remove: value => classes.delete(value), contains: value => classes.has(value) },
        setAttribute(name, value) { attributes.set(name, String(value)); }, getAttribute: name => attributes.get(name) ?? null,
        focus() { throw Error('Unexpected focus'); } });
    }
    return nodes.get(id);
  };
  const state = { settingsPanel: 'agent' };
  const agentConnection = { info: null, loading: false, error: '', request: 0, refresh: 0, timer: null, pending: {}, errors: {} };
  let calls = 0;
  const copied = [], timers = new Map();
  const dialog = { opened: [], triggers: [], updates: [], closed: [], open(value, trigger) { dialog.opened.push(value); dialog.triggers.push(trigger); }, update: value => dialog.updates.push(value), close: value => dialog.closed.push(value) };
  const context = vm.createContext({ byId, state, agentConnection, agentConnectionDialog: dialog, bridge: { agentConnectionInfo: () => { calls += 1; return load(); }, connectAgentClient: connect },
    navigator: { clipboard: { writeText: async text => { copied.push(text); await write(text); } } },
    copyFeedbackTimers: new Map(), setTimeout: callback => { const key = {}; timers.set(key, callback); return key; }, clearTimeout: key => timers.delete(key),
  });
  vm.runInContext(functions, context);
  return { byId, state, agentConnection, copied, timers, context, dialog, get calls() { return calls; } };
}

test('loads only while the agent panel is open and deduplicates renders and tab visits', async () => {
  const pending = deferred();
  const app = harness({ load: () => pending.promise });
  app.state.settingsPanel = 'appearance';
  await app.context.loadAgentConnection();
  assert.equal(app.calls, 0);
  app.state.settingsPanel = 'agent';
  const first = app.context.loadAgentConnection();
  await app.context.loadAgentConnection();
  app.context.renderAgentConnection();
  assert.equal(app.calls, 1);
  assert.equal(app.byId('settings-agent-copy-config').disabled, true);
  pending.resolve(info);
  await first;
  await app.context.loadAgentConnection();
  assert.equal(app.calls, 1);
  assert.equal(app.byId('settings-agent-path').textContent, info.executablePath);
  assert.equal(app.byId('settings-agent-copy-config').disabled, false);
});

test('missing binaries and rejected requests never expose usable configuration', async () => {
  const missing = harness({ load: async () => ({ ...info, available: false }) });
  await missing.context.loadAgentConnection();
  assert.equal(missing.byId('settings-agent-status').textContent, '연결 도구가 포함되지 않은 빌드입니다.');
  assert.equal(missing.byId('settings-agent-copy-config').disabled, true);
  assert.equal(missing.byId('settings-agent-path').textContent, '');
  await missing.context.copyAgentConnection('config');
  assert.equal(missing.copied.length, 0);
  const failed = harness({ load: async () => { throw Error('native unavailable'); } });
  await failed.context.loadAgentConnection();
  assert.match(failed.byId('settings-agent-status').textContent, /불러오지 못했습니다/);
  assert.equal(failed.byId('settings-agent-copy-cli').disabled, true);
});

test('late native replies cannot replace a reopened dialog or move focus', async () => {
  const old = deferred();
  let latest = false;
  const app = harness({ load: () => latest ? Promise.resolve(info) : old.promise });
  const first = app.context.loadAgentConnection();
  app.byId('settings-dialog').open = false;
  app.context.resetAgentConnection();
  await app.context.loadAgentConnection();
  assert.equal(app.calls, 1);
  app.byId('settings-dialog').open = true;
  latest = true;
  await app.context.loadAgentConnection();
  old.resolve({ available: false });
  await first;
  assert.equal(app.byId('settings-agent-copy-config').disabled, false);
  assert.equal(app.agentConnection.info, info);
});

test('copies exact configuration and CLI text with local success and failure feedback', async () => {
  const app = harness();
  await app.context.loadAgentConnection();
  await app.context.copyAgentConnection('config');
  assert.equal(app.copied[0], JSON.stringify(info.config, null, 2));
  assert.equal(app.byId('settings-agent-feedback').textContent, '복사됨');
  assert.equal(app.byId('settings-agent-feedback').classList.contains('soft'), true);
  await app.context.copyAgentConnection('cli');
  assert.equal(app.copied[1], info.cliExample);
  const failed = harness({ write: async () => { throw Error('clipboard denied'); } });
  await failed.context.loadAgentConnection();
  failed.byId('settings-agent-feedback').classList.add('soft');
  await failed.context.copyAgentConnection('config');
  assert.match(failed.byId('settings-agent-feedback').textContent, /복사하지 못했습니다/);
  assert.equal(failed.byId('settings-agent-feedback').classList.contains('soft'), false);
});

test('closing during a clipboard request discards late feedback', async () => {
  const pending = deferred();
  const app = harness({ write: () => pending.promise });
  await app.context.loadAgentConnection();
  const copy = app.context.copyAgentConnection('config');
  app.byId('settings-dialog').open = false;
  app.context.resetAgentConnection();
  pending.resolve();
  await copy;
  assert.equal(app.byId('settings-agent-feedback').hidden, true);
  assert.equal(app.timers.size, 0);
});


test('connection is per client and only native evidence can mark it connected', async () => {
  const pending = deferred();
  const app = harness({ connect: () => pending.promise });
  await app.context.loadAgentConnection();
  const connect = app.context.connectAgentClient('claude');
  assert.equal(app.byId('settings-claude-connect').disabled, true);
  assert.equal(app.byId('settings-claude-status').textContent, '연결 중');
  assert.equal(app.byId('settings-codex-connect').disabled, false);
  pending.resolve({ ...info, clients: [{ id: 'claude', state: 'waiting' }, info.clients[1]] });
  await connect;
  assert.notEqual(app.byId('settings-claude-status').textContent, '연결됨');
});

for (const client of ['claude', 'codex']) {
  test(`${client} exposes an update action and preserves native recovery detail`, async () => {
    const detail = '새 연결 도구를 설치해 주세요.';
    const app = harness({ load: async () => ({ ...info, clients: info.clients.map(entry => entry.id === client ? { ...entry, state: 'update_required', detail } : entry) }) });
    await app.context.loadAgentConnection();
    assert.equal(app.byId(`settings-${client}-status`).textContent, '업데이트 필요');
    assert.equal(app.byId(`settings-${client}-status`).dataset.state, 'update_required');
    assert.equal(app.byId(`settings-${client}-connect`).textContent, '업데이트');
    assert.equal(app.byId(`settings-${client}-connect`).getAttribute('aria-label'), `${client === 'claude' ? 'Claude' : 'Codex'} 업데이트`);
    assert.equal(app.byId(`settings-${client}-connect`).disabled, false);
    assert.equal(app.byId(`settings-${client}-connect`).hidden, false);
    assert.equal(app.byId(`settings-${client}-error`).textContent, detail);
    assert.equal(app.byId(`settings-${client}-error`).hidden, false);
    assert.equal(app.byId(`settings-${client}-error`).className, 'settings-client-note');
    assert.equal(app.byId(`settings-${client}-error`).getAttribute('role'), 'status');
    const other = client === 'claude' ? 'codex' : 'claude';
    assert.equal(app.byId(`settings-${other}-status`).textContent, '연결 안 됨');
    assert.equal(app.byId(`settings-${other}-connect`).textContent, '연결');
  });
}

for (const [result, label, action] of [
  ['update_required', '업데이트 필요', '업데이트'],
  ['waiting', '연결 대기', '다시 열기'],
  ['connected', '연결됨', '연결'],
]) {
  test(`update completion uses native ${result} evidence without inventing success`, async () => {
    const pending = deferred();
    let current = { ...info, clients: [info.clients[0], { ...info.clients[1], state: 'update_required' }] };
    let requests = 0;
    const app = harness({ load: async () => current, connect: () => { requests += 1; return pending.promise; } });
    await app.context.loadAgentConnection();
    const updating = app.context.connectAgentClient('codex');
    await app.context.connectAgentClient('codex');
    assert.equal(requests, 1);
    assert.equal(app.byId('settings-codex-status').textContent, '업데이트 중');
    assert.equal(app.byId('settings-codex-status').dataset.state, 'updating');
    assert.equal(app.byId('settings-codex-connect').textContent, '업데이트 중');
    assert.equal(app.byId('settings-codex-connect').getAttribute('aria-label'), 'Codex 업데이트 중');
    assert.equal(app.byId('settings-codex-connect').disabled, true);
    assert.equal(app.byId('settings-codex-spinner').hidden, false);
    assert.equal(app.byId('settings-claude-status').textContent, '연결 안 됨');
    assert.equal(app.byId('settings-claude-connect').disabled, false);
    assert.equal(app.byId('settings-claude-spinner').hidden, true);
    current = { ...current, clients: [current.clients[0], { ...current.clients[1], state: 'waiting' }] };
    await app.context.loadAgentConnection(true);
    assert.equal(app.byId('settings-codex-status').textContent, '업데이트 중');
    assert.equal(app.byId('settings-codex-connect').textContent, '업데이트 중');
    current = { ...current, clients: [current.clients[0], { ...current.clients[1], state: result }] };
    pending.resolve(current);
    await updating;
    assert.equal(app.byId('settings-codex-status').textContent, label);
    assert.equal(app.byId('settings-codex-status').dataset.state, result);
    assert.equal(app.byId('settings-codex-connect').textContent, action);
    assert.equal(app.byId('settings-codex-connect').hidden, result === 'connected');
    assert.equal(app.byId('settings-codex-connect').disabled, result === 'connected');
    assert.equal(app.byId('settings-codex-spinner').hidden, true);
  });
}

for (const client of ['claude', 'codex']) {
  test(`${client} metadata errors offer a read-only check without disabling the other client`, async () => {
    const detail = '연결 상태를 읽지 못했습니다. 잠시 후 다시 확인해 주세요.';
    const current = { ...info, clients: info.clients.map(entry => entry.id === client ? { ...entry, state: 'error', detail } : entry) };
    const app = harness({ load: async () => current });
    await app.context.loadAgentConnection();
    assert.equal(app.byId(`settings-${client}-status`).textContent, '확인 불가');
    assert.equal(app.byId(`settings-${client}-status`).dataset.state, 'error');
    assert.equal(app.byId(`settings-${client}-connect`).textContent, '다시 확인');
    assert.equal(app.byId(`settings-${client}-connect`).getAttribute('aria-label'), `${client === 'claude' ? 'Claude' : 'Codex'} 다시 확인`);
    assert.equal(app.byId(`settings-${client}-connect`).disabled, false);
    assert.equal(app.byId(`settings-${client}-error`).textContent, detail);
    assert.equal(app.byId(`settings-${client}-error`).hidden, false);
    assert.equal(app.byId(`settings-${client}-error`).className, 'notice');
    assert.equal(app.byId(`settings-${client}-error`).getAttribute('role'), 'alert');
    const other = client === 'claude' ? 'codex' : 'claude';
    assert.equal(app.byId(`settings-${other}-status`).textContent, '연결 안 됨');
    assert.equal(app.byId(`settings-${other}-connect`).disabled, false);
  });
}

for (const [result, label, action] of [
  ['error', '확인 불가', '다시 확인'],
  ['update_required', '업데이트 필요', '업데이트'],
  ['waiting', '연결 대기', '다시 열기'],
  ['connected', '연결됨', '연결'],
]) {
  test(`metadata recheck uses only one read and retains native ${result} state`, async () => {
    const pending = deferred();
    const initialInfo = { ...info, clients: [info.clients[0], { ...info.clients[1], state: 'error' }] };
    let reads = 0, installs = 0;
    const app = harness({ load: () => ++reads === 1 ? Promise.resolve(initialInfo) : pending.promise, connect: async () => { installs += 1; return info; } });
    await app.context.loadAgentConnection();
    const checking = app.context.connectAgentClient('codex');
    await app.context.connectAgentClient('codex');
    assert.equal(reads, 2);
    assert.equal(installs, 0);
    assert.equal(app.byId('settings-codex-status').textContent, '확인 중');
    assert.equal(app.byId('settings-codex-status').dataset.state, 'checking');
    assert.equal(app.byId('settings-codex-connect').textContent, '확인 중');
    assert.equal(app.byId('settings-codex-connect').disabled, true);
    assert.equal(app.byId('settings-codex-spinner').hidden, false);
    assert.equal(app.byId('settings-claude-status').textContent, '연결 안 됨');
    assert.equal(app.byId('settings-claude-connect').disabled, false);
    assert.equal(app.byId('settings-claude-spinner').hidden, true);
    const detail = result === 'error' ? '연결 상태를 읽지 못했습니다. 다시 확인해 주세요.' : '';
    pending.resolve({ ...info, clients: [info.clients[0], { ...info.clients[1], state: result, detail }] });
    await checking;
    assert.equal(reads, 2);
    assert.equal(installs, 0);
    assert.equal(app.byId('settings-codex-status').textContent, label);
    assert.equal(app.byId('settings-codex-status').dataset.state, result);
    assert.equal(app.byId('settings-codex-connect').textContent, action);
    assert.equal(app.byId('settings-codex-connect').hidden, result === 'connected');
    assert.equal(app.byId('settings-codex-connect').disabled, result === 'connected');
    assert.equal(app.byId('settings-codex-spinner').hidden, true);
    assert.equal(app.byId('settings-codex-error').textContent, detail);
  });
}

test('metadata recheck cannot downgrade the other client after its connection completes', async () => {
  const metadata = deferred(), installation = deferred();
  let current = { ...info, clients: [{ ...info.clients[0], state: 'error' }, info.clients[1]] };
  let reads = 0;
  const installs = [];
  const app = harness({ load: () => ++reads === 2 ? metadata.promise : Promise.resolve(current), connect: client => { installs.push(client); return installation.promise; } });
  await app.context.loadAgentConnection();
  const checking = app.context.connectAgentClient('claude');
  const connecting = app.context.connectAgentClient('codex');
  assert.deepEqual(installs, ['codex']);
  assert.equal(app.byId('settings-claude-status').textContent, '확인 중');
  assert.equal(app.byId('settings-codex-status').textContent, '연결 중');
  current = { ...current, clients: [current.clients[0], { ...current.clients[1], state: 'connected' }] };
  installation.resolve(current);
  await connecting;
  assert.equal(app.byId('settings-codex-status').textContent, '연결됨');
  metadata.resolve({ ...info, clients: [{ ...info.clients[0], state: 'not_connected' }, info.clients[1]] });
  await checking;
  assert.equal(app.byId('settings-claude-status').textContent, '연결 안 됨');
  assert.equal(app.byId('settings-codex-status').textContent, '연결됨');
});

test('metadata recheck invalidates an older polling response', async () => {
  const old = deferred();
  const initialInfo = { ...info, clients: [{ ...info.clients[0], state: 'error' }, info.clients[1]] };
  const checked = { ...info, clients: [{ ...info.clients[0], state: 'update_required' }, info.clients[1]] };
  let reads = 0, installs = 0;
  const app = harness({ load: () => { reads += 1; return reads === 1 ? Promise.resolve(initialInfo) : reads === 2 ? old.promise : Promise.resolve(checked); }, connect: async () => { installs += 1; return info; } });
  await app.context.loadAgentConnection();
  const polling = app.context.loadAgentConnection(true);
  await app.context.connectAgentClient('claude');
  old.resolve(initialInfo);
  await polling;
  assert.equal(installs, 0);
  assert.equal(app.byId('settings-claude-status').dataset.state, 'update_required');
  assert.equal(app.byId('settings-claude-connect').textContent, '업데이트');
});

test('metadata recheck keeps pending ownership across reopening and rejects its stale result', async () => {
  const pending = deferred();
  const initialInfo = { ...info, clients: [{ ...info.clients[0], state: 'error' }, info.clients[1]] };
  let reads = 0, installs = 0;
  const app = harness({ load: () => ++reads === 2 ? pending.promise : Promise.resolve(initialInfo), connect: async () => { installs += 1; return info; } });
  await app.context.loadAgentConnection();
  const checking = app.context.connectAgentClient('claude');
  app.byId('settings-dialog').open = false;
  app.context.resetAgentConnection();
  app.byId('settings-dialog').open = true;
  await app.context.loadAgentConnection();
  await app.context.connectAgentClient('claude');
  assert.equal(reads, 3);
  assert.equal(installs, 0);
  assert.equal(app.byId('settings-claude-status').textContent, '확인 중');
  assert.equal(app.byId('settings-claude-connect').disabled, true);
  pending.resolve({ ...info, clients: [{ ...info.clients[0], state: 'connected' }, info.clients[1]] });
  await checking;
  assert.equal(app.byId('settings-claude-status').textContent, '확인 불가');
  assert.equal(app.byId('settings-claude-connect').textContent, '다시 확인');
  assert.equal(installs, 0);
});

test('metadata query rejection preserves the error state and reports a bounded read failure', async () => {
  let reads = 0, installs = 0;
  const initialInfo = { ...info, clients: [{ ...info.clients[0], state: 'error' }, info.clients[1]] };
  const app = harness({ load: async () => { if (++reads === 1) return initialInfo; throw Error('/private/metadata'); }, connect: async () => { installs += 1; return info; } });
  await app.context.loadAgentConnection();
  await app.context.connectAgentClient('claude');
  assert.equal(installs, 0);
  assert.equal(app.byId('settings-claude-status').dataset.state, 'error');
  assert.equal(app.byId('settings-claude-connect').textContent, '다시 확인');
  assert.equal(app.byId('settings-claude-connect').disabled, false);
  assert.equal(app.byId('settings-claude-error').textContent, '확인하지 못했습니다. 다시 시도해 주세요.');
});

test('connection errors stay beside the affected client without leaking native messages', async () => {
  const app = harness({ connect: async () => { throw Error('/private/native/path'); } });
  await app.context.loadAgentConnection();
  await app.context.connectAgentClient('codex');
  assert.match(app.byId('settings-codex-error').textContent, /다시 시도/);
  assert.equal(app.byId('settings-claude-error').hidden, true);
  assert.equal(app.byId('settings-claude-connect').disabled, false);
});

test('refresh retains rows and stops when switching panels', async () => {
  const pending = deferred();
  let initial = true;
  const app = harness({ load: () => initial ? Promise.resolve(info) : pending.promise });
  await app.context.loadAgentConnection();
  initial = false;
  const refresh = app.context.loadAgentConnection(true);
  assert.equal(app.byId('settings-claude-status').textContent, '연결 안 됨');
  app.state.settingsPanel = 'appearance';
  await app.context.loadAgentConnection();
  pending.resolve({ ...info, clients: [{ id: 'claude', state: 'connected' }] });
  await refresh;
  assert.equal(app.agentConnection.info, null);
  assert.equal(app.timers.size, 0);
});

for (const initialState of ['not_connected', 'update_required']) {
test(`an older refresh cannot overwrite a successful ${initialState} request`, async () => {
  const pending = deferred();
  let initial = true;
  const initialInfo = { ...info, clients: [{ ...info.clients[0], state: initialState }, info.clients[1]] };
  const connected = { ...info, clients: [{ id: 'claude', state: 'connected' }, info.clients[1]] };
  const app = harness({ load: () => initial ? Promise.resolve(initialInfo) : pending.promise, connect: async () => connected });
  await app.context.loadAgentConnection();
  initial = false;
  const refresh = app.context.loadAgentConnection(true);
  await app.context.connectAgentClient('claude');
  pending.resolve(initialInfo);
  await refresh;
  assert.equal(app.byId('settings-claude-status').textContent, '연결됨');
  assert.equal(app.byId('settings-claude-connect').hidden, true);
  assert.equal(app.byId('settings-claude-status').dataset.state, 'connected');
});
}

test('confirmed connections have no redundant action and a lost connection restores it', async () => {
  let clientState = 'connected';
  const app = harness({ load: async () => ({ ...info, clients: [{ id: 'claude', state: clientState }, info.clients[1]] }) });
  await app.context.loadAgentConnection();
  assert.equal(app.byId('settings-claude-connect').hidden, true);
  assert.equal(app.byId('settings-codex-connect').hidden, false);
  clientState = 'waiting';
  await app.context.loadAgentConnection(true);
  assert.equal(app.byId('settings-claude-connect').hidden, false);
  assert.equal(app.byId('settings-claude-connect').disabled, false);
  assert.equal(app.byId('settings-claude-connect').textContent, '다시 열기');
  assert.equal(app.byId('settings-claude-status').dataset.state, 'waiting');
});


for (const initialState of ['not_connected', 'update_required']) {
test(`closing and reopening a ${initialState} request preserves ownership and rejects stale replies`, async () => {
  const pending = deferred();
  let connects = 0;
  const initialInfo = { ...info, clients: [{ ...info.clients[0], state: initialState }, info.clients[1]] };
  const app = harness({ load: async () => initialInfo, connect: () => { connects += 1; return pending.promise; } });
  await app.context.loadAgentConnection();
  const connecting = app.context.connectAgentClient('claude');
  app.byId('settings-dialog').open = false;
  app.context.resetAgentConnection();
  app.byId('settings-dialog').open = true;
  await app.context.loadAgentConnection();
  await app.context.connectAgentClient('claude');
  assert.equal(connects, 1);
  assert.equal(app.byId('settings-claude-connect').disabled, true);
  assert.equal(app.byId('settings-claude-status').textContent, initialState === 'update_required' ? '업데이트 중' : '연결 중');
  pending.resolve({ ...info, clients: [{ id: 'claude', state: 'connected' }] });
  await connecting;
  assert.notEqual(app.byId('settings-claude-status').textContent, '연결됨');
  assert.equal(app.byId('settings-claude-status').dataset.state, initialState);
});
}


test('waiting describes an unconfirmed live connection without assuming approval is pending', async () => {
  const app = harness({ load: async () => ({ ...info, clients: [{ id: 'claude', state: 'waiting', detail: 'Claude에서 Nero 연결을 확인해 주세요.' }, info.clients[1]] }) });
  await app.context.loadAgentConnection();
  assert.equal(app.byId('settings-claude-status').textContent, '연결 대기');
  assert.equal(app.byId('settings-claude-error').textContent, 'Claude에서 Nero 연결을 확인해 주세요.');
  assert.equal(app.byId('settings-claude-connect').disabled, false);
});


test('connection errors preserve bounded native recovery messages and discard unexpected details', async () => {
  for (const [error, expected] of [
    ['앱을 열지 못했습니다. 설치 상태를 확인해 주세요.', '앱을 열지 못했습니다. 설치 상태를 확인해 주세요.'],
    [new Error('앱을 열지 못했습니다. 설치 상태를 확인해 주세요.'), '앱을 열지 못했습니다. 설치 상태를 확인해 주세요.'],
    ['bad path /private/example', '연결하지 못했습니다. 다시 시도해 주세요.'],
    ['오류\n/private/example', '연결하지 못했습니다. 다시 시도해 주세요.'],
  ]) {
    const app = harness({ connect: async () => { throw error; } });
    await app.context.loadAgentConnection();
    await app.context.connectAgentClient('claude');
    assert.equal(app.byId('settings-claude-error').textContent, expected);
  }
});


test('client preparation has a visible activity indicator without affecting the other row', async () => {
  const pending = deferred();
  const app = harness({ connect: () => pending.promise });
  await app.context.loadAgentConnection();
  const connecting = app.context.connectAgentClient('claude');
  assert.equal(app.byId('settings-claude-spinner').hidden, false);
  assert.equal(app.byId('settings-codex-spinner').hidden, true);
  pending.resolve(info);
  await connecting;
  assert.equal(app.byId('settings-claude-spinner').hidden, true);
});


test('manual connection opens instructions with the current information', async () => {
  const app = harness();
  await app.context.loadAgentConnection();
  app.context.openAgentConnectionPrompt();
  assert.equal(app.dialog.opened[0], info);
  assert.equal(app.dialog.triggers[0], app.byId('settings-agent-manual'));
  assert.equal(app.copied.length, 0);
});

test('manual connection instructions remain available when native information cannot load', async () => {
  const app = harness({ load: async () => { throw Error('native unavailable'); } });
  await app.context.loadAgentConnection();
  app.context.openAgentConnectionPrompt();
  assert.equal(app.dialog.opened.length, 1);
  assert.equal(app.dialog.opened[0], null);
  assert.equal(app.dialog.triggers[0], app.byId('settings-agent-manual'));
  await app.context.copyAgentConnection('config');
  assert.equal(app.copied.length, 0);
});

test('manual connection obeys parent ownership and closes when leaving settings', async () => {
  const app = harness();
  app.state.settingsPanel = 'appearance';
  app.context.openAgentConnectionPrompt();
  assert.equal(app.dialog.opened.length, 0);
  app.state.settingsPanel = 'agent';
  app.context.openAgentConnectionPrompt();
  app.context.resetAgentConnection();
  assert.equal(app.dialog.closed.at(-1).restoreFocus, false);
});
