import assert from 'node:assert/strict';
import test from 'node:test';
import { createRequire } from 'node:module';
import { Window } from 'happy-dom';
import { createNoticeActions } from '../src/notice-actions.js';
import { attentionNotices, createIntegrationMonitor, integrationNotices } from '../src/integration-notifications.js';
import { mountNotificationCenter } from '../src/notification-center.js';
import { mountHome } from '../src/home-view.js';
import { mountNavigationSidebar } from '../../../design-system/navigation.mjs';

const UI = createRequire(import.meta.url)('../../../design-system/components.js');
const settle = () => new Promise(resolve => setImmediate(resolve));
const deferred = () => {
  let resolve, reject;
  const promise = new Promise((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
};
const at = new Date(2026, 10, 19, 14, 30);
const agents = clients => ({ available: true, clients });
const calendarError = (changes = {}) => ({ revision: 2, configured: true, connected: false, accountLabel: 'Test account', error: 'Reconnect', ...changes });
const [claudeUpdate] = integrationNotices({ agents: agents([{ id: 'claude', state: 'update_required', action: 'update', packageVersion: '45' }]) });
const [codexBlocked] = integrationNotices({ agents: agents([{ id: 'codex', state: 'update_required', action: 'none', packageVersion: '30', detail: 'ChatGPT에 설치된 Nero 연동과 번호가 같아 업데이트되지 않습니다.' }]) });
const [calendarReconnect] = integrationNotices({ calendar: calendarError() });

function noticeActionsWith(handlers = {}) {
  const changes = [];
  const actions = createNoticeActions({ handlers });
  actions.subscribe(() => changes.push(true));
  return { actions, changes };
}

function mountHomeIn(window, container, { notices, actions, ...extra }) {
  return mountHome({ container, ui: UI, icon: name => `<svg data-icon="${name}"></svg>`, escape: UI.escape, now: () => at, timeZone: () => 'Asia/Seoul',
    bridge: { readCalendarMeetingOccurrences: async () => ({ calendar: { connected: false }, occurrences: [] }) },
    attention: () => attentionNotices(notices.current), noticeState: id => actions.state(id), onNotice: (notice, action) => actions.run(notice, action), ...extra });
}

function homeHarness(t, { notices = [], handlers = {}, wire = true, ...extra } = {}) {
  const window = new Window();
  t.after(() => window.happyDOM.abort());
  const container = window.document.createElement('section');
  window.document.body.append(container);
  const { actions } = noticeActionsWith(handlers);
  const state = { current: notices };
  const home = mountHomeIn(window, container, { notices: state, actions, ...extra });
  if (wire) actions.subscribe(() => home.render());
  home.render();
  const sections = () => [...container.querySelectorAll('.home-attention .attention-notice')];
  const section = id => sections().find(node => node.dataset.noticeId === id);
  const button = id => [...container.querySelectorAll('[data-home-notice]')].find(node => node.dataset.homeNotice === id);
  const show = next => { state.current = next; home.render(); };
  return { window, container, home, actions, state, sections, section, button, show };
}

// Home and the notification list wired the way main.js wires them: one shared notice
// action owner, home re-rendered by it, and the list's change re-syncing both screens.
function appHarness(t, { notices = [], handlers = {} } = {}) {
  const window = new Window();
  t.after(() => window.happyDOM.abort());
  const homeContainer = window.document.createElement('section');
  const listContainer = window.document.createElement('section');
  window.document.body.append(homeContainer, listContainer);
  // `shown` is the app-wide error report; the list uses it only for its processing rows.
  const shown = [];
  const actions = createNoticeActions({ handlers });
  const state = { current: notices };
  const home = mountHomeIn(window, homeContainer, { notices: state, actions });
  actions.subscribe(() => home.render());
  let list = null;
  const sync = () => { list.update({ connected: true, meetings: [], processing: { revision: 1, jobs: [] }, integrations: state.current }); home.render(); };
  list = mountNotificationCenter({ container: listContainer, noticeActions: actions, showError: error => shown.push(error), onChanged: () => sync() });
  sync();
  const show = next => { state.current = next; sync(); };
  const homeButton = id => [...homeContainer.querySelectorAll('[data-home-notice]')].find(node => node.dataset.homeNotice === id);
  const homeSection = id => [...homeContainer.querySelectorAll('.attention-notice')].find(node => node.dataset.noticeId === id);
  const listButton = (id, action) => [...listContainer.querySelectorAll('[data-notification-action]')]
    .find(node => node.dataset.notificationJob === id && node.dataset.notificationAction === action);
  const listRow = id => [...listContainer.querySelectorAll('[data-notification-id]')].find(node => node.dataset.notificationId === id);
  return { window, homeContainer, listContainer, home, list, actions, shown, show, homeButton, homeSection, listButton, listRow };
}

function railHarness(t, { labelled = true } = {}) {
  const window = new Window({ url: 'http://nero.test' });
  t.after(() => window.happyDOM.abort());
  const document = window.document;
  const views = [['home', '홈'], ['all', '전체 회의'], ['trash', '휴지통']];
  document.body.innerHTML = `<header><button id="navigation-toggle">탐색</button></header><div class="navigation-shell">
    <nav class="navigation-rail" data-navigation-compact>${views.map(([view, label]) => `<button data-navigation-preview="${view}"${labelled ? ` aria-label="${label}" title="${label}"` : ''}>${label}</button>`).join('')}</nav>
    <div class="navigation-edge"></div><aside id="navigation-sidebar"><input id="search"></aside><main></main></div>`;
  const shell = document.querySelector('.navigation-shell');
  shell.getBoundingClientRect = () => ({ width: 640 });
  const navigation = mountNavigationSidebar({ shell, aside: document.getElementById('navigation-sidebar'), content: document.querySelector('main'),
    trigger: document.getElementById('navigation-toggle'), storage: window.localStorage });
  const preview = view => document.querySelector(`[data-navigation-preview="${view}"]`);
  const name = node => node.getAttribute('aria-label') || node.textContent.trim();
  return { window, document, navigation, preview, name };
}

const phrase = '확인할 항목 있음';
const count = (text, part) => text.split(part).length - 1;

// createNoticeActions

test('pressing the same notice twice runs its action once and the second press does not wait for the first', async () => {
  const gate = deferred();
  let calls = 0;
  const { actions } = noticeActionsWith({ update: () => { calls += 1; return gate.promise; } });
  const first = actions.run({ id: 'n1' }, 'update');
  let secondDone = false;
  actions.run({ id: 'n1', title: '새 스냅숏의 같은 알림' }, 'update').then(() => { secondDone = true; });
  await settle();
  assert.equal(calls, 1);
  assert.equal(secondDone, true);
  assert.deepEqual(actions.state('n1'), { pending: 'update', error: '', note: '' });
  gate.resolve();
  await first;
  assert.equal(actions.busy('n1'), false);
  assert.deepEqual(actions.state('n1'), { pending: null, error: '', note: '' });
});

test('another action on a running notice is refused while a different notice runs independently', async () => {
  const gate = deferred();
  const calls = [];
  const { actions } = noticeActionsWith({ update: notice => { calls.push(['update', notice.id]); return gate.promise; }, settings: async notice => { calls.push(['settings', notice.id]); } });
  const running = actions.run({ id: 'n1' }, 'update');
  await actions.run({ id: 'n1' }, 'settings');
  await actions.run({ id: 'n2' }, 'settings');
  assert.deepEqual(calls, [['update', 'n1'], ['settings', 'n2']]);
  assert.equal(actions.state('n1').pending, 'update');
  assert.equal(actions.busy('n2'), false);
  gate.resolve();
  await running;
});

test('an action without a handler or a missing notice changes nothing and announces nothing', async () => {
  const { actions, changes } = noticeActionsWith({ update: async () => 'x' });
  await actions.run({ id: 'n1' }, 'missing');
  await actions.run({ id: 'n1' }, undefined);
  await actions.run(null, 'update');
  await actions.run(undefined, 'update');
  assert.deepEqual(changes, []);
  assert.deepEqual(actions.state('n1'), { pending: null, error: '', note: '' });
  assert.deepEqual(actions.state('never-seen'), { pending: null, error: '', note: '' });
  assert.equal(actions.busy('n1'), false);
});

test('a failing handler keeps its message on the notice, frees the notice and never rejects', async () => {
  const failure = new Error('업데이트하지 못했습니다.');
  const { actions, changes } = noticeActionsWith({ update: async () => { throw failure; } });
  await assert.doesNotReject(actions.run({ id: 'n1' }, 'update'));
  assert.deepEqual(actions.state('n1'), { pending: null, error: '업데이트하지 못했습니다.', note: '' });
  assert.equal(changes.length, 2, 'one change when it starts and one when it ends');
});

test('a synchronous throw, a thrown string and a rejected promise all end as the notice error', async () => {
  const { actions } = noticeActionsWith({
    sync: () => { throw new Error('동기 오류'); },
    text: async () => { throw '문자열 오류'; },
    reject: () => Promise.reject(new Error('거부')),
  });
  await actions.run({ id: 'a' }, 'sync');
  await actions.run({ id: 'b' }, 'text');
  await actions.run({ id: 'c' }, 'reject');
  assert.deepEqual(['a', 'b', 'c'].map(id => actions.state(id).error), ['동기 오류', '문자열 오류', '거부']);
  assert.deepEqual(['a', 'b', 'c'].map(id => actions.busy(id)), [false, false, false]);
});

test('only a non-empty string a handler returns becomes the note', async () => {
  const results = [undefined, null, '', 0, 42, true, { note: '설치를 확인해 주세요.' }, ['설치를 확인해 주세요.'], new String('설치를 확인해 주세요.')];
  let next;
  const { actions } = noticeActionsWith({ update: async () => next });
  for (const [index, value] of results.entries()) {
    next = value;
    await actions.run({ id: `n${index}` }, 'update');
    assert.equal(actions.state(`n${index}`).note, '', `${String(value)} is not a note`);
  }
  next = 'Claude Desktop에서 설치를 확인해 주세요.';
  await actions.run({ id: 'n-note' }, 'update');
  assert.equal(actions.state('n-note').note, 'Claude Desktop에서 설치를 확인해 주세요.');
});

test('running again clears the previous note and error as soon as it starts', async () => {
  const steps = [deferred(), deferred(), deferred()];
  let call = 0;
  const { actions } = noticeActionsWith({ update: () => steps[call++].promise });
  const notice = { id: 'n1' };
  let run = actions.run(notice, 'update');
  steps[0].resolve('설치를 확인해 주세요.');
  await run;
  assert.equal(actions.state('n1').note, '설치를 확인해 주세요.');
  run = actions.run(notice, 'update');
  assert.deepEqual(actions.state('n1'), { pending: 'update', error: '', note: '' });
  steps[1].reject(new Error('실패'));
  await run;
  assert.deepEqual(actions.state('n1'), { pending: null, error: '실패', note: '' });
  run = actions.run(notice, 'update');
  assert.deepEqual(actions.state('n1'), { pending: 'update', error: '', note: '' });
  steps[2].resolve('');
  await run;
  assert.deepEqual(actions.state('n1'), { pending: null, error: '', note: '' });
});

test('keep drops the note and error of notices that went away but never the action still running', async () => {
  const gate = deferred();
  const { actions, changes } = noticeActionsWith({
    note: async () => '안내', fail: async () => { throw new Error('오류'); }, slow: () => gate.promise,
  });
  await actions.run({ id: 'noted' }, 'note');
  await actions.run({ id: 'failed' }, 'fail');
  const running = actions.run({ id: 'running' }, 'slow');
  changes.length = 0;
  actions.keep(['running']);
  assert.equal(changes.length, 1);
  assert.equal(actions.state('noted').note, '');
  assert.equal(actions.state('failed').error, '');
  assert.equal(actions.busy('running'), true);
  actions.keep(new Set(['running']));
  actions.keep([]);
  assert.equal(changes.length, 1, 'nothing more to drop, nothing announced');
  assert.equal(actions.state('running').pending, 'slow', 'a running action outlives its notice');
  gate.resolve();
  await running;
});

test('keep leaves the note and error of notices that are still there', async () => {
  const { actions, changes } = noticeActionsWith({ note: async () => '안내', fail: async () => { throw new Error('오류'); } });
  await actions.run({ id: 'noted' }, 'note');
  await actions.run({ id: 'failed' }, 'fail');
  changes.length = 0;
  actions.keep(['noted', 'failed', 'other']);
  assert.deepEqual(changes, []);
  assert.equal(actions.state('noted').note, '안내');
  assert.equal(actions.state('failed').error, '오류');
});

test('a result that lands after its notice went away is kept only until the owner next calls keep', async () => {
  const gate = deferred();
  const { actions } = noticeActionsWith({ update: () => gate.promise });
  const running = actions.run({ id: 'n1' }, 'update');
  actions.keep([]);
  gate.reject(new Error('늦은 실패'));
  await running;
  assert.equal(actions.state('n1').error, '늦은 실패');
  actions.keep([]);
  actions.keep(['n1']);
  assert.equal(actions.state('n1').error, '');
});

test('subscribers hear the start and the end once each and unsubscribing stops them', async () => {
  let calls = 0;
  const { actions } = noticeActionsWith({ update: async () => { calls += 1; } });
  const notice = { id: 'n1' };
  const heard = [];
  const listener = () => heard.push(actions.state('n1').pending);
  const unsubscribe = actions.subscribe(listener);
  actions.subscribe(listener);
  await actions.run(notice, 'update');
  assert.deepEqual(heard, ['update', null], 'the same listener subscribed twice hears each change once');
  assert.equal(unsubscribe(), true);
  assert.equal(unsubscribe(), false);
  heard.length = 0;
  await actions.run(notice, 'update');
  assert.deepEqual(heard, []);
  assert.equal(calls, 2);
});

test('a subscriber pressing the same notice while it starts cannot run it twice', async () => {
  const gate = deferred();
  let calls = 0;
  const { actions } = noticeActionsWith({ update: () => { calls += 1; return gate.promise; } });
  const notice = { id: 'n1' };
  let pressed = false;
  actions.subscribe(() => { if (!pressed) { pressed = true; void actions.run(notice, 'update'); } });
  const running = actions.run(notice, 'update');
  await settle();
  assert.equal(calls, 1);
  gate.resolve();
  await running;
});

// attentionNotices and integration notices

test('attentionNotices keeps only notices marked exactly true, in order, without touching its input', () => {
  assert.deepEqual(attentionNotices(), []);
  assert.deepEqual(attentionNotices(undefined), []);
  assert.deepEqual(attentionNotices(null), []);
  assert.deepEqual(attentionNotices([]), []);
  const input = [{ id: 'a', attention: true }, { id: 'b' }, { id: 'c', attention: 'true' }, { id: 'd', attention: 1 }, { id: 'e', attention: false }, { id: 'f', attention: true }];
  const result = attentionNotices(input);
  assert.deepEqual(result.map(notice => notice.id), ['a', 'f']);
  assert.equal(result[0], input[0], 'the same notice objects, not copies');
  assert.notEqual(result, input);
  assert.equal(input.length, 6);
});

test('every integration notice is an attention notice and comes out in a fixed order', () => {
  const notices = integrationNotices({
    agents: agents([
      { id: 'codex', state: 'update_required', action: 'update', packageVersion: '31' },
      { id: 'gemini', state: 'update_required', action: 'update', packageVersion: '1' },
      { id: 'claude', state: 'update_required', action: 'none', packageVersion: '44', detail: '번호가 같아 업데이트되지 않습니다.' },
    ]),
    calendar: calendarError(),
  });
  assert.deepEqual(notices.map(notice => notice.id), ['integration:claude:update:44', 'integration:codex:update:31', 'integration:google-calendar:reconnect']);
  assert.ok(notices.every(notice => notice.attention === true));
  assert.deepEqual(attentionNotices(notices), notices);
  const [claude, codex, calendar] = notices;
  assert.equal(claude.action, undefined);
  assert.equal(claude.detail, undefined);
  assert.equal(claude.message, '번호가 같아 업데이트되지 않습니다.');
  assert.equal(codex.action, 'update');
  assert.match(codex.detail, /ChatGPT/);
  assert.match(calendar.detail, /오늘 일정/);
  assert.deepEqual(integrationNotices({ agents: { available: true } }), []);
  assert.deepEqual(integrationNotices(), []);
});

test('the monitor republishes on a same-revision calendar change and goes quiet after disposal', () => {
  const published = [];
  const monitor = createIntegrationMonitor({ bridge: {}, onChange: notices => published.push(notices) });
  monitor.acceptCalendar(calendarError());
  assert.equal(published.length, 1);
  assert.equal(published[0][0].attention, true);
  monitor.acceptCalendar(calendarError());
  assert.equal(published.length, 1, 'an identical calendar is not a change');
  monitor.acceptCalendar(null);
  monitor.acceptAgents(null);
  assert.equal(published.length, 1);
  monitor.acceptCalendar(calendarError({ connected: true }));
  assert.deepEqual(published.at(-1), [], 'the same revision may confirm the reconnection');
  monitor.dispose();
  monitor.acceptCalendar(calendarError({ revision: 9 }));
  monitor.acceptAgents(agents([{ id: 'claude', state: 'update_required', action: 'update', packageVersion: '46' }]));
  assert.equal(published.length, 2);
});

// Home

test('home has no attention block without attention notices and drops it when the last one goes away', t => {
  const app = homeHarness(t);
  assert.equal(app.container.querySelector('.home-attention'), null);
  app.show([{ id: 'app-update:1', title: '앱 업데이트', message: '새 버전' }]);
  assert.equal(app.container.querySelector('.home-attention'), null, 'a notice without the attention mark stays in the list only');
  app.show([claudeUpdate]);
  assert.equal(app.sections().length, 1);
  const content = app.container.querySelector('.home-content');
  assert.equal(content.children[0].tagName, 'H1');
  assert.ok(content.children[1].classList.contains('home-attention'), 'the notice comes right after the date');
  app.show([]);
  assert.equal(app.container.querySelector('.home-attention'), null);
});

test('each attention notice on home gets its one fixing action: update when offered, otherwise settings', t => {
  const app = homeHarness(t, { notices: [claudeUpdate, codexBlocked, calendarReconnect] });
  assert.deepEqual(app.sections().map(node => node.dataset.noticeId), [claudeUpdate.id, codexBlocked.id, calendarReconnect.id]);
  const update = app.button(claudeUpdate.id);
  assert.equal(update.textContent, '업데이트');
  assert.equal(update.dataset.homeNoticeAction, 'update');
  assert.equal(update.classList.contains('primary'), false, 'recording keeps the one primary action on home');
  for (const notice of [codexBlocked, calendarReconnect]) {
    const button = app.button(notice.id);
    assert.equal(button.textContent, '설정 열기');
    assert.equal(button.dataset.homeNoticeAction, 'settings');
    assert.equal(button.classList.contains('primary'), false);
  }
  for (const node of app.sections()) assert.equal(node.querySelectorAll('button').length, 1, 'no close button on an attention notice');
  const copy = id => app.section(id).querySelector('.attention-notice-copy span').textContent;
  assert.equal(copy(claudeUpdate.id), claudeUpdate.detail, 'the detail explains what stops working');
  assert.equal(copy(codexBlocked.id), codexBlocked.message, 'without a detail the message is shown');
  assert.equal(copy(calendarReconnect.id), calendarReconnect.detail);
  assert.equal(app.section(claudeUpdate.id).querySelector('strong').textContent, 'Claude 연동');
});

test('home runs the pressed notice only, shows it in progress and ignores another press while it runs', async t => {
  const gate = deferred();
  const calls = [];
  const app = homeHarness(t, { notices: [claudeUpdate, calendarReconnect], handlers: {
    update: notice => { calls.push(['update', notice.id]); return gate.promise; },
    settings: async notice => { calls.push(['settings', notice.id]); },
  } });
  const stale = app.button(claudeUpdate.id);
  stale.click();
  assert.deepEqual(calls, [['update', claudeUpdate.id]]);
  const running = app.button(claudeUpdate.id);
  assert.equal(running.textContent, '업데이트 중');
  assert.equal(running.disabled, true);
  assert.ok(running.querySelector('.activity-spinner'), 'the running button carries the shared spinner');
  assert.equal(app.button(calendarReconnect.id).querySelector('.activity-spinner'), null);
  assert.equal(app.button(calendarReconnect.id).disabled, false, 'another notice stays usable');
  assert.equal(app.container.querySelector('[data-home-action="record"]').disabled, false, 'recording is not held by a notice');
  running.click();
  stale.click();
  await app.actions.run(claudeUpdate, 'update');
  assert.deepEqual(calls, [['update', claudeUpdate.id]]);
  gate.resolve();
  await settle();
  assert.equal(app.button(claudeUpdate.id).textContent, '업데이트');
  assert.equal(app.button(claudeUpdate.id).disabled, false);
  assert.equal(app.button(claudeUpdate.id).querySelector('.activity-spinner'), null);
});

test('a running settings action disables its button on home and keeps its label', async t => {
  const gate = deferred();
  const app = homeHarness(t, { notices: [calendarReconnect], handlers: { settings: () => gate.promise } });
  app.button(calendarReconnect.id).click();
  assert.equal(app.button(calendarReconnect.id).textContent, '설정 열기');
  assert.equal(app.button(calendarReconnect.id).disabled, true);
  gate.resolve();
  await settle();
  assert.equal(app.button(calendarReconnect.id).disabled, false);
});

test('a hidden home keeps its old buttons, but the shared guard still runs the action once', async t => {
  const gate = deferred();
  let calls = 0;
  const app = homeHarness(t, { notices: [claudeUpdate], handlers: { update: () => { calls += 1; return gate.promise; } } });
  app.container.hidden = true;
  const button = app.button(claudeUpdate.id);
  button.click();
  assert.equal(app.button(claudeUpdate.id), button, 'a hidden home does not render');
  assert.equal(button.disabled, false);
  button.click();
  assert.equal(calls, 1);
  gate.resolve();
  await settle();
});

test('a failure shows on the home notice as an alert and clears as soon as the next attempt starts', async t => {
  const second = deferred();
  let call = 0;
  const app = homeHarness(t, { notices: [claudeUpdate], handlers: { update: () => (call++ === 0 ? Promise.reject(new Error('업데이트하지 못했습니다.')) : second.promise) } });
  app.button(claudeUpdate.id).click();
  await settle();
  const alert = app.section(claudeUpdate.id).querySelector('.field-error[role="alert"]');
  assert.equal(alert.textContent, '업데이트하지 못했습니다.');
  assert.equal(app.button(claudeUpdate.id).disabled, false);
  app.button(claudeUpdate.id).click();
  assert.equal(app.section(claudeUpdate.id).querySelector('.field-error'), null);
  assert.equal(app.button(claudeUpdate.id).textContent, '업데이트 중');
  second.resolve('Claude Desktop에서 설치를 확인해 주세요.');
  await settle();
  assert.equal(app.section(claudeUpdate.id).querySelector('.attention-notice-status[role="status"]').textContent, 'Claude Desktop에서 설치를 확인해 주세요.');
  assert.equal(app.section(claudeUpdate.id).querySelector('.field-error'), null);
});

test('home renders notice titles, details, notes, errors and ids as text and still finds the notice by its id', async t => {
  const html = '"><img src=x onerror=alert(1)>';
  const notice = { ...claudeUpdate, id: `integration:${html}:update`, title: `<b>${html}</b>`, detail: `<script>bad()</script>${html}` };
  const calls = [];
  let next = Promise.resolve(`<img src=y>${html}`);
  const app = homeHarness(t, { notices: [notice], handlers: { update: received => { calls.push(received); return next; } } });
  const leaked = () => app.container.querySelectorAll('img, script, b').length;
  assert.equal(leaked(), 0);
  assert.equal(app.section(notice.id).querySelector('strong').textContent, notice.title);
  assert.equal(app.section(notice.id).querySelector('.attention-notice-copy span').textContent, notice.detail);
  app.button(notice.id).click();
  await settle();
  assert.equal(calls.length, 1);
  assert.equal(calls[0], notice);
  assert.equal(app.section(notice.id).querySelector('[role="status"]').textContent, `<img src=y>${html}`);
  next = Promise.reject(new Error(`<script>bad()</script>`));
  app.button(notice.id).click();
  await settle();
  assert.equal(app.section(notice.id).querySelector('[role="alert"]').textContent, '<script>bad()</script>');
  assert.equal(leaked(), 0);
});

test('a home button left from an earlier render does nothing once its notice is gone or replaced', async t => {
  const calls = [];
  const app = homeHarness(t, { notices: [claudeUpdate], handlers: { update: async notice => { calls.push(notice.id); } } });
  const old = app.button(claudeUpdate.id);
  app.state.current = [];
  old.click();
  app.state.current = [{ ...claudeUpdate, id: 'integration:claude:update:46' }];
  old.click();
  await settle();
  assert.deepEqual(calls, []);
});

test('home without an onNotice owner ignores a notice press', t => {
  const app = homeHarness(t, { notices: [claudeUpdate], onNotice: undefined });
  assert.doesNotThrow(() => app.button(claudeUpdate.id).click());
});

test('keyboard focus returns to the home notice button after its action finishes', async t => {
  const gate = deferred();
  const app = homeHarness(t, { notices: [claudeUpdate], handlers: { update: () => gate.promise } });
  app.button(claudeUpdate.id).focus();
  app.button(claudeUpdate.id).click();
  gate.reject(new Error('업데이트하지 못했습니다.'));
  await settle();
  const active = app.window.document.activeElement;
  assert.equal(active?.dataset?.homeNotice, claudeUpdate.id, `a keyboard user can retry without searching for the button again; focus is on ${active?.tagName}`);
});

// Home and the notification list together

test('an update started on home shows as in progress in the notification list, which cannot start it again', async t => {
  const gate = deferred();
  let calls = 0;
  const app = appHarness(t, { notices: [claudeUpdate], handlers: { update: () => { calls += 1; return gate.promise; } } });
  assert.equal(app.listButton(claudeUpdate.id, 'update').textContent, '업데이트');
  app.homeButton(claudeUpdate.id).click();
  const listUpdate = app.listButton(claudeUpdate.id, 'update');
  assert.equal(listUpdate.textContent, '업데이트 중');
  assert.equal(listUpdate.disabled, true);
  assert.equal(app.listButton(claudeUpdate.id, 'ack').disabled, true, 'closing waits for the running update');
  assert.equal(app.listRow(claudeUpdate.id).getAttribute('aria-busy'), 'true');
  listUpdate.click();
  assert.equal(calls, 1);
  gate.resolve();
  await settle();
  assert.equal(app.listButton(claudeUpdate.id, 'update').textContent, '업데이트');
  assert.equal(app.listButton(claudeUpdate.id, 'update').disabled, false);
  assert.equal(app.listRow(claudeUpdate.id).getAttribute('aria-busy'), 'false');
});

test('pressing update in the list and on home at once runs the update once', async t => {
  const gate = deferred();
  let calls = 0;
  const app = appHarness(t, { notices: [claudeUpdate], handlers: { update: () => { calls += 1; return gate.promise; } } });
  const home = app.homeButton(claudeUpdate.id), list = app.listButton(claudeUpdate.id, 'update');
  list.click();
  home.click();
  app.homeButton(claudeUpdate.id).click();
  await app.actions.run(claudeUpdate, 'update');
  assert.equal(calls, 1);
  assert.equal(app.homeButton(claudeUpdate.id).textContent, '업데이트 중');
  assert.equal(app.homeButton(claudeUpdate.id).disabled, true);
  gate.resolve();
  await settle();
  assert.equal(app.homeButton(claudeUpdate.id).disabled, false);
});

test('a failure started on home shows beside the notice on both screens and is not raised again app-wide', async t => {
  const app = appHarness(t, { notices: [claudeUpdate], handlers: { update: async () => { throw new Error('업데이트하지 못했습니다.'); } } });
  app.homeButton(claudeUpdate.id).click();
  await settle();
  assert.equal(app.homeSection(claudeUpdate.id).querySelector('[role="alert"]').textContent, '업데이트하지 못했습니다.');
  assert.equal(app.listRow(claudeUpdate.id).querySelector('.field-error[role="alert"]').textContent, '업데이트하지 못했습니다.');
  assert.deepEqual(app.shown, []);
});

test('a note from the list update shows on home too and both forget it when the notice leaves and returns', async t => {
  const app = appHarness(t, { notices: [claudeUpdate], handlers: { update: async () => 'Claude Desktop에서 설치를 확인해 주세요.' } });
  app.listButton(claudeUpdate.id, 'update').click();
  await settle();
  assert.equal(app.homeSection(claudeUpdate.id).querySelector('[role="status"]').textContent, 'Claude Desktop에서 설치를 확인해 주세요.');
  assert.equal(app.listRow(claudeUpdate.id).querySelector('.notification-status').textContent, 'Claude Desktop에서 설치를 확인해 주세요.');
  app.show([]);
  assert.equal(app.homeContainer.querySelector('.home-attention'), null);
  assert.equal(app.listRow(claudeUpdate.id), undefined);
  app.show([claudeUpdate]);
  assert.equal(app.homeSection(claudeUpdate.id).querySelector('[role="status"]'), null);
  assert.equal(app.listRow(claudeUpdate.id).querySelector('.notification-status'), null);
});

test('a notice that leaves while its update runs returns still in progress, and a late failure does not resurface', async t => {
  const gate = deferred();
  const app = appHarness(t, { notices: [claudeUpdate], handlers: { update: () => gate.promise } });
  app.homeButton(claudeUpdate.id).click();
  app.show([]);
  assert.equal(app.homeContainer.querySelector('.home-attention'), null);
  assert.equal(app.actions.busy(claudeUpdate.id), true);
  app.show([claudeUpdate]);
  assert.equal(app.homeButton(claudeUpdate.id).textContent, '업데이트 중');
  assert.equal(app.homeButton(claudeUpdate.id).disabled, true);
  assert.equal(app.listButton(claudeUpdate.id, 'update').disabled, true);
  app.show([]);
  gate.reject(new Error('늦은 실패'));
  await settle();
  assert.deepEqual(app.shown, []);
  app.show([claudeUpdate]);
  assert.equal(app.homeSection(claudeUpdate.id).querySelector('[role="alert"]'), null);
  assert.equal(app.listRow(claudeUpdate.id).querySelector('.field-error'), null);
  assert.equal(app.homeButton(claudeUpdate.id).disabled, false);
  assert.equal(app.listButton(claudeUpdate.id, 'update').textContent, '업데이트');
});

test('closing a notice in the list leaves the attention notice on home', async t => {
  const app = appHarness(t, { notices: [calendarReconnect] });
  assert.equal(app.list.counts().unread, 1);
  app.listButton(calendarReconnect.id, 'ack').click();
  await settle();
  assert.equal(app.list.counts().unread, 0);
  assert.ok(app.homeSection(calendarReconnect.id));
  assert.equal(app.homeButton(calendarReconnect.id).disabled, false);
});

test('a destroyed notification list stops following actions started on home', async t => {
  const gate = deferred();
  const app = appHarness(t, { notices: [claudeUpdate], handlers: { update: () => gate.promise } });
  app.list.destroy();
  const before = app.listContainer.innerHTML;
  app.homeButton(claudeUpdate.id).click();
  gate.resolve('안내');
  await settle();
  assert.equal(app.listContainer.innerHTML, before);
  assert.equal(app.homeSection(claudeUpdate.id).querySelector('[role="status"]').textContent, '안내');
});

test('keyboard focus returns to the list update button after the update finishes', async t => {
  const gate = deferred();
  const app = appHarness(t, { notices: [claudeUpdate], handlers: { update: () => gate.promise } });
  app.listButton(claudeUpdate.id, 'update').focus();
  app.listButton(claudeUpdate.id, 'update').click();
  gate.reject(new Error('업데이트하지 못했습니다.'));
  await settle();
  assert.equal(app.window.document.activeElement === app.listButton(claudeUpdate.id, 'update'), true);
});

// Rail dot

test('the rail dot names itself once however often it is set and clears back to the plain name', t => {
  const app = railHarness(t);
  const home = app.preview('home');
  for (let index = 0; index < 3; index += 1) app.navigation.attention('home', true);
  assert.equal(home.dataset.attention, 'true');
  assert.equal(home.getAttribute('aria-label'), `홈, ${phrase}`);
  app.navigation.attention('home', false);
  app.navigation.attention('home', false);
  assert.equal(home.hasAttribute('data-attention'), false);
  assert.equal(home.getAttribute('aria-label'), '홈');
  app.navigation.attention('home', true);
  assert.equal(home.getAttribute('aria-label'), `홈, ${phrase}`);
  assert.equal(home.title, '홈');
  assert.equal(app.preview('all').getAttribute('aria-label'), '전체 회의');
  assert.equal(app.preview('all').hasAttribute('data-attention'), false);
});

test('an unknown rail view or a missing state changes nothing it should not', t => {
  const app = railHarness(t);
  const labels = () => ['home', 'all', 'trash'].map(view => app.preview(view).getAttribute('aria-label'));
  assert.doesNotThrow(() => app.navigation.attention('missing', true));
  assert.doesNotThrow(() => app.navigation.attention(undefined, true));
  assert.deepEqual(labels(), ['홈', '전체 회의', '휴지통']);
  assert.equal(app.document.querySelectorAll('[data-attention]').length, 0);
  app.navigation.attention('home', true);
  app.navigation.attention('home');
  assert.equal(app.preview('home').hasAttribute('data-attention'), false, 'no state reads as no attention');
  assert.equal(app.preview('home').getAttribute('aria-label'), '홈');
});

test('the rail dot survives the sidebar being pinned and unpinned', t => {
  const app = railHarness(t);
  app.navigation.attention('trash', true);
  const toggle = app.document.getElementById('navigation-toggle');
  toggle.click();
  toggle.click();
  assert.equal(app.preview('trash').dataset.attention, 'true');
  assert.equal(app.preview('trash').getAttribute('aria-label'), `휴지통, ${phrase}`);
});

test('a rail button named by its text keeps its name and does not keep or repeat the attention phrase', t => {
  const app = railHarness(t, { labelled: false });
  const home = app.preview('home');
  app.navigation.attention('home', true);
  app.navigation.attention('home', true);
  assert.equal(count(app.name(home), phrase), 1, `repeated: ${app.name(home)}`);
  assert.match(app.name(home), /홈/, 'the button keeps its own name');
  app.navigation.attention('home', false);
  assert.equal(count(app.name(home), phrase), 0, `after clearing: ${app.name(home)}`);
});
