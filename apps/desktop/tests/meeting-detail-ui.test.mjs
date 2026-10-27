import assert from 'node:assert/strict';
import test from 'node:test';
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
      id, dataset: {}, hidden: false, scrollMax: Infinity, tabIndex: -1, listeners: {}, attributes: {},
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
    scroll(view, value) { const panel = node(`${view}-section`); panel.scrollTop = value; panel.listeners.scroll(); },
    clamp(view, maximum) { const panel = node(`${view}-section`); panel.scrollMax = maximum; panel.scrollTop = panel.scrollTop; },
    key(view, key, rest = {}) { let prevented = false; node(`detail-${view}-tab`).listeners.keydown({ key, preventDefault() { prevented = true; }, ...rest }); return prevented; },
  };
}

test('initial view waits for both result loads, then resolves its default once', () => {
  const app = page();
  app.update({});
  assert.equal(app.controller.active(), 'summary');
  app.update({ transcriptLoaded: true, hasTranscript: true });
  assert.equal(app.controller.active(), 'summary');
  app.update({ transcriptLoaded: true, summaryLoaded: true, hasTranscript: true });
  assert.equal(app.controller.active(), 'summary');
  app.update({ transcriptLoaded: true, summaryLoaded: true, hasTranscript: true, hasSummary: true });
  assert.equal(app.controller.active(), 'summary');
  app.update({ id: 'b', transcriptLoaded: true, summaryLoaded: true, hasTranscript: true, hasSummary: true });
  assert.equal(app.controller.active(), 'summary');
});

test('recording and meetings without generated results default to notes', () => {
  const app = page();
  app.update({ recording: true });
  assert.equal(app.controller.active(), 'notes');
  app.update({ recording: true, summaryLoaded: true, transcriptLoaded: true });
  assert.equal(app.controller.active(), 'notes');
  app.update({ id: 'b', summaryLoaded: true, transcriptLoaded: true });
  assert.equal(app.controller.active(), 'notes');
});

test('opening a recording prioritizes notes even when results finish loading', () => {
  const app = page();
  app.update({ recording: true });
  app.update({ recording: true, summaryLoaded: true, transcriptLoaded: true, hasSummary: true, hasTranscript: true });
  assert.equal(app.controller.active(), 'notes');
  app.update({ id: 'b', summaryLoaded: true, transcriptLoaded: true, hasSummary: true, hasTranscript: true });
  assert.equal(app.controller.active(), 'summary');
  app.update({ id: 'b', recording: true, summaryLoaded: true, transcriptLoaded: true, hasSummary: true, hasTranscript: true });
  assert.equal(app.controller.active(), 'summary');
});

test('explicitly choosing even the provisional tab prevents late automatic changes', async () => {
  const app = page();
  app.update({});
  assert.equal(await app.controller.select('summary'), true);
  app.update({ summaryLoaded: true, transcriptLoaded: true, hasTranscript: true });
  assert.equal(app.controller.active(), 'summary');
  assert.equal(await app.controller.select('notes'), true);
  app.update({ summaryLoaded: true, transcriptLoaded: true, hasTranscript: true, hasSummary: true });
  assert.equal(app.controller.active(), 'notes');
});

test('late loading cannot change panels while an explicit selection waits for saves', async () => {
  const saved = deferred();
  const app = page(async action => { await saved.promise; return action(); });
  app.update({});
  const selecting = app.controller.select('notes');
  app.update({ summaryLoaded: true, transcriptLoaded: true, hasTranscript: true });
  assert.equal(app.controller.active(), 'summary');
  saved.resolve();
  assert.equal(await selecting, true);
  assert.equal(app.controller.active(), 'notes');
});

test('each panel restores scroll after tab and meeting transitions', async () => {
  const app = page();
  app.update({ summaryLoaded: true, transcriptLoaded: true, hasSummary: true });
  app.scroll('summary', 410);
  await app.controller.select('transcript');
  app.scroll('transcript', 1230);
  app.scroll('summary', 0);
  await app.controller.select('summary');
  assert.equal(app.node('summary-section').scrollTop, 410);
  app.update({ id: 'b', summaryLoaded: true, transcriptLoaded: true, hasTranscript: true });
  await app.controller.select('transcript');
  app.scroll('transcript', 250);
  app.update({ id: 'a', summaryLoaded: true, transcriptLoaded: true, hasSummary: true });
  assert.equal(app.controller.active(), 'summary');
  assert.equal(app.node('summary-section').scrollTop, 410);
  await app.controller.select('transcript');
  assert.equal(app.node('transcript-section').scrollTop, 1230);
  app.update({ id: 'b', summaryLoaded: true, transcriptLoaded: true, hasTranscript: true });
  assert.equal(app.node('transcript-section').scrollTop, 250);
});

test('ordinary updates do not reset user scroll or repeatedly reveal the same panel', () => {
  const app = page();
  app.update({ summaryLoaded: true, transcriptLoaded: true, hasSummary: true });
  app.node('summary-section').scrollTop = 520;
  app.update({ summaryLoaded: true, transcriptLoaded: true, hasSummary: true });
  assert.equal(app.node('summary-section').scrollTop, 520);
  assert.equal(app.reveals.length, 1);
  assert.equal(app.reveals[0].visible, true);
});

for (const view of ['summary', 'transcript']) {
  test(`${view} restores saved scroll only after its content loads on a meeting revisit`, async () => {
    const app = page();
    const loaded = { summaryLoaded: true, transcriptLoaded: true, hasSummary: true, hasTranscript: true };
    app.update(loaded);
    await app.controller.select(view);
    app.clamp(view, 3000);
    app.scroll(view, 1230);
    app.update({ id: 'b', ...loaded });
    app.clamp(view, 0);
    app.update({ id: 'a' });
    assert.equal(app.controller.active(), view);
    assert.equal(app.node(`${view}-section`).scrollTop, 0);
    app.node(`${view}-section`).listeners.scroll();
    app.update({ id: 'a', [view === 'summary' ? 'transcriptLoaded' : 'summaryLoaded']: true });
    app.node(`${view}-section`).listeners.scroll();
    app.clamp(view, 3000);
    app.update({ id: 'a', ...loaded });
    assert.equal(app.node(`${view}-section`).scrollTop, 1230);
    app.scroll(view, 1350);
    app.update({ id: 'a', ...loaded });
    assert.equal(app.node(`${view}-section`).scrollTop, 1350);
  });
}

test('pending scroll restoration follows the selected meeting and panel', async () => {
  const app = page();
  const loaded = { summaryLoaded: true, transcriptLoaded: true, hasSummary: true, hasTranscript: true };
  app.update(loaded);
  await app.controller.select('transcript');
  app.scroll('transcript', 1200);
  app.update({ id: 'b', ...loaded });
  app.clamp('transcript', 0);
  app.update({ id: 'a' });
  await app.controller.select('notes');
  app.clamp('transcript', 3000);
  app.update({ id: 'a', ...loaded });
  assert.equal(app.controller.active(), 'notes');
  await app.controller.select('transcript');
  assert.equal(app.node('transcript-section').scrollTop, 1200);
  app.update({ id: 'b', ...loaded });
  await app.controller.select('transcript');
  assert.equal(app.node('transcript-section').scrollTop, 0);
});

test('notes textarea restores its inner scroll independently for each meeting', async () => {
  const app = page();
  app.update({ recording: true });
  app.node('meeting-notes').scrollTop = 320;
  app.node('meeting-notes').listeners.scroll();
  app.update({ id: 'b', recording: true });
  assert.equal(app.node('meeting-notes').scrollTop, 0);
  app.node('meeting-notes').scrollTop = 40;
  app.node('meeting-notes').listeners.scroll();
  app.update({ id: 'a', recording: true });
  assert.equal(app.node('meeting-notes').scrollTop, 320);
  await app.controller.select('summary');
  await app.controller.select('notes');
  assert.equal(app.node('meeting-notes').scrollTop, 320);
  app.update({ id: 'b', recording: true });
  assert.equal(app.node('meeting-notes').scrollTop, 40);
});

test('save failure keeps the original panel visible and reports the error', async () => {
  const error = new Error('저장 실패');
  const app = page(async () => { throw error; });
  app.update({ summaryLoaded: true, transcriptLoaded: true, hasSummary: true });
  assert.equal(await app.controller.select('notes', { focus: true }), false);
  assert.equal(app.controller.active(), 'summary');
  assert.equal(app.node('summary-section').hidden, false);
  assert.equal(app.node('notes-section').hidden, true);
  assert.equal(app.focused(), 'detail-summary-tab');
  assert.equal(app.errors[0], error);
});

test('a newer selection wins when pending saves complete out of order', async () => {
  const waits = [];
  const app = page(async action => { const saved = deferred(); waits.push(saved); await saved.promise; return action(); });
  app.update({ summaryLoaded: true, transcriptLoaded: true, hasSummary: true });
  const first = app.controller.select('transcript');
  const second = app.controller.select('notes');
  waits[1].resolve();
  assert.equal(await second, true);
  waits[0].resolve();
  assert.equal(await first, false);
  assert.equal(app.controller.active(), 'notes');
});

test('pending selection for a previous meeting cannot change the next meeting', async () => {
  const saved = deferred();
  const app = page(async action => { await saved.promise; return action(); });
  app.update({ summaryLoaded: true, transcriptLoaded: true, hasSummary: true });
  const selecting = app.controller.select('notes', { focus: true });
  app.update({ id: 'b', summaryLoaded: true, transcriptLoaded: true, hasTranscript: true });
  saved.resolve();
  assert.equal(await selecting, false);
  assert.equal(app.controller.active(), 'summary');
  assert.equal(app.focused(), null);
});

test('tab keyboard navigation activates and focuses with correct accessibility state', async () => {
  const app = page();
  app.update({ summaryLoaded: true, transcriptLoaded: true, hasSummary: true });
  assert.equal(app.key('summary', 'ArrowLeft'), true);
  await tick();
  assert.equal(app.controller.active(), 'notes');
  assert.equal(app.focused(), 'detail-notes-tab');
  assert.equal(app.node('detail-notes-tab').attributes['aria-selected'], 'true');
  assert.equal(app.node('detail-notes-tab').tabIndex, 0);
  assert.equal(app.node('detail-summary-tab').tabIndex, -1);
  assert.equal(app.key('notes', 'Home'), true);
  await tick();
  assert.equal(app.controller.active(), 'summary');
  app.key('summary', 'ArrowRight');
  await tick();
  assert.equal(app.controller.active(), 'transcript');
  app.key('transcript', 'End');
  await tick();
  assert.equal(app.controller.active(), 'notes');
});

test('composition and platform shortcut keys do not activate another panel', async () => {
  const app = page();
  app.update({ summaryLoaded: true, transcriptLoaded: true, hasSummary: true });
  assert.equal(app.key('summary', 'ArrowRight', { isComposing: true }), false);
  assert.equal(app.key('summary', 'ArrowRight', { keyCode: 229 }), false);
  assert.equal(app.key('summary', 'ArrowRight', { metaKey: true }), false);
  await tick();
  assert.equal(app.controller.active(), 'summary');
  assert.equal(await app.controller.select('invalid'), false);
});
