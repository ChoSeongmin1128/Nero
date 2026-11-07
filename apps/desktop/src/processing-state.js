const pendingStatuses = new Set(['queued', 'running', 'cancelling']);
export const modelKinds = new Set(['setup', 'model', 'summary-model', 'speaker-model']);

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

export const speechModelKinds = Object.freeze(['setup', 'model', 'speaker-model']);
export const summaryModelKinds = Object.freeze(['summary-model']);
const endedStatuses = new Set(['failed', 'cancelled', 'interrupted']);

export function formatBytes(bytes) {
  if (!Number.isFinite(bytes) || bytes < 0) return '';
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(2)} GB`;
  return `${Math.max(1, Math.round(bytes / 1e6))} MB`;
}

export function byteProgress(job) {
  if (!Number.isFinite(job?.totalBytes) || job.totalBytes <= 0) return null;
  const value = Math.min(job.totalBytes, Math.max(0, Number.isFinite(job.bytes) ? job.bytes : 0));
  return { value, max: job.totalBytes, percent: Math.floor(value / job.totalBytes * 100) };
}

// One reading of a model download for every screen. Byte counts come only from the
// job that owns them (processingJobs merges the running job's counters by id).
export function modelPreparation(progress, kinds) {
  const wanted = new Set(kinds);
  const jobs = processingJobs(progress).filter(job => wanted.has(job.kind));
  const active = jobs.find(job => pendingStatuses.has(job.status));
  if (active) {
    const counted = active.status === 'queued' ? null : byteProgress(active);
    return {
      state: active.status === 'queued' ? 'queued' : active.status === 'cancelling' ? 'cancelling' : 'running',
      jobId: active.id,
      percent: counted ? counted.percent : null,
      value: counted ? counted.value : 0,
      max: counted ? counted.max : 0,
      size: counted ? `${formatBytes(counted.value)} / ${formatBytes(counted.max)}` : '',
      stage: active.status === 'queued' ? '앞선 작업이 끝나면 시작합니다.' : active.status === 'cancelling' ? '취소 중' : String(active.message || '').trim() || '준비 중',
      cancellable: active.status !== 'cancelling' && active.committing !== true,
    };
  }
  const ended = jobs.filter(job => endedStatuses.has(job.status) && job.acknowledged !== true && !job.retrySuccessorId).at(-1);
  if (ended) {
    return {
      state: ended.status === 'cancelled' ? 'cancelled' : 'failed',
      jobId: ended.id,
      message: ended.status === 'cancelled' ? '다운로드를 취소했습니다. 받은 부분은 남아 있어 다시 시도하면 이어받습니다.'
        : ended.status === 'interrupted' ? '다운로드가 중단되었습니다. 다시 시도하면 이어받습니다.'
          : String(ended.message || '').trim() || '모델을 준비하지 못했습니다.',
    };
  }
  return { state: 'idle' };
}

// The selected model's memory warning applies until its files are ready.
export function memoryWarningFor(choice) {
  return choice && choice.supported === true && choice.ready !== true && typeof choice.memoryWarning === 'string' ? choice.memoryWarning : '';
}

// Preparing a choice with a memory warning waits for the user's acknowledgement.
export const memoryBlocked = (choice, acknowledged) => Boolean(memoryWarningFor(choice)) && !acknowledged.has(choice.id);

export const speakerNeedsPreparation = progress => progress?.speakerEnabled !== false && progress?.speakerSupported === true && progress?.speakerReady === false;
