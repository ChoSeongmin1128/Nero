import test from 'node:test';
import assert from 'node:assert/strict';
import { liveTranscriptRows } from '../src/live-transcript.js';
const view = words => ({ meetingId: 'm', sessionId: 's', words });
test('interim text has no invented timestamps or speaker identities and revisions replace it', () => {
  const preview = { ...view([]), text:'수정 가능한 초안' };
  assert.deepEqual(liveTranscriptRows(preview,'m','s'), [{start:null,end:null,text:preview.text,speaker:null}]);
  assert.equal(liveTranscriptRows({...preview,text:'경계를 수정한 초안'},'m','s')[0].text,'경계를 수정한 초안');
});
test('another capture cannot replace the current recording preview', () => {
  assert.deepEqual(liveTranscriptRows(view([{ word: 'hello', start: 0, end: 1 }]), 'm', 'other'), []);
});
test('a revised native hypothesis replaces the boundary rather than appending a duplicate', () => {
  const first = liveTranscriptRows(view([{ word: '이전', start: 0, end: 1 }]), 'm', 's');
  const next = liveTranscriptRows(view([{ word: '수정.', start: 0, end: 1 }, { word: '다음.', start: 2, end: 3 }]), 'm', 's');
  assert.equal(first[0].text, '이전');
  assert.deepEqual(next.map(row => row.text), ['수정.', '다음.']);
});
test('speaker changes and silence retain separate turns; unknown stays unknown', () => {
  const rows = liveTranscriptRows(view([
    { word: '첫', start: 0, end: 1, speaker: 2 }, { word: '문장', start: 1, end: 2, speaker: 2 },
    { word: '다른', start: 2, end: 3, speaker: 4 }, { word: '미지정', start: 4, end: 5, speaker: 99 },
    { word: '잘못됨', start: -1, end: 1 },
  ]), 'm', 's');
  assert.deepEqual(rows.map(row => [row.text, row.speaker]), [['첫 문장', 2], ['다른', 4], ['미지정', null]]);
});
