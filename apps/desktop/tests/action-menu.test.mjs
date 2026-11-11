import assert from 'node:assert/strict';
import test from 'node:test';
import { Window } from 'happy-dom';
import { mountActionMenus } from '../../../design-system/action-menu.mjs';
import { readFile } from 'node:fs/promises';

test('action menu enters the browser top layer and returns focus on Escape', t => {
  const window = new Window(); t.after(() => window.happyDOM.abort());
  const document = window.document;
  document.body.innerHTML = '<main><details data-meeting-menu><summary>더보기</summary><div class="action-menu"><button>이동</button></div></details></main><aside class="app-ai-sidebar">AI</aside>';
  const details = document.querySelector('details'), summary = details.querySelector('summary'), content = details.querySelector('div');
  let inTopLayer = false;
  const matches = content.matches.bind(content);
  content.matches = selector => selector === ':popover-open' ? inTopLayer : matches(selector);
  content.showPopover = () => { assert.equal(content.getAttribute('popover'), 'manual'); inTopLayer = true; };
  content.hidePopover = () => { inTopLayer = false; };
  const menus = mountActionMenus({ owner: document });
  summary.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, cancelable: true }));
  assert.equal(inTopLayer, true);
  assert.equal(menus.active, details);
  assert.equal(document.activeElement, content.querySelector('button'));
  document.activeElement.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
  assert.equal(inTopLayer, false);
  assert.equal(details.open, false);
  assert.equal(document.activeElement, summary);
});

test('the library recording menu shares keyboard entry, Escape and outside dismissal', async t => {
  const window = new Window({ settings: { disableCSSFileLoading: true, disableJavaScriptFileLoading: true } });
  t.after(() => window.happyDOM.abort());
  const document = window.document;
  document.write(await readFile(new URL('../src/index.html', import.meta.url), 'utf8'));
  const menu = document.querySelector('.library-more'), summary = menu.querySelector('summary');
  const record = document.getElementById('library-record'); record.disabled = false;
  const menus = mountActionMenus({ owner: document });
  const enter = new window.KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, cancelable: true });
  summary.dispatchEvent(enter);
  assert.equal(enter.defaultPrevented, true);
  assert.equal(menus.active, menu);
  assert.equal(document.activeElement, record);
  assert.ok(menu.querySelector('.action-menu').style.top);
  record.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
  assert.equal(menu.open, false);
  assert.equal(document.activeElement, summary);
  summary.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, cancelable: true }));
  document.body.dispatchEvent(new window.PointerEvent('pointerdown', { bubbles: true }));
  assert.equal(menu.open, false);
  assert.equal(menus.active, null);
});
