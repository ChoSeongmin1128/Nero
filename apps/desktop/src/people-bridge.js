import { invoke, isTauri } from '@tauri-apps/api/core';

function call(command, args) {
  if (!isTauri()) return Promise.reject(new Error('Nero 앱에서 열어 주세요.'));
  return invoke(command, args).catch(error => { throw new Error(typeof error === 'string' ? error : '참석자 정보를 처리하지 못했습니다.'); });
}
export const readPeopleCatalog = () => call('people_catalog');
export const replacePeopleCatalog = (expectedVersion, people, groups) => call('replace_people_catalog', { expectedVersion, people, groups });
export const readGoogleAttendeeRoster = id => call('google_attendee_roster', { id });
export const fetchGoogleAttendees = (id, includeMeet = true) => call('fetch_google_attendees', { id, includeMeet });
export const applyMeetingAttendeeDraft = (id, expectedVersion, draft) => call('apply_meeting_attendee_draft', { id, expectedVersion, draft });
export const beginMeetConnect = () => call('begin_meet_connect');
