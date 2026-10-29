import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

function call(command, args = {}) {
  if (!isTauri()) return Promise.reject(new Error('Nero 앱에서 열어 주세요.'));
  return invoke(command, args).catch(error => { throw new Error(typeof error === 'string' ? error : '일정 알림을 처리하지 못했습니다.'); });
}
export const readCalendarSettings = () => call('read_calendar_settings');
export const beginCalendarConnect = () => call('begin_calendar_connect');
export const reopenCalendarConnect = attemptId => call('reopen_calendar_connect', { attemptId });
export const finishCalendarConnect = attemptId => call('finish_calendar_connect', { attemptId });
export const cancelCalendarConnect = (attemptId = null) => call('cancel_calendar_connect', { attemptId });
export const disconnectCalendar = () => call('disconnect_calendar');
export const selectCalendars = ids => call('select_calendars', { ids });
export const syncCalendar = () => call('sync_calendar');
export const readCalendarMeetingOccurrences = ({ day, timeZone }) => call('read_calendar_meeting_occurrences', { day, timeZone });
export const createMeetingFromCalendar = (occurrenceKey, { day, timeZone, scopeRevision }) => call('create_meeting_from_calendar', { occurrenceKey, day, timeZone, scopeRevision });
export const subscribeCalendarSettings = callback => isTauri() ? listen('nero://calendar-settings', event => callback(event.payload)) : Promise.resolve(() => {});
export const readCalendarReminder = () => call('read_calendar_reminder');
export const calendarReminderReady = () => call('calendar_reminder_ready');
export const dismissCalendarReminder = () => call('dismiss_calendar_reminder');
export const startCalendarRecording = () => call('start_calendar_recording');
export const subscribeCalendarReminder = callback => isTauri() ? listen('nero://calendar-reminder', event => callback(event.payload)) : Promise.resolve(() => {});
