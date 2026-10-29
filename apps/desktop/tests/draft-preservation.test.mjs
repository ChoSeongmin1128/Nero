import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';
import { Window } from 'happy-dom';

const source = await readFile(new URL('../src/main.js', import.meta.url), 'utf8');
const qualitySource = await readFile(new URL('../src/recording-quality.js', import.meta.url), 'utf8');
const uiSource = await readFile(new URL('../src/ui.js', import.meta.url), 'utf8');
const librarySource = await readFile(new URL('../src/library-ui.js', import.meta.url), 'utf8');
const searchSource = await readFile(new URL('../src/meeting-search.js', import.meta.url), 'utf8');
const detailSource = await readFile(new URL('../src/meeting-detail-ui.js', import.meta.url), 'utf8');
const actionsSource = await readFile(new URL('../src/transcript-actions.js', import.meta.url), 'utf8');
const feedbackSource = await readFile(new URL('../src/processing-feedback.js', import.meta.url), 'utf8');
const processingStateSource = await readFile(new URL('../src/processing-state.js', import.meta.url), 'utf8');
const componentsSource = await readFile(new URL('../../../design-system/components.js', import.meta.url), 'utf8');
const markup = new Window();
markup.document.body.innerHTML = await readFile(new URL('../src/index.html', import.meta.url), 'utf8');
const settingsTabAttributes = [...markup.document.querySelectorAll('#settings-dialog .app-settings-tabs [role="tab"]')].map(tab => Object.fromEntries([...tab.attributes].map(attribute => [attribute.name, attribute.value])));
await markup.happyDOM.close();
const tick = () => new Promise(setImmediate);

function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

async function openApp(initialTranscript = null, readiness = { status: 'idle', supported: true, modelReady: true, summarySupported: true, summaryReady: true }, storage = new Map()) {
  const elements = new Map();
  const element = (id) => {
    if (!elements.has(id)) elements.set(id, {
      append(child) { child.parentNode = this; }, insertBefore(child) { child.parentNode = this; },
      id, value: '', dataset: {}, readOnly: false, disabled: false, hidden: false,
      innerHTML: '', textContent: '', scrollTop: 0, listeners: new Map(),
      addEventListener(name, callback) { this.listeners.set(name, callback); },
      querySelectorAll(selector) {
        if (this.id === 'settings-dialog' && selector === '.app-settings-tabs [role="tab"][data-settings-panel]') return settingsTabs.filter(tab => tab.dataset.settingsPanel);
        if (!['transcript-rows', 'processing-jobs'].includes(this.id)) return [];
        if (selector === 'textarea') return this.children.filter(child => child.tagName === 'TEXTAREA');
        const names = [...selector.matchAll(/\[([\w-]+)\]/g)].map(match => match[1]);
        return this.children.filter(child => names.some(name => child.attributes.has(name)));
      },
      querySelector(selector) {
        if (this.id !== 'transcript-rows') return null;
        const match = selector.match(/^\[([\w-]+)="([^"]+)"\]$/);
        return match ? this.children.find(child => child.attributes.get(match[1]) === match[2]) || null : null;
      },
      children: [], attributes: new Map(), isConnected: true,
      get ownerDocument() { return document; },
      classList: { add() {}, remove() {}, toggle() {} }, contains() { return false; },
      focus() { document.activeElement = this; }, blur() { document.activeElement = document.body; },
      select() {}, pause() {}, play() { return Promise.resolve(); }, load() {}, setAttribute(name, value) { this[name] = value; }, removeAttribute(name) { delete this[name]; }, getAttribute(name) { return this[name] ?? null; }, showModal() { this.open = true; }, close() { this.open = false; this.listeners.get('close')?.(); },
    });
    const node = elements.get(id);
    if (!Object.hasOwn(node, 'htmlWrites')) {
      let html = node.innerHTML; node.htmlWrites = 0;
      Object.defineProperty(node, 'innerHTML', { get: () => html, set(value) {
        html = value; node.htmlWrites++;
        if (!['transcript-rows', 'processing-jobs'].includes(node.id)) return;
        if (node.id === 'processing-jobs') for (const child of node.children) { child.isConnected = false; if (document.activeElement === child) document.activeElement = document.body; }
        node.children = [...value.matchAll(/<(textarea|button)\b([^>]*)>([\s\S]*?)<\/\1>/g)].map((match, index) => {
          const child = element(node.id === 'transcript-rows' ? `transcript-child-${index}` : `processing-child-${node.htmlWrites}-${index}`);
          child.tagName = match[1].toUpperCase();
          child.attributes = new Map([...match[2].matchAll(/([\w-]+)="([^"]*)"/g)].map(attribute => [attribute[1], attribute[2]]));
          child.dataset = Object.fromEntries([...child.attributes].filter(([name]) => name.startsWith('data-')).map(([name, content]) => [name.slice(5).replace(/-([a-z])/g, (_, letter) => letter.toUpperCase()), content]));
          child.readOnly = /\breadonly\b/.test(match[2]);
          child.disabled = /\bdisabled\b/.test(match[2]);
          child.closest = selector => selector === '[data-job-id]' && child.dataset.jobId ? child : null;
          child.value = match[3].replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"').replace(/&#39;/g, "'").replace(/&amp;/g, '&');
          return child;
        });
      } });
    }
    return node;
  };
  const document = {
    body: element('body'), activeElement: null, getElementById: element,
    querySelector: element, querySelectorAll: () => [], addEventListener() {},
  };
  document.activeElement = document.body;
  const settingsTabs = settingsTabAttributes.map(attributes => {
    const tab = element(attributes.id);
    for (const [name, value] of Object.entries(attributes)) tab.setAttribute(name, value);
    tab.dataset = { settingsPanel: attributes['data-settings-panel'] };
    return tab;
  });
  let snapshot = { revision: 1, session: null, meetings: [
    { id: 'a', title: '회의 A', notes: '', createdAt: 1, status: 'draft' },
    { id: 'b', title: '회의 B', notes: '', createdAt: 2, status: 'draft' },
  ] };
  const updates = [];
  const exits = [];
  let transcript = initialTranscript ? structuredClone(initialTranscript) : null;
  const transcriptUpdates = [];
  let devices = [];
  let closeRequest;
  const bridge = {
    setAutoRecorder: async () => {}, subscribeProcessingError: async () => () => {},
    readSummary: async () => null,
    exportDocument: async (id, kind, format, includeCitations) => ({ name: `회의-음성 기록.${format}`, text: id === 'a' && kind === 'transcript' ? transcript.segments.map(segment => segment.text).join('\n') : 'native summary result', kind, format, includeCitations }),
    subscribeProcessing: async () => () => {}, processingStatus: async () => structuredClone(readiness), readTranscript: async (id) => id === 'a' ? structuredClone(transcript) : null,
    saveTranscript(id, expected, texts) {
      const pending = deferred();
      transcriptUpdates.push({ id, expected, texts, pending });
      return pending.promise.then(() => {
        transcript = { ...transcript, version: { ...transcript.version, revision: transcript.version.revision + 1 }, segments: transcript.segments.map((segment,index) => ({ ...segment, text: texts[index] })) };
        return structuredClone(transcript);
      });
    },
    subscribeSnapshots: async () => () => {}, subscribeOpenMeeting: async () => () => {},
    subscribeQuitRequest: async () => () => {},
    subscribeCloseRequest: async (callback) => { closeRequest = callback; return () => {}; },
    readSnapshot: async () => structuredClone(snapshot), listDevices: async () => structuredClone(devices),
    async setMeetingDate(id, meetingDate) { snapshot = { ...snapshot, revision: snapshot.revision + 1, meetings: snapshot.meetings.map(meeting => meeting.id === id ? { ...meeting, meetingDate } : meeting) }; },
    updateMeeting(id, title, notes) {
      const pending = deferred();
      updates.push({ id, title, notes, pending });
      return pending.promise.then(() => {
        snapshot = { ...snapshot, revision: snapshot.revision + 1, meetings: snapshot.meetings.map((meeting) => meeting.id === id ? { ...meeting, title, notes } : meeting) };
      });
    },
    requestQuit() { const pending = deferred(); exits.push({ type: 'quit', pending }); return pending.promise; },
    hideMain() { const pending = deferred(); exits.push({ type: 'close', pending }); return pending.promise; },
  };
  const copied = [], agentDialogCalls = [];
  const templateSettings = { setActive() {}, beforeLeave: async () => true, isConfirming: () => false, hide() {} };
  const context = vm.createContext({
    navigator: { clipboard: { writeText: async text => { copied.push(text); } } },
    document, testBridge: bridge, testAgentDialogCalls: agentDialogCalls, testTemplateSettings: templateSettings, localStorage: { getItem: key => storage.get(key) ?? null, setItem: (key, value) => storage.set(key, value) },
    GroveControls: { enhanceAll() {} },
    setTimeout: () => 1, clearTimeout() {}, requestAnimationFrame: (callback) => callback(),
  });
  vm.runInContext(componentsSource, context);
  context.testUI = vm.runInContext(`(() => {
    ${uiSource.replace(/^export /gm, '')}
    return { icon, escape, button, duration, recordingStatus, statusMarkup, sessionControls, applySystemAppearance() {} };
  })()`, context);
  const executable = source.replace(/^import .*;\n/gm, '');
  const app = await vm.runInContext(`(async () => {
    ${qualitySource.replace(/^export /gm, '')}
    ${detailSource.replace(/^export /gm, '')}
    ${feedbackSource.replace(/^export /gm, '')}
    ${processingStateSource.replace(/^export /gm, '')}
    ${searchSource.replace(/^export /gm, '')}
    ${actionsSource.replace(/^import .*;\n/gm, '').replace(/^export /gm, '')}
    let restoreForTest;
    const createTranscriptActions = mountTranscriptActions;
    mountTranscriptActions = options => { restoreForTest = options.onRestored; return createTranscriptActions(options); };
    ${librarySource.slice(0, librarySource.indexOf('export function mountLibrary')).replace(/^import .*;\n/gm, '').replace(/^export /gm, '')}
    let summaryEvidenceOptions;
    const mountSummaryEvidence = options => { summaryEvidenceOptions = options; return { reset() {}, update() {}, restoreFocus() {} }; };
    const mountTemplateSettings = () => testTemplateSettings;
    const mountCalendarSettings = () => ({ setActive() {} });
    const mountCalendarMeetingPicker = () => ({open(){}, close(){}, isOpen(){return false;}});
    const mountCalendarMeetingReference = () => ({render(){}});
    const mountSummaryTemplatePicker = () => ({reload:async()=>{},selected:async()=>({id:'meeting',revision:'test'})});
    const mountDocumentActions = () => ({ update() {}, close() {} });
    const mountAgentConnectionDialog = () => ({ open(...args) { testAgentDialogCalls.push(args); }, update() {}, close() {}, isOpen: () => false });
    const mountLibrary = ({getState}) => ({ render() {}, resetSelection() {}, toggleSelection() {}, handleRowSelection: () => false, show: async () => {}, visibleMeetings() {
      const current = getState(), query = document.getElementById('search').value.trim();
      const ids = current.librarySearchIds == null ? null : new Set(current.librarySearchIds);
      return filterLibraryMeetings(current.snapshot.meetings, current.libraryFilter, query && !ids ? query : '').filter(meeting => !query || !ids || ids.has(meeting.id)).sort((a,b) => (b.recordedAt ?? b.createdAt ?? 0) - (a.recordedAt ?? a.createdAt ?? 0) || (b.createdAt ?? 0) - (a.createdAt ?? 0));
    } });
    const mountSpeakers = ({getState}) => ({render() {}, reset() {}, open: async () => {}, label: segment => getState().transcriptDraft?.doc?.speakers?.find(speaker=>speaker.id === segment.speakerId)?.name || '화자 미지정'});

    const returnFocusTarget = (origin, fallback) => origin || fallback;
    const bridge = testBridge;
    const { icon, escape, button, duration, recordingStatus, statusMarkup, sessionControls, applySystemAppearance } = testUI;
    ${executable}
    return { state, detail, documentSearch, summaryEvidenceOptions, restoreTranscript: restoreForTest, selectMeeting, saveDraft, finishEditing, onEdit, onQuitRequest, onAgentWriteRequest, refreshDevices, acceptSnapshot, saveTranscript, loadTranscript, acceptProcessing, loadSummary, renderSummary, createMeeting, openSetup, openSettings, changeMicrophone, changeMeetingDate, render, prepareAudio, exportDocument, editTranscriptRow, completeTranscriptRow, cancelTranscriptRow, cancelProcessing, retryProcessing, acknowledgeProcessing };
  })()`, context);
  await tick();
  return {
    app, element, updates, exits, bridge, copied, agentDialogCalls, templateSettings, settingsTabs, transcriptUpdates, savedTranscript: () => transcript,
    focused: () => document.activeElement?.id,
    editTranscript(text) {
      const target = element('transcript-rows').querySelector('[data-segment="0"]') || { dataset: { segment: '0' } };
      target.value = text;
      element('transcript-rows').listeners.get('input')({ target });
    },
    close: () => closeRequest(),
    setDevices(value) { devices = value; return app.refreshDevices(); },
    saved: () => snapshot.meetings.find((meeting) => meeting.id === 'a'),
    edit(notes) { element('meeting-notes').value = notes; app.onEdit(); },
  };
}

test('manual connection click passes its trigger even when clicking does not move focus', async () => {
  const page = await openApp();
  await page.app.openSettings('agent');
  page.element('settings-close').focus();
  const trigger = page.element('settings-agent-manual');
  trigger.listeners.get('click')({ currentTarget: trigger, target: trigger });
  assert.equal(page.focused(), 'settings-close');
  assert.equal(page.agentDialogCalls.length, 1);
  assert.equal(page.agentDialogCalls[0][1], trigger);
  assert.notEqual(page.agentDialogCalls[0][1], page.element('settings-close'));
});

test('meeting transition preserves edits arriving while earlier saves are pending', async () => {
  const page = await openApp();
  page.edit('처음 입력');
  const autosave = page.app.saveDraft();
  await tick();
  page.edit('저장 중 추가 입력');
  const transition = page.app.selectMeeting('b');
  assert.equal(page.element('meeting-notes').readOnly, true);
  page.updates[0].pending.resolve();
  await tick();
  page.edit('마지막 입력까지 보존');
  page.updates[1].pending.resolve();
  await tick();
  assert.equal(page.app.state.selectedId, 'a');
  assert.equal(page.updates[2].notes, '마지막 입력까지 보존');
  page.updates[2].pending.resolve();
  await Promise.all([autosave, transition]);
  assert.equal(page.saved().notes, '마지막 입력까지 보존');
  assert.equal(page.app.state.selectedId, 'b');
  assert.equal(page.element('meeting-notes').readOnly, false);
});

for (const kind of ['quit', 'close']) {
  test(`${kind} waits for the latest edit and keeps the editor locked until the native action finishes`, async () => {
    const page = await openApp();
    page.edit('처음 입력');
    const leaving = kind === 'quit' ? page.app.onQuitRequest({}) : page.close();
    await tick();
    assert.equal(page.element('meeting-title').readOnly, true);
    page.edit('종료 전 추가 입력');
    page.updates[0].pending.resolve();
    await tick();
    assert.equal(page.exits.length, 0);
    assert.equal(page.updates[1].notes, '종료 전 추가 입력');
    page.updates[1].pending.resolve();
    await tick();
    assert.equal(page.saved().notes, '종료 전 추가 입력');
    assert.equal(page.exits[0].type, kind);
    assert.equal(page.element('meeting-notes').readOnly, true);
    page.exits[0].pending.resolve();
    await leaving;
    assert.equal(page.element('meeting-notes').readOnly, false);
  });
}

test('failed save keeps the current draft and unlocks the editor without switching meetings', async () => {
  const page = await openApp();
  page.edit('저장되지 않은 입력');
  const transition = page.app.selectMeeting('b');
  await tick();
  page.updates[0].pending.reject(new Error('저장 실패'));
  await transition;
  assert.equal(page.app.state.selectedId, 'a');
  assert.equal(page.app.state.draft.notes, '저장되지 않은 입력');
  assert.equal(page.app.state.draft.dirty, true);
  assert.equal(page.element('meeting-notes').readOnly, false);
  assert.equal(page.element('error').hidden, false);
});

test('recording stays compact and microphone failures lead to settings', async () => {
  const page = await openApp();
  const controls = page.element('recording-controls').innerHTML;
  assert.match(controls, /<button[^>]*data-action="start"[^>]* disabled/);
  assert.doesNotMatch(controls, /<select|refresh-devices/);
  assert.match(controls, /data-action="recording-settings"/);
  assert.match(page.element('recording-message').textContent, /마이크가 없습니다/);
  await page.app.openSettings();
  assert.equal(page.element('settings-dialog').open, true);
  assert.equal(page.element('settings-refresh-devices').disabled, false);
});

test('missing preferred microphone never silently switches to another input', async () => {
  const page = await openApp();
  await page.setDevices([{ id: 'usb-input', name: 'USB 마이크', isDefault: true }]);
  let controls = page.element('recording-controls').innerHTML;
  assert.doesNotMatch(controls.match(/<button[^>]*data-action="start"[^>]*>/)[0], / disabled/);
  assert.doesNotMatch(controls, /<select|USB 마이크/);
  page.app.state.deviceId = 'usb-input'; page.app.state.deviceName = 'USB 마이크';
  await page.setDevices([{ id: 'built-in', name: '내장 마이크', isDefault: true }]);
  assert.equal(page.app.state.deviceId, 'usb-input');
  assert.match(page.element('recording-message').textContent, /선택한 마이크/);
  assert.match(page.element('recording-controls').innerHTML, /data-action="start"[^>]* disabled/);
  assert.match(page.element('settings-microphone').innerHTML, /USB 마이크.*연결 안 됨/);
  await page.setDevices([{ id: 'usb-input', name: 'USB 마이크', isDefault: true }]);
  assert.doesNotMatch(page.element('recording-controls').innerHTML.match(/<button[^>]*data-action="start"[^>]*>/)[0], / disabled/);
});

test('empty device inventory does not disable controls for an active recording', async () => {
  const page = await openApp();
  page.app.acceptSnapshot({
    revision: 2,
    meetings: [{ id: 'a', title: '회의 A', notes: '', createdAt: 1, status: 'recording' }],
    session: { id: 'session-a', meetingId: 'a', state: 'recording', elapsedMs: 1000 },
  });
  await page.setDevices([]);
  const controls = page.element('recording-controls').innerHTML;
  for (const action of ['pause', 'stop']) {
    assert.doesNotMatch(controls.match(new RegExp(`<button[^>]*data-action="${action}"[^>]*>`))[0], / disabled/);
  }
  assert.doesNotMatch(page.element('recording-message').textContent, /마이크가 없습니다/);
});

const transcriptDocument = () => ({
  version: { id: 'transcript-a', revision: 0 }, sourceSessionId: 'audio-a', model: 'MOSS',
  segments: [{ id: 'segment-a', start: 0, end: 1, originalText: '모델 원문', text: '모델 원문' }],
});

test('meeting transition saves transcript edits arriving during an earlier save', async () => {
  const page = await openApp(transcriptDocument());
  page.editTranscript('첫 수정');
  const saving = page.app.saveTranscript();
  await tick();
  page.editTranscript('추가 수정');
  const leaving = page.app.selectMeeting('b');
  page.transcriptUpdates[0].pending.resolve();
  await tick();
  assert.equal(page.app.state.selectedId, 'a');
  assert.deepEqual(page.transcriptUpdates[1].texts, ['추가 수정']);
  assert.equal(page.transcriptUpdates[1].expected.revision, 1);
  page.transcriptUpdates[1].pending.resolve();
  await Promise.all([saving, leaving]);
  assert.equal(page.app.state.selectedId, 'b');
  assert.equal(page.savedTranscript().segments[0].text, '추가 수정');
  assert.equal(page.savedTranscript().segments[0].originalText, '모델 원문');
});

test('failed transcript save preserves input and prevents leaving the meeting', async () => {
  const page = await openApp(transcriptDocument());
  page.editTranscript('미저장 수정');
  const leaving = page.app.selectMeeting('b');
  await tick();
  page.transcriptUpdates[0].pending.reject(new Error('전사 저장 실패'));
  await leaving;
  assert.equal(page.app.state.selectedId, 'a');
  assert.equal(page.app.state.transcriptDraft.dirty, true);
  assert.equal(page.app.state.transcriptDraft.doc.segments[0].text, '미저장 수정');
  assert.equal(page.element('error').hidden, false);
});

test('late transcript loading cannot replace the newly selected meeting', async () => {
  const page = await openApp();
  const pending = deferred();
  page.bridge.readTranscript = async (id) => id === 'a' ? pending.promise : null;
  const loading = page.app.loadTranscript('a');
  await page.app.selectMeeting('b');
  pending.resolve(transcriptDocument());
  await loading;
  assert.equal(page.app.state.selectedId, 'b');
  assert.equal(page.app.state.transcriptDraft.id, 'b');
  assert.equal(page.app.state.transcriptDraft.doc, null);
});

test('an old transcript read cannot replace a correction saved on the same draft while the read waited', async () => {
  const original = transcriptDocument();
  const page = await openApp(original);
  const pending = deferred();
  page.bridge.readTranscript = async () => pending.promise;
  const draft = page.app.state.transcriptDraft;
  const loading = page.app.loadTranscript('a');
  page.editTranscript('읽기 대기 중 확정한 수정');
  const saving = page.app.saveTranscript();
  await tick();
  page.transcriptUpdates[0].pending.resolve();
  await saving;
  assert.equal(page.app.state.transcriptDraft, draft);
  assert.equal(draft.dirty, false);
  assert.equal(draft.doc.version.revision, 1);
  pending.resolve(original);
  await loading;
  assert.equal(page.app.state.transcriptDraft, draft);
  assert.equal(draft.doc.version.revision, 1);
  assert.equal(draft.doc.segments[0].text, '읽기 대기 중 확정한 수정');
});

test('an old transcript read cannot replace a draft adopted through the restore callback', async () => {
  const original = transcriptDocument();
  const page = await openApp(original);
  const pending = deferred();
  page.bridge.readTranscript = async () => pending.promise;
  const before = page.app.state.transcriptDraft;
  const loading = page.app.loadTranscript('a');
  const restored = structuredClone(original);
  restored.version = { id: 'restored-transcript', revision: 3 };
  restored.segments[0].text = '이전 기록에서 되돌린 문장';
  await page.app.restoreTranscript(restored, 'a');
  const adopted = page.app.state.transcriptDraft;
  assert.notEqual(adopted, before);
  assert.equal(adopted.doc, restored);
  assert.equal(adopted.dirty, false);
  pending.resolve(original);
  await loading;
  assert.equal(page.app.state.transcriptDraft, adopted);
  assert.equal(page.app.state.transcriptDraft.doc, restored);
  assert.match(page.element('transcript-rows').innerHTML, /이전 기록에서 되돌린 문장/);
});

test('a transcript read still adopts its result when the draft owner and version are unchanged', async () => {
  const original = transcriptDocument();
  const page = await openApp(original);
  const pending = deferred();
  page.bridge.readTranscript = async () => pending.promise;
  const before = page.app.state.transcriptDraft;
  const loading = page.app.loadTranscript('a');
  const result = structuredClone(original);
  result.version.revision = 2;
  result.segments[0].text = '정상적으로 다시 읽은 문장';
  pending.resolve(result);
  await loading;
  assert.notEqual(page.app.state.transcriptDraft, before);
  assert.equal(page.app.state.transcriptDraft.doc, result);
  assert.equal(page.app.state.transcriptLoaded, true);
  assert.equal(page.app.state.transcriptLoadError, false);
  assert.match(page.element('transcript-rows').innerHTML, /정상적으로 다시 읽은 문장/);
});

test('quit commits an active transcript composition before requesting native exit', async () => {
  const page = await openApp(transcriptDocument());
  const input = page.element('segment-input');
  input.dataset.segment = '0';
  input.value = '조합 중 마지막 입력';
  input.focus();
  page.element('transcript-rows').listeners.get('compositionstart')();
  const leaving = page.app.onQuitRequest({});
  await tick();
  assert.equal(page.exits.length, 0);
  assert.deepEqual(page.transcriptUpdates[0].texts, ['조합 중 마지막 입력']);
  page.transcriptUpdates[0].pending.resolve();
  await tick();
  assert.equal(page.savedTranscript().segments[0].text, '조합 중 마지막 입력');
  assert.equal(page.exits[0].type, 'quit');
  page.exits[0].pending.resolve();
  await leaving;
});

test('late processing replies cannot replace the current running job', async () => {
  const page = await openApp();
  await page.app.acceptProcessing({ revision: 5, id: 'new-job', kind: 'model', status: 'running', supported: true });
  await page.app.acceptProcessing({ revision: 4, id: 'old-job', kind: 'model', status: 'completed', supported: true });
  assert.equal(page.app.state.processing.id, 'new-job');
  assert.equal(page.element('cancel-processing').hidden, false);
  assert.equal(page.element('import-audio').disabled, false);
});

test('queued cancellation targets only its job while the download remains visible', async () => {
  const page = await openApp();
  const cancelled = [];
  page.bridge.cancelProcessing = async id => { cancelled.push(id); };
  await page.app.acceptProcessing({ revision: 3, id: 'download', kind: 'summary-model', status: 'running', supported: true, modelReady: true,
    jobs: [{ id: 'waiting', kind: 'summary', meetingId: 'a', status: 'queued' }] });
  assert.match(page.element('processing-jobs').innerHTML, /회의 A: 회의록 요약 대기 중/);
  assert.equal(page.element('create-meeting').disabled, false);
  await page.app.cancelProcessing('waiting');
  assert.deepEqual(cancelled, ['waiting']);
  assert.equal(page.app.state.processing.id, 'download');
  assert.equal(page.element('processing-spinner').hidden, false);
  await page.app.acceptProcessing({ ...page.app.state.processing, revision: 4, jobs: [{ id: 'waiting', kind: 'summary', meetingId: 'a', status: 'cancelled' }] });
  assert.equal(page.element('processing-jobs').hidden, true);
});

test('saving recovery offers result inspection and never retries cancelled work', async () => {
  const page = await openApp();
  let retries = 0;
  page.bridge.retryProcessing = async () => { retries += 1; };
  await page.app.acceptProcessing({ revision: 5, status: 'idle', supported: true, modelReady: true, jobs: [
    { id: 'saving', kind: 'summary', meetingId: 'a', status: 'needsReview', retryable: false },
    { id: 'cancelled', kind: 'transcribe', meetingId: 'b', status: 'cancelled', retryable: false },
  ] });
  const html = page.element('processing-jobs').innerHTML;
  assert.match(html, /저장된 결과를 확인/);
  assert.match(html, /data-job-action="open" data-job-id="saving"/);
  assert.doesNotMatch(html, /data-job-action="retry"|data-job-id="cancelled"/);
  await page.app.retryProcessing('saving');
  await page.app.retryProcessing('cancelled');
  assert.equal(retries, 0);
});

test('a failed job remains actionable after another job starts and after restart', async () => {
  const failed = { id: 'failed-summary', kind: 'summary', meetingId: 'a', status: 'failed', retryable: true, failureKind: 'modelNotReady', message: 'private engine path' };
  const page = await openApp();
  await page.app.acceptProcessing({ revision: 3, ...failed, supported: true, modelReady: true, jobs: [failed] });
  assert.equal(page.element('retry-processing').hidden, false);
  assert.doesNotMatch(page.element('processing-jobs').innerHTML, /data-job-id="failed-summary"/);
  await page.app.acceptProcessing({ revision: 4, id: 'download', kind: 'summary-model', status: 'running', supported: true, modelReady: true, jobs: [failed] });
  assert.match(page.element('processing-jobs').innerHTML, /회의 A: 회의록 요약 실패/);
  assert.match(page.element('processing-jobs').innerHTML, /요약 모델을 다시 준비/);
  assert.doesNotMatch(page.element('processing-jobs').innerHTML, /private engine path/);
  assert.match(page.element('processing-jobs').innerHTML, /data-job-action="retry" data-job-id="failed-summary"/);
  const reopened = await openApp(null, { revision: 1, status: 'idle', supported: true, modelReady: true, jobs: [failed] });
  assert.match(reopened.element('processing-jobs').innerHTML, /data-job-action="retry" data-job-id="failed-summary"/);
  let retried;
  reopened.bridge.retryProcessing = async id => { retried = id; return { ...reopened.app.state.processing, revision: 2 }; };
  await reopened.app.retryProcessing('failed-summary');
  assert.equal(retried, 'failed-summary');
});

test('explicit acknowledgement hides only the notification and restores next, previous and stable keyboard focus', async () => {
  const page = await openApp(transcriptDocument());
  const jobs = ['first', 'second', 'third'].map(id => ({ id, kind: 'summary', meetingId: 'a', status: 'failed', failureKind: 'outputLimit', retryable: true }));
  await page.app.acceptProcessing({ revision: 3, status: 'idle', supported: true, modelReady: true, jobs });
  const control = (id, action) => page.element('processing-jobs').children.find(node => node.dataset.jobId === id && node.dataset.jobAction === action);
  const acknowledgements = [];
  page.bridge.acknowledgeProcessing = async id => {
    acknowledgements.push(id);
    return { ...page.app.state.processing, revision: page.app.state.processing.revision + 1,
      jobs: page.app.state.processing.jobs.map(job => job.id === id ? { ...job, acknowledged: true } : job) };
  };
  control('first', 'ack').focus();
  await page.app.acknowledgeProcessing('first');
  assert.equal(page.element(page.focused()).dataset.jobId, 'second');
  control('third', 'ack').focus();
  await page.app.acknowledgeProcessing('third');
  assert.equal(page.element(page.focused()).dataset.jobId, 'second');
  control('second', 'ack').focus();
  await page.app.acknowledgeProcessing('second');
  assert.equal(page.focused(), 'settings');
  assert.equal(page.element('processing-jobs').hidden, true);
  assert.equal(page.app.state.processing.jobs.length, 3);
  assert.deepEqual(acknowledgements, ['first', 'third', 'second']);
  assert.equal(page.app.state.processing.jobs.every(job => job.status === 'failed' && job.acknowledged), true);
});

test('opening a recovery meeting does not acknowledge it or expose an unknown summary engine error', async () => {
  const page = await openApp();
  const job = { id: 'unknown', kind: 'summary', meetingId: 'b', status: 'failed', retryable: true, message: '/private/engine response token=private' };
  await page.app.acceptProcessing({ revision: 4, status: 'idle', supported: true, modelReady: true, jobs: [job] });
  let acknowledgements = 0;
  page.bridge.acknowledgeProcessing = async () => { acknowledgements += 1; };
  const html = page.element('processing-jobs').innerHTML;
  assert.match(html, /원인을 확인하지 못했습니다/);
  assert.doesNotMatch(html, /private|token=/);
  const opener = page.element('processing-jobs').children.find(node => node.dataset.jobAction === 'open');
  page.element('processing-jobs').listeners.get('click')({ target: { closest: () => opener } });
  await tick(); await tick();
  assert.equal(page.app.state.selectedId, 'b');
  assert.equal(acknowledgements, 0);
  assert.equal(page.app.state.processing.jobs[0].acknowledged, undefined);
});

test('acknowledging an alert does not take focus from a title edited during the request', async () => {
  const page = await openApp(transcriptDocument());
  const stopped = { id: 'stopped', kind: 'summary', meetingId: 'a', status: 'interrupted', retryable: true };
  await page.app.acceptProcessing({ revision: 5, status: 'idle', supported: true, modelReady: true, jobs: [stopped] });
  const response = deferred();
  page.bridge.acknowledgeProcessing = () => response.promise;
  page.element('processing-jobs').children.find(node => node.dataset.jobAction === 'ack').focus();
  const request = page.app.acknowledgeProcessing('stopped');
  page.element('meeting-title').focus(); page.element('meeting-title').value = '계속 편집하는 제목'; page.app.onEdit();
  response.resolve({ ...page.app.state.processing, revision: 6, jobs: [{ ...stopped, acknowledged: true }] });
  await request;
  assert.equal(page.focused(), 'meeting-title');
  assert.equal(page.element('meeting-title').value, '계속 편집하는 제목');
  assert.equal(page.app.state.draft.dirty, true);
});

for (const action of ['cancel', 'retry']) {
  test(`${action} restores focus when its recovery row disappears`, async () => {
    const page = await openApp();
    const first = { id: 'first', kind: 'summary', meetingId: 'a', status: action === 'cancel' ? 'queued' : 'failed', retryable: action === 'retry' };
    const next = { id: 'next', kind: 'summary', meetingId: 'b', status: 'interrupted', retryable: true };
    await page.app.acceptProcessing({ revision: 7, status: 'idle', supported: true, modelReady: true, jobs: [first, next] });
    page.element('processing-jobs').children.find(node => node.dataset.jobId === 'first' && node.dataset.jobAction === action).focus();
    const updated = { ...page.app.state.processing, revision: 8, jobs: [{ ...first, status: action === 'cancel' ? 'cancelled' : 'failed', acknowledged: action === 'retry' }, next] };
    if (action === 'cancel') {
      page.bridge.cancelProcessing = async () => {};
      await page.app.cancelProcessing('first');
      await page.app.acceptProcessing(updated);
    } else {
      page.bridge.retryProcessing = async () => updated;
      await page.app.retryProcessing('first');
    }
    assert.equal(page.element(page.focused()).dataset.jobId, 'next');
  });
}

test('a stale-source retry error preserves editing and remains next to the stopped job', async () => {
  const page = await openApp(transcriptDocument());
  page.edit('저장 전 메모');
  const before = page.app.state.transcriptDraft.doc;
  page.bridge.retryProcessing = async () => { throw new Error('회의 내용이 변경되었습니다.'); };
  await page.app.acceptProcessing({ revision: 6, status: 'idle', supported: true, modelReady: true, jobs: [
    { id: 'stopped', kind: 'summary', meetingId: 'a', status: 'interrupted', retryable: true },
  ] });
  await page.app.retryProcessing('stopped');
  assert.equal(page.app.state.transcriptDraft.doc, before);
  assert.equal(page.element('meeting-notes').value, '저장 전 메모');
  assert.equal(page.app.state.draft.dirty, true);
  assert.match(page.element('processing-jobs').innerHTML, /회의 내용이 변경/);
});

test('a background recognition result refreshes its owner without switching meetings or tabs', async () => {
  const page = await openApp(transcriptDocument());
  let reads = 0;
  const readTranscript = page.bridge.readTranscript;
  page.bridge.readTranscript = async id => { reads += 1; return readTranscript(id); };
  const current = { revision: 7, id: 'download', kind: 'summary-model', status: 'running', supported: true, modelReady: true,
    jobs: [{ id: 'recognition', kind: 'transcribe', meetingId: 'a', status: 'queued' }] };
  await page.app.acceptProcessing(current);
  const selected = page.app.state.selectedId;
  await page.app.acceptProcessing({ ...current, revision: 8, jobs: [{ ...current.jobs[0], status: 'completed' }] });
  assert.equal(reads, 1);
  assert.equal(page.app.state.selectedId, selected);
  assert.equal(page.app.state.processing.id, 'download');
});


test('late summary response cannot leak into another meeting', async () => {
  const page = await openApp(transcriptDocument());
  const pending = deferred();
  page.bridge.readSummary = async (id) => id === 'a' ? pending.promise : null;
  const loading = page.app.loadSummary('a');
  await page.app.selectMeeting('b');
  pending.resolve({ document: { id: 'summary-a', text: '회의 A 내용', sourceVersion: transcriptDocument().version }, stale: false });
  await loading;
  assert.equal(page.app.state.summary, null);
  assert.doesNotMatch(page.element('summary-content').innerHTML, /회의 A 내용|data-summary-citations/);
});

test('summary rendering escapes model output and retains frozen evidence after editing', async () => {
  const page = await openApp(transcriptDocument());
  page.app.state.summary = { document: { id: 'summary-a', text: '<script>bad()</script> 결정 [1]', sourceVersion: structuredClone(page.app.state.transcriptDraft.doc.version) }, stale: false };
  page.app.renderSummary(page.app.state.snapshot.meetings[0]);
  assert.match(page.element('summary-content').innerHTML, /&lt;script&gt;/);
  assert.match(page.element('summary-content').innerHTML, /data-summary-citations/);
  page.editTranscript('수정 중');
  page.app.renderSummary(page.app.state.snapshot.meetings[0]);
  assert.match(page.element('summary-content').innerHTML, /data-summary-citations/);
  assert.match(page.element('summary-message').textContent, /이전 내용으로 만든 요약/);
});

test('summary starts only after pending transcript edits are saved', async () => {
  const page = await openApp(transcriptDocument());
  const calls = [];
  page.bridge.summarizeMeeting = async (id) => { calls.push(id); return { id:'summary-job', kind:'summary', status:'running', revision:10, meetingId:id }; };
  page.editTranscript('요약 전 수정');
  const click = page.element('summarize').listeners.get('click')();
  await tick();
  assert.equal(calls.length, 0);
  page.transcriptUpdates[0].pending.resolve();
  await click;
  assert.deepEqual(calls, ['a']);
  assert.equal(page.app.state.transcriptDraft.dirty, false);
});


test('missing model readiness opens setup, blocks creation and import, and preserves existing meetings', async () => {
  const page = await openApp(null, { status: 'idle', supported: true, modelReady: false, summarySupported: true, summaryReady: false, summaryBytes: 1000 });
  let creates = 0, imports = 0;
  page.bridge.createMeeting = async () => { creates++; };
  page.bridge.importAudio = async () => { imports++; };
  assert.equal(page.app.state.selectedId, null);
  assert.equal(page.element('setup-screen').hidden, false);
  await page.app.createMeeting();
  page.element('import-audio').listeners.get('click')(); await tick();
  assert.equal(creates, 0); assert.equal(imports, 0);
  await page.app.selectMeeting('a');
  assert.equal(page.element('setup-screen').hidden, true);
  assert.equal(page.element('meeting-detail').hidden, false);
  assert.equal(page.element('meeting-title').value, '회의 A');
});

test('setup starts only after the download action, cancellation does not unlock meetings', async () => {
  const page = await openApp(null, { revision: 1, status: 'idle', supported: true, modelReady: false, summarySupported: true, summaryReady: false });
  let starts = 0, cancellations = 0;
  page.bridge.prepareMeetingModels = async (includeSummary) => { assert.equal(includeSummary, false); starts++; return { revision: 2, id: 'setup-1', kind: 'setup', status: 'running', supported: true, summarySupported: true }; };
  page.bridge.cancelProcessing = async () => { cancellations++; };
  assert.equal(starts, 0);
  page.element('setup-download').listeners.get('click')(); await tick();
  assert.equal(starts, 1);
  page.element('setup-cancel').listeners.get('click')(); await tick();
  assert.equal(cancellations, 1);
  await page.app.acceptProcessing({ revision: 3, id: 'setup-1', kind: 'setup', status: 'cancelled', supported: true, modelReady: false, summarySupported: true, summaryReady: false });
  assert.equal(page.element('setup-screen').hidden, false);
  assert.equal(page.element('setup-start').hidden, true);
  await page.app.acceptProcessing({ revision: 4, id: 'setup-2', kind: 'setup', status: 'completed', supported: true, modelReady: true, summarySupported: true, summaryReady: false });
  assert.equal(page.element('setup-screen').hidden, false);
  assert.equal(page.element('setup-start').hidden, false);
});


test('speech readiness opens meetings without optional summary and optional download is explicit', async () => {
  const page = await openApp(null, { status: 'idle', supported: true, modelReady: true, summarySupported: false, summaryReady: false });
  assert.equal(page.app.state.selectedId, 'a');
  assert.equal(page.element('setup-screen').hidden, true);
  assert.equal(page.element('meeting-detail').hidden, false);
  const fresh = await openApp(null, { status: 'idle', supported: true, modelReady: false, summarySupported: true, summaryReady: false });
  let requested = null;
  fresh.bridge.prepareMeetingModels = async (includeSummary) => { requested = includeSummary; return { revision: 1, kind: 'setup', status: 'running', supported: true, summarySupported: true }; };
  fresh.element('setup-summary-option').checked = true;
  fresh.element('setup-download').listeners.get('click')(); await tick();
  assert.equal(requested, true);
  assert.equal(fresh.element('setup-download').textContent.includes('GB'), false);
});


test('opening an active setup keeps progress visible after speech becomes ready', async () => {
  const page = await openApp(null, { revision: 1, status: 'idle', supported: true, modelReady: false, summarySupported: true, summaryReady: false });
  page.app.state.snapshot.meetings = [];
  page.app.state.setupRequested = false;
  await page.app.acceptProcessing({ revision: 2, id: 'setup-active', kind: 'setup', status: 'running', modelStage: 'summary', supported: true, modelReady: true, summarySupported: true, summaryReady: false, totalBytes: 100, bytes: 50 });
  assert.equal(page.element('setup-screen').hidden, false);
  assert.equal(page.element('setup-progress-region').hidden, false);
  assert.equal(page.element('setup-start').hidden, true);
});

test('optional summary download leaves new meetings enabled and reports transfer percent', async () => {
  const page = await openApp();
  await page.app.acceptProcessing({ revision: 3, id: 'download', kind: 'summary-model', status: 'running', supported: true, modelReady: true, summarySupported: true, summaryReady: false, message: '요약 모델 다운로드 중', bytes: 25, totalBytes: 100 });
  assert.equal(page.element('create-meeting').disabled, false);
  assert.equal(page.element('setup-screen').hidden, true);
  assert.equal(page.element('processing-message').textContent, '요약 모델 다운로드 중');
  assert.equal(page.element('processing-percent').textContent, '25%');
  let created = false;
  page.bridge.createMeeting = async () => { created = true; return { id: 'b' }; };
  await page.app.createMeeting();
  assert.equal(created, true);
  assert.equal(page.app.state.selectedId, 'b');
});

test('memory warnings and same-job summary retries preserve readiness, new meetings and cancellation', async () => {
  const page = await openApp();
  const job = { id: 'summary-memory', kind: 'summary', status: 'running', meetingId: 'a', supported: true, modelReady: true, summarySupported: true, summaryReady: true, failureKind: null };
  await page.app.acceptProcessing({ ...job, revision: 3, memoryState: 'warning', retryCount: 0, message: '요약 모델을 불러오고 있습니다.' });
  assert.equal(page.element('processing-warning').hidden, true);
  assert.equal(page.element('processing-warning').textContent, '');
  assert.equal(page.element('processing-message').textContent, '');
  assert.equal(page.element('processing-spinner').hidden, false);
  assert.equal(page.element('processing-percent').hidden, true);
  assert.equal(page.element('cancel-processing').disabled, false);
  assert.equal(page.element('create-meeting').disabled, false);
  assert.equal(page.app.state.lastError, null);
  let created = false;
  page.bridge.createMeeting = async () => { created = true; return { id: 'b' }; };
  await page.app.createMeeting();
  assert.equal(created, true);
  assert.equal(page.app.state.selectedId, 'b');
  await page.app.acceptProcessing({ ...job, revision: 4, memoryState: 'critical', retryCount: 1, message: '요약을 다시 준비하고 있습니다.' });
  assert.equal(page.app.state.processing.id, 'summary-memory');
  assert.equal(page.app.state.processing.retryCount, 1);
  assert.equal(page.app.state.processing.modelReady, true);
  assert.equal(page.app.state.processing.summaryReady, true);
  assert.equal(page.element('processing-spinner').hidden, false);
  assert.equal(page.element('create-meeting').disabled, false);
  assert.equal(page.element('cancel-processing').disabled, false);
  const cancellations = [];
  page.bridge.cancelProcessing = async id => { cancellations.push(id); };
  await page.app.cancelProcessing();
  assert.deepEqual(cancellations, ['summary-memory']);
  assert.equal(page.element('processing-warning').hidden, true);
  assert.equal(page.element('processing-spinner').hidden, false);
  assert.equal(page.element('cancel-processing').disabled, true);
  await page.app.acceptProcessing({ ...job, revision: 5, status: 'cancelled', memoryState: 'critical', retryCount: 1, message: '처리를 취소했습니다.' });
  assert.equal(page.element('processing-warning').hidden, true);
  assert.equal(page.element('processing-spinner').hidden, true);
});

test('unknown memory observations and worker failures do not fabricate low-memory errors', async () => {
  const page = await openApp();
  const job = { id: 'summary-observation', kind: 'summary', meetingId: 'a', supported: true, modelReady: true, summarySupported: true, summaryReady: true, retryCount: 0 };
  await page.app.acceptProcessing({ ...job, revision: 3, status: 'running', memoryState: 'unknown', failureKind: null, message: '요약 중' });
  assert.equal(page.element('processing-warning').hidden, true);
  assert.equal(page.element('processing-spinner').hidden, false);
  assert.equal(page.app.state.lastError, null);
  await page.app.acceptProcessing({ ...job, revision: 4, status: 'failed', memoryState: 'critical', failureKind: 'workerExit', message: '요약 실행기가 종료되었습니다.' });
  assert.equal(page.element('processing-warning').hidden, true);
  assert.match(page.element('processing-message').textContent, /원인을 확인하지 못/);
  assert.equal(page.element('processing-spinner').hidden, true);
  assert.equal(page.element('create-meeting').disabled, false);
  assert.equal(page.app.state.lastError, null);
});

test('summary commit keeps its spinner and save stage without cancellation or stale pressure warnings', async () => {
  const page = await openApp();
  const job = { id: 'summary-commit', kind: 'summary', status: 'running', meetingId: 'a', supported: true, modelReady: true, summarySupported: true, summaryReady: true, memoryState: 'critical' };
  await page.app.acceptProcessing({ ...job, revision: 3, message: '요약 중' });
  assert.equal(page.element('processing-warning').hidden, true);
  assert.equal(page.element('cancel-processing').hidden, false);
  page.app.state.processingCancellingId = job.id;
  await page.app.acceptProcessing({ ...job, revision: 4, summaryCommitting: true, message: '회의록을 저장하고 있습니다.' });
  assert.equal(page.element('processing-warning').hidden, true);
  assert.equal(page.element('cancel-processing').hidden, true);
  assert.equal(page.element('cancel-processing').disabled, true);
  assert.equal(page.element('processing-spinner').hidden, false);
  assert.equal(page.element('processing').dataset.state, 'running');
  assert.equal(page.element('processing-title').textContent, '저장 중');
  assert.equal(page.element('processing-message').hidden, true);
  assert.equal(page.element('processing-percent').hidden, true);
  assert.equal(page.element('create-meeting').disabled, false);
  assert.equal(page.app.state.lastError, null);
  await page.app.acceptProcessing({ ...job, revision: 5, status: 'completed', summaryCommitting: false, message: '완료' });
  assert.equal(page.element('processing-warning').hidden, true);
  assert.equal(page.element('processing-spinner').hidden, true);
  assert.equal(page.element('processing').hidden, true);
});

test('export saves the latest transcript correction before opening the native save dialog', async () => {
  const page = await openApp(transcriptDocument());
  const exports = [];
  page.bridge.exportText = async (name, text) => { exports.push({ name, text }); return true; };
  page.editTranscript('내보낼 수정 문장');
  const exporting = page.app.exportDocument('transcript', 'md');
  await tick();
  assert.equal(exports.length, 0);
  page.transcriptUpdates[0].pending.resolve();
  await exporting;
  assert.equal(exports.length, 1);
  assert.match(exports[0].name, /음성 기록\.md$/);
  assert.match(exports[0].text, /내보낼 수정 문장/);
  assert.doesNotMatch(exports[0].text, /모델 원문/);
});


test('microphone preference persists and settings reject changes during recording', async () => {
  const storage = new Map();
  const page = await openApp(null, undefined, storage);
  await page.app.openSettings('recording');
  await page.setDevices([{ id: 'usb-input', name: 'USB 마이크', isDefault: true }]);
  page.element('settings-microphone').value = 'usb-input';
  page.app.changeMicrophone();
  const reopened = await openApp(null, undefined, storage);
  assert.equal(reopened.app.state.deviceId, 'usb-input');
  assert.equal(reopened.app.state.deviceName, 'USB 마이크');
  page.app.acceptSnapshot({revision: 2, meetings: page.app.state.snapshot.meetings, session: {id:'recording-a',meetingId:'a',state:'recording',elapsedMs:0}});
  assert.equal(page.element('settings-microphone').disabled, true);
  page.element('settings-microphone').value = '';
  page.app.changeMicrophone();
  assert.equal(page.app.state.deviceId, 'usb-input');
  assert.match(page.element('settings-message').textContent, /녹음을 마친 뒤/);
});

test('settings storage failure preserves the microphone choice and reports recovery', async () => {
  const storage = { get() { return null; }, set() { throw Error('storage full'); } };
  const page = await openApp(null, undefined, storage);
  await page.app.openSettings('recording');
  await page.setDevices([{id:'usb',name:'USB',isDefault:true}]);
  page.element('settings-microphone').value = 'usb';
  page.app.changeMicrophone();
  assert.equal(page.app.state.deviceId, null);
  assert.match(page.element('settings-message').textContent, /저장하지 못했습니다/);
  assert.equal(page.element('settings-microphone').value, '');
});

test('starting a recording rechecks device availability and uses the saved preference', async () => {
  const page = await openApp();
  let started = null;
  page.bridge.startRecording = async (meetingId, deviceId) => { started = {meetingId,deviceId}; return page.bridge.readSnapshot(); };
  await page.setDevices([{id:'usb',name:'USB',isDefault:true}]);
  page.app.state.deviceId = 'usb';
  await page.element('recording-controls').listeners.get('click')({target:{closest:()=>({dataset:{action:'start'}})}});
  await tick();
  assert.deepEqual(started, {meetingId:'a',deviceId:'usb'});
  started = null;
  page.bridge.listDevices = async () => [{id:'other',name:'내장',isDefault:true}];
  await page.element('recording-controls').listeners.get('click')({target:{closest:()=>({dataset:{action:'start'}})}});
  await tick();
  assert.equal(started, null);
});


test('settings flush edits and prepare optional summaries inline without replacing the meeting', async () => {
  const page = await openApp(null, { status: 'idle', supported: true, modelReady: true, summarySupported: true, summaryReady: false });
  page.edit('설정을 열기 전 메모');
  const opening = page.app.openSettings(); await tick();
  assert.equal(Boolean(page.element('settings-dialog').open), false);
  page.updates[0].pending.resolve(); await opening;
  assert.equal(page.saved().notes, '설정을 열기 전 메모');
  assert.equal(page.app.state.settingsPanel, 'appearance');
  let prepared = 0;
  page.bridge.prepareSummaryModel = async () => { prepared++; return { revision: 2, id: 'summary-download', kind: 'summary-model', status: 'running', supported: true, modelReady: true, summarySupported: true, summaryReady: false, bytes: 50, totalBytes: 100 }; };
  page.element('models').listeners.get('click')(); await tick();
  assert.equal(prepared, 1);
  assert.equal(page.element('settings-dialog').open, true);
  assert.equal(page.element('setup-screen').hidden, true);
  assert.equal(page.element('settings-feature-progress').value, 50);
  assert.equal(page.element('settings-feature-stage').textContent, '50%');
  page.element('settings-close').listeners.get('click')(); await tick();
  assert.equal(page.element('settings-dialog').open, false);
});

test('summary preparation returns to its meeting and focuses summary when the original preparation button disappears', async () => {
  const page = await openApp(transcriptDocument(), { revision: 1, status: 'idle', supported: true, modelReady: true, summarySupported: true, summaryReady: false });
  await page.app.detail.select('summary');
  assert.equal(page.element('prepare-summary').hidden, false);
  await page.element('prepare-summary').listeners.get('click')();
  assert.equal(page.app.state.setupFromSettings, false);
  assert.equal(page.element('setup-back').textContent, '회의로 돌아가기');
  await page.app.acceptProcessing({ ...page.app.state.processing, revision: 2, summaryReady: true });
  assert.equal(page.element('prepare-summary').hidden, true);
  assert.equal(page.element('summarize').disabled, false);
  page.element('setup-back').listeners.get('click')();
  assert.equal(page.element('setup-screen').hidden, true);
  assert.equal(page.element('meeting-detail').hidden, false);
  assert.equal(Boolean(page.element('settings-dialog').open), false);
  assert.equal(page.app.detail.active(), 'summary');
  assert.equal(page.focused(), 'summarize');
});

async function navigateSettings(page, tab, key, options = {}) {
  let prevented = false;
  page.element('settings-dialog').listeners.get('keydown')({ target: tab, key, ...options, preventDefault() { prevented = true; } });
  await tick();
  return prevented;
}

function assertSettingsSelection(page, selectedTab) {
  assert.equal(page.app.state.settingsPanel, selectedTab.dataset.settingsPanel);
  assert.equal(page.focused(), selectedTab.id);
  for (const tab of page.settingsTabs) {
    const selected = tab === selectedTab;
    assert.equal(tab.getAttribute('aria-selected'), String(selected));
    assert.equal(tab.tabIndex, selected ? 0 : -1);
    assert.equal(page.element(tab.getAttribute('aria-controls')).hidden, !selected);
  }
}

test('settings category keyboard navigation follows the displayed tab order and preserves meeting edits', async () => {
  const page = await openApp();
  await page.app.openSettings();
  assert.equal(page.app.state.settingsPanel, 'appearance');
  assert.equal(page.element('settings-appearance-panel').hidden, false);
  page.edit('설정 전환 중 보관할 메모');
  const draft = page.app.state.draft;
  const tabs = page.settingsTabs;
  assert.ok(tabs.length > 1);
  assert.ok(tabs.every(tab => tab.dataset.settingsPanel));
  for (const [index, tab] of tabs.entries()) {
    for (const [key, offset] of [['ArrowDown', 1], ['ArrowRight', 1], ['ArrowUp', -1], ['ArrowLeft', -1], ['Home', null], ['End', null]]) {
      tab.listeners.get('click')();
      await tick();
      tab.focus();
      assert.equal(await navigateSettings(page, tab, key), true);
      const expected = key === 'Home' ? tabs[0] : key === 'End' ? tabs.at(-1) : tabs[(index + offset + tabs.length) % tabs.length];
      assertSettingsSelection(page, expected);
    }
  }
  assert.equal(page.app.state.draft, draft);
  assert.equal(draft.notes, '설정 전환 중 보관할 메모');
  assert.equal(draft.dirty, true);
  assert.equal(page.updates.length, 0);
});

test('settings category navigation waits for template draft approval before changing tabs', async () => {
  const page = await openApp();
  await page.app.openSettings('templates');
  const tab = page.settingsTabs.find(tab => tab.dataset.settingsPanel === 'templates');
  const next = page.settingsTabs[(page.settingsTabs.indexOf(tab) + 1) % page.settingsTabs.length];
  const decision = deferred();
  let checks = 0;
  page.templateSettings.beforeLeave = () => { checks++; return decision.promise; };
  assert.equal(await navigateSettings(page, tab, 'ArrowDown'), true);
  assert.equal(checks, 1);
  assertSettingsSelection(page, tab);
  decision.resolve(false);
  await tick();
  assertSettingsSelection(page, tab);
  page.templateSettings.beforeLeave = async () => true;
  assert.equal(await navigateSettings(page, tab, 'ArrowDown'), true);
  assertSettingsSelection(page, next);
  assert.equal(page.updates.length, 0);
});

test('settings category navigation ignores IME input and an open template confirmation', async () => {
  const page = await openApp();
  await page.app.openSettings('templates');
  const tab = page.settingsTabs.find(tab => tab.dataset.settingsPanel === 'templates');
  let checks = 0;
  page.templateSettings.beforeLeave = async () => { checks++; return true; };
  for (const options of [{ isComposing: true }, { keyCode: 229 }]) {
    for (const key of ['ArrowDown', 'ArrowUp', 'ArrowRight', 'ArrowLeft', 'Home', 'End']) {
      assert.equal(await navigateSettings(page, tab, key, options), false);
      assertSettingsSelection(page, tab);
    }
  }
  page.templateSettings.isConfirming = () => true;
  for (const key of ['ArrowDown', 'ArrowUp', 'ArrowRight', 'ArrowLeft', 'Home', 'End']) {
    assert.equal(await navigateSettings(page, tab, key), false);
    assertSettingsSelection(page, tab);
  }
  assert.equal(checks, 0);
});

test('automatic saving shows pending feedback and clears it after persistence', async () => {
  const page = await openApp();
  assert.equal(page.element('save-status').hidden, true);
  page.element('meeting-notes').value = '자동 저장 메모';
  page.app.onEdit();
  const save = page.app.saveDraft();
  await tick();
  assert.equal(page.element('save-status').hidden, false);
  assert.equal(page.element('save-status').textContent, '저장 중');
  page.updates[0].pending.resolve();
  await save;
  assert.equal(page.element('save-status').hidden, true);
  assert.equal(page.app.state.draft.dirty, false);
});


test('unchanged snapshots and timer updates preserve recording control DOM', async () => {
  const page = await openApp();
  const initial = structuredClone(page.app.state.snapshot);
  const writes = page.element('meeting-status').htmlWrites;
  page.app.acceptSnapshot({ ...initial, revision: 2 });
  assert.equal(page.element('meeting-status').htmlWrites, writes);
  const active = { revision: 3, meetings: initial.meetings, session: { id: 's', meetingId: 'a', state: 'recording', elapsedMs: 1000 } };
  page.app.acceptSnapshot(active);
  const statusWrites = page.element('meeting-status').htmlWrites;
  const controlWrites = page.element('recording-controls').htmlWrites;
  page.app.acceptSnapshot({ ...active, revision: 4, session: { ...active.session, elapsedMs: 2000 } });
  assert.equal(page.element('recording-time').textContent, '00:02');
  assert.equal(page.element('meeting-status').htmlWrites, statusWrites);
  assert.equal(page.element('recording-controls').htmlWrites, controlWrites);
});

test('capture quality warning stays visible during recording and after completion without disabling controls', async () => {
  const page = await openApp();
  const warning = '일부 음성이 끊겼을 수 있습니다.';
  const meetings = structuredClone(page.app.state.snapshot.meetings);
  page.app.acceptSnapshot({ revision: 2, meetings, session: { id: 's', meetingId: 'a', state: 'recording', elapsedMs: 1000, warning } });
  assert.equal(page.element('recording-message').textContent, warning);
  assert.equal(page.element('recording-message').hidden, false);
  assert.match(page.element('recording-controls').innerHTML, /일시정지/);
  page.app.acceptSnapshot({ revision: 3, meetings: meetings.map(meeting => meeting.id === 'a' ? { ...meeting, status: 'completed', warning } : meeting), session: null });
  assert.equal(page.element('recording-message').textContent, warning);
  page.app.acceptSnapshot({ revision: 4, meetings: meetings.map(meeting => meeting.id === 'a' ? { ...meeting, status: 'interrupted', warning, error: '장치 연결이 끊어졌습니다.' } : meeting), session: { id: 's', meetingId: 'a', state: 'interrupted', elapsedMs: 1000, warning, error: '장치 연결이 끊어졌습니다.' } });
  assert.equal(page.element('recording-message').textContent, '장치 연결이 끊어졌습니다.');
});

test('saving notes cannot dismiss an unrelated processing error', async () => {
  const page = await openApp();
  page.edit('메모');
  const saving = page.app.saveDraft(); await tick();
  page.bridge.setAutoRecorder = async () => { throw Error('설정 오류'); };
  page.element('auto-recorder').checked = false;
  await page.element('auto-recorder').listeners.get('change')();
  page.element('error').hidden = false;
  page.app.state.errorIsSave = false;
  page.element('error-message').textContent = '처리 실패';
  page.updates[0].pending.resolve(); await saving;
  assert.equal(page.element('error').hidden, false);
  assert.equal(page.element('error-message').textContent, '처리 실패');
});

test('creating a meeting cannot create a blank record before current transcript edits persist', async () => {
  const page = await openApp(transcriptDocument());
  let creates = 0;
  page.bridge.createMeeting = async () => { creates++; };
  page.editTranscript('변경');
  const creating = page.app.createMeeting(); await tick();
  assert.equal(creates, 0);
  page.transcriptUpdates[0].pending.reject(Error('저장 실패'));
  await assert.rejects(creating);
  assert.equal(creates, 0);
  assert.equal(page.app.state.selectedId, 'a');
});

test('date classification persists without changing the recording or creation time', async () => {
  const page = await openApp();
  const createdAt = page.saved().createdAt;
  page.element('meeting-day').value = '2026-10-01';
  await page.app.changeMeetingDate();
  assert.equal(page.saved().meetingDate, '2026-10-01');
  assert.equal(page.saved().createdAt, createdAt);
  assert.equal(page.element('meeting-day').disabled, false);
  page.bridge.setMeetingDate = async () => { throw Error('storage'); };
  page.element('meeting-day').value = '2026-11-02';
  await page.app.changeMeetingDate();
  assert.equal(page.saved().meetingDate, '2026-10-01');
  assert.equal(page.element('meeting-day').value, '2026-10-01');
  assert.equal(page.element('meeting-date-message').hidden, false);
});

test('meeting list shows local recording time and refreshes it when recording starts', async () => {
  const page = await openApp();
  const createdAt = new Date(2026, 9, 20, 9, 5).getTime();
  const recordedAt = new Date(2026, 9, 20, 18, 41).getTime();
  const meeting = { ...page.saved(), createdAt, meetingDate: '2026-10-01' };
  page.app.acceptSnapshot({ revision: 2, session: null, meetings: [meeting] });
  assert.match(page.element('meeting-list').innerHTML, />10\. 20\. 09:05<\/span>/);
  assert.match(page.element('meeting-list').innerHTML, /title="생성 /);
  page.element('meeting-list').scrollTop = 80;
  page.app.acceptSnapshot({ revision: 3, session: null, meetings: [{ ...meeting, recordedAt }] });
  const html = page.element('meeting-list').innerHTML;
  assert.match(html, />10\. 20\. 18:41<\/span>/);
  assert.match(html, /title="녹음 시작 /);
  assert.match(html, /회의 날짜 2026-10-01/);
  assert.doesNotMatch(html, />10\. 1\. 18:41/);
  assert.equal(page.element('meeting-list').scrollTop, 80);
});

test('meeting list orders by the displayed timestamp without inventing imported recording time', async () => {
  const page = await openApp();
  const stamp = (hour, minute) => new Date(2026, 9, 20, hour, minute).getTime();
  const recorded = { ...page.saved(), id: 'recorded', createdAt: stamp(8, 0), recordedAt: stamp(18, 41), meetingDate: '2026-10-01', audio: {} };
  const imported = { ...page.saved(), id: 'imported', createdAt: stamp(14, 5), audio: {} };
  const midnight = { ...page.saved(), id: 'midnight', createdAt: stamp(0, 0) };
  page.app.acceptSnapshot({ revision: 2, session: null, meetings: [midnight, imported, recorded] });
  const html = page.element('meeting-list').innerHTML;
  assert.ok(html.indexOf('data-meeting-id="recorded"') < html.indexOf('data-meeting-id="imported"'));
  assert.ok(html.indexOf('data-meeting-id="imported"') < html.indexOf('data-meeting-id="midnight"'));
  assert.match(html, />10\. 20\. 00:00<\/span>/);
  const importedRow = html.slice(html.indexOf('data-meeting-id="imported"'), html.indexOf('data-meeting-id="midnight"'));
  assert.match(importedRow, /title="생성 /);
  assert.doesNotMatch(importedRow, /녹음 시작/);
});

test('automatic recorder preference persists and rolls back native preference when storage fails', async () => {
  const storage = new Map();
  const page = await openApp(null, undefined, storage);
  page.element('auto-recorder').checked = false;
  await page.element('auto-recorder').listeners.get('change')();
  assert.equal(page.app.state.autoRecorder, false);
  const reopened = await openApp(null, undefined, storage);
  assert.equal(reopened.app.state.autoRecorder, false);
  const failed = await openApp(null, undefined, { get: () => null, set: () => { throw Error('storage'); } });
  const calls = [];
  failed.bridge.setAutoRecorder = async value => { calls.push(value); };
  failed.element('auto-recorder').checked = false;
  await failed.element('auto-recorder').listeners.get('change')();
  assert.deepEqual(calls, [false,true]);
  assert.equal(failed.app.state.autoRecorder, true);
});

test('editing a transcript retains frozen citations without ordinal navigation', async () => {
  const page = await openApp(transcriptDocument());
  page.app.state.summary = { document: { id: 'summary', text: '회의록 [1]', sourceVersion: structuredClone(page.app.state.transcriptDraft.doc.version) }, stale: false };
  page.app.renderSummary(page.app.state.snapshot.meetings[0]);
  assert.match(page.element('summary-content').innerHTML, /data-summary-citations/);
  page.editTranscript('새 내용');
  assert.match(page.element('summary-content').innerHTML, /data-summary-citations/);
});

test('notes-aware summaries retain frozen citations and refresh after persistence without changing the selected tab or player', async () => {
  const page = await openApp(transcriptDocument());
  const document = { id: 'summary', text: '회의록 [1]', notesSha256: 'a'.repeat(64), sourceVersion: structuredClone(page.app.state.transcriptDraft.doc.version) };
  page.app.state.summary = { document, stale: false };
  page.app.state.summaryLoaded = true;
  page.app.renderSummary(page.app.state.snapshot.meetings[0]);
  await page.app.detail.select('notes');
  const player = page.element('audio-player');
  player.playbackRate = 1.5; player.currentTime = 15;
  let loads = 0, reads = 0;
  player.load = () => { loads += 1; };
  page.bridge.readSummary = async () => { reads += 1; return { document, stale: true }; };
  page.edit('추가 안건');
  assert.match(page.element('summary-content').innerHTML, /data-summary-citations/);
  assert.match(page.element('summary-message').textContent, /이전 내용으로 만든 요약/);
  const save = page.app.saveDraft(); await tick();
  page.updates[0].pending.resolve(); await save;
  assert.equal(reads, 1);
  assert.equal(page.app.state.summary.stale, true);
  assert.equal(page.app.detail.active(), 'notes');
  assert.equal(player.playbackRate, 1.5);
  assert.equal(player.currentTime, 15);
  assert.equal(loads, 0);
});

test('failed note persistence keeps the draft and frozen summary citations', async () => {
  const page = await openApp(transcriptDocument());
  const document = { id: 'summary', text: '회의록 [1]', notesSha256: 'a'.repeat(64), sourceVersion: structuredClone(page.app.state.transcriptDraft.doc.version) };
  page.app.state.summary = { document, stale: false }; page.app.state.summaryLoaded = true;
  page.edit('보존할 메모');
  const save = page.app.saveDraft(); await tick();
  page.updates[0].pending.reject(new Error('저장 실패'));
  await assert.rejects(save, /저장 실패/);
  assert.equal(page.app.state.draft.savedNotes, '');
  assert.equal(page.app.state.draft.notes, '보존할 메모');
  assert.equal(page.app.state.draft.dirty, true);
  assert.match(page.element('summary-content').innerHTML, /data-summary-citations/);
});

test('legacy transcript-only summaries and title-only edits do not acquire a notes dependency', async () => {
  for (const notesSha256 of [undefined, 'a'.repeat(64)]) {
    const page = await openApp(transcriptDocument());
    const document = { id: 'summary', text: '회의록 [1]', notesSha256, sourceVersion: structuredClone(page.app.state.transcriptDraft.doc.version) };
    page.app.state.summary = { document, stale: false }; page.app.state.summaryLoaded = true;
    page.element('meeting-title').value = '수정한 제목';
    page.app.onEdit();
    assert.match(page.element('summary-content').innerHTML, /data-summary-citations/);
    if (notesSha256 == null) {
      page.edit('구형 요약과 관계없는 메모');
      assert.match(page.element('summary-content').innerHTML, /data-summary-citations/);
    }
  }
});


test('action failure does not downgrade a failed save recovery button', async () => {
  const page = await openApp();
  page.edit('저장할 메모');
  page.element('create-meeting').listeners.get('click')(); await tick();
  page.updates[0].pending.reject(Error('저장 실패')); await tick(); await tick();
  assert.equal(page.element('retry-save').hidden, false);
  assert.equal(page.app.state.errorIsSave, true);
});

test('readiness arriving after the first snapshot still selects an existing meeting', async () => {
  const page = await openApp(null, { revision: 1, status: 'idle', supported: true, modelReady: false });
  assert.equal(page.app.state.selectedId, null);
  const same = structuredClone(page.app.state.snapshot);
  await page.app.acceptProcessing({ revision: 2, status: 'idle', supported: true, modelReady: true });
  page.app.acceptSnapshot(same); await tick();
  assert.equal(page.app.state.selectedId, 'a');
});

test('delayed date failure belongs to its meeting after navigating elsewhere', async () => {
  const page = await openApp();
  const pending = deferred();
  page.bridge.setMeetingDate = () => pending.promise;
  page.element('meeting-day').value = '2026-11-01';
  const saving = page.app.changeMeetingDate(); await tick();
  await page.app.selectMeeting('b');
  pending.reject(Error('storage')); await saving;
  assert.equal(page.app.state.selectedId, 'b');
  assert.equal(page.element('meeting-date-message').hidden, true);
  await page.app.selectMeeting('a');
  assert.equal(page.element('meeting-date-message').hidden, false);
});


test('transcript reading stays compact until a row is explicitly edited', async () => {
  const page = await openApp(transcriptDocument());
  assert.doesNotMatch(page.element('transcript-rows').innerHTML, /textarea|원문 보기/);
  await page.app.editTranscriptRow(0);
  assert.match(page.element('transcript-rows').innerHTML, /textarea[^>]*rows="1"/);
  page.editTranscript('읽고 수정한 내용');
  const writes = page.element('transcript-rows').htmlWrites;
  await page.app.acceptProcessing({ id: 'other', revision: 6, status: 'running', kind: 'summary', meetingId: 'b', supported: true, modelReady: true });
  assert.equal(page.element('transcript-rows').htmlWrites, writes);
  assert.equal(page.app.state.transcriptEditing, 0);
  const done = page.app.completeTranscriptRow();
  await tick();
  assert.equal(page.app.state.transcriptEditing, 0);
  page.transcriptUpdates[0].pending.resolve();
  await done;
  assert.equal(page.app.state.transcriptEditing, null);
  assert.doesNotMatch(page.element('transcript-rows').innerHTML, /textarea/);
  assert.match(page.element('transcript-rows').innerHTML, /읽고 수정한 내용/);
  assert.match(page.element('transcript-rows').innerHTML, /원문 보기/);
});

test('row completion failure preserves the editor and cancellation only discards unsaved edits', async () => {
  const page = await openApp(transcriptDocument());
  await page.app.editTranscriptRow(0);
  page.editTranscript('실패 시 유지할 내용');
  const done = page.app.completeTranscriptRow();
  await tick();
  page.transcriptUpdates[0].pending.reject(Error('저장 실패'));
  await done;
  assert.equal(page.app.state.transcriptEditing, 0);
  assert.equal(page.app.state.transcriptDraft.doc.segments[0].text, '실패 시 유지할 내용');
  await page.app.cancelTranscriptRow();
  assert.equal(page.app.state.transcriptEditing, null);
  assert.equal(page.app.state.transcriptDraft.dirty, false);
  assert.equal(page.app.state.transcriptDraft.doc.segments[0].text, '모델 원문');
  assert.doesNotMatch(page.element('transcript-rows').innerHTML, /textarea/);
});

test('successful finishEditing closes the transcript editor only after its correction persists', async () => {
  const page = await openApp(transcriptDocument());
  await page.app.editTranscriptRow(0);
  page.editTranscript('전환 전에 확정할 수정');
  let actionCalls = 0;
  const finishing = page.app.finishEditing(() => { actionCalls++; return 'finished'; });
  await tick();
  assert.equal(page.app.state.transcriptEditing, 0);
  assert.equal(actionCalls, 0);
  assert.match(page.element('transcript-rows').innerHTML, /<textarea/);
  page.transcriptUpdates[0].pending.resolve();
  assert.equal(await finishing, 'finished');
  assert.equal(actionCalls, 1);
  assert.equal(page.app.state.transcriptEditing, null);
  assert.equal(page.app.state.transcriptDraft.dirty, false);
  assert.equal(page.savedTranscript().segments[0].text, '전환 전에 확정할 수정');
  assert.doesNotMatch(page.element('transcript-rows').innerHTML, /<textarea|data-transcript-cancel=/);
});

test('returning after a successful tab transition cannot offer cancellation of an already saved edit', async () => {
  const page = await openApp(transcriptDocument());
  await page.app.detail.select('transcript');
  await page.app.editTranscriptRow(0);
  page.editTranscript('탭을 옮겨 저장된 문장');
  const changing = page.app.detail.select('summary');
  await tick();
  assert.equal(page.app.detail.active(), 'transcript');
  assert.equal(page.app.state.transcriptEditing, 0);
  page.transcriptUpdates[0].pending.resolve();
  assert.equal(await changing, true);
  assert.equal(page.app.detail.active(), 'summary');
  await page.app.detail.select('transcript');
  assert.equal(page.app.state.transcriptEditing, null);
  assert.match(page.element('transcript-rows').innerHTML, /탭을 옮겨 저장된 문장/);
  assert.doesNotMatch(page.element('transcript-rows').innerHTML, /<textarea|data-transcript-cancel=/);
  await page.app.cancelTranscriptRow();
  assert.equal(page.app.state.transcriptDraft.doc.segments[0].text, '탭을 옮겨 저장된 문장');
  assert.equal(page.transcriptUpdates.length, 1);
});

test('failed transcript persistence keeps the current tab and its cancellable editor intact', async () => {
  const page = await openApp(transcriptDocument());
  await page.app.detail.select('transcript');
  await page.app.editTranscriptRow(0);
  page.editTranscript('저장 실패 때 남겨야 하는 문장');
  const changing = page.app.detail.select('summary');
  await tick();
  page.transcriptUpdates[0].pending.reject(Error('저장 실패'));
  assert.equal(await changing, false);
  assert.equal(page.app.detail.active(), 'transcript');
  assert.equal(page.app.state.transcriptEditing, 0);
  assert.equal(page.app.state.transcriptDraft.dirty, true);
  assert.equal(page.app.state.transcriptDraft.doc.segments[0].text, '저장 실패 때 남겨야 하는 문장');
  assert.equal(page.savedTranscript().segments[0].text, '모델 원문');
  assert.equal(page.element('meeting-notes').readOnly, false);
  const editor = page.element('transcript-rows').querySelector('[data-segment="0"]');
  assert.equal(editor.value, '저장 실패 때 남겨야 하는 문장');
  assert.equal(editor.readOnly, false);
  assert.match(page.element('transcript-rows').innerHTML, /data-transcript-cancel=/);
});

test('cancellation after a flushed edit preserves the last successfully saved row', async () => {
  const page = await openApp(transcriptDocument());
  await page.app.editTranscriptRow(0);
  page.editTranscript('이미 저장한 수정');
  const saving = page.app.saveTranscript();
  await tick();
  page.transcriptUpdates[0].pending.resolve();
  await saving;
  page.editTranscript('저장 후 임시 수정');
  await page.app.cancelTranscriptRow();
  assert.equal(page.app.state.transcriptDraft.doc.segments[0].text, '이미 저장한 수정');
  assert.equal(page.app.state.transcriptDraft.dirty, false);
  assert.equal(page.savedTranscript().segments[0].originalText, '모델 원문');
});

test('processing stays visibly active until cancellation actually finishes', async () => {
  const page = await openApp();
  await page.app.acceptProcessing({ id: 'recognition', revision: 3, status: 'running', kind: 'transcribe', meetingId: 'a', supported: true, modelReady: true, message: '텍스트로 변환 중' });
  assert.equal(page.element('processing-spinner').hidden, false);
  assert.equal(page.element('processing-progress').hidden, true);
  assert.match(page.element('processing-title').textContent, /변환 중/);
  let requests = 0;
  page.bridge.cancelProcessing = async () => { requests++; };
  await page.app.cancelProcessing();
  await page.app.cancelProcessing();
  assert.equal(requests, 1);
  assert.equal(page.element('processing-spinner').hidden, false);
  assert.equal(page.element('cancel-processing').disabled, true);
  assert.equal(page.element('cancel-processing').textContent, '취소 중');
  await page.app.acceptProcessing({ id: 'recognition', revision: 4, status: 'cancelled', kind: 'transcribe', meetingId: 'a', supported: true, modelReady: true, message: '변환을 취소했습니다.' });
  assert.equal(page.element('processing-spinner').hidden, true);
  assert.equal(page.element('cancel-processing').hidden, true);
});

test('cancelled or failed terminology processing reveals its already saved recognition baseline', async () => {
  for (const status of ['cancelled', 'failed']) {
    const page = await openApp();
    const baseline = { version: { id: 'run', revision: 0 }, speakers: [], segments: [{ id: 'one', start: 0, end: 1, originalText: '인식된 원문', text: '인식된 원문' }] };
    const job = { id: 'recognition', kind: 'transcribe', meetingId: 'a', supported: true, modelReady: true };
    await page.app.acceptProcessing({ ...job, revision: 3, status: 'running', message: '용어 확인 중' });
    page.bridge.readTranscript = async () => structuredClone(baseline);
    await page.app.acceptProcessing({ ...job, revision: 4, status, message: '용어 확인 중단' });
    assert.equal(page.app.state.transcriptDraft.doc.version.id, 'run');
    assert.equal(page.app.state.transcriptDraft.doc.segments[0].text, '인식된 원문');
  }
});

test('refreshing a saved recognition result after cancellation preserves a dirty editor', async () => {
  const baseline = { version: { id: 'run', revision: 0 }, speakers: [], segments: [{ id: 'one', start: 0, end: 1, originalText: '원문', text: '원문' }] };
  const page = await openApp(baseline);
  page.editTranscript('직접 편집 중인 문장');
  const draft = page.app.state.transcriptDraft;
  assert.equal(draft.dirty, true);
  page.bridge.readTranscript = async () => ({ ...baseline, version: { id: 'later', revision: 0 } });
  await page.app.acceptProcessing({ id: 'recognition', revision: 4, kind: 'transcribe', meetingId: 'a', status: 'cancelled', supported: true, modelReady: true });
  assert.equal(page.app.state.transcriptDraft, draft);
  assert.equal(draft.doc.segments[0].text, '직접 편집 중인 문장');
  assert.equal(draft.dirty, true);
});

test('setup verification has activity without a misleading zero percent bar', async () => {
  const page = await openApp(null, { id: 'prepare', kind: 'setup', status: 'running', supported: true, modelReady: false, message: '모델 파일 확인 중', totalBytes: 0 });
  assert.equal(page.element('setup-spinner').hidden, false);
  assert.equal(page.element('setup-progress').hidden, true);
  assert.equal(page.element('setup-progress-bytes').textContent, '');
});


test('inspector switches preserve Korean composition and stay on the editor if saving fails', async () => {
  const page = await openApp();
  await page.app.detail.select('notes');
  assert.equal(page.app.detail.active(), 'notes');
  page.app.state.composing = true;
  page.element('meeting-notes').focus();
  page.element('meeting-notes').value = '회의에서 결정한 사항';
  const changing = page.app.detail.select('summary');
  await tick();
  assert.equal(page.app.detail.active(), 'notes');
  assert.equal(page.updates[0].notes, '회의에서 결정한 사항');
  page.updates[0].pending.reject(new Error('저장 실패'));
  assert.equal(await changing, false);
  assert.equal(page.app.detail.active(), 'notes');
  assert.equal(page.app.state.draft.notes, '회의에서 결정한 사항');
  assert.equal(page.app.state.draft.dirty, true);
  assert.equal(page.element('meeting-notes').readOnly, false);
});

test('a long transcript does not need scrolling to reveal the summary and loading keeps the chosen panel', async () => {
  const doc = transcriptDocument();
  doc.segments = Array.from({length: 300}, (_,i) => ({id: `segment-${i}`, start: i * 5, end: i * 5 + 4, text: `발언 ${i + 1}`, originalText: `발언 ${i + 1}`}));
  const page = await openApp(doc);
  assert.equal(page.app.detail.active(), 'summary');
  const pending = deferred();
  page.bridge.readSummary = () => pending.promise;
  const loading = page.app.loadSummary('a');
  await page.app.detail.select('notes');
  pending.resolve({document: {id: 'summary-a', text: '결정 사항 [201]', sourceVersion: doc.version}, stale: false});
  await loading;
  assert.equal(page.app.detail.active(), 'notes');
  await page.app.detail.select('summary');
  assert.equal(page.element('summary-section').hidden, false);
  assert.equal(page.element('transcript-section').hidden, true);
  assert.match(page.element('summary-content').innerHTML, /결정 사항/);
  assert.match(page.element('transcript-rows').innerHTML, /발언 300/);
});

test('summary evidence navigates by verified segment ID without playing and returns to summary', async () => {
  const doc = transcriptDocument();
  doc.segments[0].start = 23;
  const page = await openApp(doc);
  page.bridge.readSummary = async () => ({document: {id: 'summary-a', text: '결정 사항 [1]', sourceVersion: doc.version}, stale: false});
  await page.app.loadSummary('a');
  await page.app.detail.select('summary');
  let scrolled = false, focused = false, played = 0;
  page.element('transcript-rows').querySelector = () => ({setAttribute() {}, scrollIntoView() { scrolled = true; }, focus() { focused = true; }});
  page.element('audio-player').setAttribute('src', 'example.wav');
  page.element('audio-player').currentTime = 7;
  page.element('audio-player').play = async () => { played++; };
  const evidence = page.app.summaryEvidenceOptions;
  const entry = {currentSegmentIds: [doc.segments[0].id], segment: {start: 23}};
  assert.equal(await evidence.onOpen(), true);
  assert.equal(await evidence.onNavigate(entry, {currentVersion: doc.version}, evidence.getContext()), true);
  assert.equal(page.app.detail.active(), 'transcript');
  assert.equal(scrolled && focused, true);
  assert.equal(page.element('detail-inspector').hidden, false);
  assert.equal(page.element('audio-player').currentTime, 7);
  assert.equal(played, 0);
  await page.app.detail.select('summary');
  assert.equal(page.app.detail.active(), 'summary');
  assert.equal(page.element('detail-inspector').hidden, true);
});

test('frozen evidence cannot navigate missing IDs, changed versions or unsaved edits', async () => {
  const doc = transcriptDocument();
  const page = await openApp(doc);
  page.app.state.summary = { document: {id: 'summary-a', text: '결정 [1]', sourceVersion: doc.version} };
  const evidence = page.app.summaryEvidenceOptions, context = evidence.getContext();
  const entry = {currentSegmentIds: [doc.segments[0].id], segment: {start: 23}};
  for (const response of [{currentVersion: {...doc.version, revision: 8}}, {currentVersion: null}]) {
    assert.equal(await evidence.onNavigate(entry, response, context), false);
  }
  assert.equal(await evidence.onNavigate({...entry, currentSegmentIds: ['absent']}, {currentVersion: doc.version}, context), false);
  page.app.state.transcriptDraft.dirty = true;
  assert.equal(await evidence.onNavigate(entry, {currentVersion: doc.version}, context), false);
  page.app.state.transcriptDraft.dirty = false;
  await page.app.selectMeeting('b');
  assert.equal(await evidence.onNavigate(entry, {currentVersion: doc.version}, context), false);
});

test('summary audio requires explicit playback and the same source recording session', async () => {
  const page = await openApp(transcriptDocument());
  page.app.state.summary = {document: {id: 'summary-a'}};
  page.app.state.snapshot.meetings[0].audio = {sessionId: 'session-a'};
  const evidence = page.app.summaryEvidenceOptions, context = evidence.getContext();
  const player = page.element('audio-player'); player.setAttribute('src', 'audio.wav');
  player.currentTime = 7; let played = 0; player.play = async () => { played++; };
  evidence.onPlay({segment: {start: 23}}, {audioAvailable: true, sourceSessionId: 'other'}, context);
  assert.equal(played, 0);
  evidence.onPlay({segment: {start: 23}}, {audioAvailable: true, sourceSessionId: 'session-a'}, context);
  assert.equal(played, 1); assert.equal(player.currentTime, 23);
});


test('failed summary separates job retry from current-content generation and keeps diagnostics folded', async () => {
  const page = await openApp(transcriptDocument());
  const job = { id: 'summary-failed', kind: 'summary', status: 'failed', meetingId: 'a', revision: 10, supported: true, modelReady: true, summarySupported: true, summaryReady: true, retryable: true, failureKind: 'outputLimit', hasPreviousSummary: false, failureDetail: { stage: 'merge', finishReason: 'length', outputLimit: 2048 } };
  await page.app.acceptProcessing(job);
  assert.equal(page.element('summarize').textContent, '현재 내용 요약');
  assert.equal(page.element('retry-processing').hidden, false);
  assert.equal(page.element('summarize').disabled, false);
  assert.equal(page.element('summary-message').hidden, true);
  assert.equal(page.element('processing-error-info').hidden, false);
  assert.equal(page.element('processing-error-info').open, false);
  assert.match(page.element('processing-error-fields').innerHTML, /요약 통합.*출력 한도 도달.*2048/);
  assert.doesNotMatch(page.element('processing-message').textContent, /기존 요약/);
  await page.app.selectMeeting('b');
  assert.equal(page.element('summarize').textContent, '요약하기');
  await page.app.selectMeeting('a');
  const calls = [];
  page.bridge.retryProcessing = async id => { calls.push(id); return { ...job, id: 'retry', revision: 11, status: 'running', phase: 'preparing', failureDetail: null }; };
  await page.app.retryProcessing(job.id);
  assert.deepEqual(calls, ['summary-failed']);
  assert.equal(page.element('processing-title').textContent, '요약 준비 중');
  assert.equal(page.element('processing-error-info').hidden, true);
  assert.equal(page.element('summary-message').hidden, true);
});


test('summary retry and failure retain the existing summary stale-source notice', async () => {
  const page = await openApp(transcriptDocument());
  page.app.state.summary = { document: { id: 'previous-summary', text: '이전 요약 내용', sourceVersion: structuredClone(page.app.state.transcriptDraft.doc.version) }, stale: true };
  const job = { id: 'summary-stale', kind: 'summary', meetingId: 'a', supported: true, modelReady: true, summarySupported: true, summaryReady: true, hasPreviousSummary: true };
  for (const [revision, status] of [[10, 'running'], [11, 'failed']]) {
    await page.app.acceptProcessing({ ...job, revision, status, phase: 'summarizing', failureKind: status === 'failed' ? 'outputLimit' : null });
    assert.equal(page.element('summary-message').hidden, false);
    assert.match(page.element('summary-message').textContent, /이전 내용으로 만든 요약/);
    assert.match(page.element('summary-content').innerHTML, /이전 요약 내용/);
    assert.doesNotMatch(page.element('summary-content').innerHTML, /data-summary-citations/);
  }
});

test('meeting switching captures all scroll positions before replacing the shared document DOM', async () => {
  const page = await openApp(transcriptDocument());
  page.element('summary-section').scrollTop = 410;
  await page.app.detail.select('transcript'); page.element('transcript-scroll').scrollTop = 1200;
  await page.app.detail.select('notes'); page.element('meeting-notes').scrollTop = 320;
  await page.app.selectMeeting('b'); await tick();
  assert.equal(page.element('meeting-notes').scrollTop, 0);
  await page.app.selectMeeting('a'); await tick();
  assert.equal(page.app.detail.active(), 'notes');
  assert.equal(page.element('meeting-notes').scrollTop, 320);
  assert.equal(page.element('summary-section').scrollTop, 410);
  await page.app.detail.select('transcript'); assert.equal(page.element('transcript-scroll').scrollTop, 1200);
});
test('a frozen utterance mapped into multiple speaker rows highlights every verified fragment and scrolls only to the first', async () => {
  const doc = transcriptDocument();
  doc.segments = [0,1,2].map(index => ({id: `fragment-${index}`, start:index, end:index+1, text:`발언 ${index}`, originalText:`발언 ${index}`}));
  const page = await openApp(doc);
  page.app.state.summary = { document: {id:'summary-a', text:'결정 [1]', sourceVersion:doc.version} };
  const rows = doc.segments.map(() => ({ marked: false, scrolled: false, focused: false, setAttribute() { this.marked = true; }, scrollIntoView() { this.scrolled = true; }, focus() { this.focused = true; } }));
  page.element('transcript-rows').querySelector = selector => rows[Number(selector.match(/"(\d+)"/)[1])];
  const evidence = page.app.summaryEvidenceOptions; await evidence.onOpen();
  assert.equal(await evidence.onNavigate({currentSegmentIds:['fragment-0','fragment-2']}, {currentVersion:doc.version}, evidence.getContext()), true);
  assert.deepEqual(rows.map(row=>row.marked), [true,false,true]);
  assert.deepEqual(rows.map(row=>row.scrolled), [true,false,false]);
  assert.deepEqual(rows.map(row=>row.focused), [true,false,false]);
});

test('transcript copy and TXT/Markdown export include the whole saved document during search and multi-row citation navigation', async () => {
  for (const [format, copy] of [['txt',true],['txt',false],['md',false]]) {
    const doc=transcriptDocument();
    doc.segments=[0,1,2].map(index=>({id:`row-${index}`,start:index,end:index+1,text:`전체 발언 ${index}`,originalText:`전체 발언 ${index}`}));
    const page=await openApp(doc), exported=[];
    page.bridge.exportText=async(name,text)=>{exported.push({name,text});};
    await page.app.documentSearch.find('발언 0',{view:'transcript'});
    page.app.state.summary={document:{id:'summary-a',text:'결정 [1]',sourceVersion:doc.version}};
    const marked=[], query=page.element('transcript-rows').querySelector.bind(page.element('transcript-rows'));
    page.element('transcript-rows').querySelector=selector=>selector.startsWith('[data-transcript-row=') ? {setAttribute(){marked.push(selector);},scrollIntoView(){},focus(){}} : query(selector);
    const evidence=page.app.summaryEvidenceOptions; await evidence.onOpen();
    assert.equal(await evidence.onNavigate({currentSegmentIds:['row-0','row-2']},{currentVersion:doc.version},evidence.getContext()),true);
    assert.equal(marked.length,2); assert.equal(page.app.documentSearch.query(),'발언 0');
    page.editTranscript('저장할 수정 발언');
    const exporting=page.app.exportDocument('transcript',format,copy); await tick();
    assert.equal(page.copied.length,0); assert.equal(exported.length,0);
    page.transcriptUpdates[0].pending.resolve(); await exporting;
    const text=copy?page.copied[0]:exported[0].text;
    assert.match(text,/저장할 수정 발언/); assert.match(text,/전체 발언 1/); assert.match(text,/전체 발언 2/); assert.doesNotMatch(text,/전체 발언 0/);
    assert.equal(page.app.state.transcriptDraft.dirty,false);
    if(!copy) assert.ok(exported[0].name.endsWith(`.${format}`));
  }
});


test('agent writes flush pending edits and reconcile focused metadata before unlocking', async () => {
  const page = await openApp();
  page.element('meeting-title').focus();
  page.edit('저장 전 메모');
  const ack = deferred();
  const calls = [];
  page.bridge.agentWriteReady = async (requestId, error) => { calls.push({ requestId, error }); await ack.promise; };
  const request = page.app.onAgentWriteRequest({ requestId: '11111111-1111-4111-8111-111111111111' });
  await tick();
  assert.equal(calls.length, 0);
  assert.equal(page.element('meeting-title').readOnly, true);
  page.updates[0].pending.resolve();
  await tick();
  assert.equal(calls.length, 1);
  assert.equal(page.element('meeting-notes').readOnly, true);
  page.bridge.readSnapshot = async () => ({ revision: 100, session: null, meetings: [{ ...page.saved(), title: '에이전트 제목', notes: '에이전트 메모' }] });
  ack.resolve();
  await request;
  assert.equal(page.element('meeting-title').value, '에이전트 제목');
  assert.equal(page.element('meeting-notes').value, '에이전트 메모');
  assert.equal(page.app.state.draft.savedNotes, '에이전트 메모');
  assert.equal(page.element('meeting-notes').readOnly, false);
  page.edit('후속 수정');
  assert.equal(page.app.state.draft.title, '에이전트 제목');
});

test('agent request rejects failed draft saves without approving mutation', async () => {
  const page = await openApp();
  page.edit('보존할 메모');
  const calls = [];
  page.bridge.agentWriteReady = async (requestId, error) => { calls.push({ requestId, error }); };
  const request = page.app.onAgentWriteRequest({ requestId: '11111111-1111-4111-8111-111111111111' });
  await tick();
  page.updates[0].pending.reject(new Error('저장 실패'));
  await request;
  assert.equal(calls.length, 1);
  assert.equal(typeof calls[0].error, 'string');
  assert.equal(page.app.state.draft.dirty, true);
  assert.equal(page.app.state.draft.notes, '보존할 메모');
  assert.equal(page.element('meeting-notes').readOnly, false);
});


test('agent summary writes reload the open summary before releasing edit ownership', async () => {
  const page = await openApp(transcriptDocument());
  const ack = deferred();
  const loaded = deferred();
  page.app.state.summary = { document: { id: 'old', text: '이전 요약', sourceVersion: page.app.state.transcriptDraft.doc.version }, stale: false };
  page.bridge.agentWriteReady = async () => { await ack.promise; };
  page.bridge.readSnapshot = async () => ({ revision: 100, session: null, meetings: [page.saved()] });
  page.bridge.readSummary = async id => { assert.equal(id, 'a'); return loaded.promise; };
  const request = page.app.onAgentWriteRequest({ requestId: '11111111-1111-4111-8111-111111111111' });
  await tick(); ack.resolve(); await tick();
  assert.equal(page.element('meeting-notes').readOnly, true);
  loaded.resolve({ document: { id: 'new', text: '에이전트가 저장한 요약', sourceVersion: page.app.state.transcriptDraft.doc.version }, stale: false });
  await request;
  assert.equal(page.app.state.summary.document.id, 'new');
  assert.equal(page.app.state.summary.document.text, '에이전트가 저장한 요약');
  assert.equal(page.element('meeting-notes').readOnly, false);
});


test('a fresh meeting keeps summary hidden through inspector navigation until text is available', async () => {
  const page = await openApp();
  assert.equal(page.element('summary-section').hidden, true);
  await page.app.detail.select('notes');
  assert.equal(page.element('summary-section').hidden, true);
  await page.app.detail.select('summary');
  assert.equal(page.element('summary-section').hidden, true);
  page.app.state.transcriptDraft = { id:'a', doc:transcriptDocument(), dirty:false, savedTexts:[] };
  page.app.render();
  assert.equal(page.element('summary-section').hidden, false);
});


test('quality playback waits for the existing audio request and rejects a replaced source', async () => {
  const page = await openApp();
  const pending = deferred(); let reads = 0;
  page.bridge.audioUrl = async () => { reads++; return pending.promise; };
  const meeting = page.app.state.snapshot.meetings[0];
  meeting.audio = { relativePath:'sessions/a/audio.wav',frames:32000,sampleRate:16000 };
  const first = page.app.prepareAudio(meeting, false);
  let ready = false;
  const second = page.app.prepareAudio(meeting, false).then(() => { ready = true; });
  await tick(); assert.equal(reads,1); assert.equal(ready,false);
  pending.resolve('asset://audio-a'); await Promise.all([first,second]);
  assert.equal(page.element('audio-player').src,'asset://audio-a');
  assert.equal(ready,true);
});

test('summary download failures remain visible inside General settings', async () => {
  const page = await openApp(null, { revision:1,status:'idle',supported:true,modelReady:true,summarySupported:true,summaryReady:false });
  await page.app.openSettings();
  await page.app.acceptProcessing({ revision:2,id:'download',kind:'summary-model',status:'failed',message:'저장 공간이 부족합니다.',supported:true,modelReady:true,summarySupported:true,summaryReady:false });
  assert.equal(page.element('settings-summary-error').hidden,false);
  assert.match(page.element('settings-summary-error').textContent,/공간이 부족/);
  assert.equal(page.element('settings-dialog').open,true);
});

test('explicit library navigation remains available without prepared models', async () => {
  const page=await openApp();
  page.app.state.connected=true;page.app.state.selectedId=null;page.app.state.libraryOpen=true;
  page.app.state.processing.supported=false;page.app.state.processing.modelReady=false;
  page.app.render();
  assert.equal(page.element('setup-screen').hidden,true);
  assert.equal(page.element('library-screen').hidden,false);
});
