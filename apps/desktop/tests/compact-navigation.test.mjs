import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { Window } from 'happy-dom';
import { mountNavigationSidebar } from '../../../design-system/navigation.mjs';
import { mountLibrary } from '../src/library-ui.js';
import { createRequire } from 'node:module';
globalThis.NeroUI ??= createRequire(import.meta.url)('../../../design-system/components.js');
import { mountSidebarViews } from '../src/sidebar-views.js';

function setup(t, preferences = {}) {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const window = new Window({ url: 'http://nero.test' });
  t.after(() => window.happyDOM.abort());
  const document = window.document;
  document.body.innerHTML = `<header class="application-toolbar"><button id="navigation-toggle">탐색</button></header><div class="navigation-shell">
    <nav class="navigation-rail" data-navigation-compact>${['home', 'all', 'unfiled', 'folders', 'recent'].map(view => `<button data-navigation-preview="${view}">${view}</button>`).join('')}<div class="navigation-rail-footer"><button id="update" hidden>업데이트</button><button id="notifications">알림</button><button data-navigation-preview="trash">trash</button><button id="settings">설정</button></div></nav>
    <div class="navigation-edge"></div>
    <aside id="navigation-sidebar"><header id="navigation-header"></header><input id="search"><button id="meeting">회의</button><details class="recent-section" open><summary>최근 회의</summary></details></aside>
    <main><textarea id="notes">작성 중</textarea></main>
  </div>`;
  for (const [key, value] of Object.entries(preferences)) window.localStorage.setItem(key, value);
  const el = id => document.getElementById(id);
  const shell = document.querySelector('.navigation-shell'), aside = el('navigation-sidebar'), compact = document.querySelector('[data-navigation-compact]');
  shell.getBoundingClientRect = () => ({ width: 640 });
  const events = [], states = [];
  aside.addEventListener('nero:sidebar-preview', event => { events.push({ view: event.detail.view, expanded: !aside.inert }); aside.dataset.previewView = event.detail.view; });
  aside.addEventListener('nero:navigation-state', event => states.push(event.detail));
  const navigation = mountNavigationSidebar({ shell, aside, content: document.querySelector('main'), trigger: el('navigation-toggle'), storage: window.localStorage });
  const preview = view => compact.querySelector(`[data-navigation-preview="${view}"]`);
  const pointer = (node, type, pointerType = 'mouse') => node.dispatchEvent(new window.PointerEvent(type, { pointerType }));
  const escape = node => node.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
  return { window, document, el, shell, aside, compact, navigation, events, states, preview, pointer, escape, tick: milliseconds => t.mock.timers.tick(milliseconds) };
}

test('resting on a rail button shows its list, passing over one does not, and the toggle and edge reveal the sidebar', t => {
  const app = setup(t);
  app.el('notes').focus();
  for (const view of ['home', 'all', 'folders', 'trash']) {
    app.pointer(app.preview(view), 'pointerenter');
    app.tick(100);
    app.pointer(app.preview(view), 'pointerleave');
  }
  app.pointer(app.preview('all'), 'pointerenter', 'touch');
  app.tick(500);
  assert.equal(app.aside.inert, true);
  assert.deepEqual(app.events, []);
  app.pointer(app.preview('all'), 'pointerenter');
  app.tick(140);
  assert.equal(app.aside.inert, false);
  app.pointer(app.preview('all'), 'pointerleave');
  app.pointer(app.preview('trash'), 'pointerenter');
  app.tick(140);
  assert.deepEqual(app.events, [{ view: 'all', expanded: false }, { view: 'trash', expanded: true }]);
  assert.equal(app.document.activeElement, app.el('notes'), 'hovering keeps the writing focus');
  app.pointer(app.preview('trash'), 'pointerleave');
  app.pointer(app.compact, 'pointerleave');
  app.tick(500);
  assert.equal(app.aside.inert, true);
  app.events.length = 0;
  const toggle = app.el('navigation-toggle');
  app.pointer(toggle, 'pointerenter', 'touch');
  app.tick(500);
  assert.equal(app.aside.inert, true);
  app.pointer(toggle, 'pointerenter');
  app.tick(139);
  assert.equal(app.aside.inert, true);
  app.tick(1);
  assert.equal(app.aside.inert, false);
  assert.equal(app.shell.classList.contains('nav-peek'), true);
  assert.deepEqual(app.events, [], 'revealing does not choose a different list');
  app.pointer(toggle, 'pointerleave');
  app.tick(100);
  app.pointer(app.aside, 'pointerenter');
  app.tick(180);
  assert.equal(app.aside.inert, false);
  app.pointer(app.aside, 'pointerleave');
  app.tick(179);
  assert.equal(app.aside.inert, false);
  app.tick(1);
  assert.equal(app.aside.inert, true);
  app.pointer(toggle, 'pointerenter');
  app.tick(100);
  app.pointer(toggle, 'pointerleave');
  app.tick(500);
  assert.equal(app.aside.inert, true, 'passing over the toggle does not open the sidebar');
  const edge = app.document.querySelector('.navigation-edge');
  app.pointer(edge, 'pointerenter');
  assert.equal(app.aside.inert, false);
  app.pointer(edge, 'pointerleave');
  app.tick(180);
  assert.equal(app.aside.inert, true);
  assert.equal(app.document.activeElement, app.el('notes'));
  assert.equal(app.window.localStorage.getItem('nero.sidebar.pinned'), null);
});

test('rail clicks switch every list while retaining content, browser history and preferences', t => {
  const app = setup(t), notes = app.el('notes'), main = app.document.querySelector('main');
  const historyLength = app.window.history.length;
  notes.value = '저장 전 메모';
  const views = ['home', 'all', 'unfiled', 'folders', 'recent', 'trash'];
  for (const view of views) {
    app.preview(view).click();
    assert.equal(app.aside.dataset.previewView, view);
    assert.equal(app.aside.inert, false);
  }
  assert.deepEqual(app.events.map(event => event.view), views);
  assert.equal(app.el('notes'), notes);
  assert.equal(notes.value, '저장 전 메모');
  assert.equal(main.hidden, false);
  assert.equal(main.inert, false);
  assert.equal(app.window.history.length, historyLength);
  assert.equal(app.window.localStorage.getItem('nero.library.view'), null);
  assert.equal(app.window.localStorage.getItem('nero.sidebar.pinned'), null);
});

test('focus alone opens nothing, a click reveals immediately and keyboard focus prevents automatic hiding', t => {
  const app = setup(t);
  app.preview('folders').focus();
  assert.equal(app.aside.inert, true);
  assert.deepEqual(app.events, []);
  app.preview('recent').click();
  assert.equal(app.aside.dataset.previewView, 'recent');
  assert.equal(app.aside.inert, false);
  app.pointer(app.compact, 'pointerleave');
  app.tick(180);
  assert.equal(app.aside.inert, false);
  app.el('meeting').focus();
  app.pointer(app.aside, 'pointerleave');
  app.tick(180);
  assert.equal(app.aside.inert, false);
  app.escape(app.el('meeting'));
  assert.equal(app.aside.inert, true);
  assert.equal(app.document.activeElement, app.preview('recent'));
  assert.equal(app.events.length, 1);
});

test('input and opened menus retain the overlay, and Escape gives the menu priority', t => {
  const app = setup(t);
  app.preview('all').click();
  app.el('search').focus();
  app.pointer(app.aside, 'pointerleave');
  app.tick(180);
  assert.equal(app.aside.inert, false);
  app.el('search').blur();
  const menu = app.document.createElement('details');
  menu.innerHTML = '<summary>더보기</summary><div class="action-menu"><button>이름 변경</button></div>';
  menu.open = true;
  app.aside.append(menu);
  app.pointer(app.aside, 'pointerleave');
  app.tick(180);
  assert.equal(app.aside.inert, false);
  app.escape(app.document.body);
  assert.equal(app.aside.inert, false);
  menu.open = false;
  const popup = app.document.createElement('div');
  popup.dataset.slot = 'menu-popup';
  popup.innerHTML = '<button>이동</button>';
  app.document.body.append(popup);
  app.pointer(app.aside, 'pointerleave');
  app.tick(180);
  assert.equal(app.aside.inert, false);
  popup.addEventListener('keydown', event => { if (event.key === 'Escape') { popup.hidden = true; event.preventDefault(); } });
  app.escape(popup.querySelector('button'));
  assert.equal(app.aside.inert, false);
  app.escape(app.document.body);
  assert.equal(app.aside.inert, true);
});

test('closed popups and open content disclosures do not block dismissal', t => {
  const app = setup(t);
  app.aside.insertAdjacentHTML('beforeend', '<details><summary>더보기</summary><div role="menu">닫힘</div></details><div data-slot="menu-popup" data-state="closed">닫힘</div><div data-slot="menu-popup" data-closed>닫힘</div><div role="dialog" hidden>닫힘</div>');
  app.document.querySelector('main').insertAdjacentHTML('beforeend', '<details open><summary>품질 정보</summary><p>확인 사항</p><button>녹음 듣기</button></details>');
  app.preview('trash').click();
  app.pointer(app.aside, 'pointerleave');
  app.tick(180);
  assert.equal(app.aside.inert, true);
  app.preview('trash').click();
  app.escape(app.document.body);
  assert.equal(app.aside.inert, true);
});

test('the rail stays usable while pinned and its clicks choose the list without collapsing', t => {
  const app = setup(t), trigger = app.el('navigation-toggle'), toolbar = trigger.parentElement;
  const footer = app.compact.querySelector('.navigation-rail-footer');
  trigger.click();
  assert.equal(app.shell.classList.contains('nav-pinned'), true);
  assert.equal(app.shell.classList.contains('nav-peek'), false);
  assert.equal(app.compact.hidden, false);
  assert.equal(app.compact.inert, false);
  assert.equal(app.compact.getAttribute('aria-hidden'), 'false');
  assert.deepEqual(app.states.at(-1), { expanded: true, pinned: true });
  app.preview('folders').click();
  assert.deepEqual(app.events, [{ view: 'folders', expanded: true }]);
  assert.equal(app.aside.inert, false);
  assert.equal(app.shell.classList.contains('nav-peek'), false);
  assert.equal(app.window.localStorage.getItem('nero.sidebar.pinned'), 'true');
  app.pointer(app.preview('all'), 'pointerenter');
  app.tick(500);
  assert.deepEqual(app.events.at(-1), { view: 'all', expanded: true }, 'resting on a rail button switches the list while pinned');
  assert.equal(app.shell.classList.contains('nav-pinned'), true);
  app.pointer(app.preview('unfiled'), 'pointerenter');
  app.pointer(app.preview('unfiled'), 'pointerleave');
  app.tick(500);
  assert.equal(app.events.length, 2, 'passing over a rail button does not switch the list');
  trigger.click();
  assert.equal(app.compact.hidden, false);
  assert.equal(app.compact.inert, false);
  assert.equal(app.compact.getAttribute('aria-hidden'), 'false');
  assert.equal(app.aside.inert, true);
  for (const [id, parent] of [['navigation-toggle', toolbar], ['settings', footer], ['update', footer], ['notifications', footer]]) {
    assert.equal(app.document.querySelectorAll(`#${id}`).length, 1);
    assert.equal(app.el(id).parentElement, parent, `${id} stays in place`);
  }
  assert.equal(footer.lastElementChild, app.el('settings'));
  assert.deepEqual(app.states.at(-1), { expanded: false, pinned: false });
  assert.equal(app.window.localStorage.getItem('nero.sidebar.pinned'), 'false');
});

test('mode changes refresh width bounds without replacing the preferred width', t => {
  const app = setup(t, { 'nero.sidebar.pinned': 'true', 'nero.sidebar.pinned.width': '400' });
  assert.equal(app.shell.style.getPropertyValue('--navigation-width'), '240px');
  assert.equal(app.shell.classList.contains('nav-overlay'), false);
  app.el('navigation-toggle').click();
  app.preview('all').click();
  assert.equal(app.shell.style.getPropertyValue('--navigation-width'), '400px');
  assert.equal(app.shell.classList.contains('nav-overlay'), true);
  app.el('navigation-toggle').click();
  assert.equal(app.shell.style.getPropertyValue('--navigation-width'), '240px');
  assert.equal(app.window.localStorage.getItem('nero.sidebar.pinned.width'), '400');
});

test('navigation actions close only the temporary overlay and onboarding keeps navigation inert', t => {
  const app = setup(t);
  app.preview('recent').click();
  app.navigation.navigate();
  assert.equal(app.aside.inert, true);
  app.el('navigation-toggle').click();
  assert.equal(app.shell.style.getPropertyValue('--navigation-width'), '240px', 'the 272 default is clamped to the pinned bound');
  app.navigation.navigate();
  assert.equal(app.aside.inert, false);
  app.el('navigation-toggle').click();
  app.shell.dataset.onboarding = 'true';
  app.navigation.render();
  app.preview('all').click();
  app.el('navigation-toggle').click();
  assert.equal(app.events.length, 1);
  assert.equal(app.compact.inert, true);
  assert.equal(app.aside.inert, true);
  assert.equal(app.document.querySelector('.recent-section').inert, true);
  assert.equal(app.document.querySelector('main').inert, false);
});

test('navigation refresh keeps focused search and opened popups until an actual row action', t => {
  const app = setup(t);
  app.preview('all').click();
  app.el('search').focus();
  app.el('search').value = '회';
  app.navigation.navigate();
  assert.equal(app.aside.inert, false);
  assert.equal(app.document.activeElement, app.el('search'));
  app.el('search').value = '회의';
  app.navigation.navigate();
  assert.equal(app.aside.inert, false);
  const menu = app.document.createElement('details');
  menu.innerHTML = '<summary>더보기</summary><div class="action-menu"><button>이름 변경</button></div>';
  menu.open = true;
  app.aside.append(menu);
  app.el('meeting').focus();
  app.navigation.navigate();
  assert.equal(app.aside.inert, false);
  menu.open = false;
  app.navigation.navigate();
  assert.equal(app.aside.inert, true);
});

async function productSidebar(t) {
  const window = new Window({ url: 'http://nero.test', settings: { disableCSSFileLoading: true, disableJavaScriptFileLoading: true } });
  t.after(() => window.happyDOM.abort());
  const document = window.document;
  document.write(await readFile(new URL('../src/index.html', import.meta.url), 'utf8'));
  const el = id => document.getElementById(id);
  const state = { connected: true, libraryOpen: false, libraryFilter: 'project', librarySelection: new Set(['filed']), selectedId: 'filed', snapshot: {
    meetings: [{ id: 'filed', title: '폴더 회의', folderId: 'project', createdAt: 1 }, { id: 'unfiled', title: '미분류 회의', createdAt: 2 }, { id: 'deleted', title: '삭제한 회의', createdAt: 3, deletedAt: 4 }],
    folders: [{ id: 'project', name: '프로젝트' }],
  } };
  const calls = [];
  const library = mountLibrary({ bridge: {}, getState: () => state, byId: el, finishEditing: async action => action(), acceptSnapshot() {}, selectMeeting: id => calls.push(['select', id]), render() {}, showError(error) { throw error; }, beginNavigation: history => { calls.push(['navigate', history]); return () => true; }, escape: value => String(value ?? ''), icon: name => `<svg data-icon="${name}"></svg>`, duration: String, recordingStatus: String });
  library.render();
  const views = mountSidebarViews({ aside: el('navigation-sidebar'), navigation: el('sidebar-navigation-view'), notifications: el('notification-center'), trigger: el('notifications-open'), badge: el('notification-count'), storage: window.localStorage,
    count: filter => filter === 'folders' ? state.snapshot.folders.length : library.count(filter),
    folderName: id => state.snapshot.folders.find(folder => folder.id === id)?.name || '',
    onPreview: (view, folder) => library.preview(view, folder) });
  t.after(() => views.destroy());
  return { window, document, el, state, calls, library, views };
}

test('keyboard enters the list a rail button chooses and returns without changing the document', async t => {
  const { window, document, el, state, calls, views } = await productSidebar(t);
  const notes = el('meeting-notes'); notes.value = '저장 전 메모';
  const key = (node, value, options = {}) => { const event = new window.KeyboardEvent('keydown', { key: value, bubbles: true, cancelable: true, ...options }); node.dispatchEvent(event); return event; };
  const targets = { home: '#sidebar-meeting-list [data-recent-meeting="unfiled"]', all: '#sidebar-meeting-list [data-recent-meeting="unfiled"]', unfiled: '#sidebar-meeting-list [data-recent-meeting="unfiled"]',
    folders: '#library-user-folders [data-library-filter="project"]', recent: '#sidebar-meeting-list [data-recent-meeting="unfiled"]', trash: '#sidebar-meeting-list [data-recent-meeting="deleted"]' };
  const buttons = [...document.querySelectorAll('[data-navigation-preview]')];
  assert.deepEqual(buttons.map(button => button.dataset.navigationPreview), ['home', 'all', 'unfiled', 'folders', 'recent', 'trash']);
  assert.equal(views.activeView(), 'home');
  for (let index = 0; index < buttons.length; index++) {
    const button = buttons[index], view = button.dataset.navigationPreview;
    button.focus();
    assert.equal(views.activeView(), index ? buttons[index - 1].dataset.navigationPreview : 'home', 'focus alone keeps the current list');
    assert.equal(key(button, 'ArrowRight').defaultPrevented, true);
    assert.equal(views.activeView(), view);
    assert.equal(button.getAttribute('aria-current'), 'page');
    assert.equal(document.querySelectorAll('[data-navigation-preview][aria-current="page"]').length, 1);
    const target = document.querySelector(targets[view]);
    assert.ok(target, view);
    assert.equal(document.activeElement, target, view);
    const boundary = [...el('navigation-sidebar').querySelectorAll('[data-navigation-preview-action]')].find(node => !node.disabled && !node.closest('[hidden]')) || target;
    boundary.focus();
    assert.equal(key(boundary, 'Tab', { shiftKey: true }).defaultPrevented, true, view);
    assert.equal(document.activeElement, button);
    assert.equal(key(button, 'Enter').defaultPrevented, true);
    assert.equal(document.activeElement, target);
    assert.equal(key(target, 'ArrowLeft').defaultPrevented, true);
    assert.equal(document.activeElement, button);
    if (index < buttons.length - 1) assert.equal(key(button, 'Tab').defaultPrevented, false);
    assert.equal(state.libraryFilter, 'project');
    assert.equal(state.selectedId, 'filed');
    assert.equal(state.libraryOpen, false);
    assert.deepEqual([...state.librarySelection], ['filed']);
    assert.deepEqual(calls, []);
  }
  assert.equal(window.localStorage.getItem('nero.sidebar.view'), 'trash');
  assert.equal(el('meeting-notes'), notes);
  assert.equal(notes.value, '저장 전 메모');
});

test('a folder row opens its meetings in the sidebar and the back button returns to the folder list', async t => {
  const { document, el, state, calls, views } = await productSidebar(t);
  views.choose('folders');
  assert.equal(el('sidebar-library-navigation').hidden, false);
  assert.equal(el('sidebar-meeting-list').hidden, true);
  assert.equal(el('library-system-views').hidden, true);
  assert.equal(el('library-ai-view').hidden, true);
  assert.equal(el('sidebar-preview-title').textContent, '폴더');
  assert.equal(el('sidebar-preview-count').textContent, '1');
  assert.equal(el('sidebar-preview-back').hidden, true);
  assert.equal(el('sidebar-preview-open').hidden, true);
  el('library-user-folders').querySelector('[data-library-filter="project"]').click();
  assert.equal(views.activeView(), 'folder');
  assert.equal(views.folder(), 'project');
  assert.equal(el('sidebar-preview-title').textContent, '프로젝트');
  assert.equal(el('sidebar-preview-back').hidden, false);
  assert.equal(el('sidebar-library-navigation').hidden, true);
  assert.equal(el('sidebar-meeting-list').hidden, false);
  assert.deepEqual([...el('sidebar-meeting-list').querySelectorAll('[data-recent-meeting]')].map(node => node.dataset.recentMeeting), ['filed']);
  assert.equal(el('sidebar-preview-open').hidden, false);
  assert.equal(el('sidebar-preview-open').dataset.sidebarFilter, 'project');
  assert.equal(document.querySelector('[data-navigation-preview="folders"]').getAttribute('aria-current'), 'page');
  assert.equal(state.libraryOpen, false, 'the central library stays closed');
  assert.deepEqual(calls, []);
  el('sidebar-preview-back').click();
  assert.equal(views.activeView(), 'folders');
  assert.equal(el('sidebar-library-navigation').hidden, false);
  assert.equal(el('sidebar-preview-back').hidden, true);
  assert.equal(document.defaultView.localStorage.getItem('nero.sidebar.view'), 'folders', 'a single folder is not remembered as the starting list');
});

test('preview keyboard return preserves text editing and open menu priority', t => {
  const app = setup(t);
  const key = (node, value) => { const event = new app.window.KeyboardEvent('keydown', { key: value, bubbles: true, cancelable: true }); node.dispatchEvent(event); return event; };
  app.preview('all').focus();
  key(app.preview('all'), 'ArrowRight');
  assert.equal(app.document.activeElement, app.el('search'));
  assert.equal(key(app.el('search'), 'ArrowLeft').defaultPrevented, false);
  assert.equal(app.document.activeElement, app.el('search'));
  const popup = app.document.createElement('div');
  popup.dataset.slot = 'menu-popup';
  popup.innerHTML = '<button>메뉴 작업</button>';
  app.aside.append(popup);
  popup.querySelector('button').focus();
  assert.equal(key(popup.querySelector('button'), 'ArrowLeft').defaultPrevented, false);
  assert.equal(app.document.activeElement, popup.querySelector('button'));
});
