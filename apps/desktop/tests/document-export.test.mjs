import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';

const source = await readFile(new URL('../src/ui.js', import.meta.url), 'utf8');
const context = vm.createContext({ GroveUI: { escape: String } });
const documentExport = vm.runInContext(`(() => { ${source.replace(/^export /gm, '')}; return documentExport; })()`, context);
const meeting = { title: '분기/검토: 회의' };
const transcript = { segments: [{ start: 65, text: '오늘 결정은 [보류]입니다.', originalText: '수정 전 원문' }, { start: 3601, text: '다음 회의에서 다시 확인합니다.' }] };

test('TXT export preserves Korean corrections, times and line breaks without exposing the model original', () => {
  const output = documentExport(meeting, transcript, null, 'transcript', 'txt');
  assert.equal(output.name, '분기_검토_ 회의-음성 기록.txt');
  assert.match(output.text, /\[01:05\] 오늘 결정은 \[보류\]입니다\./);
  assert.match(output.text, /\[1:00:01\]/);
  assert.doesNotMatch(output.text, /수정 전 원문/);
});

test('Markdown export keeps utterance text literal and summary export uses its own content', () => {
  const output = documentExport(meeting, transcript, null, 'transcript', 'md');
  assert.match(output.text, /^# 분기\/검토: 회의\n\n## 음성 기록/);
  assert.ok(output.text.includes('\\[보류\\]'));
  const summary = documentExport(meeting, transcript, { document: { text: '## 결정\n보류 [1]' }, stale: true }, 'summary', 'md');
  assert.match(summary.name, /회의록\.md$/);
  assert.match(summary.text, /이전 내용으로 만든 요약/);
  assert.match(summary.text, /## 결정\n보류 \[1\]/);
  assert.doesNotMatch(summary.text, /다음 회의에서/);
});

test('empty results and unsupported formats fail before a save dialog is requested', () => {
  assert.throws(() => documentExport(meeting, null, null, 'transcript', 'txt'), /음성 기록이 없습니다/);
  assert.throws(() => documentExport(meeting, null, null, 'summary', 'md'), /요약이 없습니다/);
  assert.throws(() => documentExport(meeting, transcript, null, 'transcript', 'html'), /지원하지 않는/);
});

test('voice export includes the current speaker names without exposing original model labels', () => {
  const doc = { speakers: [{id:'S1',name:'진행자 [수정]'}], segments:[{start:10,speakerId:'S1',text:'현재 내용',originalText:'수정 전'}] };
  assert.match(documentExport(meeting,doc,null,'transcript','txt').text,/\[00:10 진행자 \[수정\]\] 현재 내용/);
  assert.ok(documentExport(meeting,doc,null,'transcript','md').text.includes('진행자 \\[수정\\]'));
});
