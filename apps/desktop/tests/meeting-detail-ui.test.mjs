import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { mountMeetingDetail } from '../src/meeting-detail-ui.js';

const tick = () => new Promise(resolve => setImmediate(resolve));
function deferred() { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; }

function page(finish = async action => action()) {
  const elements = new Map(), reveals = [], errors = [];
  let focused = null;
  function node(id) {
    if (!elements.has(id)) {
      let scroll = 0;
      elements.set(id, {
      id, querySelector() { return null; }, parentNode: null, contains() { return false; }, append(child) { child.parentNode = this; }, insertBefore(child) { child.parentNode = this; }, dataset: {}, hidden: false, scrollMax: Infinity, tabIndex: -1, listeners: {}, attributes: {},
      get scrollTop() { return Math.min(this.scrollMax, scroll); },
      set scrollTop(value) { scroll = Math.max(0, Math.min(this.scrollMax, value)); },
      addEventListener(name, listener) { this.listeners[name] = listener; },
      setAttribute(name, value) { this.attributes[name] = value; },
      focus() { focused = this.id; },
      });
    }
    return elements.get(id);
  }
  const tabs = ['summary', 'transcript', 'notes'].map(view => { const tab = node(`detail-${view}-tab`); tab.dataset.detailTab = view; return tab; });
  node('detail-tabs').querySelectorAll = () => tabs;
  for (const view of ['summary', 'transcript', 'notes']) node(`${view}-section`);
  const controller = mountMeetingDetail({ byId: node, finishEditing: finish, onReveal(view) { reveals.push({ view, visible: !node(`${view}-section`).hidden, scroll: node(`${view}-section`).scrollTop }); }, onError: error => errors.push(error) });
  return {
    controller, node, reveals, errors, focused: () => focused,
    update: values => controller.update({ id: 'a', ...values }),
    scroll(view, value) { const panel = node(view === 'transcript' ? 'transcript-scroll' : `${view}-section`); panel.scrollTop = value; panel.listeners.scroll(); },
    clamp(view, maximum) { const panel = node(view === 'transcript' ? 'transcript-scroll' : `${view}-section`); panel.scrollMax = maximum; panel.scrollTop = panel.scrollTop; },
    key(view, key, rest = {}) { let prevented = false; node(`detail-${view}-tab`).listeners.keydown({ key, preventDefault() { prevented = true; }, ...rest }); return prevented; },
  };
}

test('summary remains visible alongside a one-click nonmodal transcript inspector', async () => {
  const app = page(); app.update({summaryLoaded: true, transcriptLoaded: true});
  assert.equal(app.controller.active(), 'summary');
  assert.equal(app.node('detail-inspector').hidden, true);
  app.node('detail-transcript-open').listeners.click(); await tick();
  assert.equal(app.controller.active(), 'transcript');
  assert.equal(app.node('summary-section').hidden, false);
  assert.equal(app.node('transcript-section').hidden, false);
  assert.equal(app.node('detail-transcript-open').attributes['aria-expanded'], 'true');
  assert.equal(app.focused(), 'detail-inspector-close');
  assert.equal(app.node('detail-inspector').attributes['aria-modal'], undefined);
});
test('recording opens notes but ordinary empty meetings start with summary', () => {
  const app = page(); app.update({recording: true}); assert.equal(app.controller.active(), 'notes');
  app.update({id: 'b', summaryLoaded: true, transcriptLoaded: true}); assert.equal(app.controller.active(), 'summary');
});
test('summary and transcript positions survive notes, closing and meeting changes', async () => {
  const app = page(), loaded = {summaryLoaded: true, transcriptLoaded: true}; app.update(loaded);
  app.scroll('summary', 410); await app.controller.select('transcript'); app.scroll('transcript', 1200);
  await app.controller.select('notes'); app.node('meeting-notes').scrollTop = 200; app.node('meeting-notes').listeners.scroll();
  await app.controller.select('transcript'); assert.equal(app.node('transcript-scroll').scrollTop, 1200); assert.equal(app.node('summary-section').scrollTop, 410);
  app.node('detail-inspector-close').listeners.click(); await tick(); assert.equal(app.controller.active(), 'summary'); assert.equal(app.focused(), 'detail-transcript-open');
  app.update({id: 'b', ...loaded}); app.update({id: 'a', ...loaded}); await app.controller.select('notes'); assert.equal(app.node('meeting-notes').scrollTop, 200);
});
test('pending edits block close and failed save keeps the inspector open', async () => {
  let block = false; const app = page(async action => {if (block) throw Error('save'); return action();}); app.update({}); await app.controller.select('transcript'); block = true;
  assert.equal(await app.controller.select('summary'), false); assert.equal(app.controller.active(), 'transcript'); assert.equal(app.errors.length, 1);
});
test('meeting change and newer intent invalidate pending panel navigation', async () => {
  const waits = []; const app = page(async action => {const d = deferred(); waits.push(d); await d.promise; return action();}); app.update({});
  const first = app.controller.select('transcript'), second = app.controller.select('notes'); waits[1].resolve(); assert.equal(await second, true); waits[0].resolve(); assert.equal(await first, false);
  const pending = app.controller.select('transcript'); app.update({id:'b'}); waits[2].resolve(); assert.equal(await pending, false); assert.equal(app.controller.active(), 'summary');
});
test('transcript position is restored only after the revisited content loads', async () => {
  const app = page(), loaded = {summaryLoaded: true, transcriptLoaded: true}; app.update(loaded); await app.controller.select('transcript'); app.scroll('transcript', 900);
  app.update({id:'b', ...loaded}); app.clamp('transcript', 0); app.update({id:'a'}); app.node('transcript-scroll').listeners.scroll(); app.clamp('transcript', 2000); app.update(loaded); assert.equal(app.node('transcript-scroll').scrollTop, 900);
});

test('workspace markup has full-height sibling inspector, fixed search and available-detail-width narrow return', async () => {
  const html = await readFile(new URL('../src/index.html', import.meta.url), 'utf8');
  const css = await readFile(new URL('../src/style.css', import.meta.url), 'utf8');
  assert.doesNotMatch(html, /detail-summary-tab|detail-transcript-tab|detail-notes-tab|return-to-summary/);
  assert.match(html, /<aside id="detail-inspector"[^>]*aria-labelledby="detail-inspector-title"/);
  assert.match(html, /id="transcript-scroll"[\s\S]*id="transcript-rows"/);
  assert.match(css, /\.main-content \{ container-type: inline-size; container-name: detail;/);
  assert.match(html, /id="inspector-audio-toggle"/);
  assert.match(css, /@container detail \(max-width: 759px\)/);
  assert.match(css, /\.inspector-back \{ display: inline; \}/);
  assert.doesNotMatch(css, /detail-inspector[^}]*backdrop-filter/);
});

test('meeting updates never read a replacement DOM back into the previous meeting memory', async () => {
  const app=page(), loaded={summaryLoaded:true,transcriptLoaded:true}; app.update(loaded);
  app.scroll('summary',410); await app.controller.select('transcript'); app.scroll('transcript',1200); await app.controller.select('notes'); app.node('meeting-notes').scrollTop=320; app.node('meeting-notes').listeners.scroll();
  app.node('summary-section').scrollTop=0; app.node('transcript-scroll').scrollTop=0; app.node('meeting-notes').scrollTop=0;
  app.update({id:'b',...loaded}); app.update({id:'a',...loaded}); assert.equal(app.node('meeting-notes').scrollTop,320); assert.equal(app.node('summary-section').scrollTop,410);
  await app.controller.select('transcript'); assert.equal(app.node('transcript-scroll').scrollTop,1200);
});

test('starting the first recording in an already-open empty meeting reveals notes once', async () => {
  const app=page(), loaded={summaryLoaded:true,transcriptLoaded:true}; app.update(loaded); assert.equal(app.controller.active(),'summary');
  app.update({...loaded,recording:true}); assert.equal(app.controller.active(),'notes');
  await app.controller.select('summary'); app.update({...loaded,recording:true}); assert.equal(app.controller.active(),'summary');
  app.update({...loaded,recording:false}); app.update({...loaded,recording:true}); assert.equal(app.controller.active(),'summary');
});
test('explicit summary or transcript intent prevents recording snapshots from reopening notes', async () => {
  for(const view of ['summary','transcript']) {
    const app=page(), loaded={summaryLoaded:true,transcriptLoaded:true}; app.update(loaded); await app.controller.select(view);
    app.update({...loaded,recording:true}); assert.equal(app.controller.active(),view);
    app.update({...loaded,recording:true}); assert.equal(app.controller.active(),view);
  }
});
test('recording waits for result discovery and keeps completed meetings on summary', () => {
  const app=page(); app.update({}); app.update({recording:true}); assert.equal(app.controller.active(),'summary');
  app.update({recording:true,summaryLoaded:true,transcriptLoaded:true}); assert.equal(app.controller.active(),'notes');
  for(const result of [{hasTranscript:true},{hasSummary:true}]) {
    const completed=page(), loaded={summaryLoaded:true,transcriptLoaded:true,...result}; completed.update(loaded); completed.update({...loaded,recording:true}); assert.equal(completed.controller.active(),'summary');
  }
});
