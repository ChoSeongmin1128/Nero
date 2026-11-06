import assert from 'node:assert/strict';
import test from 'node:test';
import { Window } from 'happy-dom';
import { DROP_CHUNK, mountDropImport, uploadDroppedFile } from '../src/drop-import.js';

const file = (size, name = '회의.m4a') => new File([new Uint8Array(size)], name);

test('a dropped file is streamed in bounded chunks and the finished upload starts the import', async () => {
  const calls = [];
  const bridge = {
    dropImportBegin: async (...args) => { calls.push(['begin', ...args]); return 'token'; },
    dropImportChunk: async (token, bytes) => { calls.push(['chunk', token, bytes.length]); },
    dropImportFinish: async token => { calls.push(['finish', token]); return { id: 'job', kind: 'import' }; },
    dropImportCancel: async token => { calls.push(['cancel', token]); },
  };
  const progress = [];
  const result = await uploadDroppedFile(bridge, file(DROP_CHUNK * 2 + 5), { meetingId: 'm' }, (done, total) => progress.push([done, total]));
  assert.deepEqual(result, { id: 'job', kind: 'import' });
  assert.deepEqual(calls[0], ['begin', '회의.m4a', DROP_CHUNK * 2 + 5, { meetingId: 'm' }]);
  assert.deepEqual(calls.slice(1, 4).map(call => call[2]), [DROP_CHUNK, DROP_CHUNK, 5]);
  assert.deepEqual(calls.at(-1), ['finish', 'token']);
  assert.deepEqual(progress.at(-1), [DROP_CHUNK * 2 + 5, DROP_CHUNK * 2 + 5]);
});

test('a failed chunk cancels the upload and reports the error', async () => {
  const calls = [];
  const bridge = {
    dropImportBegin: async () => 'token',
    dropImportChunk: async () => { throw new Error('저장 공간이 부족합니다.'); },
    dropImportFinish: async () => assert.fail('must not finish'),
    dropImportCancel: async token => { calls.push(token); },
  };
  await assert.rejects(uploadDroppedFile(bridge, file(10)), /저장 공간/);
  assert.deepEqual(calls, ['token']);
});

test('file drags never reach the webview default, while in-app drags are left alone', async t => {
  const window = new Window();
  t.after(() => window.happyDOM.abort());
  const overlay = window.document.createElement('div'); overlay.hidden = true;
  const message = window.document.createElement('p');
  const dropped = [];
  mountDropImport({ window, overlay, message, onDrop: async files => { dropped.push(files.map(item => item.name)); } });
  const event = (type, types, files = []) => {
    const result = new window.Event(type, { cancelable: true });
    result.dataTransfer = { types, files, dropEffect: 'none' };
    window.dispatchEvent(result);
    return result;
  };
  assert.equal(event('dragenter', ['text/plain']).defaultPrevented, false);
  assert.equal(event('dragover', ['application/x-nero-meeting']).defaultPrevented, false);
  assert.equal(overlay.hidden, true);
  assert.equal(event('dragenter', ['Files']).defaultPrevented, true);
  assert.equal(overlay.hidden, false);
  assert.match(message.textContent, /음성을 가져옵니다/);
  assert.equal(event('dragover', ['Files']).defaultPrevented, true);
  const drop = event('drop', ['Files'], [file(3)]);
  assert.equal(drop.defaultPrevented, true);
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.deepEqual(dropped, [['회의.m4a']]);
  assert.equal(overlay.hidden, true);
});
