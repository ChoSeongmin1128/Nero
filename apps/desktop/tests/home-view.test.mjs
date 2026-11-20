import assert from 'node:assert/strict';
import test from 'node:test';
import { Window } from 'happy-dom';
import { homeEvents, mountHome } from '../src/home-view.js';

const hour = 3600000;
const at = new Date(2026, 10, 19, 14, 30);

function harness(calendar, occurrences = [], extra = {}) {
  const window = new Window();
  const container = window.document.createElement('section');
  window.document.body.append(container);
  const calls = [];
  const home = mountHome({ container, icon: name => `<svg data-icon="${name}"></svg>`, escape: value => String(value ?? ''), now: () => at, timeZone: () => 'Asia/Seoul',
    device: () => '내장 마이크',
    bridge: { readCalendarMeetingOccurrences: async request => { calls.push(request); return { calendar, occurrences, scopeRevision: 4, ...request }; } },
    onRecordEvent: (key, scope) => calls.push(['event', key, scope]), onRecord: () => calls.push(['record']), ...extra });
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

test('a connection still being checked at launch shows loading with the shared spinner, not a connect hint', async t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const app = harness({ connected: false, restoring: true });
  t.after(() => app.window.happyDOM.abort());
  await app.home.refresh();
  assert.equal(app.container.querySelector('[data-home-action="calendar"]'), null);
  const status = app.container.querySelector('.home-schedule [role="status"]');
  assert.ok(status.querySelector('.activity-spinner'));
  assert.match(status.textContent, /일정을 불러오는 중/);
});

test('a connection check is not polled; one wait says so when it takes too long', async t => {
  const timers = [];
  let clock = at.getTime();
  const app = harness({ connected: false, restoring: true }, [], { now: () => new Date(clock) });
  t.after(() => app.window.happyDOM.abort());
  const original = globalThis.setTimeout;
  globalThis.setTimeout = (callback, delay) => { timers.push({ callback, delay }); return timers.length; };
  t.after(() => { globalThis.setTimeout = original; });
  await app.home.refresh();
  await app.home.refresh();
  assert.equal(timers.length, 1, 'repeated reads share one wait');
  assert.equal(timers[0].delay, 30_000);
  clock += 30_000;
  timers.shift().callback();
  assert.equal(app.calls.length, 2, 'waiting reads nothing again');
  assert.equal(app.container.querySelector('.activity-spinner'), null);
  assert.match(app.container.textContent, /연결을 아직 확인하지 못했습니다/);
});

test('a calendar change reloads today once home has shown it, and the same state does not', async t => {
  const now = at.getTime();
  let calendar = { connected: false, restoring: true };
  const app = harness(undefined, [], { bridge: { readCalendarMeetingOccurrences: async request => { app.calls.push(request); return { calendar, occurrences: calendar.connected ? [{ key: 'now', title: '주간 회의', startAt: now - hour / 2, endAt: now + hour / 2 }] : [], scopeRevision: 4 }; } } });
  t.after(() => app.window.happyDOM.abort());
  app.home.acceptCalendar({ connected: false, restoring: true });
  assert.equal(app.calls.length, 0, 'home has not shown today yet');
  await app.home.refresh();
  calendar = { connected: true, restoring: false };
  app.home.acceptCalendar({ connected: true, restoring: false, lastSyncedAt: 1 });
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(app.calls.length, 2);
  assert.match(app.container.textContent, /주간 회의/);
  app.home.acceptCalendar({ connected: true, restoring: false, lastSyncedAt: 1, revision: 9 });
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(app.calls.length, 2, 'only connection, checking and sync changes reload');
  app.home.acceptCalendar({ connected: true, restoring: false, lastSyncedAt: 2 });
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(app.calls.length, 3, 'a finished sync shows its events');
});

test('reloading a connected schedule shows the shared spinner and the running event uses the status chip', async t => {
  const now = at.getTime();
  const app = harness({ connected: true }, [{ key: 'now', title: '주간 회의', startAt: now - hour / 2, endAt: now + hour / 2 }]);
  t.after(() => app.window.happyDOM.abort());
  await app.home.refresh();
  assert.equal(app.container.querySelector('.home-event .status-chip').textContent, '진행 중');
  assert.ok([...app.container.querySelectorAll('.home-actions button, .home-event button')].every(button => button.classList.contains('with-icon')));
});

test('home picks the folder new meetings start in, unfiled by default', async t => {
  let chosen = 'f2';
  const picked = [];
  const app = harness({ connected: false }, [], { folders: () => [{ id: 'f1', name: '제품 회의' }, { id: 'f2', name: '채용' }], folder: () => chosen, onFolder: id => { picked.push(id); chosen = id; } });
  t.after(() => app.window.happyDOM.abort());
  await app.home.refresh();
  const select = app.container.querySelector('[data-home-folder]');
  assert.deepEqual([...select.options].map(option => option.textContent), ['미분류', '제품 회의', '채용']);
  assert.equal(select.value, 'f2');
  select.value = '';
  select.dispatchEvent(new app.window.Event('change', { bubbles: true }));
  assert.deepEqual(picked, [null]);
  chosen = 'gone';
  app.home.render();
  assert.equal(app.container.querySelector('[data-home-folder]').value, '', 'a deleted folder shows unfiled');
  assert.match(app.container.querySelector('.home-context').textContent, /내장 마이크/);
});

