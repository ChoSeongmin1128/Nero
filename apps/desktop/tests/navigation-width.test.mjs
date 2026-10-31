import assert from 'node:assert/strict';
import test from 'node:test';
import { navigationWidthPreference, navigationWidthBounds } from '../../../design-system/navigation-width.mjs';

test('width preferences reject non-numeric shapes and retain valid numeric strings', () => {
  for (const value of [null, undefined, '', ' ', 'broken', '280px', -10, 0, Infinity, {}, [], true]) assert.equal(navigationWidthPreference(value), 224);
  assert.equal(navigationWidthPreference(' 280.4 '), 280);
  assert.equal(navigationWidthPreference('999999'), 400);
  assert.equal(navigationWidthPreference(12), 200);
});

test('desktop bounds protect the document while narrow overlay bounds leave an outside target', () => {
  assert.deepEqual(navigationWidthBounds(1000), { min: 200, max: 400, overlay: false });
  assert.deepEqual(navigationWidthBounds(700), { min: 200, max: 248, overlay: false });
  assert.deepEqual(navigationWidthBounds(640), { min: 200, max: 400, overlay: true });
  assert.deepEqual(navigationWidthBounds(320), { min: 200, max: 220, overlay: true });
  assert.deepEqual(navigationWidthBounds(200), { min: 100, max: 100, overlay: true });
});
