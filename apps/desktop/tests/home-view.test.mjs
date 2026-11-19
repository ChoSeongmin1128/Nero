import assert from 'node:assert/strict';
import test from 'node:test';
import { Window } from 'happy-dom';
import { homeEvents, mountHome } from '../src/home-view.js';

const hour = 3600000;
const at = new Date(2026, 10, 19, 14, 30);

function harness(calendar, occurrences = []) {
  const window = new Window();
  const container = window.document.createElement('section');
  window.document.body.append(container);
  const calls = [];
  const home = mountHome({ container, icon: name => `<svg data-icon="${name}"></svg>`, escape: value => String(value ?? ''), now: () => at, timeZone: () => 'Asia/Seoul',
    device: () => '내장 마이크',
    bridge: { readCalendarMeetingOccurrences: async request => { calls.push(request); return { calendar, occurrences, scopeRevision: 4, ...request }; } },
    onRecordEvent: (key, scope) => calls.push(['event', key, scope]), onRecord: () => calls.push(['record']) });
  return { window, container, home, calls };
}

test('today is ordered by start and marks the event running now', () => {
  const now = at.getTime();
  const events = homeEvents([
    { key: 'b', title: '오후', startAt: now + hour, endAt: now + 2 * hour },
    { key: 'a', title: '지금', startAt: now - hour / 2, endAt: now + hour / 2 },
    { key: 'c', title: '오전', startAt: now - 3 * hour, endAt: now - 2 * hour },
  ], now);
  assert.deepEqual(events.map(event => [event.key, event.current, event.over]), [['c', false, true], ['a', true, false], ['b', false, false]]);
});

test('the event running now carries the one emphasis and records with its scope', async t => {
  const now = at.getTime();
  const app = harness({ connected: true }, [{ key: 'now', title: '주간 회의', startAt: now - hour / 2, endAt: now + hour / 2 }, { key: 'later', title: '면접', startAt: now + hour, endAt: now + 2 * hour }]);
  t.after(() => app.window.happyDOM.abort());
  await app.home.refresh();
  const primary = [...app.container.querySelectorAll('button.primary')];
  assert.equal(primary.length, 1);
  assert.equal(primary[0].dataset.homeEvent, 'now');
  assert.match(app.container.textContent, /진행 중/);
  primary[0].click();
  assert.deepEqual(app.calls.at(-1), ['event', 'now', { day: '2026-11-19', timeZone: 'Asia/Seoul', scopeRevision: 4 }]);
});

test('without a calendar, starting a recording is the emphasis and connecting is offered', async t => {
  const app = harness({ connected: false });
  t.after(() => app.window.happyDOM.abort());
  await app.home.refresh();
  assert.equal(app.container.querySelector('button.primary').dataset.homeAction, 'record');
  assert.ok(app.container.querySelector('[data-home-action="calendar"]'));
  assert.equal(app.container.querySelector('.home-schedule'), null);
});

test('a connection still being checked at launch is not shown as disconnected', async t => {
  const app = harness({ connected: false, restoring: true });
  t.after(() => app.window.happyDOM.abort());
  await app.home.refresh();
  assert.equal(app.container.querySelector('[data-home-action="calendar"]'), null);
});
