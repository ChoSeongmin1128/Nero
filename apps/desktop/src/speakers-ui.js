import { bindDialogDismiss } from './dialog-dismiss.js';
import { pendingJob } from './processing-state.js';
import { escape, duration, icon } from './ui.js';

const newSpeakerValue = '__new__';
const sameVersion = (a, b) => a?.id === b?.id && a?.revision === b?.revision;
const copySpeakers = speakers => (speakers || []).map(speaker => ({ id: speaker.id, name: speaker.name }));
const assignmentsOf = doc => doc.segments.map(segment => segment.speakerId ?? null);
const sameState = (a, b) => JSON.stringify([a.speakers, a.assignments]) === JSON.stringify([b.speakers, b.assignments]);

function speakerLabel(doc, segment) {
  return doc?.speakers?.find(speaker => speaker.id === segment?.speakerId)?.name || '화자 미지정';
}

function copyWorking(doc) {
  return { version: { ...doc.version }, speakers: copySpeakers(doc.speakers), assignments: assignmentsOf(doc), segments: doc.segments.map(segment => ({ id: segment.id, start: segment.start, end: segment.end, text: segment.text })) };
}

function validateNames(speakers) {
  if (speakers.some(speaker => !speaker.name.trim() || new TextEncoder().encode(speaker.name).length > 256 || /[\u0000-\u001f\u007f-\u009f]/.test(speaker.name))) throw new Error('화자 이름이 비어 있거나 너무 깁니다. 짧은 이름으로 입력해 주세요.');
}

function nextSpeakerName(speakers) {
  let number = 1;
  const names = new Set(speakers.map(speaker => speaker.name));
  while (names.has(`화자 ${number}`)) number += 1;
  return `화자 ${number}`;
}

function assignSelected(working, selected, target, name, makeId) {
  if (!selected.size) throw new Error('발언을 선택해 주세요.');
  const validIds = new Set(working.segments.map(segment => segment.id));
  if ([...selected].some(id => !validIds.has(id))) throw new Error('음성 기록이 변경되었습니다. 화자 편집을 다시 열어 주세요.');
  const speakers = copySpeakers(working.speakers);
  let id = target || null;
  if (target === newSpeakerValue) {
    const speaker = { id: `manual-${makeId()}`, name: name.trim() };
    validateNames([speaker]);
    speakers.push(speaker);
    id = speaker.id;
  } else if (id && !speakers.some(speaker => speaker.id === id)) throw new Error('화자를 다시 선택해 주세요.');
  return { ...working, speakers, assignments: working.assignments.map((assignment, index) => selected.has(working.segments[index].id) ? id : assignment) };
}

function mergeSpeakers(working, from, into) {
  if (from === into || !working.speakers.some(speaker => speaker.id === from) || !working.speakers.some(speaker => speaker.id === into)) throw new Error('합칠 화자를 다시 선택해 주세요.');
  return { ...working, speakers: working.speakers.filter(speaker => speaker.id !== from).map(speaker => ({ ...speaker })), assignments: working.assignments.map(id => id === from ? into : id) };
}

export function mountSpeakers({ bridge, getState, finishEditing, onUpdated, onProcessing = async () => {}, showError, byId, onPlay, onPause, isPlaying = () => false, canPlay = () => true }) {
  const dialog = byId('speaker-dialog');
  const body = byId('speaker-dialog-body');
  const trigger = byId('speakers-button');
  const undoByMeeting = new Map();
  let editing = null;
  let opening = false;
  let saving = false;
  let preparing = false;
  let composing = false;
  let operation = 0;
  let returnFocus = null;

  function current() {
    const state = getState();
    const draft = state.transcriptDraft;
    const meeting = state.snapshot?.meetings?.find(item => item.id === state.selectedId);
    if (draft?.id !== state.selectedId || !draft?.doc?.segments?.length || meeting?.deletedAt != null) return null;
    return { state, id: state.selectedId, doc: draft.doc };
  }

  const blocked = view => !view || !view.state.connected || view.state.busy || Boolean(pendingJob(view.state.processing, { meetingId: view.id }));
  const unchanged = () => editing && sameState(editing.base, editing.working) && (editing.mode !== 'single' || editing.assignmentTarget === (editing.base.assignments[editing.singleIndex] || ''));
  const nameId = id => `speaker-name-${encodeURIComponent(id)}`;
  const mergeId = id => `speaker-merge-${encodeURIComponent(id)}`;

  function analysisBlocked(view) {
    const session = view?.state.snapshot?.session;
    const recording = session?.meetingId === view?.id && ['recording', 'paused', 'finalizing'].includes(session.state);
    return blocked(view) || Boolean(pendingJob(view?.state.processing, { kind: 'speaker-model' })) || recording || !unchanged() || !view?.state.processing?.speakerSupported;
  }

  function undoFor(view) {
    const record = view && undoByMeeting.get(view.id);
    return record && sameVersion(record.expected, view.doc.version) ? record : null;
  }

  function message(text, error = false) {
    const node = byId('speaker-editor-message');
    if (!node) return;
    node.textContent = text;
    node.hidden = !text;
    node.dataset.state = error ? 'error' : 'success';
  }

  function focusReturn() {
    if (returnFocus?.isConnected) returnFocus.focus({ preventScroll: true });
    else if (returnFocus?.dataset?.speakerEdit != null) byId('transcript-rows')?.querySelector(`[data-speaker-edit="${Number(returnFocus.dataset.speakerEdit)}"]`)?.focus({ preventScroll: true });
    else if (!trigger.hidden) trigger.focus({ preventScroll: true });
  }

  function focusField(id) {
    const input = byId(id);
    const node = input?.closest?.('.nero-select')?.querySelector('button') || input;
    if (node && !node.disabled) node.focus({ preventScroll: true });
    return node;
  }

  function stopListening() {
    if (!editing?.listeningId) return;
    editing.listeningId = null;
    try { onPause?.(); } catch (error) { showError(error); }
  }

  function syncPlayback() {
    if (!editing || !dialog.open) return;
    const view = current();
    const unavailable = blocked(view) || view.id !== editing.id || !sameVersion(view.doc.version, editing.base.version);
    body.querySelectorAll('[data-speaker-play]').forEach(button => {
      const playing = editing.listeningId === button.dataset.speakerPlay && isPlaying();
      button.innerHTML = `${icon(playing ? 'pause' : 'play')}<span>${playing ? '일시정지' : '다시 듣기'}</span>`;
      button.setAttribute('aria-pressed', String(playing));
      button.disabled = saving || unavailable || !onPlay || !canPlay();
    });
  }

  function close() {
    globalThis.NeroControls?.close();
    stopListening();
    editing = null;
    composing = false;
    if (dialog.open) dialog.close();
    focusReturn();
  }

  function controls() {
    if (!editing || !dialog.open) return;
    const view = current();
    const stale = !view || view.id !== editing.id || !sameVersion(view.doc.version, editing.base.version);
    body.querySelectorAll('button, input, select').forEach(node => { node.disabled = saving || ((stale || blocked(view)) && !node.hasAttribute('data-speaker-close')); });
    for (const id of ['speaker-analyze', 'speaker-analysis-confirm']) {
      const action = byId(id);
      if (action) action.disabled = saving || stale || analysisBlocked(view);
    }
    const analysisNote = byId('speaker-analysis-note');
    if (analysisNote) analysisNote.hidden = unchanged();
    const save = byId('speaker-save');
    save.disabled = saving || stale || blocked(view) || unchanged();
    save.innerHTML = saving ? '<span class="activity-spinner" aria-hidden="true"></span>저장 중' : '저장';
    const apply = byId('speaker-assign');
    const changesSelection = editing.assignmentTarget === newSpeakerValue || editing.working.assignments.some((id, index) => editing.selected.has(editing.working.segments[index].id) && (id || '') !== editing.assignmentTarget);
    if (apply) apply.disabled = saving || stale || blocked(view) || !editing.selected.size || !changesSelection;
    const undo = byId('speaker-undo');
    undo.hidden = !undoFor(view);
    undo.disabled = saving || stale || blocked(view) || !unchanged();
    const merge = byId('speaker-merge-apply');
    if (merge) merge.disabled = saving || stale || blocked(view) || !byId('speaker-merge-target')?.value;
    if (stale) message('음성 기록이 변경되었습니다. 수정을 취소하고 화자 편집을 다시 열어 주세요.', true);
    globalThis.NeroControls?.enhanceAll(body);
    body.querySelectorAll('select').forEach(input => globalThis.NeroControls?.refresh(input));
    syncPlayback();
  }

  function viewPosition() {
    const active = document.activeElement;
    return { scroll: byId('speaker-editor-scroll')?.scrollTop || 0, focus: active?.id || active?.dataset?.field, start: active?.selectionStart, end: active?.selectionEnd };
  }

  function draw({ focus, scroll, restore = true } = {}) {
    const position = viewPosition();
    const { working, selected, mergingId, mode } = editing;
    body.dataset.speakerMode = mode;
    const countFor = id => working.assignments.filter(assignment => assignment === id).length;
    const options = working.speakers.map(speaker => `<option value="${escape(speaker.id)}"${speaker.id === editing.assignmentTarget ? ' selected' : ''}>${escape(speaker.name)}</option>`).join('');
    const single = mode === 'single';
    const segment = single ? working.segments[editing.singleIndex] : null;
    const mergePanel = source => `<div class="speaker-merge-panel"><label for="speaker-merge-target">${escape(source.name)}의 발언 ${countFor(source.id)}개를 합칠 화자</label><div class="speaker-assignment-controls"><select id="speaker-merge-target" data-field="speaker-merge-target"><option value="">화자 선택</option>${working.speakers.filter(speaker => speaker.id !== source.id).map(speaker => `<option value="${escape(speaker.id)}">${escape(speaker.name)}</option>`).join('')}</select><button type="button" id="speaker-merge-apply">합치기</button><button type="button" class="quiet" id="speaker-merge-cancel" data-speaker-merge-cancel>취소</button></div></div>`;
    const names = mode === 'names' ? `${working.speakers.some(speaker => speaker.id.startsWith('moss:chunk:')) ? '<p class="speaker-chunk-note muted type-caption">현재 화자는 구간별로 구분되어 같은 사람이 나뉘어 있을 수 있습니다.</p>' : ''}<div class="speaker-name-list">${working.speakers.map((speaker, index) => `<div class="speaker-name-item"><div class="speaker-name-row"><label for="${escape(nameId(speaker.id))}" class="sr-only">화자 ${index + 1} 이름</label><input class="text-input" id="${escape(nameId(speaker.id))}" data-speaker-name="${escape(speaker.id)}" value="${escape(speaker.name)}" maxlength="256" autocomplete="off"><button type="button" class="quiet small" data-speaker-pick="${escape(speaker.id)}">발언 ${countFor(speaker.id)}개</button>${working.speakers.length > 1 ? `<button type="button" class="quiet small" id="${escape(mergeId(speaker.id))}" data-speaker-merge="${escape(speaker.id)}" aria-label="${escape(speaker.name)} 다른 화자와 합치기" aria-expanded="${speaker.id === mergingId}">합치기</button>` : ''}</div>${speaker.id === mergingId ? mergePanel(speaker) : ''}</div>`).join('') || '<p class="muted type-caption">발언별 변경에서 화자를 지정하세요.</p>'}</div>` : '';
    const utterances = mode === 'utterances' ? `<div class="speaker-segment-list">${working.segments.map((item, index) => `<label class="speaker-segment"><input type="checkbox" id="speaker-segment-${escape(encodeURIComponent(item.id))}" data-speaker-segment="${escape(item.id)}"${selected.has(item.id) ? ' checked' : ''}><span class="speaker-segment-time">${escape(duration(item.start * 1000))}</span><span class="speaker-segment-content"><span class="speaker-segment-name" data-speaker-current="${escape(working.assignments[index] || '')}">${escape(working.speakers.find(speaker => speaker.id === working.assignments[index])?.name || '화자 미지정')}</span><span class="speaker-segment-text">${escape(item.text || '(내용 없음)')}</span></span></label>`).join('')}</div>` : '';
    const singleContext = segment ? `<section class="speaker-current-utterance" aria-labelledby="speaker-current-title"><div class="speaker-utterance-meta"><h3 id="speaker-current-title">${editing.singleIndex + 1}번 발언</h3><span class="speaker-segment-time">${escape(duration(segment.start * 1000))}</span><button type="button" id="speaker-listen" class="quiet small" data-speaker-play="${escape(segment.id)}" aria-pressed="false">${icon('play')}<span>다시 듣기</span></button></div><p class="speaker-segment-name">현재 ${escape(working.speakers.find(speaker => speaker.id === working.assignments[editing.singleIndex])?.name || '화자 미지정')}</p><p class="speaker-utterance-text">${escape(segment.text || '(내용 없음)')}</p></section>` : '';
    const view = current();
    const analysisReady = view?.state.processing?.speakerReady === true;
    const analysis = single || !view?.state.processing?.speakerSupported ? '' : `<section class="speaker-assignment"><div class="speaker-assignment-controls"><button type="button" id="speaker-analyze" class="small">${view.doc.speakerAnalysis?.scope === 'recording' ? '다시 구분' : '화자 구분'}</button><span id="speaker-analysis-note" class="muted type-caption" hidden>화자 수정을 저장한 뒤 실행해 주세요.</span></div>${editing.analysisConfirm ? `<p class="type-caption">${analysisReady ? '녹음 전체를 분석해 화자를 다시 구분합니다. 현재 화자 이름과 발언 배정이 새 결과로 바뀝니다. 음성 기록의 내용은 유지됩니다.' : '화자 구분 모델을 다운로드합니다. 준비가 끝나면 화자 구분을 실행할 수 있습니다.'}</p><div class="speaker-assignment-controls"><button type="button" id="speaker-analysis-confirm" class="primary small">${analysisReady ? '구분하기' : '준비하기'}</button><button type="button" id="speaker-analysis-cancel" class="quiet small">취소</button></div>` : ''}</section>`;
    const assignment = mode === 'names' ? '' : `<section class="speaker-assignment" aria-labelledby="speaker-assignment-title"><div class="speaker-selection-header"><h3 id="speaker-assignment-title">${single ? '변경할 화자' : '선택한 발언 <span id="speaker-selected-count">' + selected.size + '개</span>'}</h3>${single ? '' : '<button type="button" class="quiet small" data-speaker-select-all>전체 선택</button><button type="button" class="quiet small" data-speaker-clear>선택 해제</button>'}</div><div class="speaker-assignment-controls"><label for="speaker-assignment-target" class="sr-only">${single ? '이 발언의 화자' : '선택한 발언의 화자'}</label><select id="speaker-assignment-target" data-field="speaker-assignment-target"><option value=""${editing.assignmentTarget === '' ? ' selected' : ''}>화자 미지정</option>${options}<option value="${newSpeakerValue}"${editing.assignmentTarget === newSpeakerValue ? ' selected' : ''}>새 화자</option></select><label for="speaker-new-name" class="sr-only">새 화자 이름</label><input class="text-input" id="speaker-new-name" placeholder="새 화자 이름" value="${escape(editing.newName)}" maxlength="256" autocomplete="off"${editing.assignmentTarget === newSpeakerValue ? '' : ' hidden'}>${single ? '' : '<button type="button" id="speaker-assign">선택 발언에 적용</button>'}</div></section>`;
    globalThis.NeroControls?.close();
    body.innerHTML = `<header class="speaker-dialog-header"><h2 id="speaker-dialog-title">${single ? '발언 화자 변경' : '화자 관리'}</h2><button type="button" id="speaker-close" class="icon-button quiet" data-speaker-close aria-label="닫기">${icon('x')}</button></header>${single ? '' : `<nav class="speaker-editor-views" aria-label="화자 관리 보기"><button type="button" id="speaker-view-names" class="quiet small" data-speaker-view="names" aria-pressed="${mode === 'names'}">이름과 병합</button><button type="button" id="speaker-view-utterances" class="quiet small" data-speaker-view="utterances" aria-pressed="${mode === 'utterances'}">발언별 변경</button></nav>`}
      ${analysis}${mode === 'utterances' ? assignment : ''}<div id="speaker-editor-scroll" class="speaker-dialog-scroll">${single ? singleContext : mode === 'names' ? names : utterances}</div>${single ? assignment : ''}
      <p id="speaker-editor-message" class="speaker-editor-message type-caption" role="status" hidden></p><footer class="speaker-dialog-actions"><button type="button" id="speaker-undo" class="quiet small">이전 수정 되돌리기</button><button type="button" id="speaker-cancel" data-speaker-close>취소</button><button type="button" id="speaker-save" class="primary">저장</button></footer>`;
    globalThis.NeroControls?.enhanceAll(body);
    controls();
    const viewport = byId('speaker-editor-scroll');
    if (viewport) viewport.scrollTop = scroll ?? position.scroll;
    if (restore && dialog.open) {
      const focused = focusField(focus || position.focus || 'speaker-save');
      if (!focus && focused?.setSelectionRange && position.start != null) focused.setSelectionRange(position.start, position.end);
    }
  }

  function selectionChanged() {
    const count = byId('speaker-selected-count');
    if (count) count.textContent = `${editing.selected.size}개`;
    body.querySelectorAll('[data-speaker-segment]').forEach(input => { input.checked = editing.selected.has(input.dataset.speakerSegment); });
    controls();
  }

  async function open(index = null) {
    if (opening || saving || preparing) return;
    const view = current();
    if (blocked(view) || view.state.editLocks) return;
    const meetingId = view.id;
    const segmentId = index == null ? null : view.doc.segments[index]?.id;
    if (index != null && !segmentId) return;
    opening = true;
    const request = ++operation;
    returnFocus = document.activeElement;
    try {
      await finishEditing(() => {
        const latest = current();
        if (request !== operation || blocked(latest) || latest.id !== meetingId) return;
        if (segmentId && !latest.doc.segments.some(segment => segment.id === segmentId)) throw new Error('음성 기록이 변경되었습니다. 발언을 다시 선택해 주세요.');
        const base = copyWorking(latest.doc);
        const singleIndex = segmentId ? latest.doc.segments.findIndex(segment => segment.id === segmentId) : null;
        editing = { id: meetingId, base, working: copyWorking(latest.doc), mode: segmentId ? 'single' : 'names', singleIndex, selected: new Set(segmentId ? [segmentId] : []), mergingId: null, assignmentTarget: singleIndex == null ? '' : base.assignments[singleIndex] || '', newName: nextSpeakerName(base.speakers), newId: null, scrolls: {}, listeningId: null };
        draw({ restore: false, scroll: 0 });
        if (!dialog.open) dialog.showModal();
        controls();
        focusField(segmentId ? 'speaker-assignment-target' : latest.doc.speakers?.length ? nameId(latest.doc.speakers[0].id) : 'speaker-view-utterances');
      });
    } catch (error) { showError(error); }
    finally { opening = false; render(); }
  }

  async function save(undo = false) {
    if (saving || preparing || !editing) return;
    const owner = editing;
    preparing = true;
    await flushComposition();
    preparing = false;
    if (editing !== owner || (!undo && unchanged())) return;
    saving = true;
    controls();
    try {
      await finishEditing(async () => {
        const view = current();
        if (editing !== owner || blocked(view) || view.id !== owner.id || !sameVersion(view.doc.version, owner.base.version)) throw new Error('음성 기록이 변경되었습니다. 수정을 보존하고 화자 편집을 다시 열어 주세요.');
        const record = undo ? undoFor(view) : null;
        if (undo && (!record || !unchanged())) throw new Error('되돌릴 수 있는 화자 수정이 없습니다.');
        const next = undo ? record.previous : owner.mode === 'single' ? assignSelected(owner.working, owner.selected, owner.assignmentTarget, owner.newName, () => owner.newId ||= crypto.randomUUID()) : owner.working;
        validateNames(next.speakers);
        const saved = await bridge.saveSpeakers(view.id, { ...view.doc.version }, copySpeakers(next.speakers), [...next.assignments]);
        if (undo) undoByMeeting.delete(view.id);
        else undoByMeeting.set(view.id, { expected: { ...saved.version }, previous: { speakers: copySpeakers(owner.base.speakers), assignments: [...owner.base.assignments] } });
        await onUpdated(saved, view.id);
      });
      // The owning screen re-enables utterance buttons when finishEditing ends.
      // Return focus only after that lock has been released.
      if (editing === owner) close();
    } catch (error) { message(error instanceof Error ? error.message : String(error), true); showError(error); }
    finally { saving = false; render(); controls(); }
  }

  async function analyze() {
    const owner = editing;
    const view = current();
    if (!owner || !owner.analysisConfirm || analysisBlocked(view) || !sameVersion(view.doc.version, owner.base.version)) return;
    saving = true;
    controls();
    try {
      await finishEditing(async () => {
        const latest = current();
        if (editing !== owner || analysisBlocked(latest) || latest.id !== owner.id || !sameVersion(latest.doc.version, owner.base.version)) throw new Error('음성 기록이 변경되었습니다. 화자 편집을 다시 열어 주세요.');
        const progress = latest.state.processing.speakerReady
          ? await bridge.diarizeMeeting(owner.id, { ...owner.base.version })
          : await bridge.prepareSpeakerModel();
        if (editing === owner) close();
        await onProcessing(progress);
      });
    } catch (error) { message(error instanceof Error ? error.message : String(error), true); }
    finally { saving = false; render(); }
  }

  function render() {
    const view = current();
    trigger.hidden = !view;
    trigger.disabled = blocked(view) || opening || saving || Boolean(view?.state.editLocks);
    const record = view && undoByMeeting.get(view.id);
    if (record && !sameVersion(record.expected, view.doc.version)) undoByMeeting.delete(view.id);
    if (editing && (!view || view.id !== editing.id)) close();
    controls();
  }

  function reset() {
    operation += 1;
    close();
  }

  async function flushComposition() {
    if (!composing) return;
    document.activeElement?.blur();
    await new Promise(requestAnimationFrame);
    composing = false;
  }

  trigger.addEventListener('click', () => { void open(); });
  bindDialogDismiss(dialog, { dismiss: close, blocked: () => saving });
  dialog.addEventListener('close', () => { if (!saving) editing = null; });
  dialog.addEventListener('keydown', event => {
    const shortcut = (event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 's';
    const enterName = event.key === 'Enter' && event.target.dataset?.speakerName && !event.isComposing && event.keyCode !== 229;
    if (!shortcut && !enterName) return;
    event.preventDefault();
    event.stopPropagation();
    void save();
  });
  body.addEventListener('compositionstart', () => { composing = true; });
  body.addEventListener('compositionend', () => { composing = false; });
  body.addEventListener('input', event => {
    const id = event.target.dataset.speakerName;
    if (!editing || saving) return;
    if (event.target.id === 'speaker-new-name') { editing.newName = event.target.value; controls(); return; }
    if (!id) return;
    const speaker = editing.working.speakers.find(item => item.id === id);
    if (speaker) {
      speaker.name = event.target.value;
      body.querySelectorAll('option').forEach(option => { if (option.value === id) option.textContent = speaker.name; });
      body.querySelectorAll('[data-speaker-current]').forEach(node => { if (node.dataset.speakerCurrent === id) node.textContent = speaker.name || '화자 미지정'; });
      body.querySelectorAll('select').forEach(input => globalThis.NeroControls?.refresh(input));
    }
    controls();
  });
  body.addEventListener('change', event => {
    if (!editing || saving) return;
    const segmentId = event.target.dataset.speakerSegment;
    if (segmentId) {
      if (event.target.checked) editing.selected.add(segmentId); else editing.selected.delete(segmentId);
      selectionChanged();
    } else if (event.target.id === 'speaker-assignment-target') {
      editing.assignmentTarget = event.target.value;
      byId('speaker-new-name').hidden = event.target.value !== newSpeakerValue;
      if (event.target.value === newSpeakerValue) byId('speaker-new-name').focus({ preventScroll: true });
      controls();
    } else if (event.target.id === 'speaker-merge-target') controls();
  });
  body.addEventListener('click', async event => {
    const target = event.target.closest('button');
    if (!target || !editing || saving || target.disabled) return;
    if (target.hasAttribute('data-speaker-close')) { close(); return; }
    if (target.id === 'speaker-analyze') {
      const owner = editing;
      await flushComposition();
      if (editing !== owner || analysisBlocked(current())) { controls(); return; }
      editing.analysisConfirm = true;
      draw({ focus: 'speaker-analysis-confirm' });
      return;
    }
    if (target.id === 'speaker-analysis-cancel') { editing.analysisConfirm = false; draw({ focus: 'speaker-analyze' }); return; }
    if (target.id === 'speaker-analysis-confirm') { await analyze(); return; }
    if (target.id === 'speaker-save') { void save(); return; }
    if (target.id === 'speaker-undo') { void save(true); return; }
    if (target.hasAttribute('data-speaker-select-all')) { editing.selected = new Set(editing.working.segments.map(segment => segment.id)); selectionChanged(); return; }
    if (target.hasAttribute('data-speaker-clear')) { editing.selected.clear(); selectionChanged(); return; }
    try {
      const owner = editing;
      await flushComposition();
      if (editing !== owner) return;
      if (target.dataset.speakerView || target.dataset.speakerPick) {
        const previousMode = editing.mode;
        editing.scrolls[previousMode] = byId('speaker-editor-scroll')?.scrollTop || 0;
        editing.mode = target.dataset.speakerView || 'utterances';
        editing.mergingId = null;
        if (target.dataset.speakerPick) {
          editing.selected = new Set(editing.working.segments.filter((_, index) => editing.working.assignments[index] === target.dataset.speakerPick).map(segment => segment.id));
          editing.assignmentTarget = target.dataset.speakerPick;
        }
        draw({ focus: `speaker-view-${editing.mode}`, scroll: editing.scrolls[editing.mode] || 0 });
        if (target.dataset.speakerPick) byId(`speaker-segment-${encodeURIComponent([...editing.selected][0] || '')}`)?.scrollIntoView({ block: 'nearest' });
      } else if (target.dataset.speakerMerge) {
        editing.mergeReturnScroll = byId('speaker-editor-scroll')?.scrollTop || 0;
        editing.mergingId = target.dataset.speakerMerge;
        draw({ focus: 'speaker-merge-target' });
        byId('speaker-merge-target')?.closest('.speaker-merge-panel')?.scrollIntoView({ block: 'nearest' });
      } else if (target.hasAttribute('data-speaker-merge-cancel')) {
        const source = editing.mergingId;
        editing.mergingId = null;
        draw({ focus: mergeId(source), scroll: editing.mergeReturnScroll });
      } else if (target.id === 'speaker-merge-apply') {
        editing.working = mergeSpeakers(editing.working, editing.mergingId, byId('speaker-merge-target').value);
        editing.mergingId = null;
        if (!editing.working.speakers.some(speaker => speaker.id === editing.assignmentTarget)) editing.assignmentTarget = '';
        draw({ focus: 'speaker-save' });
        message('화자를 합쳤습니다. 저장하면 반영됩니다.');
      } else if (target.id === 'speaker-assign') {
        editing.working = assignSelected(editing.working, editing.selected, editing.assignmentTarget, editing.newName, () => crypto.randomUUID());
        if (editing.assignmentTarget === newSpeakerValue) {
          editing.assignmentTarget = editing.working.assignments[editing.working.segments.findIndex(segment => editing.selected.has(segment.id))];
          editing.newName = nextSpeakerName(editing.working.speakers);
        }
        draw({ focus: 'speaker-save' });
        message(`${editing.selected.size}개 발언의 화자를 변경했습니다. 저장하면 반영됩니다.`);
      } else if (target.dataset.speakerPlay) {
        if (editing.listeningId === target.dataset.speakerPlay && isPlaying()) stopListening();
        else {
          const index = editing.working.segments.findIndex(segment => segment.id === target.dataset.speakerPlay);
          const segment = editing.working.segments[index];
          if (!segment || !onPlay || !canPlay()) return;
          editing.listeningId = segment.id;
          await onPlay(segment.start, { meetingId: editing.id, segmentId: segment.id, index, endSeconds: segment.end });
        }
        syncPlayback();
      }
    } catch (error) { message(error instanceof Error ? error.message : String(error), true); }
  });

  return { render, open, reset, syncPlayback, label: segment => speakerLabel(current()?.doc, segment) };
}
