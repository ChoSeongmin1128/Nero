import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { build } from 'esbuild';
import { Window } from 'happy-dom';

const root = fileURLToPath(new URL('../', import.meta.url));
const source = await readFile(new URL('../src/recorder.js', import.meta.url), 'utf8');
const bundled = await build({
  entryPoints: [root + 'src/recorder.js'], bundle: true, write: false, format: 'iife',
  plugins: [{ name: 'recorder-native-boundary', setup(builder) {
    builder.onResolve({ filter: /^\.\/bridge\.js$/ }, () => ({ path: 'bridge', namespace: 'test-bridge' }));
    builder.onLoad({ filter: /.*/, namespace: 'test-bridge' }, () => ({ contents: ['subscribeSnapshots', 'readSnapshot', 'openMeeting', 'pauseRecording', 'resumeRecording', 'stopRecording', 'dragRecorder', 'hideRecorder'].map(name => `export const ${name} = (...args) => globalThis.testBridge.${name}(...args);`).join('\n') }));
    builder.onLoad({ filter: /[\\/]src[\\/]recorder\.js$/ }, () => ({ contents: source.match(/^import .*;$/gm).join('\n') + '\nglobalThis.recorderBoot = (async () => {\n' + source.replace(/^import .*;\r?\n/gm, '') + '\n})();' }));
  } }],
});

function snapshot(quality, warning = '', error = '') {
  return { revision: 1, meetings: [{ id: 'meeting', title: '회의', status: 'recording' }], session: { id: 'capture', meetingId: 'meeting', state: 'recording', elapsedMs: 393000, sampleRate: 48000, savedFrames: 48000 * 393, deviceName: '마이크', captureQuality: quality, warning, error } };
}

async function open(t, initial) {
  const window = new Window({ settings: { enableJavaScriptEvaluation: true, suppressInsecureJavaScriptEnvironmentWarning: true, disableCSSFileLoading: true, disableJavaScriptFileLoading: true } });
  t.after(() => window.happyDOM.abort());
  window.document.write(await readFile(new URL('../src/recorder.html', import.meta.url), 'utf8'));
  for (const name of ['appearance', 'icons', 'components']) window.eval(await readFile(new URL(`../../../design-system/${name}.js`, import.meta.url), 'utf8'));
  let accept;
  const calls = [];
  const subscriptions = [];
  window.testBridge = {
    subscribeSnapshots: async callback => { accept = callback; subscriptions.push(callback); },
    readSnapshot: async () => initial,
    openMeeting: async (...args) => calls.push(['open', ...args]),
    pauseRecording: async id => { calls.push(['pause', id]); return { ...initial, revision: 2, session: { ...initial.session, state: 'paused' } }; },
    resumeRecording: async id => { calls.push(['resume', id]); return { ...initial, revision: 3 }; },
    stopRecording: async () => { throw new window.Error('저장 실패'); },
    hideRecorder: async () => {}, dragRecorder: async () => {},
  };
  window.eval(bundled.outputFiles[0].text);
  await window.recorderBoot;
  assert.equal(subscriptions.length, 1);
  return { window, calls, accept, element: id => window.document.getElementById(id) };
}

test('driver diagnostics retain the recording indicator, timer and pause controls across snapshots', async t => {
  const initial = snapshot({ xrunCount: 1, events: [{ kind: 'xrun', firstSavedFrame: 0 }] }, '오디오 입력 지연 신호가 기록되었습니다.');
  const page = await open(t, initial);
  assert.equal(page.element('recorder-time').textContent, '06:33');
  assert.ok(page.element('recorder-status').querySelector('.desktop-state-dot'));
  assert.equal(page.element('recorder-status').classList.contains('is-error'), false);
  assert.equal(page.element('recorder-error').textContent, '');
  assert.doesNotMatch(page.element('open-meeting').getAttribute('aria-label'), /경고|확인 필요/);
  page.accept({ ...initial, revision: 2, session: { ...initial.session, captureQuality: { xrunCount: 20 } } });
  assert.ok(page.element('recorder-status').querySelector('.desktop-state-dot'));
  page.window.document.querySelector('[data-action="pause"]').click();
  await page.window.happyDOM.whenAsyncComplete();
  assert.equal(page.element('recorder-status').classList.contains('is-paused'), true);
  assert.ok(page.window.document.querySelector('[data-action="resume"]'));
  assert.deepEqual(page.calls, [['pause', 'capture']]);
});

test('device and storage warnings still surface and open the affected meeting', async t => {
  for (const kind of ['routeChanged', 'queueOverflow', 'deviceFailure', 'writerFailure', 'formatChanged', 'captureInterrupted']) {
    const page = await open(t, snapshot({ xrunCount: 1, events: [{ kind: 'xrun' }, { kind }] }, '오디오 진단'));
    assert.ok(page.element('recorder-status').querySelector('svg'), kind);
    assert.equal(page.element('recorder-status').classList.contains('is-error'), true, kind);
    assert.notEqual(page.element('recorder-error').textContent, '', kind);
    assert.doesNotMatch(page.element('open-meeting').getAttribute('aria-label'), /음질/);
    page.element('open-meeting').click();
    await page.window.happyDOM.whenAsyncComplete();
    assert.deepEqual(page.calls[0], ['open', 'meeting', null]);
  }
});

test('unclassified legacy warnings and operation failures remain actionable', async t => {
  const page = await open(t, snapshot(null, '장치를 확인하세요.'));
  assert.equal(page.element('recorder-status').classList.contains('is-error'), true);
  page.accept(snapshot({ xrunCount: 1 }, '', '마이크 입력 중단'));
  assert.equal(page.element('recorder-time').textContent, '오류 확인');
  assert.equal(page.element('recorder-error').textContent, '마이크 입력 중단');
  page.accept({ ...snapshot({ xrunCount: 1 }), revision: 3 });
  assert.ok(page.element('recorder-status').querySelector('.desktop-state-dot'));
  page.window.document.querySelector('[data-action="stop"]').click();
  await page.window.happyDOM.whenAsyncComplete();
  assert.equal(page.element('recorder-error').textContent, '저장 실패');
  assert.equal(page.element('recorder-status').classList.contains('is-error'), true);
});
