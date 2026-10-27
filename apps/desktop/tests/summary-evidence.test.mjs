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
  const panel = doc.createElement('section'); panel.hidden = true; doc.body.append(panel);
  const controller = mountSummaryEvidence({ container, panel, onOpen: async () => true, getContext: () => context, loadCitations: (...args) => { calls.push(args); return loader ? loader(...args) : Promise.resolve(result()); }, onNavigate: (...args) => { navigations.push(args); return true; }, onPlay: (...args) => plays.push(args), onError: error => errors.push(error) });
  const all = root => [root, ...root.children.flatMap(all)];
  return { doc, win, container, anchor, panel, controller, calls, navigations, plays, errors, result, all, context: values => { context = { ...context, ...values }; }, button: action => all(panel).find(item => item.dataset.evidenceAction === action), open: async (pin = false) => { await container.emit('click', { target: anchor, detail: 1 }); await tick(); } };
}

test('click opens the transcript and navigates verified IDs without playback or a body popover', async () => {
  const app = page(); await app.open(); assert.deepEqual(app.calls, [['m1','s1',[1,2]]]); assert.equal(app.navigations.length,1); assert.equal(app.plays.length,0); assert.equal(app.panel.hidden,true); assert.equal(app.panel.attributes.role,undefined); assert.equal(app.doc.body.children.length,2);
});
test('unmapped evidence retains frozen text in the same inspector with separate playback', async () => {
  const app = page(async () => ({summaryId:'s1', audioAvailable:true, entries:[{reference:1, segment:{start:62,text:'frozen'}, currentSegmentIds:[]}]})); await app.open();
  assert.equal(app.navigations.length,0); assert.ok(app.all(app.panel).some(node=>node.textContent==='frozen')); assert.ok(app.all(app.panel).some(node=>node.textContent==='요약 당시 발언')); assert.equal(app.plays.length,0); await app.panel.emit('click',{target:app.button('play')}); assert.equal(app.plays.length,1);
});
test('late evidence is discarded across meeting, summary, version, dirty and close boundaries', async () => {
  for (const change of [{meetingId:'m2'},{summaryId:'s2'},{currentVersion:{id:'t1',revision:2}},{dirty:true},null]) {
    const pending=deferred(), app=page(()=>pending.promise); await app.open(); if(change) app.context(change); else app.controller.reset(); pending.resolve(app.result()); await tick(); assert.equal(app.navigations.length,0); assert.equal(app.panel.hidden,true);
  }
});
test('multiple links navigate one at a time and never expose unrequested references', async () => {
  const app=page(async()=>({summaryId:'s1',entries:[1,2,99].map(reference=>({reference,currentSegmentIds:['row-'+reference],segment:{text:'text'}}))})); await app.open();
  assert.equal(app.navigations.length,1); assert.ok(app.all(app.panel).some(node=>node.textContent==='연결된 발언 1 / 2')); await app.panel.emit('click',{target:app.button('next')}); assert.equal(app.navigations.length,2); assert.deepEqual(app.navigations[1][0].currentSegmentIds,['row-2']); assert.equal(app.button('next').disabled,true);
});
test('hover and focus do not open a transient evidence mode', async () => {
  const app=page(); await app.container.emit('pointerover',{target:app.anchor}); await app.container.emit('focusin',{target:app.anchor}); await tick(); assert.equal(app.calls.length,0);
});
