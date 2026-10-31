import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';

const source = await readFile(new URL('../../../design-system/appearance.js', import.meta.url), 'utf8');

function openAppearance({ saved, dark = false, native = true, blocked = false, reduced = false, startTransition } = {}) {
  const storage = new Map(saved ? [['nero.appearance.v1', saved]] : []);
  const listeners = new Map();
  const media = { matches: dark, addEventListener(_, callback) { listeners.set('system', callback); } };
  const attributes = new Set(), timers = new Map(); let timerId = 0;
  const motion = { matches: reduced, addEventListener(_, callback) { listeners.set('motion', callback); } };
  const root = { dataset: native ? { appearanceStorage: 'nero.appearance.v1' } : {}, setAttribute(name) { attributes.add(name); }, removeAttribute(name) { attributes.delete(name); } };
  const inputs = ['light', 'dark', 'system'].map(value => ({ name: 'appearance-mode', value }));
  inputs.push(...['green', 'blue', 'purple', 'mono', 'amber', 'rose'].map(value => ({ name: 'appearance-color', value })));
  const dialog = { addEventListener(name, callback) { listeners.set(`dialog-${name}`, callback); } };
  const document = {
    documentElement: root,
    ...(startTransition ? { startViewTransition: startTransition } : {}),
    querySelectorAll(selector) { return inputs.filter(input => selector.includes(`"${input.name}"`)); },
    querySelector() { return null; },
    getElementById(id) { return id === 'settings-dialog' ? dialog : { addEventListener() {} }; },
    addEventListener(name, callback) { listeners.set(`document-${name}`, callback); },
  };
  const context = vm.createContext({ document, URLSearchParams, location: { search: '' },
    localStorage: { getItem(key) { if (blocked) throw Error('blocked'); return storage.get(key) ?? null; }, setItem(key, value) { if (blocked) throw Error('blocked'); storage.set(key, value); } },
    window: { matchMedia: query => query.includes('reduced-motion') ? motion : media, addEventListener(name, callback) { listeners.set(name, callback); }, setTimeout(callback) { timers.set(++timerId, callback); return timerId; }, clearTimeout(id) { timers.delete(id); } },
  });
  vm.runInContext(source, context);
  listeners.get('document-DOMContentLoaded')();
  return { root, storage, inputs, media, motion, attributes, timers, listeners, change(name, value) { listeners.get('dialog-change')({ target: { name: `appearance-${name}`, value } }); } };
}

test('initial brightness resolves once and only explicit light or dark remains selectable', () => {
  const app = openAppearance({ dark: true });
  assert.equal(app.root.dataset.appearancePreference, 'mono-dark');
  app.media.matches = false;
  assert.equal(app.listeners.has('system'), false);
  assert.equal(app.root.dataset.mode, 'dark');
  app.change('mode', 'light');
  app.change('mode', 'system');
  assert.equal(app.root.dataset.mode, 'light');
  assert.equal(openAppearance({ saved: app.storage.get('nero.appearance.v1'), dark: true }).root.dataset.mode, 'light');
});

test('legacy system mode resolves to a persisted fixed brightness', () => {
  for (const saved of ['system', JSON.stringify({ color: 'blue', mode: 'system' })]) {
    const app = openAppearance({ saved, dark: true });
    const reopened = openAppearance({ saved: app.storage.get('nero.appearance.v1'), dark: false });
    assert.equal(reopened.root.dataset.appearancePreference, 'mono-dark');
  }
});

test('legacy colors normalize to the single accent while preserving brightness', () => {
  const app = openAppearance();
  for (const color of ['green', 'blue', 'purple', 'mono', 'amber', 'rose']) {
    app.listeners.get('storage')({ key: 'nero.appearance.v1', newValue: JSON.stringify({ color, mode: 'light' }) });
    for (const mode of ['light', 'dark']) {
      app.change('mode', mode);
      const reopened = openAppearance({ saved: app.storage.get('nero.appearance.v1') });
      assert.equal(reopened.root.dataset.appearancePreference, `mono-${mode}`);
      assert.equal(reopened.inputs.filter(input => input.checked).length, 2);
    }
  }
});

test('recorder appearance follows the shared preference storage event', () => {
  const recorder = openAppearance();
  recorder.listeners.get('storage')({ key: 'nero.appearance.v1', newValue: JSON.stringify({ color: 'mono', mode: 'dark' }) });
  assert.equal(recorder.root.dataset.appearance, 'mono-dark');
  recorder.listeners.get('storage')({ key: 'unrelated', newValue: null });
  assert.equal(recorder.root.dataset.appearance, 'mono-dark');
  recorder.listeners.get('storage')({ key: 'nero.appearance.v1', newValue: null });
  assert.equal(recorder.root.dataset.appearancePreference, 'mono-light');
});

test('invalid saved values and blocked storage keep theme controls usable', () => {
  const invalid = openAppearance({ saved: JSON.stringify({ color: 'invalid', mode: 'dark' }) });
  assert.equal(invalid.root.dataset.appearancePreference, 'mono-dark');
  const blocked = openAppearance({ blocked: true });
  blocked.change('color', 'blue');
  blocked.change('mode', 'dark');
  assert.equal(blocked.root.dataset.appearance, 'mono-dark');
  const prototype = openAppearance({ native: false });
  assert.equal(prototype.root.dataset.appearancePreference, 'mono-light');
});


function viewTransitions() {
  const entries = [];
  return { entries, start(callback) {
    let finish;
    const entry = { skipped: false, run: callback };
    const finished = new Promise(resolve => { finish = resolve; });
    entry.finish = finish; entries.push(entry);
    return { ready: Promise.resolve(), updateCallbackDone: Promise.resolve(), finished, skipTransition() { entry.skipped = true; finish(); } };
  } };
}

test('initial appearance never animates; user changes use the native view transition', () => {
  const transitions = viewTransitions();
  const app = openAppearance({ startTransition: transitions.start });
  assert.equal(transitions.entries.length, 0);
  app.change('mode', 'dark');
  assert.equal(transitions.entries.length, 1);
  assert.equal(JSON.parse(app.storage.get('nero.appearance.v1')).mode, 'dark');
  transitions.entries[0].run();
  assert.equal(app.root.dataset.mode, 'dark');
  assert.equal(app.attributes.has('data-appearance-motion'), false);
  transitions.entries[0].finish();
});

test('rapid changes cannot restore an older palette even when transition callbacks run late', () => {
  const transitions = viewTransitions();
  const app = openAppearance({ startTransition: transitions.start });
  app.change('mode', 'dark');
  app.change('mode', 'light');
  assert.equal(transitions.entries[0].skipped, true);
  transitions.entries[0].run();
  assert.equal(app.root.dataset.mode, 'light');
  app.change('mode', 'dark');
  transitions.entries[1].run(); transitions.entries[0].run();
  assert.equal(app.root.dataset.mode, 'dark');
  transitions.entries[1].finish();
});

test('fallback transitions settle on the latest storage event and reduced motion applies immediately', () => {
  const app = openAppearance();
  assert.equal(app.attributes.has('data-appearance-motion'), false);
  app.change('mode', 'dark');
  assert.equal(app.attributes.has('data-appearance-motion'), true);
  app.listeners.get('storage')({ key: 'nero.appearance.v1', newValue: JSON.stringify({ color: 'mono', mode: 'light' }) });
  assert.equal(app.root.dataset.mode, 'light');
  assert.equal(app.timers.size, 1);
  [...app.timers.values()][0]();
  assert.equal(app.attributes.has('data-appearance-motion'), false);
  const transitions = viewTransitions();
  const reduced = openAppearance({ reduced: true, startTransition: transitions.start });
  reduced.change('mode', 'dark');
  assert.equal(reduced.root.dataset.mode, 'dark');
  assert.equal(transitions.entries.length, 0);
});

test('turning reduced motion on cancels a pending transition and commits the latest preference', () => {
  const transitions = viewTransitions();
  const app = openAppearance({ startTransition: transitions.start });
  app.change('mode', 'dark');
  app.motion.matches = true; app.listeners.get('motion')();
  assert.equal(app.root.dataset.mode, 'dark');
  assert.equal(transitions.entries[0].skipped, true);
  transitions.entries[0].run();
  assert.equal(app.root.dataset.mode, 'dark');
});


test('a failing view transition API uses the bounded CSS fallback', () => {
  const app = openAppearance({ startTransition: () => { throw Error('not available'); } });
  app.change('mode', 'dark');
  assert.equal(app.root.dataset.mode, 'dark');
  assert.equal(app.attributes.has('data-appearance-motion'), true);
  [...app.timers.values()][0]();
  assert.equal(app.attributes.has('data-appearance-motion'), false);
});
