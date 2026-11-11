import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { Window } from 'happy-dom';
import { navigationWidthPreference, navigationWidthBounds, navigationRailWidth } from '../../../design-system/navigation-width.mjs';
import { mountAiChatLayout } from '../src/ai-chat-layout.js';

test('width preferences reject non-numeric shapes and retain valid numeric strings', () => {
  for (const value of [null, undefined, '', ' ', 'broken', '280px', -10, 0, Infinity, {}, [], true]) assert.equal(navigationWidthPreference(value), 224);
  assert.equal(navigationWidthPreference(' 280.4 '), 280);
  assert.equal(navigationWidthPreference('999999'), 400);
  assert.equal(navigationWidthPreference(12), 200);
});

test('pinned bounds reserve document space without subtracting a compact navigation rail', () => {
  assert.deepEqual(navigationWidthBounds(1000), { min: 200, max: 400, overlay: false });
  assert.deepEqual(navigationWidthBounds(700), { min: 200, max: 300, overlay: false });
  assert.deepEqual(navigationWidthBounds(640), { min: 200, max: 240, overlay: false });
  assert.deepEqual(navigationWidthBounds(600), { min: 200, max: 200, overlay: false });
  assert.deepEqual(navigationWidthBounds(599), { min: 200, max: 400, overlay: true });
  assert.deepEqual(navigationWidthBounds(320), { min: 200, max: 272, overlay: true });
  assert.deepEqual(navigationWidthBounds(200), { min: 152, max: 152, overlay: true });
});

test('temporary bounds retain the compact rail and leave an outside dismissal target', () => {
  const options = { overlay: true, railWidth: 44 };
  assert.deepEqual(navigationWidthBounds(1000, options), { min: 200, max: 400, overlay: true });
  assert.deepEqual(navigationWidthBounds(640, options), { min: 200, max: 400, overlay: true });
  assert.deepEqual(navigationWidthBounds(320, options), { min: 200, max: 228, overlay: true });
  assert.deepEqual(navigationWidthBounds(200, options), { min: 108, max: 108, overlay: true });
  assert.deepEqual(navigationWidthBounds(80, options), { min: 0, max: 0, overlay: true });
});

test('navigation and the narrow Nero panel reserve the rail width from the shared CSS', async t => {
  const window = new Window();
  t.after(() => window.happyDOM.abort());
  window.document.head.innerHTML = `<style>${await readFile(new URL('../../../design-system/navigation.css', import.meta.url), 'utf8')}</style>`;
  window.document.body.innerHTML = '<div id="root"><div id="handle"></div><aside id="panel"></aside></div>';
  const root = window.document.getElementById('root');
  root.getBoundingClientRect = () => ({ width: 320 });
  const railWidth = navigationRailWidth(root);
  assert.equal(railWidth, 44);
  assert.deepEqual(navigationWidthBounds(320, { overlay: true, railWidth }), { min: 200, max: 228, overlay: true });
  const layout = mountAiChatLayout({ root, handle: window.document.getElementById('handle'), panel: window.document.getElementById('panel') });
  t.after(() => layout.destroy());
  layout.update(true);
  assert.equal(root.style.getPropertyValue('--ai-width'), '276px');
});
