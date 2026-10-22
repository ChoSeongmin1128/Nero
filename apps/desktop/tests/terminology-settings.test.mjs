import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';

const source = await readFile(new URL('../src/main.js', import.meta.url), 'utf8');
const functions = source.slice(source.indexOf('function resetTerminologySettings()'), source.indexOf('function renderAgentConnection()'));
const deferred = () => { let resolve, reject; const promise = new Promise((done, fail) => { resolve = done; reject = fail; }); return { promise, resolve, reject }; };
function harness({ read = async () => ({ available: true, enabled: true }), save = async enabled => ({ available: true, enabled }) } = {}) {
  const nodes = new Map();
  const byId = id => {
    if (!nodes.has(id)) nodes.set(id, { hidden: false, checked: false, disabled: false, open: id === 'settings-dialog' });
    return nodes.get(id);
  };
  const state = { settingsPanel: 'recording' };
  const terminologySettings = { info: null, error: '', request: 0, saving: null };
  let reads = 0, running = false;
  const context = vm.createContext({ byId, state, terminologySettings, processingActive: () => running,
    bridge: { readTerminologySettings: () => { reads += 1; return read(); }, setTerminologyEnabled: save },
    renderSettings: () => context.renderTerminologySettings(),
  });
  vm.runInContext(functions, context);
  return { byId, state, terminologySettings, context, get reads() { return reads; }, set running(value) { running = value; } };
}

test('only reads in an open recording panel and hides unavailable profiles', async () => {
  const app = harness({ read: async () => ({ available: false, enabled: false }) });
  app.state.settingsPanel = 'appearance';
  await app.context.loadTerminologySettings();
  assert.equal(app.reads, 0);
  app.state.settingsPanel = 'recording';
  await app.context.loadTerminologySettings();
  assert.equal(app.byId('settings-terminology-row').hidden, true);
  app.byId('settings-dialog').open = false;
  await app.context.loadTerminologySettings();
  assert.equal(app.reads, 1);
});

test('late reads and errors cannot change a reopened panel', async () => {
  for (const reject of [false, true]) {
    const pending = deferred();
    let latest = false;
    const app = harness({ read: () => latest ? Promise.resolve({ available: true, enabled: false }) : pending.promise });
    const old = app.context.loadTerminologySettings();
    app.context.resetTerminologySettings();
    latest = true;
    await app.context.loadTerminologySettings();
    if (reject) pending.reject(Error('read failed')); else pending.resolve({ available: false, enabled: true });
    await old;
    assert.equal(app.byId('settings-terminology-row').hidden, false);
    assert.equal(app.byId('terminology-enabled').checked, false);
    assert.equal(app.terminologySettings.error, '');
  }
});

test('failed saves roll back and processing disables changes', async () => {
  const app = harness({ save: async () => { throw Error('write failed'); } });
  await app.context.loadTerminologySettings();
  app.byId('terminology-enabled').checked = false;
  await app.context.changeTerminologyEnabled();
  assert.equal(app.byId('terminology-enabled').checked, true);
  assert.match(app.terminologySettings.error, /저장하지 못했습니다/);
  app.running = true;
  app.context.renderTerminologySettings();
  assert.equal(app.byId('terminology-enabled').disabled, true);
});

test('reopening while a save is pending waits before reading current settings', async () => {
  const pending = deferred();
  let enabled = true;
  const app = harness({ read: async () => ({ available: true, enabled }), save: () => pending.promise });
  await app.context.loadTerminologySettings();
  app.byId('terminology-enabled').checked = false;
  const saving = app.context.changeTerminologyEnabled();
  assert.equal(app.byId('terminology-enabled').disabled, true);
  app.context.resetTerminologySettings();
  const reopening = app.context.loadTerminologySettings();
  assert.equal(app.reads, 1);
  enabled = false;
  pending.resolve({ available: true, enabled });
  await Promise.all([saving, reopening]);
  assert.equal(app.reads, 2);
  assert.equal(app.byId('terminology-enabled').checked, false);
  assert.equal(app.byId('terminology-enabled').disabled, false);
});

test('failed reads stay hidden with recoverable feedback', async () => {
  const app = harness({ read: async () => { throw Error('read failed'); } });
  await app.context.loadTerminologySettings();
  assert.equal(app.byId('settings-terminology-row').hidden, true);
  assert.match(app.terminologySettings.error, /불러오지 못했습니다/);
});
