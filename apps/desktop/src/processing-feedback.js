const modelJobs = new Set(['setup', 'model', 'summary-model', 'speaker-model']);
export const canRetryProcessing = job => job?.acknowledged !== true && !job?.retrySuccessorId && job?.retryable === true && ['interrupted', 'failed', 'cancelled'].includes(job?.status);
export const canDismissProcessing = job => job?.acknowledged !== true && !job?.retrySuccessorId && ['completed', 'interrupted', 'failed', 'cancelled', 'needsReview'].includes(job?.status);
const jobTitles = {
  cleanup: '임시 파일 정리',
  setup: '모델 준비',
  model: '음성 인식 모델 준비',
  'summary-model': '요약 모델 준비',
  import: '음성 가져오기',
  transcribe: '음성 기록 변환',
  summary: '회의록 요약',
  diarize: '화자 구분',
  'speaker-model': '화자 구분 모델 준비',
};

const summaryStages = {
  preparing: '요약 준비 중',
  summarizing: '요약 중',
  merging: '요약 마무리 중',
  saving: '저장 중',
};
const summaryFailures = {
  modelNotReady: '요약 모델을 다시 준비해 주세요.',
  meetingUnavailable: '회의를 찾을 수 없거나 휴지통에 있어 요약할 수 없습니다.',
  recordingActive: '녹음을 마친 뒤 요약해 주세요.',
  sourceChanged: '요약 중 회의 내용이 변경되어 저장하지 않았습니다. 다시 시도해 주세요.',
  inputLimit: '요약할 내용이 입력 한도를 넘었습니다.',
  outputLimit: '요약 분량이 처리 한도를 넘어 완성하지 못했습니다.',
  allocation: '요약에 필요한 메모리를 확보하지 못했습니다. 다른 앱을 닫은 뒤 다시 시도해 주세요.',
  backendUnavailable: '요약 처리 장치를 사용할 수 없습니다. 앱을 다시 연 뒤 시도해 주세요.',
  engineUnavailable: '요약 처리 중 오류가 발생했습니다.',
  protocol: '요약 결과를 읽지 못했습니다.',
  input: '요약할 내용을 처리하지 못했습니다.',
  output: '요약 결과를 완성하지 못했습니다.',
  storage: '요약을 저장하지 못했습니다.',
  cleanup: '요약 처리 후 정리를 마치지 못했습니다. 앱을 다시 연 뒤 시도해 주세요.',
};
const speakerFailures = {
  modelNotReady: '화자 구분 모델을 다시 준비해 주세요.',
  meetingUnavailable: '회의를 찾을 수 없거나 휴지통에 있어 화자를 구분할 수 없습니다.',
  recordingActive: '녹음을 마친 뒤 화자를 구분해 주세요.',
  sourceChanged: '화자 구분 중 음성 기록이 변경되어 저장하지 않았습니다.',
  allocation: '화자 구분에 필요한 메모리를 확보하지 못했습니다. 다른 앱을 닫은 뒤 다시 시도해 주세요.',
  backendUnavailable: '화자 구분 처리 장치를 사용할 수 없습니다. 앱을 다시 연 뒤 시도해 주세요.',
  engineUnavailable: '화자 구분 실행 중 오류가 발생했습니다.',
  input: '화자 구분에 사용할 녹음 파일을 읽지 못했습니다.',
  output: '화자 구분 결과를 읽지 못했습니다.',
  protocol: '화자 구분 결과를 읽지 못했습니다.',
  storage: '화자 구분 결과를 저장하지 못했습니다.',
  cleanup: '화자 구분 실행기의 종료를 확인하지 못했습니다. 앱을 다시 열어 주세요.',
};

function failureDiagnostics(job) {
  if (job.status !== 'failed' || job.kind !== 'summary' || !job.failureDetail) return [];
  const detail = job.failureDetail;
  const rows = [];
  const stage = { whole: '전체 요약', map: '구간 요약', merge: '요약 통합' }[detail.stage];
  const reason = { length: '출력 한도 도달', stop: '생성 종료' }[detail.finishReason];
  if (typeof stage === 'string') rows.push(['실패 단계', stage]);
  if (typeof reason === 'string') rows.push(['종료 이유', reason]);
  for (const [key, label] of [['inputTokens', '입력 토큰 수'], ['outputLimit', '출력 토큰 한도'], ['generatedTokens', '생성 토큰 수']]) {
    if (Number.isSafeInteger(detail[key]) && detail[key] >= 0) rows.push([label, String(detail[key])]);
  }
  return rows;
}

export function processingFeedback(job = {}, { cancelling = false } = {}) {
  const active = Boolean(job.id) && ['running', 'cancelling'].includes(job.status);
  const queued = Boolean(job.id) && job.status === 'queued';
  const committing = active && (job.committing === true
    || (job.kind === 'summary' && job.summaryCommitting === true));
  const cancelRequested = (active || queued) && !committing && (cancelling || job.status === 'cancelling' || job.message === '취소 중');
  const title = jobTitles[job.kind] || '처리';
  let progress = null;
  // Byte counts describe model preparation. Inference messages and chunk counts
  // do not report completed work and must not become a percentage.
  if (active && !cancelRequested && modelJobs.has(job.kind)
    && Number.isFinite(job.totalBytes) && job.totalBytes > 0) {
    const value = Math.min(job.totalBytes, Math.max(0, Number.isFinite(job.bytes) ? job.bytes : 0));
    progress = { value, max: job.totalBytes, percent: Math.floor(value / job.totalBytes * 100) };
  }
  const terminal = { completed: '완료', failed: '실패', cancelled: '취소됨', queued: '대기 중', interrupted: '중단됨', needsReview: '확인 필요' }[job.status];
  let heading = cancelRequested ? `${title} 취소 중` : `${title}${active ? ' 중' : terminal ? ` ${terminal}` : ''}`;
  if (active && job.kind === 'summary' && !cancelRequested) {
    heading = committing ? summaryStages.saving : summaryStages[job.phase] || '요약 중';
  }
  const speakerPhase = active && (job.kind === 'diarize' || (job.kind === 'transcribe' && job.phase === 'diarizing'));
  if (speakerPhase && !cancelRequested && job.phase === 'postprocessing') heading = '음성 기록 정리 중';
  else if (speakerPhase && !cancelRequested) heading = committing || job.phase === 'saving' ? '저장 중' : job.phase === 'preparing' ? '화자 구분 준비 중' : '화자 구분 중';
  let message = String(job.message || '').trim();
  if (job.status === 'cancelled' && ['취소했습니다.', '취소됨'].includes(message)) message = '';
  if (job.status === 'queued') message = message || '앞선 작업이나 녹음이 끝나면 실행합니다.';
  if (job.status === 'interrupted') message = message || '기존 녹음과 결과는 유지됩니다.';
  if (job.status === 'needsReview') message = '회의를 열어 저장된 결과를 확인해 주세요.';
  if (speakerPhase) message = '';
  if (job.kind === 'diarize' && job.status === 'failed') {
    message = (Object.hasOwn(speakerFailures, job.failureKind) ? speakerFailures[job.failureKind] : '')
      || '화자를 구분하지 못했습니다. 원인을 확인하지 못했습니다. 기존 음성 기록은 유지됩니다.';
  }
  if (active && job.kind === 'summary') message = !cancelRequested && !committing && job.retryCount > 0 ? '다시 시도 중' : '';
  if (job.kind === 'summary' && job.status === 'failed') {
    message = (Object.hasOwn(summaryFailures, job.failureKind) ? summaryFailures[job.failureKind] : '') || '요약하지 못했습니다. 원인을 확인하지 못했습니다.';
    if (job.hasPreviousSummary === true) message += ' 기존 요약은 유지되었습니다.';
  }
  const repeated = (job.kind === 'transcribe' && message === '텍스트로 변환 중')
    || (job.kind === 'summary' && message === '요약 중');
  return {
    visible: Boolean(job.id) && job.acknowledged !== true && ['running', 'cancelling', 'failed', 'cancelled', 'queued', 'interrupted', 'needsReview'].includes(job.status),
    active,
    state: cancelRequested ? 'cancelling' : active ? 'running' : job.status || 'idle',
    icon: job.status === 'queued' ? 'clock' : ['failed', 'needsReview'].includes(job.status) ? 'triangle-alert' : job.status === 'interrupted' ? 'pause' : 'circle-slash',
    title: heading,
    detail: message && !cancelRequested && !repeated && !['준비 중', '완료', heading].includes(message) ? message : '',
    warning: '',
    diagnostics: failureDiagnostics(job),
    progress,
    cancelVisible: (active || queued) && !committing,
    cancelDisabled: cancelRequested || committing || (queued && cancelling),
    cancelLabel: cancelRequested ? '취소 중' : '취소',
  };
}
