import assert from 'node:assert/strict';
import test from 'node:test';
import { processingJobs, pendingJob, processingResults, latestMeetingProcessing, processingNotices } from '../src/processing-state.js';

test('pending ownership follows the meeting and model kind rather than the foreground download', () => {
  const progress = { id: 'download', kind: 'summary-model', status: 'running', jobs: [
    { id: 'download', kind: 'summary-model', status: 'running' },
    { id: 'speech', kind: 'transcribe', meetingId: 'meeting-a', status: 'queued' },
    { id: 'cancelled', kind: 'summary', meetingId: 'meeting-b', status: 'cancelled' },
  ] };
  assert.equal(pendingJob(progress, { meetingId: 'meeting-a' }).id, 'speech');
  assert.equal(pendingJob(progress, { meetingId: 'meeting-b' }), undefined);
  assert.equal(pendingJob(progress, { models: true }).id, 'download');
  assert.equal(processingJobs(progress).length, 3);
});

test('a background result transition remains visible when the foreground job is unchanged', () => {
  const background = { id: 'speech', kind: 'transcribe', meetingId: 'meeting-a', status: 'running' };
  const previous = { id: 'download', kind: 'model', status: 'running', jobs: [background] };
  const current = { ...previous, jobs: [{ ...background, status: 'interrupted' }] };
  assert.deepEqual(processingResults(previous, current).map(job => job.id), ['speech']);
  assert.deepEqual(processingResults(current, current), []);
});

test('initial recovery does not replay every old result and a user cancellation is never pending', () => {
  const progress = { status: 'idle', jobs: [
    { id: 'old', kind: 'summary', meetingId: 'meeting-a', status: 'completed' },
    { id: 'cancelled', kind: 'transcribe', meetingId: 'meeting-b', status: 'cancelled' },
  ] };
  assert.deepEqual(processingResults({}, progress), []);
  assert.equal(pendingJob(progress), undefined);
});

test('current metadata supplies retry eligibility without replacing running transfer fields', () => {
  const progress = { id: 'current', kind: 'model', status: 'running', bytes: 20, jobs: [
    { id: 'current', kind: 'model', status: 'running', retryable: false },
    { id: 'retry', kind: 'summary', status: 'interrupted', retryable: true },
  ] };
  assert.equal(processingJobs(progress).find(job => job.id === 'current').bytes, 20);
  assert.equal(processingJobs(progress).find(job => job.id === 'retry').retryable, true);
});

test('cancellation keeps the meeting owned until worker termination is confirmed', () => {
  const progress = { status: 'idle', jobs: [{ id: 'stopping', kind: 'summary', meetingId: 'meeting-a', status: 'cancelling' }] };
  assert.equal(pendingJob(progress, { meetingId: 'meeting-a' }).id, 'stopping');
  assert.equal(pendingJob({ ...progress, jobs: [{ ...progress.jobs[0], status: 'cancelled' }] }), undefined);
});

test('recognition becomes readable before speaker analysis and stays available on speaker failure', () => {
  const recognition = { id: 'pipeline', kind: 'transcribe', meetingId: 'meeting-a', status: 'running' };
  const speaker = { ...recognition, kind: 'diarize' };
  assert.deepEqual(processingResults(recognition, speaker), [speaker]);
  assert.deepEqual(processingResults(speaker, speaker), []);
  for (const status of ['failed', 'cancelled', 'interrupted', 'needsReview']) {
    const finished = { ...speaker, status };
    assert.deepEqual(processingResults(speaker, finished), [finished]);
    assert.deepEqual(processingResults(recognition, finished), [finished]);
    assert.deepEqual(processingResults(finished, finished), []);
  }
});

const noticeJob = (id, createdAt, status = 'cancelled', extra = {}) => ({ id, createdAt, meetingId: 'a', kind: 'summary', status, retryable: true, ...extra });

test('only the selected meeting has a terminal notice and unrelated work remains discoverable', () => {
  const stopped = noticeJob('stopped', 10);
  const active = noticeJob('other', 20, 'running', { meetingId: 'b' });
  const progress = { id: active.id, ...active, jobs: [stopped, active] };
  assert.equal(processingNotices(progress, 'a').primary.id, 'stopped');
  assert.deepEqual(processingNotices(progress, 'a').background.map(job => job.id), ['other']);
  assert.equal(processingNotices(progress).primary, undefined);
  assert.deepEqual(processingNotices(progress).background.map(job => job.id), ['other']);
});

test('new success, acknowledgement and retry handoff never reveal an earlier outcome', () => {
  const old = noticeJob('old', 10);
  for (const latest of [noticeJob('new', 20, 'completed'), noticeJob('new', 20, 'cancelled', { acknowledged: true }), noticeJob('new', 20, 'cancelled', { retrySuccessorId: 'successor' })]) {
    assert.equal(processingNotices({ jobs: [latest, old] }, 'a').primary, undefined);
  }
  const latest = noticeJob('retry', 30, 'failed');
  assert.equal(processingNotices({ jobs: [{ ...old, retrySuccessorId: 'retry' }, latest] }, 'a').primary.id, 'retry');
});

test('creation time determines the latest result independently of journal array and acknowledgement update order', () => {
  const old = noticeJob('old', 10, 'cancelled', { updatedAt: 100 });
  const latest = noticeJob('latest', 20, 'completed', { updatedAt: 20 });
  assert.equal(latestMeetingProcessing({ jobs: [old, latest] }, 'a').id, 'latest');
  assert.equal(latestMeetingProcessing({ jobs: [latest, old] }, 'a').id, 'latest');
});

test('missing or tied creation evidence does not guess a historical outcome and never hides active work', () => {
  for (const createdAt of [undefined, 10]) {
    const progress = { jobs: [noticeJob('old', 10), noticeJob('unknown', createdAt)] };
    assert.equal(processingNotices(progress, 'a').primary, undefined);
    progress.jobs.push(noticeJob('running', undefined, 'cancelling'));
    assert.equal(processingNotices(progress, 'a').primary.id, 'running');
  }
});
