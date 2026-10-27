import test from 'node:test';
import assert from 'node:assert/strict';
import { mountSummaryEvidence } from '../src/summary-evidence.js';

const tick = () => new Promise(resolve => setImmediate(resolve));
const deferred = () => { let resolve, reject; const promise = new Promise((a, b) => { resolve = a; reject = b; }); return { promise, resolve, reject }; };
class Node {
  constructor(tag, ownerDocument) { this.tag = tag; this.ownerDocument = ownerDocument; this.children = []; this.parentNode = null; this.listeners = new Map(); this.dataset = {}; this.attributes = {}; this.style = {}; this.hidden = false; this.textContent = ''; }
  addEventListener(name, listener) { if (!this.listeners.has(name)) this.listeners.set(name, new Set()); this.listeners.get(name).add(listener); }
  removeEventListener(name, listener) { this.listeners.get(name)?.delete(listener); }
  emit(name, values = {}) { const event = { target: this, relatedTarget: null, preventDefault() {}, stopPropagation() {}, ...values }; return Promise.all([...this.listeners.get(name) || []].map(listener => listener(event))); }
  append(...children) { for (const child of children) { child.parentNode = this; this.children.push(child); } }
  replaceChildren(...children) { for (const child of this.children) child.parentNode = null; this.children = []; this.append(...children); }
  contains(other) { for (let item = other; item; item = item.parentNode) if (item === this) return true; return false; }
  get isConnected() { return this.ownerDocument?.body?.contains(this) ?? false; }
  get childElementCount() { return this.children.length; }
  setAttribute(name, value) { this.attributes[name] = value; }
  removeAttribute(name) { delete this.attributes[name]; }
  matches(selector) { return selector === 'button' ? this.tag === 'button' : selector === '[data-summary-citations]' ? this.dataset.summaryCitations != null : selector === '[data-evidence-action]' && this.dataset.evidenceAction != null; }
  closest(selector) { for (let item = this; item; item = item.parentNode) if (item.matches(selector)) return item; return null; }
  querySelectorAll(selector) { return this.children.flatMap(child => [...(child.matches(selector) ? [child] : []), ...child.querySelectorAll(selector)]); }
  querySelector(selector) { for (const child of this.children) { if (child.matches(selector)) return child; const nested = child.querySelector(selector); if (nested) return nested; } return null; }
  focus() { this.ownerDocument.activeElement = this; for (let item = this; item; item = item.parentNode) void item.emit('focusin', { target: this }); void this.ownerDocument.emit('focusin', { target: this }); }
  remove() { this.parentNode.children = this.parentNode.children.filter(item => item !== this); this.parentNode = null; }
  getBoundingClientRect() { return this.tag === 'button' ? { left: 290, top: 240, right: 310, bottom: 264, width: 20, height: 24 } : { width: 300, height: 200 }; }
}
function page(loader) {
  const doc = new Node('document');
  const win = new Node('window'); Object.assign(win, { innerWidth: 320, innerHeight: 280, setTimeout, clearTimeout });
  doc.defaultView = win; doc.createElement = tag => new Node(tag, doc); doc.body = doc.createElement('body');
  const container = doc.createElement('div'), anchor = doc.createElement('button'); anchor.dataset.summaryCitations = '1,2';
  doc.body.append(container); container.append(anchor);
  let context = { meetingId: 'm1', summaryId: 's1', currentVersion: { id: 't1', revision: 1 }, dirty: false, canPlay: true };
  const calls = [], navigations = [], plays = [], errors = [];
  const result = () => ({ summaryId: 's1', sourceVersion: { id: 'old', revision: 0 }, currentVersion: context.currentVersion, sourceSessionId: 'recording-1', audioAvailable: true, entries: [{ reference: 1, segment: { id: 'old-row', start: 62, end: 66, text: '<img src=x> frozen' }, speakerName: '요약 당시 이름', currentSegmentIds: ['current-row'] }] });
  const controller = mountSummaryEvidence({ container, getContext: () => context, loadCitations: (...args) => { calls.push(args); return loader ? loader(...args) : Promise.resolve(result()); }, onNavigate: (...args) => { navigations.push(args); return true; }, onPlay: (...args) => plays.push(args), onError: error => errors.push(error) });
  const panel = doc.body.children.at(-1);
  const all = root => [root, ...root.children.flatMap(all)];
  return { doc, win, container, anchor, panel, controller, calls, navigations, plays, errors, result, all, context: values => { context = { ...context, ...values }; }, button: action => all(panel).find(item => item.dataset.evidenceAction === action), open: async (pin = false) => { await container.emit(pin ? 'click' : 'pointerover', { target: anchor, detail: 1 }); await tick(); } };
}

test('hover shows frozen text without navigation or playback; only explicit actions play or navigate', async () => {
  const app = page(); await app.open();
  assert.deepEqual(app.calls, [['m1', 's1', [1, 2]]]);
  assert.ok(app.all(app.panel).some(node => node.textContent === '<img src=x> frozen'));
  assert.ok(app.all(app.panel).some(node => node.textContent === '01:02'));
  assert.equal(app.navigations.length, 0); assert.equal(app.plays.length, 0);
  await app.panel.emit('click', { target: app.button('play') });
  assert.equal(app.plays.length, 1); assert.equal(app.panel.hidden, false);
  await app.panel.emit('click', { target: app.button('navigate') });
  assert.equal(app.navigations.length, 1); assert.equal(app.panel.hidden, true);
  assert.equal(app.navigations[0][0].segment.id, 'old-row');
  assert.equal(app.controller.restoreFocus(), true); assert.equal(app.doc.activeElement, app.anchor); assert.equal(app.panel.hidden, true);
  app.controller.destroy();
});

test('late evidence cannot show after meeting switch, summary replacement, dirty edit, or reset', async () => {
  for (const change of [{ meetingId: 'm2' }, { summaryId: 's2' }, { dirty: true }, { currentVersion: { id: 't1', revision: 2 } }, null]) {
    const pending = deferred(), app = page(() => pending.promise);
    await app.open(); if (change) app.context(change); else app.controller.reset();
    pending.resolve(app.result()); await tick();
    assert.equal(app.panel.hidden, true); assert.equal(app.panel.children.length, 0);
    app.controller.destroy();
  }
});

test('replaced anchor and mismatched response never expose unrelated evidence', async () => {
  const pending = deferred(), app = page(() => pending.promise); await app.open();
  app.container.replaceChildren(); pending.resolve(app.result()); await tick();
  assert.equal(app.panel.hidden, true); app.controller.destroy();
  const wrong = page(() => Promise.resolve({ summaryId: 'other', entries: [{ reference: 1, segment: { text: 'other meeting' } }] })); await wrong.open();
  assert.ok(wrong.all(wrong.panel).some(node => node.textContent === '근거를 불러오지 못했습니다.'));
  assert.ok(!wrong.all(wrong.panel).some(node => node.textContent === 'other meeting')); wrong.controller.destroy();
});

test('dirty frozen source remains readable while current-target navigation and unavailable audio stay absent', async () => {
  const app = page(); app.context({ dirty: true, canPlay: false }); await app.open();
  assert.ok(app.all(app.panel).some(node => node.textContent === '<img src=x> frozen'));
  assert.equal(app.button('navigate'), undefined); assert.equal(app.button('play'), undefined);
  app.controller.destroy();
});

test('click pins, Escape closes and restores focus, outside interaction closes without stealing focus', async () => {
  const app = page(); await app.open(true);
  await app.panel.emit('pointerleave'); await new Promise(resolve => setTimeout(resolve, 180)); assert.equal(app.panel.hidden, false);
  await app.doc.emit('keydown', { key: 'Escape' }); assert.equal(app.panel.hidden, true); assert.equal(app.doc.activeElement, app.anchor);
  await app.open(true); const outside = app.doc.createElement('button'); app.doc.body.append(outside); outside.focus();
  assert.equal(app.panel.hidden, true); assert.equal(app.doc.activeElement, outside);
  app.controller.destroy();
});

test('keyboard enters panel and loading completion preserves panel focus; viewport clamps its geometry', async () => {
  const pending = deferred(), app = page(() => pending.promise); await app.open();
  await app.container.emit('keydown', { target: app.anchor, key: 'ArrowDown' });
  assert.equal(app.doc.activeElement, app.button('close'));
  pending.resolve(app.result()); await tick();
  assert.equal(app.doc.activeElement, app.button('close'));
  assert.equal(app.panel.style.maxWidth, '304px'); assert.equal(app.panel.style.left, '12px'); assert.equal(app.panel.style.top, '34px');
  app.context({ meetingId: 'm2' }); app.controller.update(); assert.equal(app.panel.hidden, true); assert.equal(app.controller.restoreFocus(), false);
  app.controller.destroy();
});

test('fetch failures are recoverable and destruction removes the panel and listeners', async () => {
  const app = page(() => Promise.reject(new Error('offline'))); await app.open();
  assert.equal(app.errors.length, 1); assert.ok(app.all(app.panel).some(node => node.textContent === '근거를 불러오지 못했습니다.'));
  app.controller.destroy(); assert.equal(app.panel.isConnected, false);
  await app.container.emit('click', { target: app.anchor }); assert.equal(app.calls.length, 1);
});

test('return focus finds the same citation after source edits rerender the summary', async () => {
  const app = page(); await app.open(true);
  await app.panel.emit('click', {target: app.button('navigate')});
  const replacement = app.doc.createElement('button'); replacement.dataset.summaryCitations = '1,2';
  app.container.replaceChildren(replacement);
  app.context({currentVersion: {id: 't1', revision: 2}});
  assert.equal(app.controller.restoreFocus(), true);
  assert.equal(app.doc.activeElement, replacement);
  app.controller.destroy();
});
