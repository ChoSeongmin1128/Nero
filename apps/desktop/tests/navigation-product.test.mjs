import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { Window } from 'happy-dom';
import { mountLibrary } from '../src/library-ui.js';

const settle = () => new Promise(resolve => setImmediate(resolve));
async function setup(t, preferences = {}) {
  const window = new Window({ url: 'http://nero.test', settings: { disableCSSFileLoading: true, disableJavaScriptFileLoading: true } });
  t.after(() => window.happyDOM.abort());
  const document = window.document;
  document.write(await readFile(new URL('../src/index.html', import.meta.url), 'utf8'));
  for (const [key, value] of Object.entries(preferences)) window.localStorage.setItem(key, value);
  const state = { connected: true, libraryOpen: true, libraryFilter: 'all', librarySelection: new Set(), snapshot: { meetings: [
    { id: 'filed', title: '폴더 회의', folderId: 'project', createdAt: 1 },
    { id: 'unfiled', title: '미분류 회의', createdAt: 2 },
    { id: 'deleted', title: '삭제한 회의', folderId: 'project', createdAt: 3, deletedAt: 4 },
  ], folders: [{ id: 'project', name: '프로젝트' }] } };
  const escape = value => String(value ?? '').replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);
  let library;
  library = mountLibrary({ bridge: {}, getState: () => state, byId: id => document.getElementById(id), finishEditing: async action => action(), acceptSnapshot() {}, selectMeeting(id) { state.selectedId = id; state.libraryOpen = false; library.render(); }, render() { library.render(); }, showError(error) { throw error; }, escape, icon: name => `<svg data-icon="${name}"></svg>` });
  library.render();
  return { window, document, state, library };
}

test('product shares hover, pin and close navigation without hiding the meeting content', async t => {
  const { window, document } = await setup(t);
  const shell = document.querySelector('.workspace'), trigger = document.getElementById('navigation-toggle');
  document.querySelector('.nav-hover-edge').dispatchEvent(new window.PointerEvent('pointerenter', { pointerType: 'mouse' }));
  assert.equal(shell.classList.contains('nav-peek'), true);
  assert.equal(trigger.parentElement.className, 'navigation-brand');
  assert.equal(document.querySelector('main').inert, false);
  trigger.click();
  assert.equal(window.localStorage.getItem('nero.sidebar.pinned'), 'true');
  trigger.click();
  assert.equal(shell.classList.contains('collapsed'), true);
  assert.equal(trigger.parentElement.className, 'navigation-toolbar');
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
  document.querySelector('.nav-hover-edge').dispatchEvent(new window.PointerEvent('pointerenter', { pointerType: 'mouse' }));
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
