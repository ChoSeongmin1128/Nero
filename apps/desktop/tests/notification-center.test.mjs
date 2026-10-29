import assert from 'node:assert/strict';
import test from 'node:test';
import { Window } from 'happy-dom';
import { mountNotificationCenter, notificationProjection } from '../src/notification-center.js';

const meetings = [{ id: 'a', title: '회의 A', deletedAt: null }, { id: 'b', title: '회의 B', deletedAt: null }];
const job = (id, meetingId = 'a', status = 'failed', createdAt = 10, extra = {}) => ({ id, meetingId, kind: 'transcribe', status, createdAt, acknowledged: false, retryable: true, ...extra });
const settle = () => new Promise(resolve => setImmediate(resolve));

function harness(t, options = {}) {
  const window = new Window();
  t.after(() => window.happyDOM.abort());
  const container = window.document.createElement('section');
  container.className = 'notification-center';
  window.document.body.append(container);
  const calls = [], errors = [];
  const controller = mountNotificationCenter({ container, showError: error => errors.push(error),
    onOpenMeeting: async id => { calls.push(['open', id]); }, ...options });
  const update = (jobs, revision = 1, extra = {}) => controller.update({ connected: true, meetings,
    processing: { revision, jobs }, ...extra });
  const button = (id, action) => [...container.querySelectorAll('[data-notification-action]')]
    .find(node => node.dataset.notificationJob === id && node.dataset.notificationAction === action);
  const click = async (id, action) => { button(id, action)?.click(); await settle(); };
  return { window, container, controller, calls, errors, update, button, click };
}

test('notifications use the actual latest meeting jobs and never restore a predecessor after success or acknowledgement', () => {
  const old = job('old');
  const projectedSuccess = notificationProjection({ meetings, processing: { jobs: [job('new', 'a', 'completed', 20), old] } });
  assert.deepEqual(projectedSuccess.map(item => item.id), ['new']);
  assert.equal(projectedSuccess[0].unread, true);
  assert.deepEqual(notificationProjection({ meetings, processing: { jobs: [job('new', 'a', 'cancelled', 20, { acknowledged: true }), old] } }), []);
  const retry = job('retry', 'a', 'queued', 20);
  const projected = notificationProjection({ meetings, processing: { jobs: [{ ...old, retrySuccessorId: 'retry' }, retry] } });
  assert.deepEqual(projected.map(item => item.id), ['retry']);
});

test('all live work stays visible across meetings and the current meeting never replaces another job identity', () => {
  const projected = notificationProjection({ meetings, selectedMeetingId: 'a', processing: {
    id: 'active-a', status: 'running', kind: 'transcribe', phase: 'diarizing', revision: 4,
    jobs: [job('active-a', 'a', 'running', 20), job('queued-a', 'a', 'queued', 30, { kind: 'summary' }),
      job('active-b', 'b', 'running', 15), job('older-a', 'a', 'failed', 5)],
  } });
  assert.deepEqual(projected.map(item => item.id), ['queued-a', 'active-a', 'active-b']);
  assert.equal(projected.find(item => item.id === 'active-a').feedback.title, '화자 구분 중');
  assert.equal(projected.find(item => item.id === 'active-b').feedback.title, '음성 기록 변환 중');
  assert.equal(projected.find(item => item.id === 'active-b').selected, false);
  assert.equal(projected.find(item => item.id === 'active-a').selected, true);
});

test('trashed and unavailable meetings do not expose notices, ambiguous old journals do not guess the latest outcome', () => {
  const projected = notificationProjection({ meetings: [{ ...meetings[0], deletedAt: 0 }], processing: {
    jobs: [job('a'), job('b', 'b'), job('missing', 'missing')],
  } });
  assert.deepEqual(projected, []);
  assert.deepEqual(notificationProjection({ meetings, processing: { jobs: [job('one', 'a', 'failed', 0), job('two', 'a', 'cancelled', 0)] } }), []);
  assert.equal(notificationProjection({ meetings, processing: { jobs: [job('one', 'a', 'failed', 10), job('two', 'a', 'cancelled', 10)] } }).length, 0);
});

test('global preparation notices keep their own identity and expose only measured model percentages', () => {
  const projected = notificationProjection({ meetings, processing: {
    id: 'model', kind: 'summary-model', status: 'running', totalBytes: 1000, bytes: 400,
    jobs: [job('old-model', null, 'failed', 10, { kind: 'summary-model' }), job('model', null, 'running', 20, { kind: 'summary-model' })],
  } });
  assert.deepEqual(projected.map(item => item.id), ['model']);
  assert.equal(projected[0].feedback.progress.percent, 40);
  assert.equal(projected[0].hasMeeting, false);
});

test('the actual bridge receives the row job ID and opening a notification opens its own meeting', async t => {
  const calls = [];
  const app = harness(t, { bridge: { retryProcessing: async id => {
    calls.push(['retry', id]);
    return { revision: 3, jobs: [job('a'), job('b-next', 'b', 'running', 30), job('b', 'b', 'failed', 20, { retrySuccessorId: 'b-next', acknowledged: true })] };
  } } });
  app.update([job('a'), job('b', 'b', 'failed', 20)], 2, { selectedMeetingId: 'a' });
  await app.click('b', 'open');
  assert.deepEqual(app.calls, [['open', 'b']]);
  await app.click('b', 'retry');
  assert.deepEqual(calls, [['retry', 'b']]);
  assert.equal(app.button('b', 'retry'), undefined);
  assert.ok(app.button('b-next', 'cancel'));
  assert.ok(app.button('a', 'retry'));
});

test('a detached old button cannot retry an earlier attempt or acknowledge a deleted meeting', async t => {
  const calls = [];
  const app = harness(t, { onRetry: async id => calls.push(['retry', id]), onAcknowledge: async id => calls.push(['ack', id]) });
  app.update([job('old')]);
  const stale = app.button('old', 'retry');
  app.update([job('old', 'a', 'failed', 10, { retrySuccessorId: 'new' }), job('new', 'a', 'running', 20)], 2);
  app.container.append(stale);
  stale.click();
  await settle();
  assert.deepEqual(calls, []);
  app.update([job('old')], 3);
  const ack = app.button('old', 'ack');
  app.update([job('old')], 4, { meetings: [] });
  app.container.append(ack);
  ack.click();
  await settle();
  assert.deepEqual(calls, []);
});

test('acknowledgement stores through the existing bridge and removes only the acknowledged notice', async t => {
  const calls = [];
  const first = job('first'), active = job('active', 'b', 'running', 20);
  const app = harness(t, { bridge: { acknowledgeProcessing: async id => {
    calls.push(id);
    return { revision: 2, jobs: [{ ...first, acknowledged: true }, active] };
  } } });
  app.update([first, active]);
  await app.click('first', 'ack');
  assert.deepEqual(calls, ['first']);
  assert.equal(app.button('first', 'ack'), undefined);
  assert.ok(app.button('active', 'cancel'));
  assert.deepEqual(calls, ['first']);
  assert.equal(app.controller.counts().total, 1);
  app.update([{ ...first, acknowledged: true }, active], 3);
  assert.equal(app.controller.counts().total, 1);
});

test('cancellation remains disabled until the backend reports its result and never stops another row', async t => {
  const calls = [];
  const app = harness(t, { bridge: { cancelProcessing: async id => { calls.push(id); } } });
  const first = job('first', 'a', 'running'), second = job('second', 'b', 'running');
  app.update([first, second]);
  await app.click('first', 'cancel');
  assert.equal(app.button('first', 'cancel').disabled, true);
  assert.equal(app.button('second', 'cancel').disabled, false);
  await app.click('first', 'cancel');
  assert.deepEqual(calls, ['first']);
  app.update([{ ...first, status: 'cancelled' }, second], 2);
  assert.ok(app.button('first', 'retry'));
});

test('stale snapshots and late retry responses cannot replace newer processing or selection', async t => {
  let resolve;
  const accepted = [];
  const app = harness(t, { onRetry: () => new Promise(done => { resolve = done; }), onAcceptProcessing: value => accepted.push(value) });
  app.update([job('old')], 2);
  app.button('old', 'retry').click();
  app.update([job('new', 'b', 'running', 30)], 4, { selectedMeetingId: 'b' });
  resolve({ revision: 3, jobs: [job('old-next', 'a', 'running', 20)] });
  await settle();
  assert.ok(app.button('new', 'cancel'));
  assert.equal(app.button('old-next', 'cancel'), undefined);
  assert.deepEqual(accepted, []);
  app.update([job('older')], 1);
  assert.ok(app.button('new', 'cancel'));
});

test('retry failure stays on the correct row and a shared action lock blocks duplicate requests', async t => {
  const app = harness(t, { onRetry: async () => { throw new Error('<실패>'); } });
  app.update([job('first'), job('second', 'b')], 1, { actions: new Map([['second', 'retry']]) });
  assert.equal(app.button('second', 'retry').disabled, true);
  await app.click('first', 'retry');
  assert.equal(app.container.querySelector('[data-notification-id="first"] [role="alert"]').textContent, '<실패>');
  assert.equal(app.container.querySelector('[data-notification-id="second"] [role="alert"]'), null);
  assert.equal(app.button('first', 'retry').disabled, false);
  assert.equal(app.errors.length, 1);
});

test('focus follows a retry successor and acknowledging the final notice moves focus to the heading', async t => {
  const app = harness(t, { bridge: { retryProcessing: async () => ({ revision: 2,
    jobs: [job('old', 'a', 'failed', 10, { retrySuccessorId: 'new' }), job('new', 'a', 'running', 20)] }) } });
  app.update([job('old')]);
  app.button('old', 'retry').focus();
  await app.click('old', 'retry');
  assert.equal(app.window.document.activeElement.dataset.notificationJob, 'new');
  app.update([job('new', 'a', 'cancelled', 20, { acknowledged: true })], 3);
  assert.equal(app.window.document.activeElement.tagName, 'H1');
});

test('old completed work is not injected at startup and newly observed completion can be acknowledged', async t => {
  const calls = [];
  const app = harness(t, { bridge: { acknowledgeProcessing: async id => {
    calls.push(id);
    return { revision: 4, jobs: [job('done', 'a', 'completed', 30, { acknowledged: true })] };
  } } });
  app.update([job('old', 'a', 'completed')]);
  assert.equal(app.controller.counts().total, 0);
  app.update([job('old', 'a', 'completed'), job('done', 'a', 'running', 30)], 2);
  app.update([job('old', 'a', 'completed'), job('done', 'a', 'completed', 30)], 3);
  assert.equal(app.controller.counts().unread, 1);
  assert.ok(app.button('done', 'open'));
  assert.ok(app.button('done', 'ack'));
  await app.click('done', 'ack');
  assert.deepEqual(calls, ['done']);
  assert.equal(app.controller.counts().total, 0);
});

test('notification titles and failure details render as text', t => {
  const app = harness(t);
  app.update([job('failed', 'a', 'failed', 10, { message: '<script>bad()</script>' })], 1, { meetings: [{ id: 'a', title: '<img src=x onerror=bad()>', deletedAt: null }] });
  assert.equal(app.container.querySelector('img'), null);
  assert.equal(app.container.querySelector('script'), null);
  assert.equal(app.controller.counts().unread, 1);
  assert.match(app.container.textContent, /<script>bad\(\)<\/script>/);
});

test('all confirmation runs serially, retains a failure and rechecks newer attempts before their turn', async t => {
  let release;
  const calls = [], accepted = [];
  const first = job('first'), second = job('second', 'b', 'failed', 20);
  const model = job('model', null, 'failed', 30, { kind: 'summary-model' });
  let app;
  app = harness(t, { bridge: { acknowledgeProcessing: async id => {
    calls.push(id);
    if (id === 'model') { await new Promise(resolve => { release = resolve; }); throw new Error('확인 실패'); }
    return { revision: 3, jobs: [{ ...first, acknowledged: true }, second, model, job('successor', 'b', 'running', 40)] };
  } }, onAcceptProcessing: value => accepted.push(value) });
  app.update([first, second, model]);
  app.container.querySelector('[data-notification-action="ack-all"]').click();
  await settle();
  assert.deepEqual(calls, ['model']);
  assert.equal(app.container.querySelector('[data-notification-action="ack-all"]').disabled, true);
  app.update([first, { ...second, retrySuccessorId: 'successor' }, model, job('successor', 'b', 'running', 40)], 2);
  release(); await settle(); await settle();
  assert.deepEqual(calls, ['model', 'first']);
  assert.ok(app.button('model', 'ack'));
  assert.ok(app.button('successor', 'cancel'));
  assert.equal(app.button('first', 'ack'), undefined);
  assert.equal(app.errors.length, 1);
  assert.equal(accepted.length, 1);
});

test('destroyed notifications ignore pending results and release their event listener', async t => {
  let resolve;
  const accepted = [];
  const app = harness(t, { onRetry: () => new Promise(done => { resolve = done; }), onAcceptProcessing: value => accepted.push(value) });
  app.update([job('old')]);
  app.button('old', 'retry').click();
  app.controller.destroy();
  resolve({ revision: 2, jobs: [job('new', 'a', 'running')] });
  await settle();
  assert.deepEqual(accepted, []);
});

test('reading failure details survives another job progress update', t => {
  const view = harness(t);
  const failed = job('failure', 'a', 'failed', 10, { kind: 'summary', failureKind: 'outputLimit', failureDetail: { stage: 'merge', finishReason: 'length', inputTokens: 4000, outputLimit: 2000, generatedTokens: 2000 } });
  view.update([failed, job('live', 'b', 'running', 11)]);
  const details = view.container.querySelector('details[data-notification-diagnostics]');
  assert.ok(details); details.open = true;
  view.update([failed, job('live', 'b', 'running', 11, { phase: 'saving' })], 2);
  assert.equal(view.container.querySelector('details[data-notification-diagnostics]').open, true);
});
