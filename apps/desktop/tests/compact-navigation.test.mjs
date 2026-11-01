import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { Window } from 'happy-dom';
import { mountNavigationSidebar } from '../../../design-system/navigation.mjs';
import { mountLibrary } from '../src/library-ui.js';
import { mountSidebarViews } from '../src/sidebar-views.js';

function setup(t, preferences = {}) {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const window = new Window({ url: 'http://nero.test' });
  t.after(() => window.happyDOM.abort());
  const document = window.document;
  document.body.innerHTML = `<div class="navigation-shell">
    <nav class="navigation-rail" data-navigation-compact>${['all', 'folders', 'trash', 'recent'].map(view => `<button data-navigation-preview="${view}">${view}</button>`).join('')}<footer id="navigation-toggle-collapsed"><button id="navigation-toggle">탐색</button></footer><div id="utilities-compact"><button id="settings">설정</button><button id="update">업데이트</button></div></nav>
    <div class="navigation-edge"></div>
    <aside id="navigation-sidebar"><header id="navigation-header"><span id="navigation-toggle-expanded"></span></header><input id="search"><button id="meeting">회의</button><details class="recent-section" open><summary>최근 회의</summary></details><div id="utilities-expanded"></div></aside>
    <main><textarea id="notes">작성 중</textarea></main>
  </div>`;
  for (const [key, value] of Object.entries(preferences)) window.localStorage.setItem(key, value);
  const el = id => document.getElementById(id);
  const shell = document.querySelector('.navigation-shell'), aside = el('navigation-sidebar'), compact = document.querySelector('[data-navigation-compact]');
  shell.getBoundingClientRect = () => ({ width: 640 });
  const events = [], states = [];
  aside.addEventListener('nero:sidebar-preview', event => { events.push({ view: event.detail.view, expanded: !aside.inert }); aside.dataset.previewView = event.detail.view; });
  aside.addEventListener('nero:navigation-state', event => states.push(event.detail));
  const navigation = mountNavigationSidebar({ shell, aside, content: document.querySelector('main'), trigger: el('navigation-toggle'), storage: window.localStorage,
    relocate: ['settings', 'update'].map(id => ({ node: el(id), expanded: el('utilities-expanded'), compact: el('utilities-compact') })) });
  const preview = view => compact.querySelector(`[data-navigation-preview="${view}"]`);
  const pointer = (node, type, pointerType = 'mouse') => node.dispatchEvent(new window.PointerEvent(type, { pointerType }));
  const escape = node => node.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
  return { window, document, el, shell, aside, compact, navigation, events, states, preview, pointer, escape, tick: milliseconds => t.mock.timers.tick(milliseconds) };
}

test('compact previews use hover intent and bridge pointer movement into the overlay', t => {
  const app = setup(t);
  app.el('notes').focus();
  app.pointer(app.document.querySelector('.navigation-edge'), 'pointerenter');
  app.tick(140);
  assert.equal(app.aside.inert, true);
  app.pointer(app.preview('all'), 'pointerenter', 'touch');
  app.tick(140);
  assert.equal(app.aside.inert, true);
  app.pointer(app.preview('all'), 'pointerenter');
  app.tick(139);
  assert.equal(app.aside.inert, true);
  app.tick(1);
  assert.deepEqual(app.events, [{ view: 'all', expanded: false }]);
  assert.equal(app.shell.classList.contains('nav-peek'), true);
  app.pointer(app.preview('all'), 'pointerleave');
  app.tick(100);
  app.pointer(app.aside, 'pointerenter');
  app.tick(180);
  assert.equal(app.aside.inert, false);
  app.pointer(app.aside, 'pointerleave');
  app.tick(179);
  assert.equal(app.aside.inert, false);
  app.tick(1);
  assert.equal(app.aside.inert, true);
  assert.equal(app.document.activeElement, app.el('notes'));
});

test('hover switches all four previews while retaining content, browser history and preferences', t => {
  const app = setup(t), notes = app.el('notes'), main = app.document.querySelector('main');
  const historyLength = app.window.history.length;
  notes.value = '저장 전 메모';
  for (const view of ['all', 'folders', 'trash', 'recent']) {
    app.pointer(app.preview(view), 'pointerenter');
    app.tick(140);
    assert.equal(app.aside.dataset.previewView, view);
    app.pointer(app.preview(view), 'pointerleave');
  }
  assert.deepEqual(app.events.map(event => event.view), ['all', 'folders', 'trash', 'recent']);
  assert.equal(app.el('notes'), notes);
  assert.equal(notes.value, '저장 전 메모');
  assert.equal(main.hidden, false);
  assert.equal(main.inert, false);
  assert.equal(app.window.history.length, historyLength);
  assert.equal(app.window.localStorage.getItem('nero.library.view'), null);
  assert.equal(app.window.localStorage.getItem('nero.sidebar.pinned'), null);
});

test('focus and click reveal previews immediately and keyboard focus prevents automatic hiding', t => {
  const app = setup(t);
  app.preview('folders').focus();
  assert.equal(app.aside.dataset.previewView, 'folders');
  assert.equal(app.aside.inert, false);
  app.pointer(app.compact, 'pointerleave');
  app.tick(180);
  assert.equal(app.aside.inert, false);
  app.preview('recent').click();
  assert.equal(app.aside.dataset.previewView, 'recent');
  app.el('meeting').focus();
  app.pointer(app.aside, 'pointerleave');
  app.tick(180);
  assert.equal(app.aside.inert, false);
  app.escape(app.el('meeting'));
  assert.equal(app.aside.inert, true);
  assert.equal(app.document.activeElement, app.preview('recent'));
  assert.equal(app.events.length, 2);
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

test('pinning relocates singleton controls and disables the compact navigation', t => {
  const app = setup(t), trigger = app.el('navigation-toggle'), settings = app.el('settings'), update = app.el('update');
  assert.equal(trigger.parentElement, app.el('navigation-toggle-collapsed'));
  app.preview('all').click();
  assert.equal(trigger.parentElement, app.el('navigation-toggle-expanded'));
  trigger.click();
  assert.equal(app.compact.hidden, true);
  assert.equal(app.compact.inert, true);
  assert.equal(app.compact.getAttribute('aria-hidden'), 'true');
  assert.equal(settings.parentElement, app.el('utilities-expanded'));
  assert.equal(update.parentElement, app.el('utilities-expanded'));
  assert.deepEqual(app.states.at(-1), { expanded: true, pinned: true });
  app.preview('folders').click();
  assert.equal(app.events.length, 1);
  trigger.click();
  assert.equal(app.compact.hidden, false);
  assert.equal(app.compact.inert, false);
  assert.equal(app.compact.getAttribute('aria-hidden'), 'false');
  assert.equal(trigger.parentElement, app.el('navigation-toggle-collapsed'));
  assert.equal(settings.parentElement, app.el('utilities-compact'));
  assert.equal(update.parentElement, app.el('utilities-compact'));
  for (const id of ['navigation-toggle', 'settings', 'update']) assert.equal(app.document.querySelectorAll(`#${id}`).length, 1);
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
  assert.equal(app.shell.style.getPropertyValue('--navigation-width'), '224px');
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

test('keyboard enters and returns from each product preview without advancing the category or document', async t => {
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
  const library = mountLibrary({ bridge: {}, getState: () => state, byId: el, finishEditing: async action => action(), acceptSnapshot() {}, selectMeeting: id => calls.push(['select', id]), render() {}, showError(error) { throw error; }, beginNavigation: history => { calls.push(['navigate', history]); return () => true; }, escape: value => String(value ?? ''), icon: name => `<svg data-icon="${name}"></svg>`, duration: String });
  library.render();
  const views = mountSidebarViews({ aside: el('navigation-sidebar'), navigation: el('sidebar-navigation-view'), notifications: el('notification-center'), trigger: el('notifications-open'), badge: el('notification-count'), onPreview: view => library.preview(view) });
  t.after(() => views.destroy());
  const notes = el('meeting-notes'); notes.value = '저장 전 메모';
  const key = (node, value, options = {}) => { const event = new window.KeyboardEvent('keydown', { key: value, bubbles: true, cancelable: true, ...options }); node.dispatchEvent(event); return event; };
  const targets = { all: '#sidebar-meeting-list [data-recent-meeting="unfiled"]', folders: '#library-system-views [data-library-filter="unfiled"]', trash: '#sidebar-meeting-list [data-recent-meeting="deleted"]', recent: '#recent-meetings [data-recent-meeting="unfiled"]' };
  const buttons = [...document.querySelectorAll('[data-navigation-preview]')];
  for (let index = 0; index < buttons.length; index++) {
    const button = buttons[index], view = button.dataset.navigationPreview;
    button.focus();
    assert.equal(views.activeView(), view);
    assert.equal(key(button, 'ArrowRight').defaultPrevented, true);
    const target = document.querySelector(targets[view]);
    assert.equal(document.activeElement, target);
    assert.equal(views.activeView(), view);
    const hasHeaderActions = ['all', 'trash'].includes(view);
    assert.equal(key(target, 'Tab', { shiftKey: true }).defaultPrevented, !hasHeaderActions);
    if (hasHeaderActions) {
      const action = el(view === 'trash' ? 'sidebar-empty-trash' : 'sidebar-preview-open');
      action.focus();
      assert.equal(key(action, 'Tab', { shiftKey: true }).defaultPrevented, true);
    }
    assert.equal(document.activeElement, button);
    assert.equal(key(button, 'Enter').defaultPrevented, true);
    assert.equal(document.activeElement, target);
    assert.equal(key(target, 'ArrowLeft').defaultPrevented, true);
    assert.equal(document.activeElement, button);
    assert.equal(views.activeView(), view);
    if (index < buttons.length - 1) assert.equal(key(button, 'Tab').defaultPrevented, false);
    assert.equal(state.libraryFilter, 'project');
    assert.equal(state.selectedId, 'filed');
    assert.equal(state.libraryOpen, false);
    assert.deepEqual([...state.librarySelection], ['filed']);
    assert.deepEqual(calls, []);
  }
  assert.equal(el('meeting-notes'), notes);
  assert.equal(notes.value, '저장 전 메모');
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
