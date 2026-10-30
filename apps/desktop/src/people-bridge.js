import { invoke, isTauri } from '@tauri-apps/api/core';

function call(command, args) {
  if (!isTauri()) return Promise.reject(new Error('Nero 앱에서 열어 주세요.'));
  return invoke(command, args).catch(error => { throw new Error(typeof error === 'string' ? error : '참석자 정보를 처리하지 못했습니다.'); });
}
export const readPeopleCatalog = () => call('people_catalog');
export const replacePeopleCatalog = (expectedVersion, people, groups) => call('replace_people_catalog', { expectedVersion, people, groups });
export const readMeetingAttendees = meetingId => call('meeting_attendees', { id: meetingId });
export const replaceMeetingAttendees = (meetingId, expectedVersion, attendees) => call('replace_meeting_attendees', { id: meetingId, expectedVersion, attendees });
export const promoteMeetingAttendee = (meetingId, expectedVersion, expectedCatalogVersion, attendeeId, name) => call('promote_meeting_attendee', { id: meetingId, expectedVersion, expectedCatalogVersion, attendeeId, name });
