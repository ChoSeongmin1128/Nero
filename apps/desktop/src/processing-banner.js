import { jobActions } from './processing-feedback.js';
import { processingJobs, processingNotices, queuedBehindModel } from './processing-state.js';

const buttons = ['pause-processing', 'cancel-processing', 'retry-processing', 'open-processing-meeting', 'dismiss-processing'];

// The processing banner above the content: the one job the person should see now, its
// progress and the actions jobActions allows. `view()` reads the app state it shows:
// { processing, meetingId, meetings, selectedId, cancellingId, actions, errors, recording, summaryEnabled }.
// A button asked for an action keeps focus on that job, or moves to the next one when it goes.
export function mountProcessingBanner({ byId, document, icon, escape, view, onAction, onOpenMeeting }) {
  let focus = null;
  const setText = (id, text) => { if (byId(id).textContent !== text) byId(id).textContent = text; };
  const setHTML = (id, html) => { if (byId(id).innerHTML !== html) byId(id).innerHTML = html; };

  function remember() {
    if (focus) return;
    const target = document.activeElement;
    const jobId = target?.dataset?.jobId;
    // Other job buttons, such as model preparation in Settings, keep their own focus.
    if (!jobId || !buttons.some(id => byId(id) === target)) return;
    const ids = [...new Set([...byId('processing').querySelectorAll('[data-job-id]')].map(node => node.dataset.jobId))];
    focus = { target, id: jobId, action: target.dataset.jobAction, ids, index: ids.indexOf(jobId) };
  }

  function restore(current) {
    const saved = focus;
    if (!saved) return;
    const active = document.activeElement;
    if (active && active !== document.body && active !== saved.target && active.isConnected !== false) { focus = null; return; }
    if (current.actions.has(saved.id)) return;
    const usable = node => node && !node.disabled && !node.hidden && node.isConnected !== false && !node.closest?.('[hidden], [inert]');
    const banner = buttons.map(byId).filter(node => !byId('processing').hidden && usable(node));
    const forJob = id => banner.filter(node => node.dataset.jobId === id);
    const same = forJob(saved.id);
    const next = saved.ids.slice(saved.index + 1).map(id => forJob(id)[0]).find(Boolean);
    const previous = (saved.index < 0 ? [] : saved.ids.slice(0, saved.index).reverse()).map(id => forJob(id)[0]).find(Boolean);
    const successor = forJob(processingJobs(current.processing).find(job => job.id === saved.id)?.retrySuccessorId)[0];
    const target = same.find(node => node.dataset.jobAction === saved.action) || same[0] || successor || next || previous
      || banner[0]
      || ['settings', 'navigation-toggle', 'library-title'].map(byId).find(usable);
    focus = null;
    target?.focus({ preventScroll: true });
  }

  function render() {
    remember();
    const current = view();
    const notices = processingNotices(current.processing, current.meetingId);
    const job = notices.primary ? { ...(current.processing.id === notices.primary.id ? current.processing : {}), ...notices.primary } : {};
    const pendingAction = current.actions.get(job.id);
    const allowed = jobActions(job, {
      cancelling: current.cancellingId === job.id || pendingAction === 'cancel',
      waitingForModel: queuedBehindModel(current.processing, job),
      recording: current.recording,
      pausing: ['pause', 'resume'].includes(pendingAction),
      summaryEnabled: current.summaryEnabled,
    });
    const { feedback } = allowed;
    byId('processing').hidden = !feedback.visible || job.kind === 'setup';
    byId('processing').dataset.state = feedback.state;
    const owner = current.meetings.find(meeting => meeting.id === job.meetingId);
    setText('processing-title', `${owner && owner.id !== current.selectedId ? owner.title + ': ' : ''}${feedback.title}`);
    const detail = current.errors.get(job.id) || feedback.detail;
    setText('processing-message', detail);
    byId('processing-message').hidden = !detail;
    const errorInfo = byId('processing-error-info');
    const diagnosticKey = `${job.id}:${job.status}`;
    if (errorInfo.dataset.job !== diagnosticKey) errorInfo.open = false;
    errorInfo.dataset.job = diagnosticKey;
    errorInfo.hidden = !feedback.diagnostics.length;
    setHTML('processing-error-fields', feedback.diagnostics.map(([label, value]) => `<div>${escape(label)}: ${escape(value)}</div>`).join(''));
    setText('processing-warning', feedback.warning);
    byId('processing-warning').hidden = !feedback.warning;
    byId('processing-spinner').hidden = !feedback.active || feedback.paused;
    byId('processing-result-icon').hidden = feedback.active && !feedback.paused;
    setHTML('processing-result-icon', icon(feedback.icon));
    byId('cancel-processing').hidden = !feedback.cancelVisible;
    // While another request for the job is on its way a cancel would be refused.
    byId('cancel-processing').disabled = feedback.cancelDisabled || (current.actions.has(job.id) && pendingAction !== 'cancel');
    setText('cancel-processing', feedback.cancelLabel);
    byId('pause-processing').hidden = !feedback.pauseVisible;
    byId('pause-processing').disabled = feedback.pauseDisabled;
    byId('pause-processing').dataset.jobAction = feedback.pauseAction;
    setText('pause-processing', feedback.pauseLabel);
    byId('processing-progress').hidden = !feedback.progress;
    setText('processing-percent', feedback.progress ? `${feedback.progress.percent}%` : '');
    byId('processing-percent').hidden = !feedback.progress;
    if (feedback.progress) { byId('processing-progress').max = feedback.progress.max; byId('processing-progress').value = feedback.progress.value; }
    byId('retry-processing').hidden = !allowed.retry;
    byId('retry-processing').disabled = current.actions.has(job.id);
    // The meeting already open needs no button to open it.
    byId('open-processing-meeting').hidden = job.status !== 'needsReview' || !owner || owner.id === current.meetingId;
    byId('dismiss-processing').hidden = !allowed.ack;
    byId('dismiss-processing').disabled = current.actions.has(job.id);
    for (const [id, action] of [['cancel-processing', 'cancel'], ['retry-processing', 'retry'], ['open-processing-meeting', 'open'], ['dismiss-processing', 'ack']]) {
      byId(id).dataset.jobId = job.id || '';
      byId(id).dataset.jobAction = action;
    }
    byId('pause-processing').dataset.jobId = job.id || '';
    restore(current);
  }

  for (const id of ['pause-processing', 'cancel-processing', 'retry-processing', 'dismiss-processing']) byId(id).addEventListener('click', () => onAction(byId(id).dataset.jobAction, byId(id).dataset.jobId));
  byId('open-processing-meeting').addEventListener('click', () => {
    const job = processingJobs(view().processing).find(item => item.id === byId('open-processing-meeting').dataset.jobId);
    if (job?.meetingId) onOpenMeeting(job.meetingId);
  });
  return { render, remember };
}
