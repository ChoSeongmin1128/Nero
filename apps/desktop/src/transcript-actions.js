import { pendingJob } from './processing-state.js';
const sameVersion = (a, b) => (a == null && b == null) || (a?.id === b?.id && a?.revision === b?.revision);
const versionOf = context => context?.transcript?.version ? { ...context.transcript.version } : null;
const messageOf = error => error instanceof Error ? error.message : String(error);

export function mountTranscriptActions({ byId, bridge, getContext, finishEditing, onRebuild, onRestored, showError, escape }) {
  const dialog = byId('transcript-actions-dialog');
  const body = byId('transcript-actions-body');
  let pending = null;
  let serial = 0;
  let preparing = false;
  let returnFocus = null;

  function recording(context) {
    return context?.recording === true || ['recording', 'paused', 'finalizing'].includes(context?.recording?.state);
  }
  const missing = context => !context?.meeting || context.meeting.deletedAt != null;
  function blocked(context, kind) {
    if (missing(context)) return '회의를 찾을 수 없습니다.';
    if (recording(context)) return '녹음을 마친 뒤 다시 시도해 주세요.';
    if (pendingJob(context.processing, { meetingId: context.meeting.id })) return '진행 중인 처리가 끝난 뒤 다시 시도해 주세요.';
    return '';
  }
  function focusReturn() {
    const target = returnFocus?.isConnected ? returnFocus : byId('transcript-history');
    target?.focus?.({ preventScroll: true });
  }
  function reset() {
    serial += 1;
    pending = null;
    preparing = false;
    if (dialog.open) dialog.close();
  }
  function close() {
    if (pending?.saving) return;
    reset();
    focusReturn();
  }
  function currentOwner(owner, checkVersion = true) {
    const context = getContext();
    if (!context?.meeting || context.meeting.id !== owner.meetingId || context.meeting.deletedAt != null) throw new Error('회의가 변경되었습니다. 원하는 회의에서 다시 열어 주세요.');
    const reason = blocked(context, owner.kind);
    if (reason) throw new Error(reason);
    if (checkVersion && !sameVersion(versionOf(context), owner.expected)) throw new Error('음성 기록이 변경되었습니다. 창을 닫고 다시 열어 주세요.');
    return context;
  }
  const active = owner => pending === owner && owner.serial === serial;
  function report(owner, error) {
    if (!active(owner)) return;
    owner.error = messageOf(error);
    draw();
  }
  function dateLabel(value) {
    const date = new Date(value);
    return Number.isFinite(value) && value > 0 && Number.isFinite(date.getTime())
      ? new Intl.DateTimeFormat('ko-KR', { year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' }).format(date)
      : '저장 시각 미확인';
  }
  function mismatch(owner) {
    const current = owner.entries.find(entry => entry.current);
    return !current || !sameVersion(current.version, owner.expected);
  }
  function canConfirm(owner) {
    if (owner.loading || owner.saving || owner.loadFailed) return false;
    try { currentOwner(owner); } catch { return false; }
    return owner.kind === 'rebuild' || (!mismatch(owner) && owner.entries.some(entry => !entry.current && sameVersion(entry.version, owner.selected)));
  }
  function draw() {
    const owner = pending;
    if (!owner) return;
    const oldContent = body.querySelector('.transcript-actions-content');
    const scrollTop = oldContent?.scrollTop || 0;
    const focused = dialog.ownerDocument?.activeElement;
    const focusRole = ['close', 'reload', 'confirm'].find(role => focused?.hasAttribute?.(`data-transcript-actions-${role}`));
    const title = owner.kind === 'rebuild' ? '음성 기록을 다시 변환할까요?' : '이전 음성 기록';
    const entries = owner.entries || [];
    let content;
    if (owner.kind === 'rebuild') {
      content = '<p class="transcript-actions-description">현재 문장과 화자 수정이 새 결과로 바뀝니다. 이전 기록에서 되돌릴 수 있습니다.</p>';
    } else if (owner.loading) {
      content = '<div class="transcript-actions-loading" role="status"><span class="activity-spinner" aria-hidden="true"></span><span>이전 기록을 불러오는 중</span></div>';
    } else if (owner.loadFailed) {
      content = '<p>이전 기록을 불러오지 못했습니다.</p>';
    } else {
      content = `${entries.some(entry => !entry.current) ? '' : '<p class="transcript-actions-empty">이전 음성 기록이 없습니다.</p>'}${entries.length ? `<fieldset class="transcript-history-list"><legend class="sr-only">되돌릴 음성 기록 선택</legend>${entries.map((entry, index) => `<label class="transcript-history-choice${entry.current ? ' is-current' : ''}"><input type="radio" name="transcript-history-choice" data-transcript-history-choice="${index}"${entry.current || owner.saving ? ' disabled' : ''}${sameVersion(entry.version, owner.selected) ? ' checked' : ''}><span class="transcript-history-description"><span class="transcript-history-title"><strong>${escape(dateLabel(entry.updatedAt))}</strong>${entry.current ? '<span class="transcript-history-current">현재 사용 중</span>' : ''}</span><span class="transcript-history-meta">발언 ${Number(entry.segmentCount) || 0}개${entry.edited ? ' / 문장 또는 화자 수정됨' : ''}</span></span></label>`).join('')}</fieldset>` : ''}${owner.unavailable ? `<p class="transcript-actions-unavailable" role="status">불러올 수 없는 기록 ${Number(owner.unavailable)}개가 있습니다.</p>` : ''}${entries.length && mismatch(owner) ? '<p class="transcript-actions-unavailable">현재 음성 기록이 변경되었습니다. 창을 닫고 다시 열어 주세요.</p>' : ''}`;
    }
    const label = owner.saving ? (owner.kind === 'rebuild' ? '변환 시작 중' : '되돌리는 중') : (owner.kind === 'rebuild' ? '다시 변환' : '이 기록으로 되돌리기');
    body.innerHTML = `<form class="transcript-actions-form"><header class="transcript-actions-header"><h2 id="transcript-actions-title">${title}</h2></header><div class="transcript-actions-content" tabindex="0">${content}<p id="transcript-actions-error" class="field-error" role="alert"${owner.error ? '' : ' hidden'}>${escape(owner.error || '')}</p></div><footer class="transcript-actions-footer">${owner.kind === 'history' ? `<button type="button" class="quiet" data-transcript-actions-reload${owner.loading || owner.saving ? ' disabled' : ''}>다시 불러오기</button>` : ''}<span class="transcript-actions-footer-spacer"></span><button type="button" data-transcript-actions-close${owner.saving ? ' disabled' : ''}>취소</button><button type="submit" class="primary" data-transcript-actions-confirm${canConfirm(owner) ? '' : ' disabled'}>${owner.saving ? '<span class="activity-spinner" aria-hidden="true"></span>' : ''}<span>${label}</span></button></footer></form>`;
    dialog.setAttribute('aria-labelledby', 'transcript-actions-title');
    dialog.setAttribute('aria-busy', String(owner.loading || owner.saving));
    const scrollRegion = body.querySelector('.transcript-actions-content');
    if (scrollRegion) scrollRegion.scrollTop = scrollTop;
    if (focusRole) {
      const target = body.querySelector(`[data-transcript-actions-${focusRole}]`);
      (target && !target.disabled ? target : body.querySelector('[data-transcript-actions-close]'))?.focus?.({ preventScroll: true });
    }
  }
  function show(owner) {
    pending = owner;
    draw();
    if (!dialog.open) dialog.showModal();
    body.querySelector('[data-transcript-actions-close]')?.focus?.({ preventScroll: true });
  }
  async function prepare(kind) {
    if (preparing || pending?.saving) return null;
    const initial = getContext();
    const reason = kind === 'history' ? (missing(initial) ? '회의를 찾을 수 없습니다.' : '') : blocked(initial, kind);
    if (reason) { showError(reason); return null; }
    const meetingId = initial.meeting.id;
    const request = ++serial;
    preparing = true;
    returnFocus = dialog.ownerDocument?.activeElement;
    try {
      const context = await finishEditing(() => getContext());
      if (request !== serial) return null;
      if (context?.meeting?.id !== meetingId) throw new Error('회의가 변경되었습니다. 원하는 회의에서 다시 열어 주세요.');
      const blockedReason = kind === 'history' ? (missing(context) ? '회의를 찾을 수 없습니다.' : '') : blocked(context, kind);
      if (blockedReason) throw new Error(blockedReason);
      return { kind, meetingId, expected: versionOf(context), serial: request, entries: [], selected: null, unavailable: 0, loading: false, loadFailed: false, saving: false, error: '' };
    } catch (error) {
      if (request === serial) showError(error);
      return null;
    } finally { if (request === serial) preparing = false; }
  }
  async function rebuild() {
    const owner = await prepare('rebuild');
    if (!owner) return;
    if (owner.expected) { show(owner); return; }
    preparing = true;
    try {
      if (owner.serial !== serial) return;
      currentOwner(owner);
      await onRebuild(owner.meetingId);
    } catch (error) { showError(error); }
    finally { if (owner.serial === serial) preparing = false; }
  }
  async function load(owner, keepError = false) {
    if (!active(owner) || owner.saving || owner.loading) return;
    const context = getContext();
    if (missing(context) || context.meeting.id !== owner.meetingId) { reset(); return; }
    owner.loading = true;
    owner.loadFailed = false;
    owner.selected = null;
    if (!keepError) owner.error = '';
    draw();
    try {
      const result = await bridge.readTranscriptHistory(owner.meetingId);
      if (!active(owner)) return;
      if (getContext()?.meeting?.id !== owner.meetingId) { reset(); return; }
      owner.entries = Array.isArray(result?.entries) ? result.entries : [];
      owner.unavailable = Number(result?.unavailable) || 0;
    } catch (error) { if (active(owner)) { owner.error = messageOf(error); owner.loadFailed = true; } }
    finally {
      if (active(owner)) {
        owner.loading = false;
        draw();
        if (owner.error) body.querySelector('[data-transcript-actions-reload]')?.focus?.({ preventScroll: true });
      }
    }
  }
  async function openHistory() {
    const owner = await prepare('history');
    if (!owner) return;
    show(owner);
    await load(owner);
  }
  async function confirm() {
    const owner = pending;
    if (!owner || owner.loading || owner.saving) return;
    if (!canConfirm(owner)) {
      try { currentOwner(owner); }
      catch (error) { report(owner, error); }
      return;
    }
    const target = owner.selected ? { ...owner.selected } : null;
    owner.saving = true;
    draw();
    try {
      await finishEditing(async () => {
        if (!active(owner)) return;
        currentOwner(owner);
        if (owner.kind === 'rebuild') await onRebuild(owner.meetingId);
        else {
          if (!owner.entries.some(entry => !entry.current && sameVersion(entry.version, target)) || mismatch(owner)) throw new Error('선택한 이전 기록이 변경되었습니다. 목록을 다시 불러와 주세요.');
          const restored = await bridge.restoreTranscript(owner.meetingId, { ...owner.expected }, target);
          await onRestored(restored, owner.meetingId);
        }
      });
      if (active(owner)) { owner.saving = false; close(); }
    } catch (error) {
      if (!active(owner)) { showError(error); return; }
      owner.saving = false;
      owner.error = messageOf(error);
      draw();
      if (owner.kind === 'history') await load(owner, true);
    } finally { if (active(owner)) { owner.saving = false; draw(); } }
  }
  dialog.addEventListener('cancel', event => { event.preventDefault(); close(); });
  dialog.addEventListener('submit', event => { event.preventDefault(); void confirm(); });
  body.addEventListener('click', event => {
    if (event.target.closest?.('[data-transcript-actions-close]')) close();
    else if (event.target.closest?.('[data-transcript-actions-reload]') && pending) void load(pending);
  });
  body.addEventListener('change', event => {
    const input = event.target;
    if (!pending || pending.saving || pending.loading || input.dataset?.transcriptHistoryChoice == null) return;
    const entry = pending.entries[Number(input.dataset.transcriptHistoryChoice)];
    if (!entry || entry.current) return;
    pending.selected = { ...entry.version };
    const confirmButton = body.querySelector('[data-transcript-actions-confirm]');
    if (confirmButton) confirmButton.disabled = !canConfirm(pending);
  });
  return { rebuild, openHistory, reset };
}
