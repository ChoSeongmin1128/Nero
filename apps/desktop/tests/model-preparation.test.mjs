import assert from 'node:assert/strict';
import test from 'node:test';
import { formatBytes, memoryWarningFor, modelPreparation, speechModelKinds, summaryModelKinds } from '../src/processing-state.js';
import { bindPreparationProgress } from '../src/model-preparation-progress.js';

test('progress counters belong only to the job that reports them', () => {
  const progress = { id: 'summary', kind: 'summary-model', status: 'running', bytes: 50, totalBytes: 100,
    jobs: [{ id: 'speech', kind: 'setup', status: 'queued' }, { id: 'summary', kind: 'summary-model', status: 'running' }] };
  const speech = modelPreparation(progress, speechModelKinds);
  assert.equal(speech.state, 'queued');
  assert.equal(speech.percent, null);
  assert.equal(speech.size, '');
  const summary = modelPreparation(progress, summaryModelKinds);
  assert.equal(summary.state, 'running');
  assert.equal(summary.percent, 50);
  assert.equal(summary.size, '1 MB / 1 MB');
});

test('sizes, cancelling, failures and cancellations read the same everywhere', () => {
  assert.equal(formatBytes(4_400_000_000), '4.40 GB');
  assert.equal(formatBytes(612_000_000), '612 MB');
  const running = modelPreparation({ id: 'a', kind: 'setup', status: 'running', bytes: 1_100_000_000, totalBytes: 4_400_000_000, message: '음성 인식 모델 다운로드 중' }, speechModelKinds);
  assert.deepEqual([running.percent, running.size, running.stage], [25, '1.10 GB / 4.40 GB', '음성 인식 모델 다운로드 중']);
  assert.equal(modelPreparation({ jobs: [{ id: 'a', kind: 'setup', status: 'cancelling' }] }, speechModelKinds).cancellable, false);
  const failed = modelPreparation({ jobs: [{ id: 'a', kind: 'setup', status: 'failed', message: '저장 공간이 부족합니다.' }] }, speechModelKinds);
  assert.deepEqual([failed.state, failed.message], ['failed', '저장 공간이 부족합니다.']);
  const cancelled = modelPreparation({ jobs: [{ id: 'a', kind: 'setup', status: 'cancelled' }] }, speechModelKinds);
  assert.equal(cancelled.state, 'cancelled');
  assert.match(cancelled.message, /이어받습니다/);
  assert.equal(modelPreparation({ jobs: [{ id: 'a', kind: 'setup', status: 'failed', acknowledged: true }] }, speechModelKinds).state, 'idle');
  assert.equal(modelPreparation({ jobs: [{ id: 'a', kind: 'transcribe', status: 'running' }] }, speechModelKinds).state, 'idle');
});

test('memory warnings apply only to supported models that still need files', () => {
  assert.equal(memoryWarningFor({ supported: true, ready: false, memoryWarning: '경고' }), '경고');
  assert.equal(memoryWarningFor({ supported: true, ready: true, memoryWarning: '경고' }), '');
  assert.equal(memoryWarningFor({ supported: false, ready: false, memoryWarning: '경고' }), '');
});

test('the progress view writes only changed values', () => {
  const writes = [];
  const node = (name) => new Proxy({ dataset: {} }, { set(target, key, value) { writes.push(`${name}.${String(key)}`); target[key] = value; return true; } });
  const view = bindPreparationProgress({ region: node('region'), bar: node('bar'), stage: node('stage'), size: node('size'), cancel: node('cancel') });
  const running = { state: 'running', jobId: 'a', percent: 25, value: 25, max: 100, size: '25 MB / 100 MB', stage: '받는 중', cancellable: true };
  view.update(running);
  const first = writes.length;
  view.update({ ...running });
  assert.equal(writes.length, first);
  view.update({ ...running, percent: 26, value: 26 });
  assert.deepEqual(writes.slice(first).sort(), ['bar.value', 'size.textContent']);
});
