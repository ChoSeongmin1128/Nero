import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';

const source = await readFile(new URL('../../../design-system/appearance.js', import.meta.url), 'utf8');

function openAppearance({ saved, dark = false, native = true, blocked = false } = {}) {
  const storage = new Map(saved ? [['nero.appearance.v1', saved]] : []);
  const listeners = new Map();
  const media = { matches: dark, addEventListener(_, callback) { listeners.set('system', callback); } };
  const root = { dataset: native ? { appearanceStorage: 'nero.appearance.v1' } : {}, setAttribute() {}, removeAttribute() {} };
  const inputs = ['light', 'dark', 'system'].map(value => ({ name: 'appearance-mode', value }));
  inputs.push(...['green', 'blue', 'purple', 'mono', 'amber', 'rose'].map(value => ({ name: 'appearance-color', value })));
  const dialog = { addEventListener(name, callback) { listeners.set(`dialog-${name}`, callback); } };
  const document = {
    documentElement: root,
    querySelectorAll(selector) { return inputs.filter(input => selector.includes(`"${input.name}"`)); },
    querySelector() { return null; },
    getElementById(id) { return id === 'settings-dialog' ? dialog : { addEventListener() {} }; },
    addEventListener(name, callback) { listeners.set(`document-${name}`, callback); },
  };
  const context = vm.createContext({ document, URLSearchParams, location: { search: '' },
    localStorage: { getItem(key) { if (blocked) throw Error('blocked'); return storage.get(key) ?? null; }, setItem(key, value) { if (blocked) throw Error('blocked'); storage.set(key, value); } },
    window: { matchMedia: () => media, addEventListener(name, callback) { listeners.set(name, callback); } },
  });
  vm.runInContext(source, context);
  listeners.get('document-DOMContentLoaded')();
  return { root, storage, inputs, media, listeners, change(name, value) { listeners.get('dialog-change')({ target: { name: `appearance-${name}`, value } }); } };
}

test('native default follows system while explicit light or dark ignores system changes', () => {
  const app = openAppearance({ dark: true });
  assert.equal(app.root.dataset.appearancePreference, 'green-system');
  assert.equal(app.root.dataset.mode, 'dark');
  app.change('mode', 'light');
  app.listeners.get('system')();
  assert.equal(app.root.dataset.mode, 'light');
  app.change('mode', 'system');
  app.media.matches = false;
  app.listeners.get('system')();
  assert.equal(app.root.dataset.mode, 'light');
});

test('legacy colors normalize to the single accent while preserving brightness', () => {
  const app = openAppearance();
  for (const color of ['green', 'blue', 'purple', 'mono', 'amber', 'rose']) {
    app.listeners.get('storage')({ key: 'nero.appearance.v1', newValue: JSON.stringify({ color, mode: 'light' }) });
    for (const mode of ['light', 'dark', 'system']) {
      app.change('mode', mode);
      const reopened = openAppearance({ saved: app.storage.get('nero.appearance.v1') });
      assert.equal(reopened.root.dataset.appearancePreference, `green-${mode}`);
      assert.equal(reopened.inputs.filter(input => input.checked).length, 2);
    }
  }
});

test('recorder appearance follows the shared preference storage event', () => {
  const recorder = openAppearance();
  recorder.listeners.get('storage')({ key: 'nero.appearance.v1', newValue: JSON.stringify({ color: 'mono', mode: 'dark' }) });
  assert.equal(recorder.root.dataset.appearance, 'green-dark');
  recorder.listeners.get('storage')({ key: 'unrelated', newValue: null });
  assert.equal(recorder.root.dataset.appearance, 'green-dark');
  recorder.listeners.get('storage')({ key: 'nero.appearance.v1', newValue: null });
  assert.equal(recorder.root.dataset.appearancePreference, 'green-system');
});

test('invalid saved values and blocked storage keep theme controls usable', () => {
  const invalid = openAppearance({ saved: JSON.stringify({ color: 'invalid', mode: 'dark' }) });
  assert.equal(invalid.root.dataset.appearancePreference, 'green-dark');
  const blocked = openAppearance({ blocked: true });
  blocked.change('color', 'blue');
  blocked.change('mode', 'dark');
  assert.equal(blocked.root.dataset.appearance, 'green-dark');
  const prototype = openAppearance({ native: false });
  assert.equal(prototype.root.dataset.appearancePreference, 'green-light');
});
