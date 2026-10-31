import assert from 'node:assert/strict';
import test from 'node:test';
import { navigationWidthPreference, navigationWidthBounds } from '../../../design-system/navigation-width.mjs';

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
  assert.deepEqual(navigationWidthBounds(1000, { overlay: true }), { min: 200, max: 400, overlay: true });
  assert.deepEqual(navigationWidthBounds(640, { overlay: true }), { min: 200, max: 400, overlay: true });
  assert.deepEqual(navigationWidthBounds(320, { overlay: true }), { min: 200, max: 220, overlay: true });
  assert.deepEqual(navigationWidthBounds(200, { overlay: true }), { min: 100, max: 100, overlay: true });
  assert.deepEqual(navigationWidthBounds(80, { overlay: true }), { min: 0, max: 0, overlay: true });
});
