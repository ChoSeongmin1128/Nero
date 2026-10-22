const modelJobs = new Set(['setup', 'model', 'summary-model']);
const jobTitles = {
  cleanup: '임시 파일 정리',
  setup: '모델 준비',
  model: '음성 인식 모델 준비',
  'summary-model': '요약 모델 준비',
  import: '음성 가져오기',
  transcribe: '음성 기록 변환',
  summary: '회의록 요약',
};

export function processingFeedback(job = {}, { cancelling = false } = {}) {
  const active = Boolean(job.id) && job.status === 'running';
  const committing = active && (job.committing === true
    || (job.kind === 'summary' && job.summaryCommitting === true));
  const cancelRequested = active && !committing && (cancelling || job.message === '취소 중');
  const title = jobTitles[job.kind] || '처리';
  let progress = null;
  // Byte counts describe model preparation. Inference messages and chunk counts
  // do not report completed work and must not become a percentage.
  if (active && !cancelRequested && modelJobs.has(job.kind)
    && Number.isFinite(job.totalBytes) && job.totalBytes > 0) {
    const value = Math.min(job.totalBytes, Math.max(0, Number.isFinite(job.bytes) ? job.bytes : 0));
    progress = { value, max: job.totalBytes, percent: Math.floor(value / job.totalBytes * 100) };
  }
  const terminal = { completed: '완료', failed: '실패', cancelled: '취소됨' }[job.status];
  const heading = cancelRequested ? `${title} 취소 중` : `${title}${active ? ' 중' : terminal ? ` ${terminal}` : ''}`;
  const message = String(job.message || '').trim();
  const repeated = (job.kind === 'transcribe' && message === '텍스트로 변환 중')
    || (job.kind === 'summary' && message === '요약 중');
  return {
    visible: Boolean(job.id) && ['running', 'failed', 'cancelled'].includes(job.status),
    active,
    state: cancelRequested ? 'cancelling' : active ? 'running' : job.status || 'idle',
    title: heading,
    detail: message && !cancelRequested && !repeated && !['준비 중', '완료', heading].includes(message) ? message : '',
    warning: active && !cancelRequested && !committing && job.kind === 'summary' && ['warning', 'critical'].includes(job.memoryState)
      ? '시스템 메모리 경고가 감지되었습니다. 다른 앱을 닫거나 요약을 취소할 수 있습니다.' : '',
    progress,
    cancelVisible: active && !committing,
    cancelDisabled: cancelRequested || committing,
    cancelLabel: cancelRequested ? '취소 중' : '취소',
  };
}
