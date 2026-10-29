import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { Window } from 'happy-dom';
import { mountLibrary, renderMeetingMenu } from '../src/library-ui.js';

const settle = () => new Promise(resolve => setImmediate(resolve));
async function setup(t, preferences = {}) {
  const window = new Window({ url: 'http://nero.test', settings: { disableCSSFileLoading: true, disableJavaScriptFileLoading: true } });
  t.after(() => window.happyDOM.abort());
  const document = window.document;
  document.write(await readFile(new URL('../src/index.html', import.meta.url), 'utf8'));
  for (const [key, value] of Object.entries(preferences)) window.localStorage.setItem(key, value);
  const state = { connected: true, libraryOpen: true, libraryFilter: 'all', librarySelection: new Set(), snapshot: { meetings: [
    { id: 'filed', title: '폴더 회의', folderId: 'project', createdAt: 1, audio: { frames: 96000, sampleRate: 48000 } },
    { id: 'unfiled', title: '미분류 회의', createdAt: 2 },
    { id: 'deleted', title: '삭제한 회의', folderId: 'project', createdAt: 3, deletedAt: 4 },
  ], folders: [{ id: 'project', name: '프로젝트' }] } };
  const escape = value => String(value ?? '').replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);
  let library;
  library = mountLibrary({ bridge: {}, duration: milliseconds => `${milliseconds / 1000}초`, getState: () => state, byId: id => document.getElementById(id), finishEditing: async action => action(), acceptSnapshot() {}, selectMeeting(id) { state.selectedId = id; state.libraryOpen = false; library.render(); }, render() { library.render(); }, showError(error) { throw error; }, escape, icon: name => `<svg data-icon="${name}"></svg>` });
  library.render();
  return { window, document, state, library };
}

test('product shares hover, pin and close navigation without hiding the meeting content', async t => {
  const { window, document } = await setup(t);
  const shell = document.querySelector('.workspace'), trigger = document.getElementById('navigation-toggle');
  document.querySelector('.navigation-rail').dispatchEvent(new window.PointerEvent('pointerenter', { pointerType: 'mouse' }));
  assert.equal(shell.classList.contains('nav-peek'), true);
  assert.equal(trigger.parentElement.className, 'navigation-brand');
  assert.equal(document.querySelector('.navigation-rail').inert, true);
  assert.equal(document.querySelector('main').inert, false);
  trigger.click();
  assert.equal(window.localStorage.getItem('nero.sidebar.pinned'), 'true');
  trigger.click();
  assert.equal(shell.classList.contains('collapsed'), true);
  assert.equal(trigger.parentElement.className, 'navigation-rail-head');
  assert.equal(document.querySelector('.navigation-rail').inert, false);
  assert.equal(document.querySelectorAll('#navigation-toggle').length, 1);
});

test('folder overview, all view and actual-parent breadcrumbs retain real library scope', async t => {
  const { document, state, library, window } = await setup(t);
  assert.deepEqual(library.visibleMeetings().map(meeting => meeting.id), ['unfiled']);
  assert.equal(document.querySelector('#library-folders button strong').textContent, '프로젝트');
  assert.equal(document.getElementById('library-count').textContent, '2개');
  document.querySelector('[data-library-view="all"]').click();
  assert.equal(window.localStorage.getItem('nero.library.view'), 'all');
  assert.deepEqual(library.visibleMeetings().map(meeting => meeting.id), ['unfiled', 'filed']);
  state.librarySelection.add('filed');
  document.querySelector('[data-library-view="folders"]').click();
  assert.equal(state.librarySelection.size, 0);
  document.querySelector('#library-folders button').click(); await settle();
  assert.equal(state.libraryFilter, 'project');
  assert.deepEqual(library.visibleMeetings().map(meeting => meeting.id), ['filed']);
  document.querySelector('[data-recent-meeting="unfiled"]').click();
  assert.match(document.getElementById('navigation-breadcrumbs').textContent, /회의 목록.*미분류.*미분류 회의/);
  document.querySelector('#navigation-breadcrumbs [data-library-filter="unfiled"]').click(); await settle();
  assert.equal(state.libraryFilter, 'unfiled');
  document.querySelector('#navigation-breadcrumbs [data-library-filter="all"]').click(); await settle();
  assert.equal(state.libraryView, 'folders');
});

test('content searches span filed meetings and remembered recent disclosure remains independent', async t => {
  const { document, library, window } = await setup(t, { 'nero.library.recents': 'false' });
  const recent = document.getElementById('recent-section');
  assert.equal(recent.open, false);
  recent.open = true; recent.dispatchEvent(new window.Event('toggle'));
  assert.equal(window.localStorage.getItem('nero.library.recents'), 'true');
  document.getElementById('search').value = '폴더 회의'; library.render();
  assert.deepEqual(library.visibleMeetings().map(meeting => meeting.id), ['filed']);
  assert.equal(document.getElementById('library-folders').hidden, true);
});

test('title and playback have one shared owner outside the narrow inspector replacement', async t => {
  const { document } = await setup(t);
  const common = document.querySelector('.detail-common');
  assert.equal(common.contains(document.getElementById('meeting-title')), true);
  assert.equal(common.contains(document.getElementById('audio-player')), true);
  assert.equal(document.querySelectorAll('#audio-player').length, 1);
  assert.equal(common.contains(document.getElementById('detail-transcript-open')), true);
  assert.equal(document.querySelector('.detail-primary').contains(document.getElementById('summary-section')), true);
  assert.equal(document.querySelector('.detail-inspector').contains(document.getElementById('transcript-section')), true);
  assert.equal(document.getElementById('audio-speed').selectedOptions[0].textContent, '1x');
});


test('initial setup suppresses navigation while preserving the saved pin preference', async t => {
  const { window, document, library } = await setup(t, { 'nero.sidebar.pinned': 'true' });
  const shell = document.querySelector('.workspace'), trigger = document.getElementById('navigation-toggle');
  shell.dataset.onboarding = 'true'; library.render();
  document.querySelector('.navigation-rail').dispatchEvent(new window.PointerEvent('pointerenter', { pointerType: 'mouse' }));
  trigger.click();
  document.dispatchEvent(new window.KeyboardEvent('keydown', { code: 'Backslash', ctrlKey: true, bubbles: true }));
  assert.equal(trigger.getAttribute('aria-expanded'), 'false');
  assert.equal(document.getElementById('navigation-sidebar').inert, true);
  assert.equal(window.localStorage.getItem('nero.sidebar.pinned'), 'true');
  shell.dataset.onboarding = 'false'; library.render();
  assert.equal(trigger.getAttribute('aria-expanded'), 'true');
  assert.equal(document.getElementById('navigation-sidebar').inert, false);
  assert.equal(shell.classList.contains('nav-pinned'), true);
});


test('recent rows stay compact, show folder/time/length and use the shared meeting actions', async t => {
  const { window, document, state, library } = await setup(t, { 'nero.sidebar.pinned': 'true' });
  const row = document.querySelector('[data-recent-row="filed"]');
  assert.equal(row.querySelectorAll('strong').length, 1);
  assert.match(row.querySelector('.recent-meeting-meta').textContent, /프로젝트/);
  assert.equal(row.querySelector('.recent-meeting-duration').textContent, '2초');
  assert.equal(row.querySelector('.recent-meeting-meta').children.length, 2);
  assert.deepEqual([...row.querySelectorAll('[data-library-action]')].map(button => button.dataset.libraryAction), ['rename', 'move', 'trash']);
  assert.equal(row.querySelector('svg[data-icon="audio-lines"]'), null);
  row.dispatchEvent(new window.MouseEvent('contextmenu', { bubbles: true, cancelable: true, button: 2 }));
  const menu = row.querySelector('details');
  assert.equal(menu.open, true);
  assert.equal(document.activeElement.dataset.libraryAction, 'rename');
  document.activeElement.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, cancelable: true }));
  assert.equal(document.activeElement.dataset.libraryAction, 'move');
  document.activeElement.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'End', bubbles: true, cancelable: true }));
  assert.equal(document.activeElement.dataset.libraryAction, 'trash');
  document.activeElement.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
  assert.equal(menu.open, false);
  assert.equal(document.activeElement, menu.querySelector('summary'));
  state.snapshot.folders[0].name = '변경한 폴더'; library.render();
  assert.match(document.querySelector('[data-recent-row="filed"] .recent-meeting-meta').textContent, /변경한 폴더/);
});

test('recent context actions target their row while preserving the current meeting and return focus', async t => {
  const { window, document, state, library } = await setup(t, { 'nero.sidebar.pinned': 'true' });
  state.selectedId = 'filed'; state.libraryOpen = false; library.render();
  const row = document.querySelector('[data-recent-row="unfiled"]');
  const open = () => row.dispatchEvent(new window.MouseEvent('contextmenu', { bubbles: true, cancelable: true, button: 2 }));
  open(); row.querySelector('[data-library-action="rename"]').click();
  assert.equal(document.getElementById('library-dialog').open, true);
  assert.equal(document.getElementById('library-meeting-name').value, '미분류 회의');
  assert.equal(state.selectedId, 'filed');
  document.querySelector('[data-library-close]').click();
  assert.equal(document.activeElement, row.querySelector('summary'));
  row.querySelector('[data-recent-meeting]').focus();
  document.activeElement.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'F10', shiftKey: true, bubbles: true, cancelable: true }));
  assert.equal(row.querySelector('details').open, true);
  row.querySelector('[data-library-action="move"]').click();
  assert.equal(document.getElementById('library-dialog-title').textContent, '폴더로 이동');
  document.querySelector('[data-library-close]').click();
  open(); row.querySelector('[data-library-action="trash"]').click();
  assert.equal(document.getElementById('library-dialog-title').textContent, '휴지통으로 이동할까요?');
  assert.match(document.getElementById('library-dialog-body').textContent, /미분류 회의/);
  assert.equal(state.selectedId, 'filed');
});

test('split new meeting keeps the primary action direct and calendar/import separate', async t => {
  const { window, document } = await setup(t);
  let created = 0, fromCalendar = 0, openedSettings = 0;
  const primary = document.getElementById('create-meeting'), calendar = document.getElementById('create-calendar-meeting');
  primary.disabled = false; calendar.disabled = false;
  primary.addEventListener('click', () => created++);
  calendar.addEventListener('click', () => fromCalendar++);
  document.getElementById('settings').addEventListener('click', () => openedSettings++);
  primary.click(); assert.equal(created, 1); assert.equal(fromCalendar, 0);
  const trigger = document.getElementById('create-meeting-options'), menu = document.getElementById('new-meeting-menu');
  document.querySelector('.navigation-rail').dispatchEvent(new window.PointerEvent('pointerenter', { pointerType: 'mouse' }));
  trigger.click(); await settle(); assert.equal(menu.open, false);
  trigger.setAttribute('aria-disabled', 'false');
  trigger.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, cancelable: true }));
  assert.equal(menu.open, true); assert.equal(document.activeElement, calendar);
  calendar.click(); assert.equal(fromCalendar, 1); assert.equal(menu.open, false); assert.equal(created, 1);
  assert.equal(menu.contains(document.getElementById('import-audio')), false);
  document.getElementById('rail-new-meeting').click(); assert.equal(created, 2);
  document.getElementById('rail-settings').click(); assert.equal(openedSettings, 1);
  assert.equal(document.querySelectorAll('.navigation-rail [data-recent-meeting]').length, 0);
});


test('closing a recent-action dialog returns to the rail after temporary navigation closes', async t => {
  const { window, document } = await setup(t);
  document.querySelector('.navigation-rail').dispatchEvent(new window.PointerEvent('pointerenter', { pointerType: 'mouse' }));
  const row = document.querySelector('[data-recent-row="unfiled"]');
  row.dispatchEvent(new window.MouseEvent('contextmenu', { bubbles: true, cancelable: true, button: 2 }));
  row.querySelector('[data-library-action="rename"]').click();
  document.getElementById('library-meeting-name').dispatchEvent(new window.PointerEvent('pointerdown', { bubbles: true }));
  document.querySelector('[data-library-close]').click();
  assert.equal(document.activeElement, document.getElementById('navigation-toggle'));
});


test('main-list actions close their menu before opening a dialog and restore the visible summary', async t => {
  const { window, document, state } = await setup(t);
  const meeting = state.snapshot.meetings.find(meeting => meeting.id === 'filed');
  document.getElementById('meeting-list').innerHTML = `<article data-meeting-id="filed">${renderMeetingMenu(meeting, { escape: value => String(value), icon: name => `<svg data-icon="${name}"></svg>` })}</article>`;
  const row = document.querySelector('#meeting-list [data-meeting-id="filed"]'), menu = row.querySelector('details'), summary = menu.querySelector('summary');
  for (const [action, title] of [['rename', '회의 이름 변경'], ['move', '폴더로 이동'], ['trash', '휴지통으로 이동할까요?']]) {
    row.dispatchEvent(new window.MouseEvent('contextmenu', { bubbles: true, cancelable: true, button: 2 }));
    const command = row.querySelector(`[data-library-action="${action}"]`);
    command.focus(); command.click();
    assert.equal(menu.open, false);
    assert.equal(document.getElementById('library-dialog').open, true);
    assert.equal(document.getElementById('library-dialog-title').textContent, title);
    document.querySelector('[data-library-close]').click();
    assert.equal(document.activeElement, summary, `${action} returns to the visible menu trigger`);
    assert.equal(menu.open, false);
  }
});


for (const popup of [
  '<section role="dialog" data-slot="dialog-content" data-open><button>모달 작업</button></section>',
  '<section role="alertdialog" data-open><button>확인 작업</button></section>',
  '<section role="dialog" data-slot="popover-content" data-open><button>팝업 작업</button></section>',
  '<section role="listbox" data-slot="select-content" data-open><button>선택 작업</button></section>',
]) test(`navigation shortcuts defer to the active common popup: ${popup}`, async t => {
  const { window, document, library } = await setup(t, { 'nero.sidebar.pinned': 'true' });
  const portal = document.createElement('div'); portal.innerHTML = popup; document.body.append(portal);
  const button = portal.querySelector('button'); button.focus();
  const shell = document.querySelector('.workspace'), content = document.querySelector('.main-content');
  content.inert = true;
  for (const modifiers of [{ ctrlKey: true }, { metaKey: true }]) {
    button.dispatchEvent(new window.KeyboardEvent('keydown', { code: 'Backslash', bubbles: true, cancelable: true, ...modifiers }));
    assert.equal(window.localStorage.getItem('nero.sidebar.pinned'), 'true');
    assert.equal(shell.classList.contains('nav-pinned'), true);
    assert.equal(document.activeElement, button);
  }
  shell.dataset.onboarding = 'true'; library.render();
  shell.dataset.onboarding = 'false'; library.render();
  assert.equal(content.inert, true);
  portal.remove(); content.inert = false;
  document.dispatchEvent(new window.KeyboardEvent('keydown', { code: 'Backslash', ctrlKey: true, bubbles: true, cancelable: true }));
  assert.equal(window.localStorage.getItem('nero.sidebar.pinned'), 'false');
});

test('open shared menus retain shortcut priority and Escape closes only the menu', async t => {
  const { window, document } = await setup(t);
  document.querySelector('.navigation-rail').dispatchEvent(new window.PointerEvent('pointerenter', { pointerType: 'mouse' }));
  const row = document.querySelector('[data-recent-row="filed"]');
  row.dispatchEvent(new window.MouseEvent('contextmenu', { bubbles: true, cancelable: true, button: 2 }));
  const menu = row.querySelector('details'), button = menu.querySelector('button');
  button.dispatchEvent(new window.KeyboardEvent('keydown', { code: 'Backslash', ctrlKey: true, bubbles: true, cancelable: true }));
  assert.equal(window.localStorage.getItem('nero.sidebar.pinned'), null);
  assert.equal(document.activeElement, button);
  assert.equal(menu.open, true);
  button.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
  assert.equal(menu.open, false);
  assert.equal(document.querySelector('.workspace').classList.contains('nav-peek'), true);
});
