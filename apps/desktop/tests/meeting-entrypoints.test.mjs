import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { build } from 'esbuild';
import { Window } from 'happy-dom';
import { fileURLToPath } from 'node:url';

const app = new URL('../', import.meta.url);
const html = await readFile(new URL('src/index.html', app), 'utf8');
const main = await readFile(new URL('src/main.js', app), 'utf8');
const sources = await Promise.all(['bridge.js', 'calendar-bridge.js'].map(name => readFile(new URL(`src/${name}`, app), 'utf8')));
const names = [...new Set(sources.flatMap(source => [...source.matchAll(/^export (?:async )?(?:const|function) (\w+)/gm)].map(match => match[1])))];
const bundle = await build({
  entryPoints: [fileURLToPath(new URL('src/main.js', app))], bundle: true, write: false, format: 'iife', jsx: 'automatic',
  nodePaths: [fileURLToPath(new URL('node_modules', app))],
  define: { 'process.env.NODE_ENV': '"production"' }, loader: { '.css': 'empty' },
  plugins: [{ name: 'native-fixture', setup(builder) {
    builder.onResolve({ filter: /^\.\/bridge\.js$/ }, () => ({ path: 'bridge', namespace: 'fixture' }));
    builder.onLoad({ filter: /.*/, namespace: 'fixture' }, () => ({ contents: names.map(name => `export const ${name} = (...args) => globalThis.testBridge.${name}(...args);`).join('\n') }));
    builder.onLoad({ filter: /[\\/]src[\\/]main\.js$/ }, () => ({ contents: `${main.match(/^import .*;$/gm).join('\n')}\nglobalThis.neroBoot=(async()=>{${main.replace(/^import .*;\r?\n/gm, '')}})();`, loader: 'js' }));
  } }],
});

async function fixture(t, { devices = [{ id: 'mic', name: 'Microphone', isDefault: true }], session = null, ready = true, startError = false, folder = false } = {}) {
  const window = new Window({ url: 'http://nero.test', settings: { enableJavaScriptEvaluation: true, suppressInsecureJavaScriptEnvironmentWarning: true, disableCSSFileLoading: true, disableJavaScriptFileLoading: true } });
  t.after(() => window.happyDOM.abort());
  window.document.write(html);
  for (const name of ['appearance', 'icons', 'components']) window.eval(await readFile(new URL(`../../design-system/${name}.js`, app), 'utf8'));
  let snapshot = { revision: 1, session, meetings: session ? [{ id: 'active', title: '기존 회의', notes: '', createdAt: 1, status: session.state }] : [], folders: folder ? [{ id: 'folder', name: '업무' }] : [] };
  const calls = [], subscriptions = {};
  const stubs = {
    readSnapshot: () => snapshot, listDevices: () => devices,
    processingStatus: () => ({ status: 'idle', supported: true, modelReady: ready, summarySupported: true, summaryReady: true }),
    setAutoRecorder() {}, readTerminologySettings: () => ({ available: false }), agentConnectionInfo: () => ({ available: false, clients: [] }),
    readCalendarSettings: () => ({ revision: 1, configured: false, connected: false, calendars: [], selectedCalendars: [] }),
    listTemplates: () => [], readTranscript: () => null, readSummary: () => null,
    createMeeting: (title, notes) => { const meeting = { id: 'new', title, notes, createdAt: 2, status: 'draft' }; snapshot = { ...snapshot, revision: snapshot.revision + 1, meetings: [...snapshot.meetings, meeting] }; return meeting; },
    moveMeetings: (ids, folderId) => { snapshot = { ...snapshot, revision: snapshot.revision + 1, meetings: snapshot.meetings.map(meeting => ids.includes(meeting.id) ? { ...meeting, folderId } : meeting) }; return snapshot; },
    startRecording: (meetingId, deviceId) => { assert.equal(meetingId, 'new'); assert.equal(deviceId, null); if (startError) throw new Error('마이크를 열지 못했습니다.'); snapshot = { ...snapshot, revision: snapshot.revision + 1, session: { id: 'capture', meetingId, state: 'recording', elapsedMs: 0 } }; return snapshot; },
  };
  window.testBridge = Object.fromEntries(names.map(name => [name, async (...args) => { calls.push({ name, args }); if (name.startsWith('subscribe')) { subscriptions[name] = args[0]; return () => {}; } assert.ok(Object.hasOwn(stubs, name), `Unexpected native call: ${name}`); return stubs[name](...args); }]));
  window.eval(bundle.outputFiles[0].text);
  await window.neroBoot;
  const get = id => window.document.getElementById(id);
  const wait = async predicate => { for (let i = 0; i < 100; i++) { if (predicate()) return; await new Promise(resolve => setTimeout(resolve, 5)); } assert.fail(`UI timeout: ${get('error-message').textContent}`); };
  const count = name => calls.filter(call => call.name === name).length;
  return { window, get, wait, count, calls, subscriptions, snapshot: () => snapshot };
}

test('list entry points remain inside the library with a collapsed sidebar; create focuses the title', async t => {
  const f = await fixture(t);
  assert.equal(f.get('library-create').closest('#library-screen'), f.get('library-screen'));
  assert.equal(f.get('meeting-list').closest('#library-screen'), f.get('library-screen'));
  assert.match(f.get('meeting-list').textContent, /첫 회의를 시작/);
  f.get('library-create').click();
  await f.wait(() => f.window.document.activeElement.id === 'meeting-title');
  assert.equal(f.count('createMeeting'), 1);
  assert.equal(f.count('startRecording'), 0);
});

test('quick record creates once, starts through native boundary, and focuses recording controls', async t => {
  const f = await fixture(t);
  f.get('library-record').click(); f.get('library-record').click();
  await f.wait(() => f.count('startRecording') === 1 && !f.get('library-record').disabled);
  assert.equal(f.count('createMeeting'), 1);
  assert.equal(f.snapshot().session.meetingId, 'new');
  assert.match(f.get('library-record').textContent, /녹음으로 돌아가기/);
  assert.equal(f.window.document.activeElement.closest('#recording-controls'), f.get('recording-controls'));
});

for (const phase of ['recording', 'paused', 'finalizing']) test(`quick record returns to an existing ${phase} session without creating a meeting`, async t => {
  const f = await fixture(t, { session: { id: 'capture', meetingId: 'active', state: phase, elapsedMs: 1000 } });
  f.window.document.querySelector('[data-library-filter="all"]').click();
  await f.wait(() => !f.get('library-screen').hidden);
  f.get('library-record').click();
  await f.wait(() => f.get('library-screen').hidden && !f.get('library-record').disabled);
  assert.equal(f.get('meeting-title').value, '기존 회의');
  assert.equal(f.count('createMeeting'), 0); assert.equal(f.count('startRecording'), 0);
});

test('missing microphone opens recording settings without creating an empty meeting', async t => {
  const f = await fixture(t, { devices: [] });
  f.get('library-record').click();
  await f.wait(() => f.get('settings-dialog').open);
  assert.equal(f.get('settings-recording-tab').getAttribute('aria-selected'), 'true');
  assert.match(f.get('settings-message').textContent, /마이크/);
  assert.equal(f.count('createMeeting'), 0); assert.equal(f.count('startRecording'), 0);
});

test('missing speech models opens setup without creating a meeting or starting capture', async t => {
  const f = await fixture(t, { ready: false });
  f.get('library-record').click();
  await f.wait(() => !f.get('setup-screen').hidden && !f.get('library-record').disabled);
  assert.equal(f.count('createMeeting'), 0); assert.equal(f.count('startRecording'), 0);
});

test('native start failure leaves the created draft available with an actionable error', async t => {
  const f = await fixture(t, { startError: true });
  f.get('library-record').click();
  await f.wait(() => !f.get('error').hidden && !f.get('library-record').disabled);
  assert.match(f.get('error-message').textContent, /마이크/);
  assert.equal(f.get('meeting-title').value, '새 회의');
  assert.equal(f.snapshot().session, null);
});

test('quick record respects the current folder', async t => {
  const f = await fixture(t, { folder: true });
  f.window.document.querySelector('[data-library-filter="folder"]').click();
  await f.wait(() => f.get('library-title').textContent === '업무');
  f.get('library-record').click();
  await f.wait(() => f.count('startRecording') === 1 && !f.get('library-record').disabled);
  assert.equal(f.snapshot().meetings[0].folderId, 'folder');
});
