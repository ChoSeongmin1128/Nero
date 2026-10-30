import assert from 'node:assert/strict';
import test from 'node:test';
import { recordingQuality } from '../src/recording-quality.js';
import { readFile } from 'node:fs/promises';
import { Window } from 'happy-dom';

test('native entry markup provides every directly bound element before startup', async t => {
  const window = new Window();
  t.after(() => window.happyDOM.abort());
  window.document.body.innerHTML = await readFile(new URL('../src/index.html', import.meta.url), 'utf8');
  const main = await readFile(new URL('../src/main.js', import.meta.url), 'utf8');
  const ids = new Set([...main.matchAll(/byId\('([^']+)'\)/g)].map(match => match[1]));
  for (const id of ids) assert.ok(window.document.getElementById(id), `Missing startup element: ${id}`);
});

test('scheduling and driver delay diagnostics do not claim missing audio or require review', () => {
  assert.equal(recordingQuality({ realtimeDeniedCount: 2, events: [{ kind:'realtimeDenied' }] },16000,32000).visible,false);
  for (const quality of [{xrunCount:1, events:[{kind:'xrun',firstSavedFrame:0}]},{xrunCount:20}]) {
    const result = recordingQuality(quality,16000,32000);
    assert.equal(result.severity,'info');
    assert.equal(result.requiresAttention,false);
    assert.equal(result.visible,true);
    assert.equal(result.title,'녹음 진단');
    assert.match(result.message,/확인할 수는 없습니다/);
  }
});
test('device change asks for device selection while terminal faults explain recovery', () => {
  assert.equal(recordingQuality({routeChangedCount:1},16000,32000).severity,'warning');
  assert.equal(recordingQuality({routeChangedCount:1},16000,32000).requiresAttention,true);
  for (const kind of ['queueOverflow','writerFailure','deviceFailure','formatChanged','captureInterrupted']) {
    const result=recordingQuality({events:[{kind,firstSavedFrame:16000}]},16000,32000);
    assert.equal(result.severity,'error');
    assert.equal(result.requiresAttention,true);
    assert.match(result.message,/새 녹음/);
    assert.equal(result.locations[0].seconds,1);
  }
});
test('observed positions never synthesize a gap or seek beyond saved audio', () => {
  const result=recordingQuality({events:[{kind:'xrun',firstSavedFrame:16000,lastSavedFrame:32000,coalesced:true}]},16000,48000);
  assert.equal(result.locations[0].seconds,1);
  assert.equal('end' in result.locations[0],false);
  assert.equal(recordingQuality({events:[{kind:'deviceFailure',firstSavedFrame:50000}]},16000,48000).locations[0].seconds,null);
});
