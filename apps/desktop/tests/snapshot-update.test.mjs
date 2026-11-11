import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { snapshotSignatures, compareSnapshotSignatures } from '../src/snapshot-update.js';

const tauriStub = 'export const isTauri = () => true; export const convertFileSrc = value => value; export const invoke = (...args) => globalThis.testInvoke(...args); export const listen = async (name, handler) => { globalThis.testListeners[name] = handler; return () => {}; };';
const bridgeBundle = await build({ entryPoints: [fileURLToPath(new URL('../src/bridge.js', import.meta.url))], bundle: true, write: false, format: 'esm', plugins: [{ name: 'tauri-stub', setup(builder) {
  builder.onResolve({ filter: /^@tauri-apps\/api\// }, args => ({ path: args.path, namespace: 'tauri-stub' }));
  builder.onLoad({ filter: /.*/, namespace: 'tauri-stub' }, () => ({ contents: tauriStub }));
} }] });

test('recording ticks update the session of the last full snapshot', async () => {
  globalThis.testListeners = {};
  globalThis.testInvoke = async () => { throw new Error('unexpected native call'); };
  const bridge = await import(`data:text/javascript;base64,${Buffer.from(bridgeBundle.outputFiles[0].text).toString('base64')}`);
  const received = [];
  await bridge.subscribeSnapshots(snapshot => received.push(snapshot));
  const tick = globalThis.testListeners['nero://snapshot-session'];
  const session = { id: 'capture', meetingId: 'a', state: 'recording', elapsedMs: 1000, savedFrames: 16000 };
  tick({ payload: { revision: 1, session } });
  assert.equal(received.length, 0);
  const full = { revision: 2, meetings: [{ id: 'a', title: 'Meeting', notes: 'Notes' }], folders: [], session };
  globalThis.testListeners['nero://snapshot']({ payload: full });
  tick({ payload: { revision: 3, session: { ...session, elapsedMs: 1750 } } });
  assert.deepEqual(received.at(-1), { ...full, revision: 3, session: { ...session, elapsedMs: 1750 } });
  assert.equal(compareSnapshotSignatures(snapshotSignatures(received[0]), snapshotSignatures(received.at(-1))).timerOnly, true);
  tick({ payload: { revision: 2, session: { ...session, elapsedMs: 9999 } } });
  tick({ payload: { revision: 4, session: null } });
  assert.deepEqual(received.at(-1), { ...full, revision: 4, session: null });
  assert.equal(received.length, 3);
  delete globalThis.testListeners; delete globalThis.testInvoke;
});

const snapshot = () => ({
  meetings: [{ id: 'a', title: 'Meeting', notes: 'Notes', attendees: [{ id: 'person', name: 'Person' }], folderId: 'folder', deletedAt: null, audio: { frames: 10 }, captureQuality: { xrunCount: 0 }, calendarReference: { title: 'Event' } }],
  folders: [{ id: 'folder', name: 'Folder' }],
  session: { id: 'capture', meetingId: 'a', state: 'recording', elapsedMs: 1000, savedFrames: 16000, warning: '' },
});

test('only the recording clock and saved frame count bypass the full render', () => {
  const initial = snapshot(), signatures = snapshotSignatures(initial);
  assert.deepEqual(compareSnapshotSignatures(null, signatures), { changed: true, timerOnly: false });
  assert.deepEqual(compareSnapshotSignatures(signatures, snapshotSignatures(structuredClone(initial))), { changed: false, timerOnly: true });
  initial.session.elapsedMs += 1000;
  initial.session.savedFrames += 16000;
  assert.deepEqual(compareSnapshotSignatures(signatures, snapshotSignatures(initial)), { changed: true, timerOnly: true });
});

test('meeting, folder and non-clock session changes remain visible', () => {
  const initial = snapshot(), signatures = snapshotSignatures(initial);
  const edits = [
    value => { value.meetings[0].title = 'Updated'; },
    value => { value.meetings[0].notes += ' updated'; },
    value => { value.meetings[0].attendees[0].name = 'Updated'; },
    value => { value.meetings[0].folderId = 'another'; },
    value => { value.meetings[0].deletedAt = 1; },
    value => { value.meetings[0].audio.frames++; },
    value => { value.meetings[0].captureQuality.xrunCount++; },
    value => { value.meetings[0].calendarReference.title = 'Updated'; },
    value => { value.meetings[0].futureField = { value: 'kept' }; },
    value => { value.meetings = []; },
    value => { value.folders[0].name = 'Updated'; },
    value => { value.session.state = 'paused'; },
    value => { value.session.warning = 'Device interrupted'; },
    value => { value.session = null; },
  ];
  for (const edit of edits) {
    const next = structuredClone(initial); edit(next);
    assert.deepEqual(compareSnapshotSignatures(signatures, snapshotSignatures(next)), { changed: true, timerOnly: false });
  }
});

test('empty folders and absent sessions retain their existing normalized identity', () => {
  const initial = snapshotSignatures({ meetings: [] });
  assert.deepEqual(compareSnapshotSignatures(initial, snapshotSignatures({ revision: 9, meetings: [], folders: [], session: null })), { changed: false, timerOnly: true });
});

// Tauri delivers an event to a listener registered without a target whatever window it
// was emitted to (`match_any_or_filter`), so each window must listen to its own event name.
test('each window keeps only the snapshot published for it', async () => {
  const main = await readFile(new URL('../src-tauri/src/main.rs', import.meta.url), 'utf8');
  const publish = main.slice(main.indexOf('fn publish('), main.indexOf('\n}\n', main.indexOf('fn publish(')));
  const emits = [...publish.matchAll(/emit_to\(\s*"([^"]+)",\s*"([^"]+)"/g)].map(([, label, event]) => ({ label, event }));
  assert.deepEqual(emits.map(emit => emit.label), ['main', 'recorder']);
  const stub = 'export const isTauri = () => true; export const convertFileSrc = value => value; export const invoke = async () => { throw new Error("unexpected native call"); }; export const listen = async (name, handler, options) => globalThis.testListen(name, handler, options);';
  const bundle = await build({ entryPoints: [fileURLToPath(new URL('../src/bridge.js', import.meta.url))], bundle: true, write: false, format: 'esm', plugins: [{ name: 'tauri-targets', setup(builder) {
    builder.onResolve({ filter: /^@tauri-apps\/api\// }, args => ({ path: args.path, namespace: 'tauri-targets' }));
    builder.onLoad({ filter: /.*/, namespace: 'tauri-targets' }, () => ({ contents: stub }));
  } }] });
  const webviews = {};
  async function open(label, subscribe) {
    const listeners = webviews[label] = [];
    globalThis.testListen = async (name, handler, options) => { listeners.push({ name, handler, target: options?.target ?? { kind: 'Any' } }); return () => {}; };
    const bridge = await import(`data:text/javascript;base64,${Buffer.from(`${bundle.outputFiles[0].text}\n// ${label}`).toString('base64')}`);
    const received = [];
    await bridge[subscribe](snapshot => received.push(snapshot));
    return received;
  }
  const emitTo = (label, name, payload) => {
    for (const listeners of Object.values(webviews)) {
      for (const listener of listeners) {
        const targeted = listener.target.kind === 'Any' || listener.target.label === label;
        if (listener.name === name && targeted) listener.handler({ payload });
      }
    }
  };
  const mainReceived = await open('main', 'subscribeSnapshots');
  const recorderReceived = await open('recorder', 'subscribeRecorderSnapshots');
  delete globalThis.testListen;
  const full = { revision: 7, meetings: [{ id: 'a', title: 'A', notes: 'Private' }, { id: 'b', title: 'B', notes: '' }], folders: [{ id: 'f', name: 'Folder' }], session: null };
  const reduced = { revision: 7, meetings: [], folders: [], session: null };
  for (const { label, event } of emits) emitTo(label, event, label === 'main' ? full : reduced);
  assert.deepEqual(mainReceived.at(-1), full);
  assert.ok(mainReceived.every(snapshot => snapshot.meetings.length === 2));
  assert.deepEqual(recorderReceived, [reduced]);
});
