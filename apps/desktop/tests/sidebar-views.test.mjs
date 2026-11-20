import assert from 'node:assert/strict';
import test from 'node:test';
import { Window } from 'happy-dom';
import { mountSidebarViews } from '../src/sidebar-views.js';
import { mountNotificationCenter } from '../src/notification-center.js';
import { mountNavigationSidebar } from '../../../design-system/navigation.mjs';

function setup(t) {
  const window = new Window({ url: 'http://nero.test' });
  t.after(() => window.happyDOM.abort());
  const document = window.document;
  document.body.innerHTML = '<div class="navigation-shell"><nav class="navigation-rail"><button id="notifications">알림<span id="badge"></span></button><button id="settings">설정</button></nav><aside class="navigation-sidebar"><div id="navigation">회의 목록</div><section id="notifications-panel" hidden></section></aside><main><button id="toggle">탐색</button><textarea id="notes">작성 중인 메모</textarea></main></div>';
  const el = id => document.getElementById(id);
  const aside = document.querySelector('aside'), shell = document.querySelector('.navigation-shell');
  let views = null;
  const sidebar = mountNavigationSidebar({ shell, aside, content: document.querySelector('main'), trigger: el('toggle'), storage: window.localStorage, onStateChange: () => views?.refresh() });
  const center = mountNotificationCenter({ container: el('notifications-panel') });
  center.update({ connected: true });
  views = mountSidebarViews({ aside, navigation: el('navigation'), notifications: el('notifications-panel'), trigger: el('notifications'), badge: el('badge'), reveal: () => sidebar.reveal() });
  t.after(() => { views.destroy(); center.destroy(); });
  return { window, document, aside, shell, el, center, views };
}

test('rail notifications reveal only the sidebar panel and retain the document and draft', t => {
  const app = setup(t);
  const notes = app.el('notes'); notes.value = '저장 전 메모';
  assert.equal(app.aside.inert, true);
  app.el('notifications').click();
  assert.equal(app.views.isOpen(), true);
  assert.equal(app.shell.classList.contains('nav-peek'), true);
  assert.equal(app.el('navigation').hidden, true);
  assert.equal(app.el('notifications-panel').hidden, false);
  assert.equal(app.document.querySelector('main').hidden, false);
  assert.equal(app.document.querySelector('main').inert, false);
  assert.equal(app.el('notes'), notes);
  assert.equal(notes.value, '저장 전 메모');
  assert.ok(app.document.activeElement.hasAttribute('data-notification-heading'));
  assert.equal(app.el('notifications-panel').querySelector('[data-notification-action="back"]'), null);
  assert.equal(app.el('notifications').getAttribute('aria-expanded'), 'true');
  app.el('notifications').click();
  assert.equal(app.views.isOpen(), false);
  assert.equal(app.el('navigation').hidden, false);
  assert.equal(app.document.activeElement, app.el('notifications'));
});

test('Escape returns to navigation before closing the temporary panel', t => {
  const app = setup(t);
  app.views.open();
  app.document.activeElement.dispatchEvent(new app.window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
  assert.equal(app.views.isOpen(), false);
  assert.equal(app.shell.classList.contains('nav-peek'), true);
  assert.equal(app.document.activeElement, app.el('notifications'));
  app.document.activeElement.dispatchEvent(new app.window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
  assert.equal(app.shell.classList.contains('nav-peek'), false);
});

test('collapsing the temporary panel keeps rail controls usable and the bell reveals notifications again', t => {
  const app = setup(t);
  const notes = app.el('notes'); notes.value = '저장 전 메모';
  app.views.open();
  app.document.querySelector('main').dispatchEvent(new app.window.PointerEvent('pointerdown', { bubbles: true }));
  assert.equal(app.shell.classList.contains('nav-peek'), false);
  assert.equal(app.el('notifications').closest('nav').inert, false);
  assert.equal(app.el('notifications').getAttribute('aria-expanded'), 'false', 'a hidden panel is not announced as expanded');
  app.el('notifications').click();
  assert.equal(app.shell.classList.contains('nav-peek'), true, 'one bell click shows the notifications again');
  assert.equal(app.views.isOpen(), true);
  assert.equal(app.el('notifications-panel').hidden, false);
  assert.equal(app.el('notifications').getAttribute('aria-expanded'), 'true');
  assert.equal(notes.value, '저장 전 메모');
});

test('resting on the bell while the sidebar is not pinned shows notifications, and a click keeps them open', t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const app = setup(t);
  const pointer = (type, pointerType = 'mouse') => app.el('notifications').dispatchEvent(new app.window.PointerEvent(type, { pointerType }));
  pointer('pointerenter'); t.mock.timers.tick(100); pointer('pointerleave'); t.mock.timers.tick(500);
  assert.equal(app.views.isOpen(), false, 'passing over the bell does not open it');
  pointer('pointerenter'); t.mock.timers.tick(140);
  assert.equal(app.views.isOpen(), true);
  assert.equal(app.shell.classList.contains('nav-peek'), true);
  assert.equal(app.document.activeElement === app.el('notes') || !app.el('notifications-panel').contains(app.document.activeElement), true, 'hovering does not move focus into the list');
  app.el('notifications').click();
  assert.equal(app.views.isOpen(), true, 'a click after hovering keeps them open');
  app.el('notifications').click();
  assert.equal(app.views.isOpen(), false);
});

test('the badge shows the attention count with an accessible label while active work is separate', t => {
  const app = setup(t);
  app.views.updateCounts({ unread: 3, active: 1 });
  assert.equal(app.el('badge').hidden, false);
  assert.equal(app.el('badge').textContent, '3');
  assert.equal(app.el('badge').getAttribute('aria-hidden'), 'true');
  assert.equal(app.el('notifications').dataset.unread, 'true');
  assert.match(app.el('notifications').getAttribute('aria-label'), /확인할 알림 3개/);
  app.views.updateCounts({ unread: 0, active: 1 });
  assert.equal(app.el('notifications').dataset.unread, 'false');
  assert.equal(app.el('notifications').dataset.active, 'true');
  assert.equal(app.el('notifications').getAttribute('aria-label'), '알림, 작업 진행 중');
  assert.equal(app.el('badge').hidden, true);
  assert.equal(app.el('badge').textContent, '');
  app.views.open();
  assert.equal(app.el('notifications').getAttribute('aria-pressed'), 'true');
});
