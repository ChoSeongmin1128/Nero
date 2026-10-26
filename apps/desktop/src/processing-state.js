const pendingStatuses = new Set(['queued', 'running', 'cancelling']);
const modelKinds = new Set(['setup', 'model', 'summary-model', 'speaker-model']);

export function processingJobs(progress = {}) {
  progress ||= {};
  const jobs = new Map();
  for (const job of Array.isArray(progress.jobs) ? progress.jobs : []) {
    if (typeof job?.id === 'string' && job.id) jobs.set(job.id, job);
  }
  if (progress.id && (!jobs.has(progress.id) || progress.status === 'running')) jobs.set(progress.id, progress);
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
