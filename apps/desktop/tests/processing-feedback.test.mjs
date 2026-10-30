import assert from 'node:assert/strict';
import test from 'node:test';
import { processingFeedback, canRetryProcessing, canDismissProcessing } from '../src/processing-feedback.js';

test('cancelled processing exposes recovery and dismissal without a false close icon', () => {
  const job = { id: 'cancelled', kind: 'transcribe', status: 'cancelled', retryable: true, message: '취소했습니다.' };
  assert.equal(canRetryProcessing(job), true);
  assert.equal(canDismissProcessing(job), true);
  const feedback = processingFeedback(job);
  assert.equal(feedback.visible, true);
  assert.equal(feedback.cancelVisible, false);
  assert.notEqual(feedback.icon, 'x');
  assert.equal(feedback.detail, '');
  assert.equal(canRetryProcessing({ ...job, acknowledged: true }), false);
  assert.equal(canDismissProcessing({ ...job, acknowledged: true }), false);
  assert.equal(canRetryProcessing({ ...job, kind: 'import', retryable: false }), false);
});

const running = (kind, extra = {}) => ({ id: 'job', kind, status: 'running', ...extra });

test('a retry predecessor has no retry or acknowledgement action even without legacy acknowledgement metadata', () => {
  const job = { id: 'prior', kind: 'summary', status: 'cancelled', retryable: true, acknowledged: false, retrySuccessorId: 'next' };
  assert.equal(canRetryProcessing(job), false);
  assert.equal(canDismissProcessing(job), false);
});

test('queued work is cancellable without pretending inference is running', () => {
  const job = { id: 'waiting', kind: 'summary', status: 'queued' };
  const feedback = processingFeedback(job);
  assert.equal(feedback.visible, true);
  assert.equal(feedback.active, false);
  assert.equal(feedback.title, '회의록 요약 대기 중');
  assert.equal(feedback.progress, null);
  assert.equal(feedback.cancelVisible, true);
  assert.equal(feedback.icon, 'clock');
  const cancelling = processingFeedback(job, { cancelling: true });
  assert.equal(cancelling.cancelDisabled, true);
  assert.equal(cancelling.cancelLabel, '취소 중');
});

test('interrupted work and uncertain saving have distinct recovery instructions', () => {
  const interrupted = processingFeedback({ id: 'stopped', kind: 'transcribe', status: 'interrupted' });
  assert.equal(interrupted.active, false);
  assert.equal(interrupted.cancelVisible, false);
  assert.match(interrupted.title, /중단됨/);
  const review = processingFeedback({ id: 'saving', kind: 'summary', status: 'needsReview', message: 'old internal detail' });
  assert.equal(review.title, '회의록 요약 확인 필요');
  assert.equal(review.detail, '회의를 열어 저장된 결과를 확인해 주세요.');
  assert.equal(review.cancelVisible, false);
  assert.equal(review.icon, 'triangle-alert');
  assert.equal(processingFeedback({ id: 'acknowledged', kind: 'summary', status: 'failed', acknowledged: true }).visible, false);
});

test('recognition and summary stay visibly active without inventing an inference percentage', () => {
  const recognition = processingFeedback(running('transcribe', { message: '텍스트로 변환 중' }));
  assert.equal(recognition.visible, true);
  assert.equal(recognition.active, true);
  assert.equal(recognition.title, '음성 기록 처리 중');
  assert.equal(recognition.detail, '');
  assert.equal(recognition.progress, null);
  assert.equal(recognition.cancelVisible, true);
  const summary = processingFeedback(running('summary', { message: '요약 중 (12/80)', totalBytes: 80, bytes: 12 }));
  assert.equal(summary.title, '요약 중');
  assert.equal(summary.detail, '');
  assert.equal(summary.progress, null);
});

test('only finite model preparation byte counts provide determinate progress', () => {
  for (const kind of ['setup', 'model', 'summary-model']) {
    const feedback = processingFeedback(running(kind, { totalBytes: 1000, bytes: 250 }));
    assert.deepEqual(feedback.progress, { value: 250, max: 1000, percent: 25 });
    assert.equal(processingFeedback(running(kind)).progress, null);
    assert.equal(processingFeedback(running(kind, { totalBytes: 0 })).progress, null);
    assert.equal(processingFeedback(running(kind, { totalBytes: Infinity })).progress, null);
  }
  assert.equal(processingFeedback(running('import', { totalBytes: 100, bytes: 50 })).progress, null);
});

test('progress clamps malformed byte counters without changing task completion state', () => {
  assert.deepEqual(processingFeedback(running('model', { totalBytes: 100, bytes: -5 })).progress,
    { value: 0, max: 100, percent: 0 });
  const verified = processingFeedback(running('model', { totalBytes: 100, bytes: 120 }));
  assert.deepEqual(verified.progress, { value: 100, max: 100, percent: 100 });
  assert.equal(verified.active, true);
  assert.equal(verified.visible, true);
});

test('cancel requests remain active and disable repeated cancellation until acknowledged', () => {
  for (const [job, options] of [
    [running('summary', { message: '취소 중' }), {}],
    [running('summary', { status: 'cancelling' }), {}],
    [running('summary', { message: '요약 중' }), { cancelling: true }],
  ]) {
    const feedback = processingFeedback(job, options);
    assert.equal(feedback.state, 'cancelling');
    assert.equal(feedback.active, true);
    assert.equal(feedback.title, '회의록 요약 취소 중');
    assert.equal(feedback.cancelDisabled, true);
    assert.equal(feedback.cancelLabel, '취소 중');
    assert.equal(feedback.progress, null);
  }
});

test('terminal states stop activity and keep failure or cancellation messages available', () => {
  const failed = processingFeedback({ id: 'job', kind: 'transcribe', status: 'failed', message: '모델 파일을 다시 준비해 주세요.' });
  assert.equal(failed.visible, true);
  assert.equal(failed.active, false);
  assert.equal(failed.title, '음성 기록 변환 실패');
  assert.equal(failed.detail, '모델 파일을 다시 준비해 주세요.');
  assert.equal(failed.cancelVisible, false);
  const cancelled = processingFeedback({ id: 'job', kind: 'summary', status: 'cancelled', message: '처리를 취소했습니다.' });
  assert.equal(cancelled.visible, true);
  assert.equal(cancelled.state, 'cancelled');
  assert.equal(cancelled.active, false);
  assert.equal(processingFeedback({ id: 'job', kind: 'summary', status: 'completed' }).visible, false);
  assert.equal(processingFeedback().visible, false);
  assert.equal(processingFeedback({ status: 'running' }).active, false);
});

test('OS memory observations do not interrupt normal summary activity', () => {
  for (const memoryState of ['warning', 'critical']) {
    const job = Object.freeze(running('summary', { memoryState, message: '요약 모델을 불러오고 있습니다.', modelReady: true, summaryReady: true }));
    const feedback = processingFeedback(job);
    assert.equal(feedback.warning, '');
    assert.equal(feedback.detail, '');
    assert.equal(feedback.state, 'running');
    assert.equal(feedback.active, true);
    assert.equal(feedback.cancelVisible, true);
    assert.equal(feedback.cancelDisabled, false);
    assert.equal(feedback.progress, null);
    assert.equal(job.modelReady, true);
    assert.equal(job.summaryReady, true);
  }
});

test('unknown observations and other job types do not become memory warnings', () => {
  for (const memoryState of [undefined, null, 'normal', 'unknown', 'unrecognized']) {
    assert.equal(processingFeedback(running('summary', { memoryState })).warning, '');
  }
  for (const kind of ['transcribe', 'import', 'model', 'summary-model', 'setup']) {
    assert.equal(processingFeedback(running(kind, { memoryState: 'critical' })).warning, '');
  }
});

test('memory warnings disappear when cancelling or terminal without inventing a failure cause', () => {
  const base = { id: 'job', kind: 'summary', memoryState: 'critical' };
  for (const status of ['completed', 'failed', 'cancelled', 'idle']) {
    const feedback = processingFeedback({ ...base, status, failureKind: 'workerExit', message: '요약 실행기가 종료되었습니다.' });
    assert.equal(feedback.warning, '');
    assert.equal(feedback.detail, status === 'failed' ? '요약하지 못했습니다. 원인을 확인하지 못했습니다.' : '요약 실행기가 종료되었습니다.');
  }
  assert.equal(processingFeedback({ ...base, status: 'running', message: '취소 중' }).warning, '');
  assert.equal(processingFeedback({ ...base, status: 'running' }, { cancelling: true }).warning, '');
});

test('retrying the same summary job keeps normal activity and never turns attempts into percent', () => {
  const job = running('summary', { memoryState: 'warning', modelReady: true, summaryReady: true, totalBytes: 100, bytes: 50 });
  for (const [retryCount, message] of [[0, '요약 모델을 불러오고 있습니다.'], [1, '요약을 다시 준비하고 있습니다.'], [1, '요약 중']]) {
    const retry = { ...job, retryCount, message };
    const feedback = processingFeedback(retry);
    assert.equal(retry.id, 'job');
    assert.equal(feedback.active, true);
    assert.equal(feedback.state, 'running');
    assert.equal(feedback.progress, null);
    assert.equal(feedback.cancelDisabled, false);
    assert.equal(retry.modelReady, true);
    assert.equal(retry.summaryReady, true);
  }
});

test('summary commit keeps saving activity while hiding cancellation and memory warnings', () => {
  const job = running('summary', { summaryCommitting: true, memoryState: 'critical', message: '회의록을 저장하고 있습니다.' });
  for (const options of [{}, { cancelling: true }]) {
    const feedback = processingFeedback(job, options);
    assert.equal(feedback.active, true);
    assert.equal(feedback.state, 'running');
    assert.equal(feedback.title, '저장 중');
    assert.equal(feedback.detail, '');
    assert.equal(feedback.cancelVisible, false);
    assert.equal(feedback.cancelDisabled, true);
    assert.equal(feedback.warning, '');
    assert.equal(feedback.progress, null);
  }
});

test('missing summary commit metadata retains cancellation and model jobs keep byte progress', () => {
  for (const summaryCommitting of [undefined, false]) {
    const feedback = processingFeedback(running('summary', { summaryCommitting, memoryState: 'warning' }));
    assert.equal(feedback.cancelVisible, true);
    assert.equal(feedback.cancelDisabled, false);
    assert.equal(feedback.warning, '');
  }
  const download = processingFeedback(running('summary-model', { summaryCommitting: true, totalBytes: 100, bytes: 25 }));
  assert.equal(download.cancelVisible, true);
  assert.equal(download.cancelDisabled, false);
  assert.equal(download.progress.percent, 25);
});

test('terminology classification stays cancellable until the final transcript commit', () => {
  const classification = running('transcribe', { message: '용어 확인 중' });
  assert.equal(processingFeedback(classification).cancelVisible, true);
  const saving = processingFeedback({ ...classification, committing: true, message: '음성 기록 저장 중' });
  assert.equal(saving.active, true);
  assert.equal(saving.cancelVisible, false);
  assert.equal(saving.cancelDisabled, true);
  assert.equal(saving.detail, '');
  assert.equal(saving.progress, null);
});


test('summary phases replace raw source indexes with one stage label', () => {
  for (const [phase, title] of Object.entries({ preparing: '요약 준비 중', summarizing: '요약 중', merging: '요약 마무리 중', saving: '저장 중' })) {
    const feedback = processingFeedback(running('summary', { phase, message: '요약 중 (1/80)' }));
    assert.equal(feedback.title, title);
    assert.equal(feedback.detail, '');
    assert.equal(feedback.progress, null);
  }
  assert.equal(processingFeedback(running('summary', { retryCount: 1 })).detail, '다시 시도 중');
});

test('summary failure explains confirmed causes and preserves only an existing summary', () => {
  const job = { id: 'job', kind: 'summary', status: 'failed', failureKind: 'outputLimit', message: 'unsafe raw output' };
  assert.equal(processingFeedback(job).detail, '요약 분량이 처리 한도를 넘어 완성하지 못했습니다.');
  assert.match(processingFeedback({ ...job, hasPreviousSummary: true }).detail, /기존 요약은 유지/);
  assert.doesNotMatch(processingFeedback({ ...job, hasPreviousSummary: false }).detail, /기존 요약/);
  assert.match(processingFeedback({ ...job, failureKind: 'allocation' }).detail, /메모리.*다른 앱/);
  assert.match(processingFeedback({ ...job, failureKind: 'unknown', memoryState: 'critical' }).detail, /원인을 확인하지 못/);
});

test('failure diagnostics expose only known labels and valid optional counts', () => {
  const job = { id: 'job', kind: 'summary', status: 'failed', failureDetail: { stage: 'merge', finishReason: 'length', inputTokens: 3000, outputLimit: 2048, generatedTokens: 2048, response: 'private', path: '/private/file' } };
  assert.deepEqual(processingFeedback(job).diagnostics, [['실패 단계', '요약 통합'], ['종료 이유', '출력 한도 도달'], ['입력 토큰 수', '3000'], ['출력 토큰 한도', '2048'], ['생성 토큰 수', '2048']]);
  assert.deepEqual(processingFeedback({ ...job, failureDetail: { stage: 'private text', finishReason: '<script>', inputTokens: -1, outputLimit: Infinity, generatedTokens: null } }).diagnostics, []);
  assert.deepEqual(processingFeedback({ ...job, status: 'running' }).diagnostics, []);
});


test('typed preflight failures keep their confirmed cause without exposing raw messages', () => {
  const expected = {
    modelNotReady: '요약 모델을 다시 준비해 주세요.',
    meetingUnavailable: '회의를 찾을 수 없거나 휴지통에 있어 요약할 수 없습니다.',
    recordingActive: '녹음을 마친 뒤 요약해 주세요.',
    sourceChanged: '요약 중 회의 내용이 변경되어 저장하지 않았습니다. 다시 시도해 주세요.',
    inputLimit: '요약할 내용이 입력 한도를 넘었습니다.',
  };
  for (const [failureKind, detail] of Object.entries(expected)) {
    const feedback = processingFeedback({ id: 'job', kind: 'summary', status: 'failed', failureKind, message: '/private/model/path raw response', hasPreviousSummary: false });
    assert.equal(feedback.detail, detail);
    assert.doesNotMatch(feedback.detail, /private|raw response|기존 요약/);
  }
});


test('speaker analysis uses real phases and model bytes without exposing chunk labels', () => {
  const feedback = processingFeedback(running('diarize', { phase: 'diarizing', message: 'chunk 12 speaker 51' }));
  assert.equal(feedback.title, '화자 구분 중');
  assert.equal(feedback.detail, '');
  assert.equal(feedback.cancelVisible, true);
  const saving = processingFeedback(running('diarize', { phase: 'saving', committing: true }));
  assert.equal(saving.title, '저장 중');
  assert.equal(saving.cancelVisible, false);
  assert.equal(processingFeedback(running('transcribe', { phase: 'diarizing' })).title, '음성 기록 처리 중');
  assert.equal(processingFeedback(running('speaker-model', { bytes: 50, totalBytes: 100 })).progress.percent, 50);
});

test('speaker failures retain confirmed causes without displaying private worker messages', () => {
  const expected = {
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
  for (const [failureKind, detail] of Object.entries(expected)) {
    const feedback = processingFeedback({ id: 'job', kind: 'diarize', status: 'failed', failureKind,
      message: '/private/meeting/audio.wav: unsupported WAV encoding',
      failureDetail: { stage: '/private/path', response: 'private output' } });
    assert.equal(feedback.title, '화자 구분 실패');
    assert.equal(feedback.detail, detail);
    assert.equal(feedback.active, false);
    assert.equal(feedback.cancelVisible, false);
    assert.deepEqual(feedback.diagnostics, []);
    assert.doesNotMatch(feedback.detail, /private|unsupported WAV|audio\.wav/);
  }
});

test('unclassified speaker failures do not infer a cause from raw messages or memory observations', () => {
  for (const failureKind of [undefined, null, 'unknown', '__proto__', 'constructor']) {
    const feedback = processingFeedback({ id: 'job', kind: 'diarize', status: 'failed', failureKind,
      message: 'std::bad_alloc /private/meeting/audio.wav', memoryState: 'critical' });
    assert.equal(feedback.detail, '화자를 구분하지 못했습니다. 원인을 확인하지 못했습니다. 기존 음성 기록은 유지됩니다.');
    assert.equal(feedback.warning, '');
    assert.doesNotMatch(feedback.detail, /private|메모리|bad_alloc/);
  }
});
