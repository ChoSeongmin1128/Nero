import assert from 'node:assert/strict';
import test from 'node:test';
import { processingFeedback } from '../src/processing-feedback.js';

const running = (kind, extra = {}) => ({ id: 'job', kind, status: 'running', ...extra });

test('recognition and summary stay visibly active without inventing an inference percentage', () => {
  const recognition = processingFeedback(running('transcribe', { message: '텍스트로 변환 중' }));
  assert.equal(recognition.visible, true);
  assert.equal(recognition.active, true);
  assert.equal(recognition.title, '음성 기록 변환 중');
  assert.equal(recognition.detail, '');
  assert.equal(recognition.progress, null);
  assert.equal(recognition.cancelVisible, true);
  const summary = processingFeedback(running('summary', { message: '요약 중 (12/80)', totalBytes: 80, bytes: 12 }));
  assert.equal(summary.title, '회의록 요약 중');
  assert.equal(summary.detail, '요약 중 (12/80)');
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

test('OS memory signals show an independent warning without replacing the active stage', () => {
  for (const memoryState of ['warning', 'critical']) {
    const job = Object.freeze(running('summary', { memoryState, message: '요약 모델을 불러오고 있습니다.', modelReady: true, summaryReady: true }));
    const feedback = processingFeedback(job);
    assert.match(feedback.warning, /시스템 메모리 경고/);
    assert.equal(feedback.detail, '요약 모델을 불러오고 있습니다.');
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
    assert.equal(feedback.detail, '요약 실행기가 종료되었습니다.');
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
    assert.equal(feedback.title, '회의록 요약 중');
    assert.equal(feedback.detail, '회의록을 저장하고 있습니다.');
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
    assert.notEqual(feedback.warning, '');
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
  assert.equal(saving.detail, '음성 기록 저장 중');
  assert.equal(saving.progress, null);
});
