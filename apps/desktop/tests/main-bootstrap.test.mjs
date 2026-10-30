import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { build } from 'esbuild';
import { Window } from 'happy-dom';

const root = fileURLToPath(new URL('../', import.meta.url));
const html = await readFile(new URL('../src/index.html', import.meta.url), 'utf8');
const main = await readFile(new URL('../src/main.js', import.meta.url), 'utf8');
const bridgeFiles = await Promise.all(['bridge.js', 'calendar-bridge.js'].map(name => readFile(new URL(`../src/${name}`, import.meta.url), 'utf8')));
const bridgeNames = [...new Set(bridgeFiles.flatMap(source => [...source.matchAll(/^export (?:async )?(?:const|function) (\w+)/gm)].map(match => match[1])))];
const bundled = await build({
  entryPoints: [root + 'src/main.js'], bundle: true, write: false, format: 'iife', jsx: 'automatic',
  nodePaths: [root + 'node_modules'], define: { 'process.env.NODE_ENV': '"production"' }, loader: { '.css': 'empty' },
  plugins: [{ name: 'test-native-boundary', setup(builder) {
    builder.onResolve({ filter: /^\.\/bridge\.js$/ }, () => ({ path: 'bridge', namespace: 'test-bridge' }));
    builder.onLoad({ filter: /.*/, namespace: 'test-bridge' }, () => ({ contents: bridgeNames.map(name => `export const ${name} = (...args) => globalThis.testBridge.${name}(...args);`).join('\n') }));
    builder.onLoad({ filter: /[\\/]src[\\/]main\.js$/ }, () => {
      const imports = main.match(/^import .*;$/gm).join('\n');
      return { contents: `${imports}\nglobalThis.neroBoot = (async () => {\n${main.replace(/^import .*;\r?\n/gm, '')}\n})();`, loader: 'js' };
    });
  } }],
});

test('recording completion reveals its draft, then saved text, failure recovery and the finished summary', async t => {
  const window = new Window({ url: 'http://nero.test', settings: { enableJavaScriptEvaluation: true, suppressInsecureJavaScriptEnvironmentWarning: true, disableCSSFileLoading: true, disableJavaScriptFileLoading: true } });
  t.after(() => window.happyDOM.abort());
  window.document.write(html);
  for (const name of ['appearance', 'icons', 'components']) window.eval(await readFile(new URL(`../../../design-system/${name}.js`, import.meta.url), 'utf8'));
  const errors = [];
  window.addEventListener('error', event => errors.push(event.error || event.message));
  const subscriptions = {};
  let snapshot = { revision: 1, folders: [], meetings: [{ id: 'one', title: '회의', notes: '', createdAt: 1, status: 'recording' }], session: { id: 'capture', meetingId: 'one', state: 'recording', elapsedMs: 1000 } };
  const ready = { supported: true, modelReady: true, summarySupported: true, summaryReady: true };
  let transcript = null, summary = null;
  const preview = { meetingId: 'one', sessionId: 'capture', generation: 1, words: [], text: '검토 내용을 담은 초안' };
  const stubs = {
    readSnapshot: () => snapshot, listDevices: () => [], setAutoRecorder() {},
    processingStatus: () => ({ ...ready, revision: 1, status: 'idle', liveTranscript: preview }),
    readTerminologySettings: () => ({ available: false }), agentConnectionInfo: () => ({ available: false, clients: [] }),
    readCalendarSettings: () => ({ revision: 1, configured: false, connected: false, calendars: [], selectedCalendars: [] }),
    listTemplates: () => [], readTranscript: id => id === 'one' ? transcript : null, readSummary: id => id === 'one' ? summary : null,
    audioUrl: () => 'asset://recording.wav',
  };
  window.testBridge = Object.fromEntries(bridgeNames.map(name => [name, async (...args) => {
    if (name.startsWith('subscribe')) { subscriptions[name] = args[0]; return () => {}; }
    assert.ok(Object.hasOwn(stubs, name), `Unexpected native call: ${name}`);
    return stubs[name](...args);
  }]));
  window.eval(bundled.outputFiles[0].text);
  await window.neroBoot;
  const el = id => window.document.getElementById(id);
  const wait = async predicate => {
    for (let attempt = 0; attempt < 100; attempt++) {
      if (predicate()) return;
      await new Promise(resolve => setTimeout(resolve, 5));
    }
    assert.fail('Missing result state: ' + el('error-message').textContent + ' / ' + el('summary-message').textContent);
  };
  await wait(() => el('meeting-detail').dataset.inlineNotes === 'true');
  snapshot = { ...snapshot, revision: 2, session: { ...snapshot.session, state: 'finalizing' } };
  subscriptions.subscribeSnapshots(snapshot);
  await wait(() => !el('meeting-result-preview').hidden);
  assert.equal(el('detail-inspector').hidden, true);
  assert.match(el('meeting-result-rows').textContent, /검토 내용을 담은 초안/);
  assert.ok(el('summary-message').querySelector('.activity-spinner'));
  snapshot = { ...snapshot, revision: 3, session: null, meetings: [{ ...snapshot.meetings[0], status: 'completed', audio: { sessionId: 'capture', relativePath: 'audio.wav', frames: 48000, sampleRate: 48000, channels: 1 } }] };
  subscriptions.subscribeSnapshots(snapshot);
  const recognizing = { ...ready, id: 'job', meetingId: 'one', kind: 'transcribe', status: 'running', phase: 'recognizing', message: '마지막 구간과 화자를 정리하고 있습니다.', retryable: true };
  await subscriptions.subscribeProcessing({ ...recognizing, revision: 2, liveTranscript: null });
  await wait(() => !el('meeting-result-preview').hidden);
  assert.equal(el('detail-inspector').hidden, true);
  assert.equal(el('meeting-result-title').textContent, '음성 기록');
  assert.match(el('meeting-result-rows').textContent, /검토 내용을 담은 초안/);
  assert.equal(el('meeting-result-draft').hidden, false);
  assert.equal(el('meeting-result-rows').querySelector('.transcript-preview-meta'), null);
  assert.ok(el('processing').querySelector('.activity-spinner'));
  assert.doesNotMatch(el('processing').textContent, /마지막 구간|화자를 정리/);
  transcript = { version: { id: 'transcript', revision: 1 }, speakers: [{ id: 'speaker', name: '화자 1' }], segments: [{ id: 'segment', speakerId: 'speaker', start: 1, end: 2, text: '확정된 검토 내용', originalText: '확정된 검토 내용' }] };
  await subscriptions.subscribeProcessing({ ...recognizing, revision: 3, kind: 'diarize', phase: 'diarizing' });
  await wait(() => el('meeting-result-rows').textContent.includes('확정된 검토 내용'));
  assert.equal(el('meeting-result-draft').hidden, true);
  assert.match(el('meeting-result-rows').textContent, /화자 1/);
  await subscriptions.subscribeProcessing({ ...recognizing, revision: 4, kind: 'diarize', status: 'completed' });
  await wait(() => !el('summarize').disabled);
  assert.equal(el('summary-message').textContent, '');
  el('meeting-result-open').click();
  await wait(() => !el('detail-inspector').hidden);
  assert.match(el('transcript-rows').textContent, /확정된 검토 내용/);
  assert.equal(el('detail-inspector-close').getAttribute('aria-label'), '본문으로 돌아가기');
  el('detail-inspector-close').click();
  await wait(() => el('detail-inspector').hidden);
  await subscriptions.subscribeProcessing({ ...ready, revision: 5, id: 'summary-job', kind: 'summary', meetingId: 'one', status: 'running' });
  assert.match(el('meeting-result-rows').textContent, /확정된 검토 내용/);
  await subscriptions.subscribeProcessing({ ...ready, revision: 6, id: 'summary-job', kind: 'summary', meetingId: 'one', status: 'failed', failureKind: 'engineUnavailable', retryable: true });
  assert.equal(el('retry-processing').hidden, false);
  assert.match(el('processing').textContent, /오류/);
  assert.match(el('meeting-result-rows').textContent, /확정된 검토 내용/);
  summary = { document: { id: 'summary', text: '## 결정 사항\n검토를 완료했습니다.', sourceVersion: transcript.version }, stale: false };
  await subscriptions.subscribeProcessing({ ...ready, revision: 7, id: 'summary-job', kind: 'summary', meetingId: 'one', status: 'completed' });
  await wait(() => el('summary-content').textContent.includes('검토를 완료했습니다.'));
  assert.equal(el('meeting-result-preview').hidden, true);
  assert.equal(el('meeting-result-title').textContent, '요약');
  assert.equal(el('summary-content').hidden, false);
  assert.deepEqual(errors, []);
});

test('the real application HTML boots and opens all settings tabs without stale element bindings', async t => {
  const window = new Window({ url: 'http://nero.test', settings: { enableJavaScriptEvaluation: true, suppressInsecureJavaScriptEnvironmentWarning: true, disableCSSFileLoading: true, disableJavaScriptFileLoading: true } });
  t.after(() => window.happyDOM.abort());
  const errors = [];
  window.addEventListener('error', event => errors.push(event.error || event.message));
  window.document.write(html);
  for (const name of ['appearance', 'icons', 'components']) window.eval(await readFile(new URL(`../../../design-system/${name}.js`, import.meta.url), 'utf8'));
  const calls = [];
  const subscriptions = {};
  const snapshot = { revision: 1, session: null, meetings: [], folders: [] };
  const stubs = {
    readSnapshot: () => snapshot, listDevices: () => [],
    processingStatus: () => ({ status: 'idle', supported: true, modelReady: true, summarySupported: true, summaryReady: true }),
    setAutoRecorder() {},
    readTerminologySettings: () => ({ available: false }),
    agentConnectionInfo: () => ({ available: false, clients: [] }),
    readCalendarSettings: () => ({ revision: 1, configured: false, connected: false, calendars: [], selectedCalendars: [] }),
    listTemplates: () => [],
    readTranscript: () => null, readSummary: () => null,
    showRecorder: id => assert.equal(id, 'capture'),
    acknowledgeProcessing: id => ({ id, status: 'acknowledged', jobs: [{ id, kind: 'transcribe', status: 'cancelled', retryable: false, acknowledged: true }] }),
    retryProcessing: id => ({ revision: 5, id: 'retry', kind: 'transcribe', status: 'queued', jobs: [{ id, kind: 'transcribe', status: 'cancelled', acknowledged: true }, { id: 'retry', kind: 'transcribe', status: 'queued' }] }),
  };
  window.testBridge = Object.fromEntries(bridgeNames.map(name => [name, async (...args) => {
    calls.push(name);
    if (name.startsWith('subscribe')) { subscriptions[name] = args[0]; return () => {}; }
    assert.ok(Object.hasOwn(stubs, name), `Unexpected native call: ${name}`);
    return stubs[name](...args);
  }]));
  window.eval(bundled.outputFiles[0].text);
  await window.neroBoot;
  assert.deepEqual(errors, []);
  assert.equal(window.document.getElementById('error').hidden, true);
  assert.equal(window.document.getElementById('create-meeting').disabled, false);
  const wait = async predicate => {
    for (let attempt = 0; attempt < 100; attempt++) {
      if (predicate()) return;
      await new Promise(resolve => setTimeout(resolve, 5));
    }
    assert.fail('Expected UI state was not rendered: ' + window.document.getElementById('error-message').textContent + ' / ' + window.document.getElementById('notification-center').textContent);
  };
  window.document.getElementById('settings').click();
  await wait(() => window.document.getElementById('settings-dialog').open);
  for (const tab of window.document.querySelectorAll('#settings-dialog [role="tab"][data-settings-panel]')) {
    tab.click();
    await wait(() => tab.getAttribute('aria-selected') === 'true');
    assert.equal(window.document.getElementById(`settings-${tab.dataset.settingsPanel}-panel`).hidden, false);
  }
  window.document.getElementById('settings-close').click();
  await wait(() => !window.document.getElementById('settings-dialog').open);
  await wait(() => window.document.activeElement.id === 'navigation-toggle');
  assert.equal(window.document.activeElement.id, 'navigation-toggle');
  assert.deepEqual(errors, []);
  assert.equal(window.document.getElementById('error').hidden, true);
  assert.ok(calls.includes('readSnapshot'));
  const recording = { revision: 2, folders: [], meetings: [{ id: 'one', title: '회의', createdAt: 1, notes: '', status: 'recording' }], session: { id: 'capture', meetingId: 'one', state: 'paused', elapsedMs: 1000 } };
  subscriptions.subscribeSnapshots(recording);
  await wait(() => window.document.querySelector('[data-action="show-recorder"]'));
  const reopen = window.document.querySelector('[data-action="show-recorder"]');
  assert.equal(reopen.getAttribute('aria-label'), '녹음 제어창 표시');
  assert.ok(reopen.querySelector('svg'));
  reopen.click();
  await wait(() => calls.includes('showRecorder'));
  await wait(() => !reopen.disabled);
  const cancelled = id => ({ id, kind: 'transcribe', status: 'cancelled', retryable: true, message: '취소했습니다.', meetingId: 'one' });
  subscriptions.subscribeProcessing({ ...cancelled('cancel1'), jobs: [cancelled('cancel1')] });
  await wait(() => !window.document.getElementById('retry-processing').hidden);
  assert.equal(window.document.getElementById('dismiss-processing').hidden, false);
  window.document.getElementById('dismiss-processing').click();
  await wait(() => window.document.getElementById('processing').hidden);
  subscriptions.subscribeProcessing({ ...cancelled('cancel2'), jobs: [cancelled('cancel2')] });
  await wait(() => !window.document.getElementById('retry-processing').hidden);
  window.document.getElementById('retry-processing').click();
  await wait(() => calls.includes('retryProcessing'));
  const background = { id: 'background', kind: 'summary-model', status: 'running', createdAt: 1 };
  subscriptions.subscribeProcessing({ revision: 20, status: 'idle', supported: true, modelReady: true, jobs: [background] });
  const region = window.document.getElementById('notification-center');
  window.document.getElementById('notifications-open').click();
  await wait(() => !region.hidden);
  await wait(() => region.querySelector('.activity-spinner'));
  const cancel = region.querySelector('[data-notification-action="cancel"]');
  cancel.focus();
  subscriptions.subscribeProcessing({ revision: 21, status: 'idle', supported: true, modelReady: true, jobs: [{ ...background, phase: 'preparing' }] });
  await wait(() => region.textContent.includes('요약 모델 준비 중'));
  assert.equal(window.document.activeElement.dataset.notificationJob, 'background');
  subscriptions.subscribeProcessing({ revision: 22, status: 'idle', supported: true, modelReady: true, jobs: [{ ...background, status: 'completed' }] });
  await wait(() => region.textContent.includes('완료'));
  assert.ok(region.querySelector('[data-notification-action="ack"]'));
  assert.equal(window.document.getElementById('processing-jobs'), null);
  assert.deepEqual(errors, []);
});
