import * as bridge from './bridge.js';
import { duration, recordingStatus, icon, ui, applySystemAppearance } from './ui.js';
import { recordingQuality } from './recording-quality.js';

applySystemAppearance();
const byId = (id) => document.getElementById(id);
let snapshot = { meetings: [], session: null };
let busy = false;
let controlsKey = '';
let lastMeetingId = null;
let localError = '';

byId('hide-recorder').innerHTML = icon('x');

function control(text, action, iconName, className = '') {
  return ui.button({ html: icon(iconName), action, className: `desktop-button ${className}`, disabled: busy, attrs: { title: text, 'aria-label': text } });
}

function render() {
  const session = snapshot.session;
  if (session?.meetingId) lastMeetingId = session.meetingId;
  const meeting = snapshot.meetings.find((item) => item.id === lastMeetingId);
  const active = ['recording', 'paused', 'finalizing'].includes(session?.state);
  const phase = active ? session.state : meeting?.status || 'idle';
  const error = localError || session?.error || meeting?.error || '';
  const quality = session?.captureQuality || meeting?.captureQuality;
  const review = recordingQuality(quality, session?.sampleRate || meeting?.audio?.sampleRate, session?.savedFrames ?? meeting?.audio?.frames);
  const warning = review.visible ? (review.requiresAttention ? review.message : '') : session?.warning || meeting?.warning || '';
  const status = error ? '녹음 확인 필요' : warning ? (review.requiresAttention ? review.title : '녹음 확인 필요') : recordingStatus(phase);
  const opener = byId('open-meeting');
  opener.disabled = !meeting || busy;
  opener.title = [meeting?.title || '녹음 대기', status, session?.deviceName, error || warning].filter(Boolean).join('\n');
  opener.setAttribute('aria-label', `${meeting?.title || '회의'} 열기, ${status}`);
  const stateMark = byId('recorder-status');
  stateMark.className = `desktop-overlay-state is-${error || warning ? 'error' : phase}`;
  stateMark.setAttribute('aria-label', status);
  const mark = error || warning ? icon('triangle-alert') : '<span class="desktop-state-dot" aria-hidden="true"></span>';
  if (stateMark.innerHTML !== mark) stateMark.innerHTML = mark;
  byId('recorder-time').textContent = error ? '오류 확인' : active ? duration(session.elapsedMs) : meeting?.audio?.sampleRate ? duration(meeting.audio.frames / meeting.audio.sampleRate * 1000) : '녹음 대기';
  byId('recorder-error').textContent = error || warning;
  const key = `${session?.id}:${phase}:${busy}:${Boolean(meeting)}`;
  if (key !== controlsKey) {
    controlsKey = key;
    byId('recorder-controls').innerHTML = active && phase !== 'finalizing'
      ? control(phase === 'paused' ? '계속 녹음' : '일시정지', phase === 'paused' ? 'resume' : 'pause', phase === 'paused' ? 'play' : 'pause') + control('녹음 종료', 'stop', 'square', 'is-stop')
      : meeting ? control('회의 열기', 'open', 'arrow-right') : '';
  }
}

function showError(error) {
  localError = error instanceof Error ? error.message : String(error);
  render();
}

function acceptSnapshot(value) {
  if (value.revision != null && snapshot.revision != null && value.revision < snapshot.revision) return;
  snapshot = value;
  render();
}

async function openMeeting() {
  const meeting = snapshot.meetings.find((item) => item.id === lastMeetingId);
  await bridge.openMeeting(lastMeetingId, localError || snapshot.session?.error || meeting?.error || null);
}

async function run(action) {
  if (busy) return;
  const focusedAction = document.activeElement?.dataset.action;
  busy = true;
  render();
  try { await action(); localError = ''; } catch (error) { showError(error); } finally {
    busy = false;
    render();
    if (document.activeElement === document.body && focusedAction) {
      const nextAction = focusedAction === 'pause' ? 'resume' : focusedAction === 'resume' ? 'pause' : focusedAction === 'stop' ? 'open' : focusedAction;
      document.querySelector(`[data-action="${nextAction}"]`)?.focus({ preventScroll: true });
    }
  }
}

byId('recorder-drag').addEventListener('mousedown', event => { if (event.button === 0) { event.preventDefault(); void bridge.dragRecorder().catch(showError); } });
byId('open-meeting').addEventListener('click', () => run(openMeeting));
byId('hide-recorder').addEventListener('click', () => run(() => bridge.hideRecorder()));
byId('recorder-controls').addEventListener('click', (event) => {
  const action = event.target.closest('[data-action]')?.dataset.action;
  const sessionId = snapshot.session?.id;
  if (!action) return;
  run(async () => {
    if (action === 'open') return openMeeting();
    if (!sessionId) return;
    if (action === 'pause') acceptSnapshot(await bridge.pauseRecording(sessionId));
    if (action === 'resume') acceptSnapshot(await bridge.resumeRecording(sessionId));
    if (action === 'stop') acceptSnapshot(await bridge.stopRecording(sessionId));
  });
});

try {
  await bridge.subscribeSnapshots(acceptSnapshot);
  acceptSnapshot(await bridge.readSnapshot());
} catch (error) { showError(error); }
render();
