import { restoreNotificationFlow } from '../notification-flow.mjs';

export function readStoredSnapshot(value, initialFolders) {
  if (!value || (value.version !== undefined && value.version !== 1) || !Array.isArray(value.meetings) || !value.meetings.length) return null;
  const ids = new Set();
  if (!value.meetings.every(meeting => {
    if (!meeting || !Number.isSafeInteger(meeting.id) || ids.has(meeting.id) || typeof meeting.title !== 'string' || (meeting.folder != null && typeof meeting.folder !== 'string')) return false;
    ids.add(meeting.id);
    return true;
  })) return null;
  const storedFolders = Array.isArray(value.userFolders) && value.userFolders.every(folder => typeof folder === 'string' && folder.trim()) ? value.userFolders : initialFolders;
  return {
    meetings: value.meetings.map(meeting => ({ ...meeting, recording: false })),
    userFolders: [...new Set([...storedFolders, ...value.meetings.map(meeting => meeting.folder).filter(Boolean)])],
    flow: restoreNotificationFlow(value.flow),
  };
}
