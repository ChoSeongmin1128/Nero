import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';

const source = await readFile(new URL('../src/main.js', import.meta.url), 'utf8');
const functions = source.slice(source.indexOf('function renderAgentConnection()'), source.indexOf('function renderSettings()'));
const deferred = () => { let resolve; const promise = new Promise(done => { resolve = done; }); return { promise, resolve }; };
const info = { available: true, executablePath: '/Example App/nero', config: { mcpServers: { nero: { command: '/Example App/nero', args: ['mcp', '--data-dir', '/Example Library'] } } }, cliExample: 'nero meetings list', clients: [{ id: 'claude', state: 'not_connected' }, { id: 'codex', state: 'not_connected' }] };

function harness({ load = async () => info, write = async () => {}, connect = async () => info } = {}) {
  const nodes = new Map();
  const byId = id => {
    if (!nodes.has(id)) nodes.set(id, { textContent: '', dataset: {}, hidden: false, disabled: false, open: id === 'settings-dialog', focus() { throw Error('Unexpected focus'); } });
    return nodes.get(id);
  };
  const state = { settingsPanel: 'agent' };
  const agentConnection = { info: null, loading: false, error: '', request: 0, refresh: 0, timer: null, pending: {}, errors: {} };
  let calls = 0;
  const copied = [], timers = new Map();
  const context = vm.createContext({ byId, state, agentConnection, bridge: { agentConnectionInfo: () => { calls += 1; return load(); }, connectAgentClient: connect },
    navigator: { clipboard: { writeText: async text => { copied.push(text); await write(text); } } },
    copyFeedbackTimers: new Map(), setTimeout: callback => { const key = {}; timers.set(key, callback); return key; }, clearTimeout: key => timers.delete(key),
  });
  vm.runInContext(functions, context);
  return { byId, state, agentConnection, copied, timers, context, get calls() { return calls; } };
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
  await app.context.copyAgentConnection('cli');
  assert.equal(app.copied[1], info.cliExample);
  const failed = harness({ write: async () => { throw Error('clipboard denied'); } });
  await failed.context.loadAgentConnection();
  await failed.context.copyAgentConnection('config');
  assert.match(failed.byId('settings-agent-feedback').textContent, /복사하지 못했습니다/);
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

test('an older refresh cannot overwrite a successful connection', async () => {
  const pending = deferred();
  let initial = true;
  const connected = { ...info, clients: [{ id: 'claude', state: 'connected' }, info.clients[1]] };
  const app = harness({ load: () => initial ? Promise.resolve(info) : pending.promise, connect: async () => connected });
  await app.context.loadAgentConnection();
  initial = false;
  const refresh = app.context.loadAgentConnection(true);
  await app.context.connectAgentClient('claude');
  pending.resolve(info);
  await refresh;
  assert.equal(app.byId('settings-claude-status').textContent, '연결됨');
  assert.equal(app.byId('settings-claude-connect').hidden, true);
  assert.equal(app.byId('settings-claude-status').dataset.state, 'connected');
});

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


test('closing and reopening during connection preserves pending ownership and rejects stale replies', async () => {
  const pending = deferred();
  let connects = 0;
  const app = harness({ connect: () => { connects += 1; return pending.promise; } });
  await app.context.loadAgentConnection();
  const connecting = app.context.connectAgentClient('claude');
  app.byId('settings-dialog').open = false;
  app.context.resetAgentConnection();
  app.byId('settings-dialog').open = true;
  await app.context.loadAgentConnection();
  await app.context.connectAgentClient('claude');
  assert.equal(connects, 1);
  assert.equal(app.byId('settings-claude-connect').disabled, true);
  pending.resolve({ ...info, clients: [{ id: 'claude', state: 'connected' }] });
  await connecting;
  assert.notEqual(app.byId('settings-claude-status').textContent, '연결됨');
});


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
