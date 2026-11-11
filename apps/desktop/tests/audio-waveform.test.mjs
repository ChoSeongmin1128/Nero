import assert from 'node:assert/strict';
import test from 'node:test';
import { Window } from 'happy-dom';
import { waveformBarHeights, localAudioUrl, parseWaveHeader, fetchAudioRange, sampleAudioWaveform, connectAudioWaveform } from '../src/audio-waveform.js';

function wav({ bits = 16, encoding = 1, frames = 128, channels = 1, fill = true } = {}) {
  const align = channels * bits / 8, bytes = new Uint8Array(44 + frames * align), view = new DataView(bytes.buffer);
  const mark = (offset, text) => [...text].forEach((char, index) => view.setUint8(offset + index, char.charCodeAt(0)));
  mark(0, 'RIFF'); view.setUint32(4, bytes.length - 8, true); mark(8, 'WAVE'); mark(12, 'fmt '); view.setUint32(16, 16, true); view.setUint16(20, encoding, true); view.setUint16(22, channels, true); view.setUint32(24, 16000, true); view.setUint32(28, 16000 * align, true); view.setUint16(32, align, true); view.setUint16(34, bits, true); mark(36, 'data'); view.setUint32(40, frames * align, true);
  if (fill) for (let frame = 0; frame < frames; frame++) for (let channel = 0; channel < channels; channel++) {
    const at = 44 + frame * align + channel * bits / 8, value = frame < frames / 2 ? .5 : 0;
    if (encoding === 3) view.setFloat32(at, value, true);
    else if (bits === 16) view.setInt16(at, value * 32768, true);
    else if (bits === 24) { const raw = value * 8388608; view.setUint8(at, raw & 255); view.setUint8(at + 1, raw >> 8 & 255); view.setUint8(at + 2, raw >> 16 & 255); }
    else if (bits === 32) view.setInt32(at, value * 2147483648, true);
  }
  return bytes;
}
const rangeFetcher = (bytes, calls = []) => async (_url, options) => {
  const [, rawStart, rawEnd] = /^bytes=(\d+)-(\d+)$/.exec(options.headers.Range), start = Number(rawStart), end = Math.min(Number(rawEnd), bytes.length - 1);
  calls.push([start, end, options]);
  return new Response(bytes.subarray(start, end + 1), { status: 206, headers: { 'Content-Range': `bytes ${start}-${end}/${bytes.length}`, 'Content-Length': end - start + 1 } });
};

test('waveform accepts only local asset URLs and no remote playback source', () => {
  assert.equal(localAudioUrl('https://example.com/meeting.wav', 'tauri://localhost'), null);
  assert.equal(localAudioUrl('blob:http://localhost/example', 'tauri://localhost'), null);
  assert.equal(localAudioUrl('http://asset.localhost/meeting.wav', 'tauri://localhost'), 'http://asset.localhost/meeting.wav');
  assert.equal(localAudioUrl('/meeting.wav', 'tauri://localhost'), 'tauri://localhost/meeting.wav');
});

test('PCM16/24/32 and float32 preview peaks reflect real samples including silence', async () => {
  for (const [bits, encoding] of [[16, 1], [24, 1], [32, 1], [32, 3]]) {
    const bytes = wav({ bits, encoding, channels: 2 });
    const peaks = await sampleAudioWaveform('http://asset.localhost/audio.wav', { fetcher: rangeFetcher(bytes) });
    assert.equal(peaks.length, 64); assert.ok(peaks.slice(0, 32).every(value => value === .5)); assert.ok(peaks.slice(32).every(value => value === 0));
  }
});

test('long recording reads bounded distributed windows rather than its full audio', async () => {
  const bytes = wav({ frames: 1024 * 1024, fill: false }), calls = [];
  await sampleAudioWaveform('http://asset.localhost/audio.wav', { fetcher: rangeFetcher(bytes, calls) });
  const requested = calls.reduce((total, [start, end]) => total + end - start + 1, 0);
  assert.ok(requested <= 1024 * 1024); assert.ok(calls.length > 1);
  assert.ok(calls.every(([start, end]) => end - start + 1 <= 256 * 1024));
  assert.ok(calls.some(([start]) => start > bytes.length * .9));
  assert.ok(calls.every(([, , options]) => options.redirect === 'error' && options.credentials === 'omit'));
});

test('partial response validation rejects full-file fallback, mismatched range and oversized body', async () => {
  for (const response of [new Response(new Uint8Array(16), { status: 200 }), new Response(new Uint8Array(16), { status: 206, headers: { 'Content-Range': 'bytes 1-16/100' } }), new Response(new Uint8Array(17), { status: 206, headers: { 'Content-Range': 'bytes 0-15/100' } })]) {
    await assert.rejects(fetchAudioRange(async () => response, 'http://asset.localhost/audio.wav', 0, 15, undefined, { bytes: 0 }));
  }
  await assert.rejects(fetchAudioRange(() => assert.fail('must not fetch'), 'http://asset.localhost/audio.wav', 0, 256 * 1024, undefined, { bytes: 0 }));
});

test('invalid WAV geometry and unsupported encodings keep preview unavailable', () => {
  const unsupported = wav({ bits: 16, encoding: 6 });
  assert.throws(() => parseWaveHeader(unsupported, unsupported.length));
  const invalid = wav(); new DataView(invalid.buffer).setUint16(32, 9, true);
  assert.throws(() => parseWaveHeader(invalid, invalid.length));
  const truncated = wav(); new DataView(truncated.buffer).setUint32(40, 100000, true);
  assert.throws(() => parseWaveHeader(truncated, truncated.length));
});

test('source replacement aborts stale waveform loading and remote source stays on slider', async t => {
  const window = new Window(), document = window.document;
  t.after(() => window.happyDOM.abort());
  document.body.innerHTML = '<audio src="http://asset.localhost/first.wav"></audio><div><div id="wave" hidden></div><input type="range"></div>';
  const original = globalThis.fetch; let signal, calls = 0;
  globalThis.fetch = async (_url, options) => { calls++; signal = options.signal; return new Promise((_resolve, reject) => options.signal.addEventListener('abort', () => reject(new DOMException('Aborted', 'AbortError')), { once: true })); };
  t.after(() => { globalThis.fetch = original; });
  const audio = document.querySelector('audio'), host = document.getElementById('wave'), seek = document.querySelector('input');
  const controller = connectAudioWaveform(audio, host, seek);
  audio.setAttribute('src', 'https://example.com/no.wav');
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(signal.aborted, true); assert.equal(calls, 1); assert.equal(host.hidden, true); assert.equal(host.parentElement.hasAttribute('data-waveform-ready'), false); assert.equal(seek.disabled, false);
  controller.dispose();
});

test('Nero extensible float WAV uses the declared subtype and rejects unrelated GUIDs', async () => {
  const original = wav({ bits: 32, encoding: 3 });
  const bytes = new Uint8Array(original.length + 24), view = new DataView(bytes.buffer);
  bytes.set(original.subarray(0, 36)); bytes.set(original.subarray(36), 60);
  view.setUint32(4, bytes.length - 8, true); view.setUint32(16, 40, true); view.setUint16(20, 0xfffe, true);
  view.setUint16(36, 22, true); view.setUint16(38, 32, true); view.setUint32(40, 1, true);
  bytes.set([3, 0, 0, 0, 0, 0, 16, 0, 128, 0, 0, 170, 0, 56, 155, 113], 44);
  const peaks = await sampleAudioWaveform('http://asset.localhost/audio.wav', { fetcher: rangeFetcher(bytes) });
  assert.ok(peaks.slice(0, 32).every(value => value === .5));
  assert.ok(peaks.slice(32).every(value => value === 0));
  bytes[59] = 0;
  assert.throws(() => parseWaveHeader(bytes, bytes.length));
});

test('quiet recordings use the available visual height without inventing sound in silence', () => {
  assert.deepEqual(waveformBarHeights([0, .0001, .0002, 0]), [2, 15, 30, 2]);
  assert.deepEqual(waveformBarHeights([0, 0]), [2, 2]);
});
