import assert from 'node:assert/strict';
import test from 'node:test';
import { connectAudioPlayer, formatAudioTime } from '../src/audio-player.js';

function fixture() {
  class Element {
    constructor() { this.events = new Map(); this.attributes = new Map(); this.value = ''; this.textContent = ''; }
    addEventListener(name, fn) { const handlers = this.events.get(name) || []; handlers.push(fn); this.events.set(name, handlers); }
    async emit(name) { for (const fn of this.events.get(name) || []) await fn(); }
    setAttribute(name, value) { this.attributes.set(name, value); }
    getAttribute(name) { return this.attributes.get(name); }
  }
  const nodes = Object.fromEntries(['audio-player', 'audio-toggle', 'audio-seek', 'audio-mute', 'audio-speed', 'audio-player-status', 'audio-current', 'audio-duration', 'audio-toggle-label', 'audio-toggle-icon', 'inspector-audio-toggle', 'inspector-audio-status'].map(id => [id, new Element()]));
  const audio = nodes['audio-player'];
  Object.assign(audio, { readyState: 0, paused: true, ended: false, error: null, duration: NaN, currentTime: 0, playbackRate: 1, muted: false });
  audio.play = async () => { audio.paused = false; await audio.emit('play'); };
  audio.pause = () => { audio.paused = true; audio.emit('pause'); };
  connectAudioPlayer({ getElementById: id => nodes[id] }, name => `<svg>${name}</svg>`);
  return { nodes, audio, get: id => nodes[`audio-${id}`], async load(seconds = 150) {
    audio.setAttribute('src', 'mock-audio.wav'); await audio.emit('loadstart');
    audio.readyState = 1; audio.duration = seconds; audio.error = null; await audio.emit('loadedmetadata');
  } };
}

test('loading and replacing audio reset times and disable seeking until metadata is available', async () => {
  const f = fixture();
  assert.equal(f.get('toggle').disabled, true);
  await f.load();
  assert.equal(f.get('duration').textContent, '2:30');
  f.audio.currentTime = 70; await f.audio.emit('timeupdate');
  assert.equal(f.get('current').textContent, '1:10');
  f.audio.readyState = 0; await f.audio.emit('emptied');
  assert.equal(f.get('current').textContent, '0:00');
  assert.equal(f.get('duration').textContent, '0:00');
  assert.equal(f.get('seek').disabled, true);
  await f.load(Infinity);
  assert.equal(f.get('seek').disabled, true);
  assert.equal(f.get('toggle').disabled, false);
});

test('playback, transcript seeking, speed and mute track the audio element', async () => {
  const f = fixture(); await f.load();
  await f.get('toggle').emit('click');
  assert.equal(f.audio.paused, false);
  assert.equal(f.get('toggle-label').textContent, '일시정지');
  await f.get('toggle').emit('click');
  assert.equal(f.audio.paused, true);
  f.get('seek').value = '45.5'; await f.get('seek').emit('input');
  assert.equal(f.audio.currentTime, 45.5);
  f.audio.currentTime = 80; await f.audio.emit('seeked');
  assert.equal(f.get('seek').value, '80');
  assert.equal(f.get('seek').getAttribute('aria-valuetext'), '1분 20초 / 2분 30초');
  f.get('speed').value = '1.5'; await f.get('speed').emit('change');
  assert.equal(f.audio.playbackRate, 1.5);
  await f.get('mute').emit('click');
  assert.equal(f.audio.muted, true);
  assert.equal(f.get('mute').getAttribute('aria-pressed'), 'true');
  assert.equal(f.get('mute').getAttribute('aria-label'), '음소거 해제');
  f.audio.ended = true; f.audio.paused = true; await f.audio.emit('ended');
  assert.equal(f.get('toggle-label').textContent, '다시 재생');
  await f.get('toggle').emit('click');
  assert.equal(f.audio.currentTime, 0);
});

test('failed play has Korean feedback and a rejected old request cannot affect a new source', async () => {
  const f = fixture(); await f.load();
  f.audio.play = async () => { throw Error('unsupported'); };
  await f.get('toggle').emit('click');
  assert.equal(f.get('player-status').hidden, false);
  assert.match(f.get('player-status').textContent, /재생을 시작하지 못했습니다/);
  let reject;
  f.audio.play = () => new Promise((_, fail) => { reject = fail; });
  const pending = f.get('toggle').emit('click');
  await f.load(60); reject(Error('stale')); await pending;
  assert.equal(f.get('player-status').hidden, true);
  f.audio.error = Error('decode'); await f.audio.emit('error');
  assert.equal(f.get('toggle').disabled, true);
  assert.equal(f.get('seek').disabled, true);
  assert.equal(f.get('duration').textContent, '0:00');
});

test('time formatting supports long recordings and unknown duration', () => {
  assert.equal(formatAudioTime(3661.8), '1:01:01');
  assert.equal(formatAudioTime(NaN), '0:00');
  assert.equal(formatAudioTime(Infinity), '0:00');
  assert.equal(formatAudioTime(-1), '0:00');
});

test('compact inspector playback shares media state and can pause playback started from a citation', async () => {
  const f = fixture(), compact = f.nodes['inspector-audio-toggle'];
  assert.equal(compact.disabled, true); await f.load(); assert.equal(compact.disabled, false);
  f.audio.currentTime = 23; await f.audio.play();
  assert.equal(compact.textContent, '일시정지');
  await compact.emit('click'); assert.equal(f.audio.paused, true); assert.equal(f.audio.currentTime, 23); assert.equal(f.get('toggle-label').textContent, '재생');
  await compact.emit('click'); assert.equal(f.audio.paused, false); assert.equal(f.get('toggle-label').textContent, '일시정지');
  f.audio.readyState = 0; await f.audio.emit('emptied'); assert.equal(compact.disabled, true);
});
test('compact playback shows the same failed-play feedback and clears it on a source replacement', async () => {
  const f = fixture(); await f.load(); f.audio.play = async () => { throw Error('unsupported'); };
  await f.nodes['inspector-audio-toggle'].emit('click'); assert.equal(f.nodes['inspector-audio-status'].textContent, f.get('player-status').textContent); assert.equal(f.nodes['inspector-audio-status'].hidden, false);
  await f.load(60); assert.equal(f.nodes['inspector-audio-status'].hidden, true);
});
