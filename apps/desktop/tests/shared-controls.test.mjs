import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { Window } from 'happy-dom';
import { buildControls } from '../scripts/build-controls.mjs';
import { dateAt, dateKey, dateValue, initialDate, parseDate } from '../../../design-system/control-values.mjs';

const generated = join(await mkdtemp(join(tmpdir(), 'nero-controls-')), 'controls.js');
await buildControls(generated);
const bundle = await readFile(generated, 'utf8');

function page(t, html) {
  const window = new Window({ url: 'http://nero.test', settings: { enableJavaScriptEvaluation: true, suppressInsecureJavaScriptEnvironmentWarning: true } });
  window.document.body.innerHTML = html;
  window.eval(bundle);
  window.GroveControls.enhanceAll();
  t.after(() => window.happyDOM.abort());
  return { window, document: window.document, controls: window.GroveControls, day: value => window.document.querySelector(`[data-day="${value}"] button`) };
}

const dateInput = (attributes = '') => `<label>회의 날짜<input id="date" type="date" value="2026-10-01" ${attributes}></label>`;

test('calendar dates preserve local years and reject invalid dates, time, and full datetime bounds', () => {
  assert.equal(dateKey(dateAt(1, 0, 1)), '0001-01-01');
  assert.equal(parseDate('2026-02-30'), null);
  assert.equal(dateKey(initialDate('', '2026-11-10', '2026-11-15', dateAt(2026, 9, 21))), '2026-11-10');
  assert.equal(dateValue('2026-10-01', '09:59', { timed: true, min: '2026-10-01T10:00' }), null);
  assert.equal(dateValue('2026-10-01', '24:00', { timed: true }), null);
  assert.equal(dateValue('2026-10-01', '14:30', { timed: true }), '2026-10-01T14:30');
  assert.equal(dateValue('', '', { timed: true }), '');
});

test('native select changes emit one input/change pair and refresh programmatic values/options', t => {
  const app = page(t, '<label>속도<select id="rate"><option value="1">1배</option><option value="2">2배</option></select></label>');
  const native = app.document.querySelector('#rate');
  const events = [];
  for (const type of ['input', 'change']) native.addEventListener(type, () => events.push(type));
  app.document.querySelector('.grove-select-trigger').click();
  const option = [...app.document.querySelectorAll('[role="option"]')].find(node => node.textContent.includes('2배'));
  assert.ok(option);
  option.click();
  assert.equal(native.value, '2');
  assert.deepEqual(events, ['input', 'change']);
  assert.equal(app.controls.isOpen(), false);
  native.value = '1';
  native.options[0].textContent = '보통';
  app.controls.refresh();
  assert.match(app.document.querySelector('.grove-select-trigger').textContent, /보통/);
});

test('select respects disabled optgroups and keeps its original native form element', t => {
  const app = page(t, '<select id="rate"><option value="1">1배</option><optgroup label="사용 불가" disabled><option value="2">2배</option></optgroup></select>');
  const original = app.document.querySelector('#rate');
  app.document.querySelector('.grove-select-trigger').click();
  const option = [...app.document.querySelectorAll('[role="option"]')].find(node => node.textContent.includes('2배'));
  assert.equal(option.hasAttribute('data-disabled'), true);
  option.click();
  assert.equal(original.value, '1');
  app.controls.close();
  original.disabled = true; app.controls.refresh();
  assert.equal(app.document.querySelector('.grove-select-trigger').disabled, true);
  assert.equal(app.document.querySelector('#rate'), original);
});

test('select displays the native empty-string option label and preserves it through changes', t => {
  const app = page(t, '<label>녹음 마이크<select id="microphone"><option value="">시스템 기본 마이크</option><option value="built-in">내장 마이크</option></select></label>');
  const native = app.document.querySelector('#microphone');
  const trigger = app.document.querySelector('.grove-select-trigger');
  const events = [];
  native.addEventListener('change', () => events.push(native.value));
  assert.equal(trigger.textContent, '시스템 기본 마이크');
  assert.equal(trigger.getAttribute('aria-label'), '녹음 마이크: 시스템 기본 마이크');
  for (const label of ['내장 마이크', '시스템 기본 마이크']) {
    trigger.click();
    [...app.document.querySelectorAll('[role="option"]')].find(node => node.textContent === label).click();
    assert.equal(trigger.textContent, label);
  }
  assert.equal(native.value, '');
  assert.deepEqual(events, ['built-in', '']);
  native.options[0].textContent = '기본 입력';
  app.controls.refresh();
  assert.equal(trigger.textContent, '기본 입력');
});

test('date month navigation remains stable after focus/effects and Escape restores the trigger', async t => {
  const app = page(t, dateInput());
  const trigger = app.document.querySelector('.grove-date-trigger');
  trigger.click();
  await new Promise(resolve => setTimeout(resolve, 30));
  assert.match(app.document.querySelector('.rdp-caption_label').textContent, /2026년 10월/);
  const next = app.document.querySelector('[aria-label="다음 달"]');
  next.focus(); next.click();
  await new Promise(resolve => setTimeout(resolve, 30));
  assert.match(app.document.querySelector('.rdp-caption_label').textContent, /2026년 11월/);
  assert.equal(app.document.querySelector('#date').value, '2026-10-01');
  const event = new app.window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
  app.document.querySelector('.grove-calendar').dispatchEvent(event);
  assert.equal(event.defaultPrevented, true);
  assert.equal(app.controls.isOpen(), false);
  assert.equal(app.document.activeElement, trigger);
});

test('external date changes close a stale pending calendar instead of overwriting the new source', t => {
  const app = page(t, '<input id="date" type="datetime-local" value="2026-10-01T14:30">');
  const native = app.document.querySelector('#date');
  app.document.querySelector('.grove-date-trigger').click();
  app.day('2026-10-03').click();
  native.value = '2026-10-05T16:00'; app.controls.refresh();
  assert.equal(app.controls.isOpen(), false);
  assert.equal(native.value, '2026-10-05T16:00');
  app.document.querySelector('.grove-date-trigger').click();
  assert.equal(app.document.querySelector('.grove-calendar-time input').value, '16:00');
});

test('calendar enforces date bounds and commits a chosen date with native events', t => {
  const app = page(t, dateInput('min="2026-10-10" max="2026-11-05"'));
  const native = app.document.querySelector('#date'), events = [];
  native.value = '2026-10-10'; app.controls.refresh();
  native.addEventListener('change', () => events.push(native.value));
  app.document.querySelector('.grove-date-trigger').click();
  assert.equal(app.day('2026-10-09').disabled, true);
  app.document.querySelector('[aria-label="다음 달"]').click();
  assert.equal(app.day('2026-11-06').disabled, true);
  app.day('2026-11-03').click();
  assert.equal(native.value, '2026-11-03');
  assert.deepEqual(events, ['2026-11-03']);
  assert.equal(app.controls.isOpen(), false);
});

test('datetime browsing and pending choice only commit on apply', t => {
  const app = page(t, '<input id="date" aria-label="시작" type="datetime-local" value="2026-10-01T14:30" min="2026-10-01T10:00" max="2026-11-05T18:00">');
  const native = app.document.querySelector('#date');
  app.document.querySelector('.grove-date-trigger').click();
  app.document.querySelector('[aria-label="다음 달"]').click();
  app.day('2026-11-03').click();
  assert.equal(native.value, '2026-10-01T14:30');
  [...app.document.querySelectorAll('.grove-calendar-footer button')].find(node => node.textContent === '적용').click();
  assert.equal(native.value, '2026-11-03T14:30');
});

test('popup portals stay inside a native dialog and Escape does not close the owner', t => {
  const app = page(t, `<dialog open>${dateInput()}</dialog>`);
  const owner = app.document.querySelector('dialog');
  let escapedToOwner = false;
  owner.addEventListener('keydown', () => { escapedToOwner = true; });
  app.document.querySelector('.grove-date-trigger').click();
  assert.equal(owner.contains(app.document.querySelector('.grove-calendar')), true);
  app.document.querySelector('.grove-calendar').dispatchEvent(new app.window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
  assert.equal(escapedToOwner, false);
  assert.equal(owner.open, true);
  assert.equal(app.controls.isOpen(), false);
});

test('disabled/read-only fields cannot open and removed controls dispose portal roots', async t => {
  const app = page(t, dateInput('readonly'));
  assert.equal(app.document.querySelector('.grove-date-trigger').disabled, true);
  app.document.querySelector('.grove-date-trigger').click();
  assert.equal(app.controls.isOpen(), false);
  const native = app.document.querySelector('#date');
  native.readOnly = false; app.controls.refresh();
  app.document.querySelector('.grove-date-trigger').click();
  const oldWrapper = app.document.querySelector('.grove-date');
  oldWrapper.remove(); app.controls.enhanceAll();
  assert.equal(app.controls.isOpen(), false);
  assert.equal(app.document.querySelector('.grove-calendar'), null);
  app.document.body.append(native); app.controls.enhanceAll();
  assert.equal(app.document.querySelectorAll('.grove-date-trigger').length, 1);
});
