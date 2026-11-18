import assert from 'node:assert/strict';
import test from 'node:test';
import { mountStorageSettings } from '../src/storage-settings.js';

const usage = (extra = {}) => ({ meetingsBytes: 9_800_000_000, processingBytes: 1_700_000_000, modelsBytes: 10_600_000_000, reducibleBytes: 9_000_000_000, reducibleMeetings: 9, heldMeetings: 0, ...extra });
function harness({ read = async () => usage(), compact = async () => ({ freedBytes: 9_000_000_000, meetings: 9, failed: 0, usage: usage({ reducibleBytes: 0, reducibleMeetings: 0 }) }), confirm = async () => true } = {}) {
  const nodes = new Map();
  const byId = id => {
    if (!nodes.has(id)) nodes.set(id, { hidden: false, disabled: false, textContent: '', addEventListener() {} });
    return nodes.get(id);
  };
  const calls = { read: 0, compact: 0, confirm: [] };
  const settings = mountStorageSettings({ byId,
    bridge: { storageUsage: () => { calls.read += 1; return read(); }, compactRecordings: () => { calls.compact += 1; return compact(); } },
    confirm: options => { calls.confirm.push(options); return confirm(options); } });
  return { byId, settings, calls };
}

test('reads usage only while the recording panel is open and offers compaction when something can shrink', async () => {
  const app = harness();
  assert.equal(app.calls.read, 0);
  await app.settings.setActive(true);
  assert.equal(app.calls.read, 1);
  assert.equal(app.byId('storage-meetings').textContent, '9.80 GB');
  assert.equal(app.byId('storage-models').textContent, '10.60 GB');
  assert.equal(app.byId('storage-compact-row').hidden, false);
  assert.match(app.byId('storage-compact-hint').textContent, /녹음 9개에서 약 9\.00 GB/);
  app.settings.setActive(true);
  assert.equal(app.calls.read, 1);
});

test('hides compaction when nothing can shrink and names recordings held by unsettled work', async () => {
  const none = harness({ read: async () => usage({ reducibleBytes: 0, reducibleMeetings: 0 }) });
  await none.settings.setActive(true);
  assert.equal(none.byId('storage-compact-row').hidden, true);
  await none.settings.compact();
  assert.equal(none.calls.confirm.length, 0);
  const held = harness({ read: async () => usage({ heldMeetings: 2 }) });
  await held.settings.setActive(true);
  assert.match(held.byId('storage-compact-hint').textContent, /녹음 2개는 제외/);
});

test('compacts only after a destructive confirmation and reports what was freed', async () => {
  const declined = harness({ confirm: async () => false });
  await declined.settings.setActive(true);
  await declined.settings.compact();
  assert.equal(declined.calls.compact, 0);
  assert.equal(declined.calls.confirm[0].destructive, true);
  const app = harness();
  await app.settings.setActive(true);
  await app.settings.compact();
  assert.equal(app.calls.compact, 1);
  assert.equal(app.byId('storage-compact-row').hidden, true);
  assert.equal(app.byId('storage-message').textContent, '9.00 GB를 확보했습니다.');
  assert.equal(app.byId('storage-compact').disabled, false);
});

test('a failed or partial compaction says the original files are kept', async () => {
  const partial = harness({ compact: async () => ({ freedBytes: 1_000_000_000, meetings: 1, failed: 2, usage: usage({ reducibleMeetings: 2 }) }) });
  await partial.settings.setActive(true);
  await partial.settings.compact();
  assert.match(partial.byId('storage-message').textContent, /1\.00 GB를 확보했습니다\. 녹음 2개는 줄이지 못했습니다/);
  const failed = harness({ compact: async () => { throw new Error('busy'); } });
  await failed.settings.setActive(true);
  await failed.settings.compact();
  assert.match(failed.byId('storage-message').textContent, /원래 파일은 그대로/);
  const unread = harness({ read: async () => { throw new Error('unavailable'); } });
  await unread.settings.setActive(true);
  assert.equal(unread.byId('storage-message').hidden, false);
  assert.equal(unread.byId('storage-compact-row').hidden, true);
});
