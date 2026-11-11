import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { build } from 'esbuild';
import { Window } from 'happy-dom';

const root = fileURLToPath(new URL('../', import.meta.url));
const source = await readFile(new URL('../src/calendar.js', import.meta.url), 'utf8');
const bundled = await build({
  entryPoints: [root + 'src/calendar.js'], bundle: true, write: false, format: 'iife',
  plugins: [{ name: 'calendar-native-boundary', setup(builder) {
    builder.onResolve({ filter: /^\.\/calendar-bridge\.js$/ }, () => ({ path: 'bridge', namespace: 'test-bridge' }));
    builder.onLoad({ filter: /.*/, namespace: 'test-bridge' }, () => ({ contents: [
      'subscribeCalendarReminder', 'readCalendarReminder', 'calendarReminderReady',
      'startCalendarRecording', 'dismissCalendarReminder',
    ].map(name => `export const ${name} = (...args) => globalThis.testBridge.${name}(...args);`).join('\n') }));
    builder.onLoad({ filter: /[\\/]src[\\/]calendar\.js$/ }, () => ({ contents:
      source.match(/^import .*;$/gm).join('\n') + '\nglobalThis.calendarBoot = (async () => {\n'
      + source.replace(/^import .*;\r?\n/gm, '') + '\n})();',
    }));
  } }],
});

const startAt = 1_900_000_000_000;
const event = { key: 'synthetic', title: 'Synthetic meeting', calendarName: 'Synthetic calendar', startAt, endAt: startAt + 3_600_000 };
const settle = () => new Promise(resolve => setImmediate(resolve));

async function open(t, initial = event, overrides = {}, now = startAt - 60_000) {
  const window = new Window({ settings: {
    enableJavaScriptEvaluation: true, suppressInsecureJavaScriptEnvironmentWarning: true,
    disableCSSFileLoading: true, disableJavaScriptFileLoading: true,
  } });
  t.after(() => window.happyDOM.abort());
  window.document.write(await readFile(new URL('../src/calendar.html', import.meta.url), 'utf8'));
  let accept, tick;
  const calls = [];
  window.Date.now = () => now;
  window.setInterval = callback => { tick = callback; return 1; };
  window.testBridge = {
    subscribeCalendarReminder: async callback => { accept = callback; },
    readCalendarReminder: async () => initial,
    calendarReminderReady: async () => { calls.push('ready'); },
    startCalendarRecording: async () => { calls.push('record'); },
    dismissCalendarReminder: async () => { calls.push('dismiss'); },
    ...overrides,
  };
  window.eval(bundled.outputFiles[0].text);
  await window.calendarBoot;
  return { window, calls, accept, element: id => window.document.getElementById(id), advance: value => { now = value; tick(); } };
}

test('a reminder keeps recording available across the start time until the backend withdraws it', async t => {
  const page = await open(t);
  const record = page.element('calendar-record');
  assert.equal(page.element('calendar-title').textContent, event.title);
  assert.equal(page.element('calendar-countdown').textContent, '1분 후');
  assert.equal(record.disabled, false);
  assert.deepEqual(page.calls, ['ready']);
  page.advance(startAt);
  assert.equal(page.element('calendar-countdown').textContent, '지금 시작');
  assert.equal(record.disabled, false);
  page.advance(startAt + 299_999);
  assert.equal(page.element('calendar-countdown').textContent, '4분 지남');
  assert.equal(record.disabled, false);
  record.click(); await settle();
  assert.deepEqual(page.calls, ['ready', 'record']);
  page.accept(null);
  assert.equal(record.disabled, true);
  assert.equal(page.element('calendar-countdown').textContent, '');
  record.click(); await settle();
  assert.deepEqual(page.calls, ['ready', 'record']);
});

test('a reminder first opened after its start still permits recording', async t => {
  const page = await open(t, event, {}, startAt + 120_000);
  assert.equal(page.element('calendar-record').disabled, false);
  assert.equal(page.element('calendar-countdown').textContent, '2분 지남');
  page.element('calendar-record').click(); await settle();
  assert.deepEqual(page.calls, ['ready', 'record']);
});

test('recording locks both controls and ignores dismissal until the request finishes', async t => {
  let finish;
  const page = await open(t, event, { startCalendarRecording: () => {
    page.calls.push('record'); return new Promise(resolve => { finish = resolve; });
  } });
  page.element('calendar-record').click();
  assert.equal(page.element('calendar-record').disabled, true);
  assert.equal(page.element('calendar-record').textContent, '확인 중');
  assert.equal(page.element('calendar-dismiss').disabled, true);
  page.element('calendar-record').click();
  page.element('calendar-dismiss').click();
  page.window.document.dispatchEvent(new page.window.KeyboardEvent('keydown', { key: 'Escape' }));
  assert.deepEqual(page.calls, ['ready', 'record']);
  finish(); await settle();
  assert.equal(page.element('calendar-record').disabled, false);
  assert.equal(page.element('calendar-dismiss').disabled, false);
});

test('dismissal locks recording and escape does not submit another request', async t => {
  let finish;
  const page = await open(t, event, { dismissCalendarReminder: () => {
    page.calls.push('dismiss'); return new Promise(resolve => { finish = resolve; });
  } });
  page.element('calendar-dismiss').click();
  assert.equal(page.element('calendar-record').disabled, true);
  assert.equal(page.element('calendar-dismiss').disabled, true);
  page.element('calendar-record').click();
  page.window.document.dispatchEvent(new page.window.KeyboardEvent('keydown', { key: 'Escape' }));
  assert.deepEqual(page.calls, ['ready', 'dismiss']);
  finish(); await settle();
  assert.equal(page.element('calendar-dismiss').disabled, false);
});

test('a failed recording surfaces its error and allows retry after the start', async t => {
  let attempts = 0;
  const page = await open(t, event, { startCalendarRecording: async () => {
    attempts += 1;
    if (attempts === 1) throw Error('Synthetic recording failure');
  } }, startAt + 60_000);
  page.element('calendar-record').click(); await settle();
  assert.equal(page.element('calendar-error').textContent, 'Synthetic recording failure');
  assert.equal(page.element('calendar-title').textContent, '일정 알림 확인 필요');
  assert.equal(page.element('calendar-record').disabled, false);
  assert.equal(page.element('calendar-dismiss').disabled, false);
  page.element('calendar-record').click(); await settle();
  assert.equal(attempts, 2);
  assert.equal(page.element('calendar-error').textContent, '');
  assert.equal(page.element('calendar-title').textContent, event.title);
});

test('an absent or unreadable reminder never offers recording', async t => {
  const empty = await open(t, null);
  assert.equal(empty.element('calendar-record').disabled, true);
  assert.deepEqual(empty.calls, []);
  const failed = await open(t, null, { readCalendarReminder: async () => { throw Error('Synthetic read failure'); } });
  assert.equal(failed.element('calendar-record').disabled, true);
  assert.equal(failed.element('calendar-error').textContent, 'Synthetic read failure');
  assert.equal(failed.element('calendar-dismiss').disabled, false);
  assert.deepEqual(failed.calls, []);
});
