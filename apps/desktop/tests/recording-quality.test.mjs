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

test('scheduling diagnostics alone do not create a user warning', () => {
  assert.deepEqual(recordingQuality({ realtimeDeniedCount: 2, events: [{ kind:'realtimeDenied',firstSavedFrame:1600 }] }, 16000, 32000), { visible:false,locations:[] });
});
test('quality locations use observed stored frames and never synthesize a gap duration', () => {
  const result = recordingQuality({ xrunCount:2,events:[{kind:'xrun',firstSavedFrame:16000,lastSavedFrame:32000,coalesced:true}] },16000,48000);
  assert.deepEqual(result,{ visible:true,locations:[{kind:'xrun',label:'입력 상태 변화',seconds:1,grouped:true}] });
  assert.equal('end' in result.locations[0],false);
});
test('old counters remain visible without an invented audio position', () => {
  assert.deepEqual(recordingQuality({ xrunCount:3 },16000,48000),{visible:true,locations:[]});
  assert.equal(recordingQuality({events:[{kind:'deviceFailure',firstSavedFrame:50000}]},16000,48000).locations[0].seconds,null);
});
