import { processingJobs, latestMeetingProcessing, queuedBehindModel } from './processing-state.js';
import { processingFeedback, canRetryProcessing, canDismissProcessing } from './processing-feedback.js';

const activeStatuses = new Set(['queued', 'running', 'cancelling']);
const terminalStatuses = new Set(['completed', 'failed', 'cancelled', 'interrupted', 'needsReview']);
const canAcknowledge = job => ['integration', 'notice'].includes(job.category) ? job.unread : canDismissProcessing(job) || (job.status === 'completed' && job.acknowledged !== true && !job.retrySuccessorId);
const attentionStatuses = new Set(['failed', 'needsReview', 'interrupted']);
// What needs the person comes first, then work in progress, then what is done.
export function notificationGroup(item) {
  if (['integration', 'notice'].includes(item.category)) return 'attention';
  if (activeStatuses.has(item.status)) return 'active';
  return attentionStatuses.has(item.status) ? 'attention' : 'done';
}
const groupTitles = { attention: '확인 필요', active: '진행 중', done: '완료' };
export function relativeTime(value, now = Date.now()) {
  if (!Number.isFinite(value) || value <= 0) return '';
  const minutes = Math.floor((now - value) / 60000);
  if (minutes < 1) return '방금';
  if (minutes < 60) return `${minutes}분 전`;
  if (minutes < 24 * 60) return `${Math.floor(minutes / 60)}시간 전`;
  const date = new Date(value);
  return `${date.getMonth() + 1}. ${date.getDate()}.`;
}
const escape = value => String(value ?? '').replace(/[&<>"']/g, character => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[character]);

export function notificationProjection({ processing = {}, meetings = [], selectedMeetingId = null } = {}) {
  const available = new Map(meetings.filter(meeting => meeting.deletedAt == null).map(meeting => [meeting.id, meeting]));
  const jobs = processingJobs(processing);
  const latest = new Set();
  for (const meetingId of available.keys()) {
    const job = latestMeetingProcessing(processing, meetingId);
    if (job) latest.add(job.id);
  }
  for (const kind of new Set(jobs.filter(job => !job.meetingId).map(job => job.kind))) {
    const group = jobs.filter(job => !job.meetingId && job.kind === kind);
    const job = latestMeetingProcessing({ jobs: group.map(item => ({ ...item, meetingId: kind })) }, kind);
    if (job) latest.add(job.id);
  }
  return jobs.filter(job => !job.retrySuccessorId && (!job.meetingId || available.has(job.meetingId))
    && (activeStatuses.has(job.status) || (terminalStatuses.has(job.status) && latest.has(job.id) && job.acknowledged !== true)))
    .sort((a, b) => Number(activeStatuses.has(b.status)) - Number(activeStatuses.has(a.status))
      || (b.createdAt || 0) - (a.createdAt || 0) || a.id.localeCompare(b.id))
    .map(job => {
      const current = processing.id === job.id && processing.status === job.status ? { ...processing, ...job } : job;
      const waitingForModel = queuedBehindModel(processing, job);
      const feedback = processingFeedback(current, { waitingForModel });
      return { ...current, feedback, waitingForModel, title: available.get(job.meetingId)?.title || feedback.title,
        hasMeeting: available.has(job.meetingId), selected: Boolean(job.meetingId && job.meetingId === selectedMeetingId),
        active: activeStatuses.has(job.status), unread: true };
    });
}

export function mountNotificationCenter({ container, onOpenMeeting, onProcessingAction, onOpenIntegration, onUpdateIntegration, onNoticeAction, onChanged = () => {}, showError = () => {}, icon = () => '', now = () => Date.now() }) {
  const confirmedNotices = new Set();
  const pending = new Map();
  const errors = new Map();
  const notes = new Map();
  const observedActive = new Set();
  let snapshot = { processing: {}, meetings: [], connected: false };
  let renderKey = '';
  let disposed = false;
  let focusReturn = null;
  let confirming = false;
  const rows = () => [...notificationProjection(snapshot).filter(job => job.status !== 'completed' || observedActive.has(job.id)), ...(snapshot.integrations || []).map(item => ({ ...item, category: 'integration', active: false, unread: !confirmedNotices.has(item.id) })), ...(snapshot.notices || []).map(item => ({ ...item, category: 'notice', active: false, unread: !confirmedNotices.has(item.id) }))];
  const actionPending = id => pending.has(id) || snapshot.actions?.has(id);
  const canAct = job => !disposed && snapshot.connected !== false && !actionPending(job.id);

  function rememberFocus() {
    const active = container.ownerDocument.activeElement;
    if (!container.contains(active)) return null;
    return { node: active, heading: active.hasAttribute('data-notification-heading'), id: active.dataset.notificationJob, action: active.dataset.notificationAction,
      ids: rows().map(job => job.id) };
  }

  function restoreFocus(saved) {
    if (!saved || disposed || !container.isConnected || container.closest('[hidden], [inert]')) return;
    if (actionPending(saved.id)) return;
    const active = container.ownerDocument.activeElement;
    if (active && active !== container.ownerDocument.body && active !== saved.node && active.isConnected) { focusReturn = null; return; }
    if (saved.heading) { container.querySelector('[data-notification-heading]')?.focus({ preventScroll: true }); focusReturn = null; return; }
    const controls = [...container.querySelectorAll('button')].filter(button => !button.disabled && !button.hidden);
    const same = controls.filter(button => button.dataset.notificationJob === saved.id);
    const successorId = processingJobs(snapshot.processing).find(job => job.id === saved.id)?.retrySuccessorId;
    const index = saved.ids.indexOf(saved.id);
    const following = [...saved.ids.slice(index + 1), ...saved.ids.slice(0, index).reverse()];
    const target = same.find(button => button.dataset.notificationAction === saved.action) || same[0]
      || (successorId && controls.find(button => button.dataset.notificationJob === successorId))
      || following.map(id => controls.find(button => button.dataset.notificationJob === id)).find(Boolean)
      || controls.find(button => button.dataset.notificationAction === 'open')
      || container.querySelector('[data-notification-heading]');
    target?.focus({ preventScroll: true });
    focusReturn = null;
  }

  function render() {
    if (disposed) return;
    const jobs = rows();
    const saved = focusReturn = rememberFocus() || focusReturn;
    const key = JSON.stringify([jobs, [...pending], [...errors], [...snapshot.actions || []], snapshot.connected, confirming]);
    if (key === renderKey) return;
    renderKey = key;
    const active = jobs.filter(job => job.active).length;
    const unread = jobs.filter(job => notificationGroup(job) === 'attention' && job.unread).length;
    const button = (job, action, label, quiet = true, disabled = false) => `<button type="button" class="${quiet ? 'quiet ' : ''}small" data-notification-job="${escape(job.id)}" data-notification-action="${action}"${disabled || !canAct(job) ? ' disabled' : ''}>${label}</button>`;
    const expandedDiagnostics = new Set([...container.querySelectorAll('details[data-notification-diagnostics]')].filter(node => node.open).map(node => node.dataset.notificationDiagnostics));
    const listScroll = container.querySelector('.notification-list')?.scrollTop || 0;
    const glyph = (name, tone = '') => `<span class="notification-glyph"${tone ? ` data-tone="${tone}"` : ''} aria-hidden="true">${name === 'spinner' ? '<span class="activity-spinner"></span>' : icon(name)}</span>`;
    const row = job => {
      const time = job.active ? '' : relativeTime(job.updatedAt || job.createdAt, now());
      const timeMark = time ? `<span class="notification-time">${escape(time)}</span>` : '';
      if (['integration', 'notice'].includes(job.category)) return `<article class="notification-item" data-notification-id="${escape(job.id)}" aria-busy="${actionPending(job.id)}">${glyph(job.category === 'integration' ? 'link' : 'download')}<div class="notification-copy"><div class="notification-title-line"><strong class="notification-title">${escape(job.title)}</strong>${timeMark}</div><p>${escape(job.message)}</p>${notes.has(job.id) ? `<p class="notification-status" role="status">${escape(notes.get(job.id))}</p>` : ''}${errors.has(job.id) ? `<p class="field-error" role="alert">${escape(errors.get(job.id))}</p>` : ''}</div><div class="notification-actions">${job.category === 'integration' && job.action === 'update' ? button(job, 'update', pending.get(job.id) === 'update' ? '업데이트 중' : '업데이트', false) : button(job, job.category === 'integration' ? 'settings' : 'notice', escape(job.category === 'integration' ? '설정 열기' : job.actionLabel || '열기'), false, job.actionDisabled)}${job.unread ? button(job, 'ack', '닫기') : ''}</div></article>`;
      const cancelling = snapshot.actions?.get(job.id) === 'cancel';
      const feedback = processingFeedback(job, { cancelling, waitingForModel: job.waitingForModel, pausing: ['pause', 'resume'].includes(snapshot.actions?.get(job.id)) });
      const group = notificationGroup(job);
      const mark = job.active ? (job.status === 'queued' || feedback.paused ? glyph(feedback.paused ? 'pause' : 'clock') : glyph('spinner'))
        : group === 'attention' ? glyph('triangle-alert', 'danger') : job.status === 'cancelled' ? glyph('circle-slash') : glyph('check');
      const title = job.hasMeeting ? `<button type="button" class="quiet notification-title" data-notification-job="${escape(job.id)}" data-notification-action="open"${snapshot.connected === false || actionPending(job.id) ? ' disabled' : ''}>${escape(job.title)}</button>` : `<strong class="notification-title">${escape(job.title)}</strong>`;
      const progress = feedback.progress ? `<span class="notification-time">${feedback.progress.percent}%</span>` : timeMark;
      const controls = `${canRetryProcessing(job) ? button(job, 'retry', '다시 시도', false) : ''}${feedback.pauseVisible ? button(job, feedback.pauseAction, feedback.pauseLabel, true, feedback.pauseDisabled) : ''}${feedback.cancelVisible ? button(job, 'cancel', feedback.cancelLabel, true, feedback.cancelDisabled) : ''}${canAcknowledge(job) ? button(job, 'ack', '닫기') : ''}`;
      return `<article class="notification-item" data-notification-id="${escape(job.id)}" data-state="${escape(feedback.state)}" aria-busy="${actionPending(job.id)}"${job.selected ? ' data-selected="true"' : ''}>${mark}<div class="notification-copy"><div class="notification-title-line">${title}${progress}</div>${job.title === feedback.title ? '' : `<p class="notification-status">${escape(feedback.title)}</p>`}${feedback.detail ? `<p>${escape(feedback.detail)}</p>` : ''}${errors.has(job.id) ? `<p class="field-error" role="alert">${escape(errors.get(job.id))}</p>` : ''}${feedback.diagnostics.length ? `<details class="notification-diagnostics" data-notification-diagnostics="${escape(`${job.id}:${job.status}`)}"><summary>오류 정보</summary>${feedback.diagnostics.map(([label, value]) => `<p>${escape(label)}: ${escape(value)}</p>`).join('')}</details>` : ''}${feedback.progress ? `<progress max="${feedback.progress.max}" value="${feedback.progress.value}" aria-label="${escape(feedback.title)}"></progress>` : ''}</div>${controls ? `<div class="notification-actions">${controls}</div>` : ''}</article>`;
    };
    const groups = ['attention', 'active', 'done'].map(group => [group, jobs.filter(job => notificationGroup(job) === group)]).filter(([, items]) => items.length);
    container.innerHTML = `<header class="notification-heading"><h2 class="notification-heading-title" tabindex="-1" data-notification-heading>알림</h2><button type="button" class="quiet small" data-notification-action="ack-all"${jobs.some(job => canAcknowledge(job) && canAct(job)) && !confirming ? '' : ' disabled'}>${confirming ? '확인 중' : '모두 확인'}</button><span class="sr-only" role="status">${active ? '작업 진행 중' : ''}${unread ? ' 확인할 알림 있음' : ''}</span></header><div class="notification-list">${groups.map(([group, items]) => `<section class="notification-group" data-group="${group}"><h3>${groupTitles[group]}</h3>${items.map(row).join('')}</section>`).join('') || '<p class="notification-empty">알림이 없습니다.</p>'}</div>`;
    container.querySelector('.notification-list').scrollTop = listScroll;
    for (const node of container.querySelectorAll('details[data-notification-diagnostics]')) if (expandedDiagnostics.has(node.dataset.notificationDiagnostics)) node.open = true;
    restoreFocus(saved);
    return { active, unread, total: jobs.length };
  }

  function update(value = {}) {
    if (disposed) return;
    const previous = snapshot.processing?.revision;
    const incoming = value.processing?.revision;
    const processing = incoming != null && previous != null && incoming < previous ? snapshot.processing : value.processing || snapshot.processing;
    snapshot = { ...snapshot, ...value, processing };
    const noticeIds = new Set([...(snapshot.integrations || []), ...(snapshot.notices || [])].map(item => item.id));
    for (const id of confirmedNotices) if (!noticeIds.has(id)) confirmedNotices.delete(id);
    for (const id of notes.keys()) if (!noticeIds.has(id)) notes.delete(id);
    for (const job of processingJobs(processing)) if (activeStatuses.has(job.status)) observedActive.add(job.id);
    return render();
  }

  async function perform(action, id) {
    const job = rows().find(item => item.id === id);
    if (!job || !canAct(job)) return;
    if (['integration', 'notice'].includes(job.category)) {
      if (action === 'notice' && job.actionDisabled) return;
      pending.set(job.id, action); errors.delete(job.id); render();
      try {
        if (action === 'settings') await onOpenIntegration?.(job.service);
        else if (action === 'update' && job.action === 'update') {
          notes.delete(job.id);
          const note = await onUpdateIntegration?.(job.service);
          if (typeof note === 'string' && note) notes.set(job.id, note);
        }
        else if (action === 'notice') await onNoticeAction?.(job.service);
        else if (action === 'ack') confirmedNotices.add(job.id);
        else return;
      } catch (error) { if (!disposed) { errors.set(job.id, error instanceof Error ? error.message : String(error)); showError(error); } }
      finally { pending.delete(job.id); render(); onChanged(); }
      return;
    }
    const feedback = processingFeedback(job);
    if (action === 'open' && !job.hasMeeting) return;
    if (action === 'retry' && !canRetryProcessing(job)) return;
    if (action === 'ack' && !canAcknowledge(job)) return;
    if (action === 'cancel' && (!feedback.cancelVisible || feedback.cancelDisabled)) return;
    if (['pause', 'resume'].includes(action) && (!feedback.pauseVisible || feedback.pauseAction !== action)) return;
    if (!['open', 'retry', 'ack', 'cancel', 'pause', 'resume'].includes(action)) return;
    // The owner reports pending job actions through snapshot.actions.
    if (action === 'open') pending.set(job.id, action);
    errors.delete(job.id);
    render();
    try { await (action === 'open' ? onOpenMeeting?.(job.meetingId) : onProcessingAction?.(action, job.id)); }
    catch (error) {
      if (!disposed && rows().some(item => item.id === job.id)) {
        errors.set(job.id, error instanceof Error ? error.message : String(error));
        showError(error);
      }
    } finally {
      pending.delete(job.id);
      render(); onChanged();
    }
  }

  async function acknowledgeAll() {
    if (confirming || snapshot.connected === false) return;
    const ids = rows().filter(job => canAcknowledge(job) && canAct(job)).map(job => job.id);
    confirming = true;
    render();
    try {
      for (const id of ids) {
        const current = rows().find(job => job.id === id);
        if (current && canAcknowledge(current)) await perform('ack', id);
      }
    } finally {
      confirming = false;
      render();
      const active = container.ownerDocument.activeElement;
      if (!disposed && !container.closest('[hidden], [inert]') && (!active || active === container.ownerDocument.body || container.contains(active))) {
        container.querySelector('[data-notification-heading]')?.focus({ preventScroll: true });
      }
    }
  }

  const click = event => {
    const target = event.target.closest?.('[data-notification-action]');
    if (!target || !container.contains(target) || target.disabled) return;
    if (target.dataset.notificationAction === 'back') return;
    if (target.dataset.notificationAction === 'ack-all') void acknowledgeAll();
    else void perform(target.dataset.notificationAction, target.dataset.notificationJob);
  };
  container.addEventListener('click', click);
  return { update, render, counts: () => ({ active: rows().filter(job => job.active).length, unread: rows().filter(job => notificationGroup(job) === 'attention' && job.unread).length, total: rows().length }),
    destroy() { disposed = true; container.removeEventListener('click', click); } };
}
