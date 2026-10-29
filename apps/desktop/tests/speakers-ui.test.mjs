import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';

const source = await readFile(new URL('../src/speakers-ui.js', import.meta.url), 'utf8');
const processingStateSource = await readFile(new URL('../src/processing-state.js', import.meta.url), 'utf8');
const tick = () => new Promise(setImmediate);
const plain = value => JSON.parse(JSON.stringify(value));
const escape = value => String(value ?? '').replace(/[&<>"']/g, character => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[character]);
const decode = value => value.replace(/&(?:amp|lt|gt|quot|#39);/g, entity => ({ '&amp;': '&', '&lt;': '<', '&gt;': '>', '&quot;': '"', '&#39;': "'" })[entity]);
const documentForTest = () => ({
  version: { id: 'document', revision: 0 },
  speakers: [{ id: 'S1', name: '화자 1' }, { id: 'S2', name: '화자 2' }],
  segments: [
    { id: 'a', start: 1, end: 2, speakerId: 'S1', originalText: '첫 발언', text: '첫 발언' },
    { id: 'b', start: 3, end: 4, speakerId: 'S2', originalText: '다음 발언', text: '다음 발언' },
    { id: 'c', start: 5, end: 6, speakerId: 'S1', originalText: '마지막 발언', text: '마지막 발언' },
  ],
});

function openController(doc = documentForTest()) {
  const elements = new Map();
  const document = { activeElement: null };
  function element(tagName, attributes = {}) {
    const node = { tagName: tagName.toUpperCase(), attributes, id: attributes.id || '', dataset: {}, listeners: new Map(), children: [], textContent: '', value: attributes.value || '', hidden: Object.hasOwn(attributes, 'hidden'), disabled: false, checked: Object.hasOwn(attributes, 'checked'), isConnected: true, open: false, scrollTop: 0,
      addEventListener(name, callback) { this.listeners.set(name, callback); },
      hasAttribute(name) { return Object.hasOwn(this.attributes, name); },
      setAttribute(name, value) { this.attributes[name] = value; },
      closest(selector) { return selector === 'button' && this.tagName === 'BUTTON' ? this : null; },
      focus() { if (this.isConnected && !this.disabled) document.activeElement = this; },
      blur() { document.activeElement = null; this.onBlur?.(); },
      setSelectionRange(start, end) { this.selectionStart = start; this.selectionEnd = end; },
      scrollIntoView() { this.revealed = true; },
      showModal() { this.open = true; }, close() { this.open = false; this.listeners.get('close')?.(); },
      querySelectorAll(selector) { return this.children.filter(child => selector.split(',').some(part => { const name = part.trim(); return name.startsWith('[') ? child.hasAttribute(name.slice(1, -1)) : child.tagName.toLowerCase() === name; })); },
      querySelector(selector) { return this.querySelectorAll(selector)[0] || null; },
    };
    for (const [key, value] of Object.entries(attributes)) if (key.startsWith('data-')) node.dataset[key.slice(5).replace(/-([a-z])/g, (_, character) => character.toUpperCase())] = value;
    if (node.id) elements.set(node.id, node);
    return node;
  }
  document.body = element('body');
  const trigger = element('button', { id: 'speakers-button' });
  const dialog = element('dialog', { id: 'speaker-dialog' });
  const body = element('div', { id: 'speaker-dialog-body' });
  element('div', { id: 'transcript-rows' });
  let html = '', writes = 0;
  Object.defineProperty(body, 'innerHTML', { get: () => html, set(value) {
    for (const node of body.children) {
      node.isConnected = false;
      if (document.activeElement === node) document.activeElement = document.body;
      if (node.id) elements.delete(node.id);
    }
    html = value; writes += 1;
    body.children = [...value.matchAll(/<(button|input|select|option|span|p|h2|h3|div|footer|header|section|nav)\b([^>]*)>/g)].map(match => {
      const attributes = Object.fromEntries([...match[2].matchAll(/([\w-]+)(?:="([^"]*)")?/g)].map(attribute => [attribute[1], decode(attribute[2] || '')]));
      return element(match[1], attributes);
    });
    for (const match of value.matchAll(/<select\b([^>]*)>([\s\S]*?)<\/select>/g)) {
      const id = /id="([^"]+)"/.exec(match[1])?.[1];
      const options = [...match[2].matchAll(/<option\b([^>]*)>/g)];
      const chosen = options.find(option => /\bselected\b/.test(option[1])) || options[0];
      if (id && chosen) elements.get(id).value = decode(/value="([^"]*)"/.exec(chosen[1])?.[1] || '');
    }
  } });
  document.activeElement = trigger;
  const state = { selectedId: 'meeting', connected: true, busy: false, editLocks: 0, snapshot: { meetings: [{ id: 'meeting' }] }, processing: { status: 'idle' }, transcriptDraft: { id: 'meeting', doc: structuredClone(doc) } };
  const calls = [], errors = [], updates = [], flushes = [], plays = [], analysisCalls = [], processing = [];
  let playing = false, playable = true, pauses = 0;
  let beforeFlush = null;
  const bridge = { diarizeMeeting: async (id, expected) => { analysisCalls.push({ id, expected: plain(expected) }); return { id: 'analysis', kind: 'diarize', status: 'running' }; }, prepareSpeakerModel: async () => { analysisCalls.push({ prepare: true }); return { id: 'model', kind: 'speaker-model', status: 'running' }; }, saveSpeakers(id, expected, speakers, assignments) {
    return new Promise((resolve, reject) => calls.push({ id, expected: plain(expected), speakers: plain(speakers), assignments: plain(assignments), reject, resolve() {
      const current = state.transcriptDraft.doc;
      resolve({ ...structuredClone(current), version: { ...current.version, revision: current.version.revision + 1 }, speakers: plain(speakers), segments: current.segments.map((segment, index) => ({ ...segment, speakerId: assignments[index] })) });
    } }));
  } };
  const context = vm.createContext({ escape, duration: value => `00:${String(Math.floor(value / 1000)).padStart(2, '0')}`, icon: () => '', TextEncoder, document, requestAnimationFrame: callback => callback(), crypto: { randomUUID: () => 'generated-id' }, NeroControls: { enhanceAll() {}, refresh() {}, close() {} } });
  const functions = vm.runInContext(`${processingStateSource.replace(/^export /gm, '')}\n${source.replace(/^import .*;\r?\n/gm, '').replace(/^export /gm, '')}\n({ mountSpeakers, speakerLabel, copyWorking, assignSelected, mergeSpeakers, nextSpeakerName, validateNames })`, context);
  const controller = functions.mountSpeakers({ bridge, getState: () => state, finishEditing: async action => {
    flushes.push(true); state.editLocks += 1;
    try { await beforeFlush?.(); return await action(); }
    finally { state.editLocks -= 1; }
  }, onProcessing: progress => processing.push(plain(progress)), onUpdated: (saved, meetingId) => { updates.push(meetingId); if (state.selectedId === meetingId) state.transcriptDraft.doc = saved; }, showError: error => errors.push(String(error.message || error)), byId: id => elements.get(id) || null, onPlay: (seconds, context) => { plays.push({ seconds, context: plain(context) }); playing = true; }, onPause: () => { pauses += 1; playing = false; }, isPlaying: () => playing, canPlay: () => playable });
  const click = async node => {
    const target = typeof node === 'string' ? elements.get(node) : node;
    assert.ok(target, `missing click target ${node}`);
    // Keyboard activation keeps a composing input focused until the controller
    // explicitly flushes it. Other tests focus buttons themselves before redraw.
    await body.listeners.get('click')({ target }); await tick();
  };
  const change = node => body.listeners.get('change')({ target: node });
  const nameInput = id => body.children.find(node => node.dataset.speakerName === id);
  const name = (id, value) => { const input = nameInput(id); input.value = value; body.listeners.get('input')({ target: input }); return input; };
  const input = (id, value) => { const node = elements.get(id); node.value = value; body.listeners.get('input')({ target: node }); return node; };
  const choose = value => { const node = elements.get('speaker-assignment-target'); node.value = value; change(node); };
  return { functions, controller, elements, body, dialog, trigger, state, calls, analysisCalls, processing, errors, updates, flushes, plays, click, change, choose, input, name, nameInput, writes: () => writes, beforeFlush: callback => { beforeFlush = callback; }, find: (attribute, value) => body.children.find(node => node.attributes[attribute] === value), compositionStart: () => body.listeners.get('compositionstart')(), compositionEnd: () => body.listeners.get('compositionend')(), document, pauses: () => pauses, endPlayback() { playing = false; controller.syncPlayback(); }, setPlayable(value) { playable = value; controller.syncPlayback(); } };
}

test('speaker operations preserve text and work without model speaker labels', () => {
  const app = openController();
  const { functions } = app;
  const doc = documentForTest();
  const working = functions.copyWorking(doc);
  const assigned = functions.assignSelected(working, new Set(['a', 'c']), '__new__', '진행자', () => 'one');
  assert.deepEqual(plain(assigned.assignments), ['manual-one', 'S2', 'manual-one']);
  assert.equal(assigned.speakers.at(-1).name, '진행자');
  assert.deepEqual(plain(assigned.segments), plain(working.segments));
  assert.deepEqual(plain(working.assignments), ['S1', 'S2', 'S1']);
  const merged = functions.mergeSpeakers(assigned, 'manual-one', 'S2');
  assert.deepEqual(plain(merged.assignments), ['S2', 'S2', 'S2']);
  assert.equal(merged.speakers.some(speaker => speaker.id === 'manual-one'), false);
  const unknown = functions.assignSelected(merged, new Set(['b']), '', '', () => 'unused');
  assert.equal(unknown.assignments[1], null);
  assert.equal(functions.speakerLabel({ speakers: [] }, { speakerId: null }), '화자 미지정');
  assert.throws(() => functions.assignSelected(working, new Set(['missing']), 'S1'));
  assert.throws(() => functions.assignSelected(working, new Set(), 'S1'));
  assert.throws(() => functions.mergeSpeakers(working, 'S1', 'S1'));
  assert.throws(() => functions.validateNames([{ name: ' ' }]));
  assert.throws(() => functions.validateNames([{ name: '가'.repeat(86) }]));
  assert.equal(functions.nextSpeakerName([{ name: '화자 1' }, { name: '화자 3' }]), '화자 2');
});

test('opening flushes pending text, names update without replacing focused inputs, and cancel writes nothing', async () => {
  const app = openController();
  app.beforeFlush(() => { app.state.transcriptDraft.doc.version.revision = 1; app.state.transcriptDraft.doc.segments[0].text = '저장된 수정'; });
  await app.controller.open(0);
  assert.equal(app.dialog.open, true);
  assert.equal(app.elements.get('speaker-assignment-target').value, 'S1');
  assert.equal(app.find('data-speaker-segment', 'a'), undefined);
  assert.match(app.body.innerHTML, /저장된 수정/);
  assert.equal(app.nameInput('S1'), undefined);
  assert.equal(app.elements.has('speaker-assign'), false);
  await app.click('speaker-cancel');
  await app.controller.open();
  const writes = app.writes();
  const input = app.name('S1', '새 이름');
  assert.equal(app.writes(), writes);
  assert.equal(app.nameInput('S1'), input);
  await app.click(app.find('data-speaker-close', ''));
  assert.equal(app.dialog.open, false);
  assert.equal(app.calls.length, 0);
  assert.equal(app.state.transcriptDraft.doc.speakers[0].name, '화자 1');
});

test('one utterance saves its new speaker directly and the last saved action can be undone once', async () => {
  const app = openController();
  await app.controller.open(0);
  assert.equal(app.elements.get('speaker-save').disabled, true);
  app.choose('__new__');
  app.input('speaker-new-name', '새 화자');
  assert.equal(app.calls.length, 0);
  await app.click('speaker-save');
  assert.equal(app.calls.length, 1);
  assert.deepEqual(app.calls[0].assignments, ['manual-generated-id', 'S2', 'S1']);
  assert.deepEqual(app.calls[0].expected, { id: 'document', revision: 0 });
  assert.equal(app.elements.get('speaker-save').disabled, true);
  const cancel = { preventDefault() {} };
  app.dialog.listeners.get('cancel')(cancel);
  assert.equal(app.dialog.open, true);
  app.calls[0].resolve(); await tick();
  assert.equal(app.dialog.open, false);
  assert.deepEqual(app.updates, ['meeting']);
  assert.equal(app.state.transcriptDraft.doc.segments[0].text, '첫 발언');
  await app.controller.open();
  assert.equal(app.elements.get('speaker-undo').hidden, false);
  await app.click('speaker-save');
  assert.equal(app.calls.length, 1);
  assert.equal(app.elements.get('speaker-undo').hidden, false);
  await app.click('speaker-undo');
  assert.equal(app.calls.length, 2);
  assert.deepEqual(app.calls[1].assignments, ['S1', 'S2', 'S1']);
  assert.deepEqual(app.calls[1].expected, { id: 'document', revision: 1 });
  app.calls[1].resolve(); await tick();
  await app.controller.open();
  assert.equal(app.elements.get('speaker-undo').hidden, true);
});

test('merge applies to every source utterance and native failure keeps the staged correction', async () => {
  const app = openController();
  await app.controller.open();
  await app.click(app.find('data-speaker-merge', 'S1'));
  app.elements.get('speaker-merge-target').value = 'S2';
  app.change(app.elements.get('speaker-merge-target'));
  await app.click('speaker-merge-apply');
  await app.click('speaker-save');
  assert.deepEqual(app.calls[0].speakers, [{ id: 'S2', name: '화자 2' }]);
  assert.deepEqual(app.calls[0].assignments, ['S2', 'S2', 'S2']);
  app.calls[0].reject(new Error('저장 실패')); await tick();
  assert.equal(app.dialog.open, true);
  assert.equal(app.elements.get('speaker-save').disabled, false);
  assert.match(app.elements.get('speaker-editor-message').textContent, /저장 실패/);
  await app.click('speaker-save');
  assert.deepEqual(app.calls[1].assignments, ['S2', 'S2', 'S2']);
  app.calls[1].resolve(); await tick();
});

test('IME names finish before save and stale versions cannot overwrite a newer transcript', async () => {
  const app = openController();
  await app.controller.open();
  const input = app.name('S1', 'ㅎ');
  app.document.activeElement = input;
  app.compositionStart();
  input.onBlur = () => { app.name('S1', '한글 이름'); app.compositionEnd(); };
  await app.click('speaker-save');
  assert.equal(app.calls[0].speakers[0].name, '한글 이름');
  app.calls[0].resolve(); await tick();
  await app.controller.open();
  app.name('S1', '추가 수정');
  app.state.transcriptDraft.doc.version.revision += 1;
  app.controller.render();
  assert.equal(app.elements.get('speaker-save').disabled, true);
  assert.equal(app.elements.get('speaker-undo').hidden, true);
  await app.click('speaker-save');
  assert.equal(app.calls.length, 1);
  assert.equal(app.nameInput('S1').value, '추가 수정');
  assert.match(app.elements.get('speaker-editor-message').textContent, /변경/);
});

test('dialog escapes arbitrary transcript/name content and resets safely on navigation', async () => {
  const doc = documentForTest();
  doc.speakers[0].name = '"><script>alert(1)</script>';
  doc.segments[0].text = '<img src=x onerror=alert(1)> & 문장';
  const app = openController(doc);
  await app.controller.open(0);
  assert.ok(app.body.innerHTML.includes('&lt;script&gt;'));
  assert.ok(app.body.innerHTML.includes('&lt;img'));
  assert.equal(app.body.innerHTML.includes('<script>'), false);
  app.state.selectedId = 'another';
  app.controller.reset();
  assert.equal(app.dialog.open, false);
  app.controller.render();
  assert.equal(app.trigger.hidden, true);
  assert.equal(app.calls.length, 0);
  app.state.selectedId = 'meeting';
  app.state.snapshot.meetings[0].deletedAt = 1;
  app.controller.render();
  assert.equal(app.trigger.hidden, true);
});

test('utterance 201 opens on its sentence and replays its exact recording range', async () => {
  const doc = documentForTest();
  doc.segments = Array.from({ length: 205 }, (_, index) => ({ id: `segment-${index}`, start: index * 2, end: index * 2 + 1.5, text: `고유한 ${index + 1}번 내용`, originalText: `원문 ${index + 1}`, speakerId: index === 200 ? 'S1' : 'S2' }));
  const app = openController(doc);
  await app.controller.open(200);
  assert.match(app.body.innerHTML, /201번 발언/);
  assert.match(app.body.innerHTML, /고유한 201번 내용/);
  assert.equal(app.body.innerHTML.includes('고유한 1번 내용'), false);
  assert.equal(app.elements.has('speaker-view-names'), false);
  assert.equal(app.elements.has('speaker-segment-segment-0'), false);
  assert.equal(app.elements.get('speaker-editor-scroll').scrollTop, 0);
  app.setPlayable(false);
  assert.equal(app.elements.get('speaker-listen').disabled, true);
  await app.click('speaker-listen');
  assert.equal(app.plays.length, 0);
  app.setPlayable(true);
  await app.click('speaker-listen');
  assert.deepEqual(app.plays, [{ seconds: 400, context: { meetingId: 'meeting', segmentId: 'segment-200', index: 200, endSeconds: 401.5 } }]);
  assert.equal(app.elements.get('speaker-listen').attributes['aria-pressed'], 'true');
  await app.click('speaker-listen');
  assert.equal(app.pauses(), 1);
  assert.equal(app.elements.get('speaker-listen').attributes['aria-pressed'], 'false');
  await app.click('speaker-listen');
  app.endPlayback();
  assert.equal(app.elements.get('speaker-listen').attributes['aria-pressed'], 'false');
  app.choose('S2');
  await app.click('speaker-save');
  assert.equal(app.calls.length, 1);
  assert.equal(app.calls[0].assignments[200], 'S2');
  assert.equal(app.calls[0].assignments.filter(id => id !== 'S2').length, 0);
  app.calls[0].resolve(); await tick();
  assert.equal(app.state.transcriptDraft.doc.segments[200].originalText, '원문 201');
  assert.equal(app.state.transcriptDraft.doc.segments[200].text, '고유한 201번 내용');
});

test('batch apply retains selected utterances, connected focus, target and scroll across redraw', async () => {
  const app = openController();
  await app.controller.open();
  await app.click(app.find('data-speaker-pick', 'S1'));
  assert.equal(app.body.dataset.speakerMode, 'utterances');
  assert.equal(app.find('data-speaker-segment', 'a').checked, true);
  assert.equal(app.find('data-speaker-segment', 'b').checked, false);
  assert.equal(app.find('data-speaker-segment', 'c').checked, true);
  app.elements.get('speaker-editor-scroll').scrollTop = 260;
  app.choose('__new__');
  app.input('speaker-new-name', '분리한 화자');
  const oldApply = app.elements.get('speaker-assign');
  oldApply.focus();
  await app.click(oldApply);
  assert.equal(oldApply.isConnected, false);
  assert.equal(app.document.activeElement, app.elements.get('speaker-save'));
  assert.equal(app.document.activeElement.isConnected, true);
  assert.equal(app.elements.get('speaker-editor-scroll').scrollTop, 260);
  assert.equal(app.elements.get('speaker-assignment-target').value, 'manual-generated-id');
  assert.equal(app.find('data-speaker-segment', 'a').checked, true);
  assert.equal(app.find('data-speaker-segment', 'c').checked, true);
  assert.equal(app.calls.length, 0);
  await app.click('speaker-save');
  assert.deepEqual(app.calls[0].assignments, ['manual-generated-id', 'S2', 'manual-generated-id']);
  app.calls[0].resolve(); await tick();
});

test('merge cancellation and application restore a surviving control without losing selection or scroll', async () => {
  const app = openController();
  await app.controller.open();
  await app.click(app.find('data-speaker-pick', 'S1'));
  app.elements.get('speaker-editor-scroll').scrollTop = 170;
  await app.click('speaker-view-names');
  app.elements.get('speaker-editor-scroll').scrollTop = 240;
  await app.click(app.find('data-speaker-merge', 'S1'));
  assert.equal(app.document.activeElement, app.elements.get('speaker-merge-target'));
  app.elements.get('speaker-editor-scroll').scrollTop = 330;
  const oldCancel = app.elements.get('speaker-merge-cancel');
  oldCancel.focus();
  await app.click(oldCancel);
  assert.equal(oldCancel.isConnected, false);
  assert.equal(app.document.activeElement, app.find('data-speaker-merge', 'S1'));
  assert.equal(app.document.activeElement.isConnected, true);
  assert.equal(app.elements.get('speaker-editor-scroll').scrollTop, 240);
  assert.equal(app.elements.get('speaker-save').disabled, true);
  await app.click(app.find('data-speaker-merge', 'S1'));
  app.elements.get('speaker-merge-target').value = 'S2';
  app.change(app.elements.get('speaker-merge-target'));
  const oldMerge = app.elements.get('speaker-merge-apply');
  oldMerge.focus();
  await app.click(oldMerge);
  assert.equal(oldMerge.isConnected, false);
  assert.equal(app.document.activeElement, app.elements.get('speaker-save'));
  assert.equal(app.document.activeElement.isConnected, true);
  assert.equal(app.elements.get('speaker-editor-scroll').scrollTop, 240);
  await app.click('speaker-view-utterances');
  assert.equal(app.elements.get('speaker-editor-scroll').scrollTop, 170);
  assert.equal(app.find('data-speaker-segment', 'a').checked, true);
  assert.equal(app.find('data-speaker-segment', 'b').checked, false);
  assert.equal(app.find('data-speaker-segment', 'c').checked, true);
  await app.click('speaker-save');
  assert.deepEqual(app.calls[0].assignments, ['S2', 'S2', 'S2']);
  app.calls[0].resolve(); await tick();
});

test('switching management views commits Korean composition before replacing inputs', async () => {
  const app = openController();
  await app.controller.open();
  app.elements.get('speaker-editor-scroll').scrollTop = 90;
  const input = app.name('S1', 'ㅎ');
  input.focus();
  app.compositionStart();
  input.onBlur = () => { app.name('S1', '한국어 이름'); app.compositionEnd(); };
  await app.click('speaker-view-utterances');
  assert.equal(app.document.activeElement, app.elements.get('speaker-view-utterances'));
  assert.equal(app.nameInput('S1'), undefined);
  await app.click('speaker-view-names');
  assert.equal(app.nameInput('S1').value, '한국어 이름');
  assert.equal(app.elements.get('speaker-editor-scroll').scrollTop, 90);
  await app.click('speaker-save');
  assert.equal(app.calls[0].speakers[0].name, '한국어 이름');
  app.calls[0].resolve(); await tick();
});

test('single new-speaker composition is saved once and cancellation restores the originating row', async () => {
  const app = openController();
  const row = { isConnected: true, dataset: { speakerEdit: '0' }, focus() { if (!app.state.editLocks) app.document.activeElement = this; } };
  app.document.activeElement = row;
  await app.controller.open(0);
  app.choose('__new__');
  const input = app.input('speaker-new-name', 'ㅎ');
  input.focus();
  app.compositionStart();
  input.onBlur = () => { app.input('speaker-new-name', '한글 화자'); app.compositionEnd(); };
  await app.click('speaker-save');
  assert.equal(app.calls[0].speakers.at(-1).name, '한글 화자');
  app.calls[0].reject(new Error('연결 끊김')); await tick();
  assert.equal(app.dialog.open, true);
  assert.equal(app.elements.get('speaker-new-name').value, '한글 화자');
  await app.click('speaker-save');
  assert.deepEqual(app.calls[1].speakers, app.calls[0].speakers);
  app.calls[1].resolve(); await tick();
  assert.equal(app.document.activeElement, row);
  await app.controller.open(0);
  await app.click('speaker-listen');
  await app.click('speaker-cancel');
  assert.equal(app.dialog.open, false);
  assert.equal(app.pauses() > 0, true);
  assert.equal(app.document.activeElement, row);
});


test('whole-recording analysis requires confirmation and submits the pinned transcript version', async () => {
  const app = openController();
  app.state.processing.speakerSupported = true;
  app.state.processing.speakerReady = true;
  await app.controller.open();
  await app.click('speaker-analyze');
  assert.match(app.body.innerHTML, /현재 화자 이름과 발언 배정이 새 결과로 바뀝니다/);
  assert.equal(app.analysisCalls.length, 0);
  await app.click('speaker-analysis-confirm');
  assert.deepEqual(app.analysisCalls, [{ id: 'meeting', expected: { id: 'document', revision: 0 } }]);
  assert.equal(app.processing[0].kind, 'diarize');
  assert.equal(app.dialog.open, false);
});

test('missing speaker model needs explicit preparation without automatically analyzing', async () => {
  const app = openController();
  app.state.processing.speakerSupported = true;
  app.state.processing.speakerReady = false;
  await app.controller.open();
  await app.click('speaker-analyze');
  assert.match(app.body.innerHTML, /모델을 다운로드/);
  assert.equal(app.analysisCalls.length, 0);
  await app.click('speaker-analysis-confirm');
  assert.deepEqual(app.analysisCalls, [{ prepare: true }]);
  assert.equal(app.processing[0].kind, 'speaker-model');
});

test('analysis cannot replace unsaved names or a changed transcript', async () => {
  const app = openController();
  app.state.processing.speakerSupported = true;
  app.state.processing.speakerReady = true;
  await app.controller.open();
  app.name('S1', '수정한 이름');
  assert.equal(app.elements.get('speaker-analyze').disabled, true);
  app.name('S1', '화자 1');
  await app.click('speaker-analyze');
  app.beforeFlush(() => { app.state.transcriptDraft.doc.version.revision += 1; });
  await app.click('speaker-analysis-confirm');
  assert.equal(app.analysisCalls.length, 0);
  assert.match(app.elements.get('speaker-editor-message').textContent, /변경되었습니다/);
});

test('recording scope offers another analysis without claiming an attendee count', async () => {
  const doc = documentForTest(); doc.speakerAnalysis = { scope: 'recording', assignmentsEdited: true };
  const app = openController(doc);
  app.state.processing.speakerSupported = true;
  app.state.processing.speakerReady = true;
  await app.controller.open();
  assert.match(app.body.innerHTML, /id="speaker-analyze"[^>]*>다시 구분/);
  assert.doesNotMatch(app.body.innerHTML, /2명/);
  app.state.processing = { ...app.state.processing, status: 'running', meetingId: 'other' };
  app.controller.render();
  assert.equal(app.elements.get('speaker-analyze').disabled, false);
});
