import assert from 'node:assert/strict';
import test from 'node:test';
import { Window } from 'happy-dom';
import { mountActionMenus } from '../../../design-system/action-menu.mjs';

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
