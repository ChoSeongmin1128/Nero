import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { Window } from 'happy-dom';
import { mountMeetingDetail } from '../src/meeting-detail-ui.js';
const html = await readFile(new URL('../src/index.html', import.meta.url), 'utf8');
const tick = () => new Promise(resolve => setImmediate(resolve));
function page(t, finishEditing = async action => action()) {
  const window = new Window({ settings: { disableCSSFileLoading: true, disableJavaScriptFileLoading: true } });
  window.document.write(html); t.after(() => window.happyDOM.abort());
  const node = id => window.document.getElementById(id), errors = [], reveals = [];
  node('meeting-detail').hidden = false;
  const controller = mountMeetingDetail({ byId: node, finishEditing, onError: error => errors.push(error), onReveal: view => reveals.push(view) });
  return { window, node, controller, errors, reveals, update: values => controller.update({ id: 'a', summaryLoaded: true, transcriptLoaded: true, hasTranscript: true, ...values }) };
}
test('summary and transcript share a permanent body while notes are independent', async t => {
  const app = page(t); app.update({ hasSummary: true });
  const body = app.node('detail-body-scroll');
  assert.ok(body.contains(app.node('summary-section'))); assert.ok(body.contains(app.node('transcript-section')));
  for (const view of ['notes', 'transcript', 'summary']) {
    assert.equal(await app.controller.select(view), true);
    assert.equal(app.node('summary-section').hidden, false); assert.equal(app.node('transcript-section').hidden, false);
    assert.ok(body.contains(app.node('transcript-section')));
    assert.equal(app.controller.notesOpen(), true);
  }
  app.node('detail-inspector-close').click(); await tick();
  assert.equal(app.controller.notesOpen(), false); assert.equal(app.node('transcript-section').hidden, false);
  assert.equal(app.window.document.activeElement, app.node('detail-notes-open'));
});
test('pending saves block notes closing without losing visible text', async t => {
  let block = false; const app = page(t, async action => { if (block) throw Error('save'); return action(); }); app.update();
  await app.controller.select('notes'); app.node('meeting-notes').value = '보존할 메모'; block = true;
  app.node('detail-inspector-close').click(); await tick();
  assert.equal(app.controller.notesOpen(), true); assert.equal(app.node('meeting-notes').value, '보존할 메모'); assert.equal(app.errors.length, 1);
});
test('meeting changes and newer intent invalidate old pending navigation', async t => {
  const pending = []; const app = page(t, action => new Promise(resolve => pending.push(() => resolve(action())))); app.update();
  const a = app.controller.select('notes'), b = app.controller.select('transcript'); pending[1](); assert.equal(await b, true); pending[0](); assert.equal(await a, false);
  const c = app.controller.select('notes'); app.update({ id: 'b' }); pending[2](); assert.equal(await c, false); assert.equal(app.controller.notesOpen(), false);
});
test('body and notes scroll restore only after the current meeting content is loaded', async t => {
  const app = page(t); app.update(); await app.controller.select('notes');
  app.node('detail-body-scroll').scrollTop = 900; app.node('meeting-notes').scrollTop = 220; app.controller.rememberScroll();
  app.update({ id: 'b' }); app.update({ id: 'a', summaryLoaded: false, transcriptLoaded: false });
  app.node('detail-body-scroll').scrollTop = 0; app.node('meeting-notes').scrollTop = 0; app.controller.rememberScroll();
  app.update(); assert.equal(app.node('detail-body-scroll').scrollTop, 900); assert.equal(app.node('meeting-notes').scrollTop, 220); assert.equal(app.controller.notesOpen(), true);
});
test('recording reveals notes once and never displaces the shared transcript', async t => {
  const app = page(t); app.update({ recording: true, hasPreview: true }); assert.equal(app.controller.notesOpen(), true);
  app.node('detail-notes-open').click(); await tick(); assert.equal(app.controller.notesOpen(), false);
  app.update({ recording: true }); app.update({ recording: false }); assert.equal(app.controller.notesOpen(), false); assert.equal(app.node('transcript-section').hidden, false);
});
test('empty and completed meetings keep stable navigation entries and contextual availability', t => {
  const app = page(t); app.update({ hasTranscript: false, showSummary: false });
  assert.equal(app.node('detail-transcript-open').hidden, false); assert.equal(app.node('detail-transcript-open').disabled, true);
  app.update({ hasAudio: true }); assert.equal(app.node('detail-transcript-open').disabled, false);
  assert.equal(app.node('transcript-section').hidden, false);
});
test('narrow notes layout keeps body present and transcript maintenance is secondary', async t => {
  const css = await readFile(new URL('../src/style.css', import.meta.url), 'utf8');
  assert.doesNotMatch(css, /data-inspector[^}]*\.detail-primary\s*\{\s*display:\s*none/);
  assert.match(css, /grid-row: 3/); assert.match(html, /transcript-maintenance/);
  const app = page(t); app.update(); await app.controller.select('notes');
  assert.equal(app.node('detail-primary')?.hidden ?? false, false);
  assert.equal(app.node('notes-section').parentElement, app.node('detail-inspector'));
});
