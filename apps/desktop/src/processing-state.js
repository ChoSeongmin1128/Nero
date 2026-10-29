const pendingStatuses = new Set(['queued', 'running', 'cancelling']);
const modelKinds = new Set(['setup', 'model', 'summary-model', 'speaker-model']);

export function processingJobs(progress = {}) {
  progress ||= {};
  const jobs = new Map();
  for (const job of Array.isArray(progress.jobs) ? progress.jobs : []) {
    if (typeof job?.id === 'string' && job.id) jobs.set(job.id, job);
  }
  if (progress.id && (!jobs.has(progress.id) || progress.status === 'running')) jobs.set(progress.id, { ...jobs.get(progress.id), ...progress });
  if (!jobs.size && pendingStatuses.has(progress.status)) return [progress];
  return [...jobs.values()];
}

export function pendingJob(progress, { meetingId, kind, models = false } = {}) {
  return processingJobs(progress).find(job => pendingStatuses.has(job.status)
    && (meetingId === undefined || job.meetingId === meetingId)
    && (kind === undefined || job.kind === kind)
    && (!models || modelKinds.has(job.kind)));
}

export function processingResults(previous, progress) {
  const prior = new Map(processingJobs(previous).map(job => [job.id, job]));
  return processingJobs(progress).filter(job => {
    const before = prior.get(job.id);
    const recognized = before?.kind === 'transcribe' && job.kind === 'diarize';
    const available = job.status === 'completed'
      || recognized
      || (['transcribe', 'diarize'].includes(job.kind) && ['cancelled', 'failed', 'interrupted', 'needsReview'].includes(job.status));
    return available && job.id && job.meetingId && (before || job.id === progress.id)
      && (recognized || before?.status !== job.status);
  });
}

export function latestMeetingProcessing(progress, meetingId) {
  if (!meetingId) return undefined;
  const jobs = processingJobs(progress).filter(job => job.meetingId === meetingId);
  const active = jobs.filter(job => pendingStatuses.has(job.status));
  if (active.length) return active.find(job => job.id === progress.id)
    || active.find(job => ['running', 'cancelling'].includes(job.status)) || active[0];
  if (jobs.length === 1) return jobs[0];
  if (!jobs.length || jobs.some(job => !Number.isSafeInteger(job.createdAt) || job.createdAt <= 0)) return undefined;
  const newest = Math.max(...jobs.map(job => job.createdAt));
  const latest = jobs.filter(job => job.createdAt === newest && !jobs.some(other => other.id === job.retrySuccessorId));
  return latest.length === 1 ? latest[0] : undefined;
}

export function processingNotices(progress = {}, meetingId) {
  const latest = latestMeetingProcessing(progress, meetingId);
  const actionable = job => job?.acknowledged !== true && !job?.retrySuccessorId
    && ['failed', 'cancelled', 'interrupted', 'needsReview'].includes(job?.status);
  const primary = latest && (pendingStatuses.has(latest.status) || actionable(latest)) ? latest : undefined;
  const background = processingJobs(progress).filter(job => pendingStatuses.has(job.status) && job.id !== primary?.id);
  return { primary, background };
}
