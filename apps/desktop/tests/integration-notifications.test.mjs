import assert from 'node:assert/strict';
import test from 'node:test';
import { createIntegrationMonitor, integrationNotices } from '../src/integration-notifications.js';
const agents = (state = 'update_required', version = '28') => ({ available: true, clients: [{ id: 'codex', state, packageVersion: version }] });
const calendar = (changes = {}) => ({ revision: 2, configured: true, connected: false, accountLabel: 'Test account', error: 'Reconnect', ...changes });
const settle = () => new Promise(resolve => setImmediate(resolve));

test('only actionable installed client updates and confirmed calendar connection failures are projected', () => {
  assert.equal(integrationNotices({ agents: agents(), calendar: calendar() }).length, 2);
  for (const state of ['connected', 'not_connected', 'installed', 'checking', 'installation_unknown', 'restart_required', 'disabled', 'error']) {
    assert.deepEqual(integrationNotices({ agents: agents(state) }), []);
  }
  assert.deepEqual(integrationNotices({ agents: { ...agents(), available: false } }), []);
  for (const changes of [{ connected: true }, { connecting: true }, { restoring: true }, { configured: false }, { accountLabel: null }, { error: null }]) {
    assert.deepEqual(integrationNotices({ calendar: calendar(changes) }), []);
  }
});

test('repeated snapshots remain one notice while new target versions have a new identity', () => {
  const updates = [], monitor = createIntegrationMonitor({ bridge: {}, onChange: notices => updates.push(notices) });
  monitor.acceptAgents(agents()); monitor.acceptAgents(agents());
  assert.equal(updates.length, 1);
  monitor.acceptAgents(agents('update_required', '29'));
  assert.notEqual(updates[0][0].id, updates[1][0].id);
  monitor.acceptAgents(agents('connected')); assert.deepEqual(updates.at(-1), []);
});

test('a stale background response cannot restore an update after explicit connection completion', async () => {
  let resolve; const pending = new Promise(done => { resolve = done; });
  const updates = [], monitor = createIntegrationMonitor({ bridge: { agentConnectionInfo: () => pending, readCalendarSettings: async () => calendar({ connected: true }) }, onChange: value => updates.push(value) });
  const first = monitor.refresh(); assert.equal(monitor.refresh(), first);
  monitor.acceptAgents(agents('connected'));
  resolve(agents()); await first;
  assert.deepEqual(updates, []);
});

test('older calendar errors and transient request failures preserve the latest confirmed state', async () => {
  const updates = [], monitor = createIntegrationMonitor({ bridge: { agentConnectionInfo() { throw Error('Unavailable'); }, readCalendarSettings: async () => calendar({ revision: 1 }) }, onChange: value => updates.push(value) });
  monitor.acceptAgents(agents()); monitor.acceptCalendar(calendar({ connected: true, revision: 3 }));
  await monitor.refresh(); assert.equal(updates.length, 1); assert.equal(updates[0].length, 1);
  monitor.acceptCalendar(calendar({ revision: 2 })); assert.equal(updates.length, 1);
});

test('focus refresh is bounded, hidden views skip polling and disposal removes listeners and timers', async () => {
  const events = new Map(), docEvents = new Map(); let timer, calls = 0, cleared = false;
  const target = { addEventListener: (name, fn) => events.set(name, fn), removeEventListener: name => events.delete(name), setInterval: fn => { timer = fn; return 1; }, clearInterval: () => { cleared = true; } };
  const document = { visibilityState: 'hidden', addEventListener: (name, fn) => docEvents.set(name, fn), removeEventListener: name => docEvents.delete(name) };
  const monitor = createIntegrationMonitor({ bridge: { agentConnectionInfo: async () => { calls += 1; return agents(); }, readCalendarSettings: async () => null } });
  monitor.start(target, document); timer(); await settle(); assert.equal(calls, 0);
  document.visibilityState = 'visible'; events.get('focus')(); await settle(); assert.equal(calls, 1);
  monitor.dispose(); assert.ok(cleared); assert.equal(events.size, 0); assert.equal(docEvents.size, 0);
  await monitor.refresh(); assert.equal(calls, 1);
});
