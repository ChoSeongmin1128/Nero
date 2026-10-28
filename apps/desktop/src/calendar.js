import * as bridge from './calendar-bridge.js';
import { reminderCountdown } from './calendar-settings-flow.js';

const byId = id => document.getElementById(id);
let event = null;
let busy = false;
let error = '';
function render() {
  byId('calendar-title').textContent = error ? '일정 알림 확인 필요' : event?.title || '일정 알림';
  byId('calendar-title').title = error || [event?.title, event?.calendarName].filter(Boolean).join('\n');
  byId('calendar-countdown').textContent = event ? reminderCountdown(event.startAt) : '';
  byId('calendar-record').disabled = busy || !event || event.startAt <= Date.now();
  byId('calendar-record').textContent = busy ? '확인 중' : '녹음 시작';
  byId('calendar-dismiss').disabled = busy;
  byId('calendar-error').textContent = error;
}
async function run(action) {
  if (busy) return;
  busy = true; error = ''; render();
  try { await action(); } catch (failure) { error = failure?.message || '일정 알림을 처리하지 못했습니다.'; }
  finally { busy = false; render(); }
}
byId('calendar-record').addEventListener('click', () => { void run(() => bridge.startCalendarRecording()); });
byId('calendar-dismiss').addEventListener('click', () => { void run(() => bridge.dismissCalendarReminder()); });
document.addEventListener('keydown', key => { if (key.key === 'Escape' && !key.isComposing) { key.preventDefault(); void run(() => bridge.dismissCalendarReminder()); } });
await bridge.subscribeCalendarReminder(value => { event = value; render(); });
try { event = await bridge.readCalendarReminder(); render(); if (event) await bridge.calendarReminderReady(); }
catch (failure) { error = failure?.message || '일정 알림을 불러오지 못했습니다.'; render(); }
setInterval(render, 1000);
