const terminal = new Set(['completed', 'failed', 'cancelled', 'interrupted']);
const phases = ['preparing', 'transcribing', 'saving'];
export const notificationLabels = {
  preparing: '음성 준비 중', transcribing: '음성 기록 변환 중', saving: '음성 기록 저장 중',
  completed: '음성 기록 변환 완료', failed: '음성 기록 변환 실패',
  cancelled: '음성 기록 변환 취소됨', interrupted: '음성 기록 변환 중단됨',
};
export function createNotificationFlow() { return { version: 1, sequence: 0, jobs: [] }; }
export function notificationTransition(state, event) {
  const previous = state.jobs.find(job => job.meetingId === event.meetingId);
  if (event.type === 'start' || event.type === 'retry') {
    if (previous?.status === 'processing' || (event.type === 'retry' && !['failed', 'cancelled', 'interrupted'].includes(previous?.status))) return state;
    const job = { meetingId: event.meetingId, attempt: (previous?.attempt || 0) + 1, status: 'processing', phase: 'preparing', acknowledged: false, dismissed: false, order: state.sequence + 1 };
    return { ...state, sequence: job.order, jobs: [...state.jobs.filter(item => item.meetingId !== event.meetingId), job] };
  }
  if (event.type === 'clearAcknowledged') return { ...state, jobs: state.jobs.map(job => terminal.has(job.status) && job.acknowledged ? { ...job, dismissed: true } : job) };
  if (!previous || event.attempt !== previous.attempt) return state;
  let next;
  if (event.type === 'advance' && previous.status === 'processing') {
    const phase = phases[Math.min(phases.indexOf(previous.phase) + 1, phases.length - 1)];
    if (phase === previous.phase) return state;
    next = { ...previous, phase };
  } else if (['completed', 'failed', 'cancelled'].includes(event.type) && previous.status === 'processing') {
    next = { ...previous, status: event.type, acknowledged: false, dismissed: false };
  } else if (event.type === 'acknowledge' && terminal.has(previous.status)) {
    next = { ...previous, acknowledged: true };
  } else if (event.type === 'dismiss' && terminal.has(previous.status) && previous.acknowledged) {
    next = { ...previous, dismissed: true };
  } else return state;
  return { ...state, jobs: state.jobs.map(job => job === previous ? next : job) };
}
export function notificationProjection(state, meetings) {
  const available = new Map(meetings.filter(meeting => !meeting.deletedAt).map(meeting => [meeting.id, meeting]));
  return state.jobs.filter(job => available.has(job.meetingId) && !job.dismissed).sort((a, b) => b.order - a.order).map(job => ({ ...job, title: available.get(job.meetingId).title, label: notificationLabels[job.status === 'processing' ? job.phase : job.status] }));
}
export function restoreNotificationFlow(value) {
  if (value?.version !== 1 || !Number.isSafeInteger(value.sequence) || value.sequence < 0 || !Array.isArray(value.jobs)) return createNotificationFlow();
  const seen = new Set();
  const jobs = value.jobs.filter(job => {
    if (!job || !Number.isSafeInteger(job.meetingId) || seen.has(job.meetingId) || !Number.isSafeInteger(job.attempt) || job.attempt < 1 || !Number.isSafeInteger(job.order) || job.order < 1 || job.order > value.sequence || !['processing', ...terminal].includes(job.status) || !phases.includes(job.phase) || typeof job.acknowledged !== 'boolean' || typeof job.dismissed !== 'boolean') return false;
    seen.add(job.meetingId); return true;
  }).map(job => ({ ...job, ...(job.status === 'processing' ? { status: 'interrupted', acknowledged: false, dismissed: false } : {}) }));
  return { version: 1, sequence: value.sequence, jobs };
}
