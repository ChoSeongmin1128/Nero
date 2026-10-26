import assert from 'node:assert/strict';
import test from 'node:test';
import { mountTranscriptActions } from '../src/transcript-actions.js';

const settle = () => new Promise(resolve => setImmediate(resolve));
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const escape = value => String(value ?? '').replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);
const version = (id = 'current', revision = 1) => ({ id, revision });
const history = () => ({ entries: [
  { version: version(), current: true, updatedAt: 10000, segmentCount: 30, speakerCount: 2, edited: false },
  { version: version('previous', 3), current: false, updatedAt: 5000, segmentCount: 28, speakerCount: 2, edited: true },
], unavailable: 0 });

function harness({ context: override = {}, bridge: overrides = {}, finish } = {}) {
  const nodes = new Map(), calls = [], errors = [];
  const owner = { activeElement: null };
  class Node {
    constructor(id) { this.id = id; this.ownerDocument = owner; this.events = {}; this.open = false; this.isConnected = true; this.disabled = false; this.dataset = {}; }
    addEventListener(name, callback) { this.events[name] = callback; }
    setAttribute(name, value) { this[name] = value; }
    showModal() { this.open = true; }
    close() { this.open = false; }
    focus() { owner.activeElement = this; this.focused = true; }
    set innerHTML(value) {
      this.html = value;
      for (const type of ['confirm', 'close', 'reload']) {
        const element = byId(type);
        const match = new RegExp(`<button[^>]*data-transcript-actions-${type}[^>]*>`).exec(value);
        element.present = Boolean(match);
        element.disabled = Boolean(match && / disabled/.test(match[0]));
      }
    }
    get innerHTML() { return this.html || ''; }
    querySelector(selector) {
      const type = /data-transcript-actions-(confirm|close|reload)/.exec(selector)?.[1];
      const node = type && byId(type);
      return node?.present ? node : null;
    }
  }
  const byId = id => { if (!nodes.has(id)) nodes.set(id, new Node(id)); return nodes.get(id); };
  const context = { meeting: { id: 'meeting-a', deletedAt: null }, transcript: { version: version(), segments: [] }, processing: { status: 'idle' }, recording: false, ...override };
  const trigger = byId('trigger');
  owner.activeElement = trigger;
  const bridge = {
    readTranscriptHistory: async id => { calls.push(['history', id]); return history(); },
    restoreTranscript: async (...args) => { calls.push(['restore', ...args]); return { version: version('previous', 4) }; },
    ...overrides,
  };
  const controller = mountTranscriptActions({ byId, bridge, getContext: () => context,
    finishEditing: async action => { calls.push(['flush']); if (finish) await finish(context); return action(); },
    onRebuild: async id => { calls.push(['rebuild', id]); },
    onRestored: async (doc, id) => { calls.push(['restored', doc, id]); context.transcript = doc; },
    showError: error => errors.push(error instanceof Error ? error.message : String(error)), escape,
  });
  const select = index => byId('transcript-actions-body').events.change({ target: { dataset: { transcriptHistoryChoice: String(index) } } });
  const submit = async () => { byId('transcript-actions-dialog').events.submit({ preventDefault() {} }); await settle(); await settle(); };
  const click = async action => { byId('transcript-actions-body').events.click({ target: { closest: selector => selector === `[data-transcript-actions-${action}]` ? {} : null } }); await settle(); };
  return { byId, context, calls, errors, controller, select, submit, click, trigger };
}

test('rebuild flushes first and requires confirmation before replacing a document', async () => {
  const app = harness();
  await app.controller.rebuild();
  assert.deepEqual(app.calls, [['flush']]);
  assert.equal(app.byId('transcript-actions-dialog').open, true);
  assert.match(app.byId('transcript-actions-body').innerHTML, /현재 문장과 화자 수정이 새 결과로/);
  assert.match(app.byId('transcript-actions-body').innerHTML, /이전 기록에서 되돌릴 수/);
  await app.submit();
  assert.deepEqual(app.calls.at(-1), ['rebuild', 'meeting-a']);
  assert.equal(app.byId('transcript-actions-dialog').open, false);
  assert.equal(app.trigger.focused, true);
});

test('a first conversion starts without a replacement confirmation', async () => {
  const app = harness({ context: { transcript: null } });
  await app.controller.rebuild();
  assert.deepEqual(app.calls, [['flush'], ['rebuild', 'meeting-a']]);
  assert.equal(app.byId('transcript-actions-dialog').open, false);
});

test('replacement confirmation rejects a changed meeting, version, recording or processing job', async () => {
  for (const change of [
    context => { context.meeting = { id: 'meeting-b' }; },
    context => { context.transcript.version = version('current', 2); },
    context => { context.recording = true; },
    context => { context.processing = { status: 'running', meetingId: 'meeting-a' }; },
  ]) {
    const app = harness();
    await app.controller.rebuild();
    change(app.context);
    await app.submit();
    assert.equal(app.calls.some(call => call[0] === 'rebuild'), false);
    assert.match(app.byId('transcript-actions-body').innerHTML, /회의가 변경|음성 기록이 변경|녹음을 마친|처리가 끝난/);
  }
});

test('rebuild captures the version after the edit flush instead of rejecting its own save', async () => {
  const app = harness({ finish: context => { context.transcript.version.revision = 2; } });
  await app.controller.rebuild();
  await app.submit();
  assert.equal(app.calls.filter(call => call[0] === 'rebuild').length, 1);
});

test('history loads with visible activity and requires an explicit noncurrent selection', async () => {
  const read = deferred();
  const app = harness({ bridge: { readTranscriptHistory: async () => read.promise } });
  const opening = app.controller.openHistory();
  await settle();
  assert.match(app.byId('transcript-actions-body').innerHTML, /activity-spinner/);
  assert.equal(app.byId('confirm').disabled, true);
  read.resolve(history());
  await opening;
  assert.equal(app.byId('confirm').disabled, true);
  assert.match(app.byId('transcript-actions-body').innerHTML, /현재 사용 중/);
  assert.match(app.byId('transcript-actions-body').innerHTML, /문장 또는 화자 수정됨/);
  assert.doesNotMatch(app.byId('transcript-actions-body').innerHTML.replace(/<[^>]*>/g, ''), /previous|current|sourceSession|\.json/);
  app.select(0);
  await app.submit();
  assert.equal(app.calls.some(call => call[0] === 'restore'), false);
  app.select(1);
  assert.equal(app.byId('confirm').disabled, false);
  await app.submit();
  assert.deepEqual(app.calls.find(call => call[0] === 'restore'), ['restore', 'meeting-a', version(), version('previous', 3)]);
  assert.equal(app.calls.at(-1)[0], 'restored');
  assert.equal(app.byId('transcript-actions-dialog').open, false);
});

test('history read failures differ from an empty history and can be retried', async () => {
  let attempts = 0;
  const app = harness({ bridge: { readTranscriptHistory: async () => { attempts += 1; if (attempts === 1) throw new Error('저장 정보를 읽을 수 없습니다.'); return { entries: [], unavailable: 2 }; } } });
  await app.controller.openHistory();
  assert.match(app.byId('transcript-actions-body').innerHTML, /불러오지 못했습니다/);
  assert.doesNotMatch(app.byId('transcript-actions-body').innerHTML, /이전 음성 기록이 없습니다/);
  await app.click('reload');
  assert.match(app.byId('transcript-actions-body').innerHTML, /이전 음성 기록이 없습니다/);
  assert.match(app.byId('transcript-actions-body').innerHTML, /불러올 수 없는 기록 2개/);
  assert.equal(app.byId('confirm').disabled, true);
});

test('restore conflict keeps the dialog, reloads history and never silently changes expected version', async () => {
  let reads = 0;
  const app = harness({ bridge: {
    readTranscriptHistory: async () => { reads += 1; const result = history(); if (reads > 1) result.entries[0].version.revision = 2; return result; },
    restoreTranscript: async () => { throw new Error('음성 기록이 변경되었습니다.'); },
  } });
  await app.controller.openHistory();
  app.select(1);
  await app.submit();
  assert.equal(reads, 2);
  assert.equal(app.byId('transcript-actions-dialog').open, true);
  assert.match(app.byId('transcript-actions-body').innerHTML, /음성 기록이 변경되었습니다/);
  app.select(1);
  assert.equal(app.byId('confirm').disabled, true);
  assert.equal(app.calls.some(call => call[0] === 'restored'), false);
});

test('related processing blocks history restoration while an unrelated download does not', async () => {
  const app = harness();
  await app.controller.openHistory();
  app.select(1);
  app.context.processing = { status: 'running', meetingId: 'meeting-a' };
  await app.submit();
  assert.equal(app.calls.some(call => call[0] === 'restore'), false);
  app.context.processing = { status: 'running', meetingId: null, kind: 'summary-model' };
  app.select(1);
  await app.submit();
  assert.equal(app.calls.filter(call => call[0] === 'restore').length, 1);
});

test('reset and meeting changes invalidate late history responses', async () => {
  for (const reset of [true, false]) {
    const read = deferred();
    const app = harness({ bridge: { readTranscriptHistory: async () => read.promise } });
    const opening = app.controller.openHistory();
    await settle();
    app.context.meeting.id = 'meeting-b';
    if (reset) app.controller.reset();
    read.resolve(history());
    await opening;
    assert.equal(app.byId('transcript-actions-dialog').open, false);
    assert.equal(app.calls.some(call => call[0] === 'restore'), false);
  }
});

test('closing while loading restores focus and does not reopen from a late result', async () => {
  const read = deferred();
  const app = harness({ bridge: { readTranscriptHistory: async () => read.promise } });
  const opening = app.controller.openHistory();
  await settle();
  await app.click('close');
  assert.equal(app.trigger.focused, true);
  read.resolve(history());
  await opening;
  assert.equal(app.byId('transcript-actions-dialog').open, false);
});

test('an accepted restore finishes for its original meeting without reopening a reset dialog', async () => {
  const save = deferred();
  const app = harness({ bridge: { restoreTranscript: async () => save.promise } });
  await app.controller.openHistory();
  app.select(1);
  app.byId('transcript-actions-dialog').events.submit({ preventDefault() {} });
  await settle();
  assert.match(app.byId('transcript-actions-body').innerHTML, /되돌리는 중/);
  app.controller.reset();
  app.context.meeting.id = 'meeting-b';
  const result = { version: version('previous', 4) };
  save.resolve(result);
  await settle(); await settle();
  assert.deepEqual(app.calls.at(-1), ['restored', result, 'meeting-a']);
  assert.equal(app.byId('transcript-actions-dialog').open, false);
});


test('history remains readable during processing and recording but cannot restore', async () => {
  for (const context of [ { recording: { state: 'paused' } }, { processing: { status: 'running', meetingId: 'meeting-a' } } ]) {
    const app = harness({ context });
    await app.controller.openHistory();
    assert.equal(app.calls.some(call => call[0] === 'history'), true);
    assert.equal(app.byId('transcript-actions-dialog').open, true);
    app.select(1);
    assert.equal(app.byId('confirm').disabled, true);
    await app.submit();
    assert.equal(app.calls.some(call => call[0] === 'restore'), false);
  }
});

test('failed edits and changed meetings stop opening before any bridge action', async () => {
  const failed = harness({ finish: () => { throw new Error('수정 저장 실패'); } });
  await failed.controller.rebuild();
  assert.equal(failed.byId('transcript-actions-dialog').open, false);
  assert.equal(failed.calls.some(call => call[0] === 'rebuild'), false);
  assert.deepEqual(failed.errors, ['수정 저장 실패']);
  const changed = harness({ finish: context => { context.meeting.id = 'meeting-b'; } });
  await changed.controller.openHistory();
  assert.equal(changed.calls.some(call => call[0] === 'history'), false);
  assert.match(changed.errors[0], /회의가 변경/);
});

test('repeated confirmation and Escape do not interrupt or duplicate an accepted restore', async () => {
  const save = deferred();
  let calls = 0;
  const app = harness({ bridge: { restoreTranscript: async () => { calls += 1; return save.promise; } } });
  await app.controller.openHistory();
  app.select(1);
  await app.submit();
  await app.submit();
  app.byId('transcript-actions-dialog').events.cancel({ preventDefault() {} });
  assert.equal(calls, 1);
  assert.equal(app.byId('transcript-actions-dialog').open, true);
  save.resolve({ version: version('previous', 4) });
  await settle(); await settle();
  assert.equal(app.byId('transcript-actions-dialog').open, false);
});
