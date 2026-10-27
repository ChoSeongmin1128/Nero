import { invoke, convertFileSrc, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

const unavailable = 'Nero 앱에서 열어 주세요.';
let latestSnapshot = null;

function call(command, args = {}) {
  if (!isTauri()) return Promise.reject(new Error(unavailable));
  return invoke(command, args).catch((error) => {
    throw new Error(typeof error === 'string' ? error : '요청을 처리하지 못했습니다. 다시 시도해 주세요.');
  });
}

function accept(snapshot) {
  if (!latestSnapshot || snapshot.revision >= latestSnapshot.revision) latestSnapshot = snapshot;
  return latestSnapshot;
}

export const readSnapshot = () => call('read_snapshot').then(accept);
export const agentConnectionInfo = () => call('agent_connection_info');
export const connectAgentClient = (client) => call('agent_connect_client', { client });
export const createMeeting = (title, notes = '') => call('create_meeting', { title, notes });
export const updateMeeting = (id, title, notes) => call('update_meeting', { id, title, notes });
export const setMeetingDate = (id, meetingDate) => call('set_meeting_date', { id, meetingDate });
export const listDevices = () => call('list_devices');
export const startRecording = (meetingId, deviceId = null) => call('start_recording', { meetingId, deviceId }).then(accept);
export const pauseRecording = (sessionId) => call('pause_recording', { sessionId }).then(accept);
export const resumeRecording = (sessionId) => call('resume_recording', { sessionId }).then(accept);
export const stopRecording = (sessionId) => call('stop_recording', { sessionId }).then(accept);
export const readTerminologySettings = () => call('read_terminology_settings');
export const setTerminologyEnabled = (enabled) => call('set_terminology_enabled', { enabled });
export const setAutoRecorder = (enabled) => call('set_auto_recorder', { enabled });
export const dragRecorder = () => call('drag_recorder');
export const hideRecorder = () => call('hide_recorder');
export const hideMain = () => call('hide_main');
export const openMeeting = (meetingId, error = null) => call('open_meeting', { meetingId, error });
export const requestQuit = (sessionId = null) => call('request_quit', { sessionId });
export const audioUrl = async (meetingId) => convertFileSrc(await call('audio_file', { meetingId }));
export const exportText = (suggestedName, contents) => call('export_text', { suggestedName, contents });

export async function subscribeSnapshots(callback) {
  if (!isTauri()) return () => {};
  return listen('grove://snapshot', (event) => {
    const snapshot = event.payload;
    if (latestSnapshot && snapshot.revision < latestSnapshot.revision) return;
    callback(accept(snapshot));
  });
}

export async function subscribeOpenMeeting(callback) {
  if (!isTauri()) return () => {};
  return listen('grove://open-meeting', (event) => callback(event.payload.meetingId, event.payload.error));
}

export async function subscribeQuitRequest(callback) {
  if (!isTauri()) return () => {};
  return listen('grove://quit-requested', (event) => callback(event.payload));
}

export async function subscribeCloseRequest(callback) {
  if (!isTauri()) return () => {};
  return listen('grove://close-requested', () => callback());
}

export const processingStatus = () => call('processing_status');
export const prepareModel = () => call('prepare_model');
export const prepareMeetingModels = (includeSummary = false) => call('prepare_meeting_models', { includeSummary });
export const importAudio = () => call('import_audio');
export const transcribeAudio = (meetingId) => call('transcribe_audio', { meetingId });
export const cancelProcessing = (jobId) => call('cancel_processing', { jobId });
export const retryProcessing = (jobId) => call('retry_processing', { jobId });
export const acknowledgeProcessing = (jobId) => call('acknowledge_processing', { jobId });
export const readTranscript = (meetingId) => call('read_transcript', { meetingId });
export const saveTranscript = (meetingId, expected, texts) => call('save_transcript', { meetingId, expected, texts });
export async function subscribeProcessing(callback) {
  if (!isTauri()) return () => {};
  return listen('grove://processing', (event) => callback(event.payload));
}

export const prepareSpeakerModel = () => call('prepare_diarization_model');
export const diarizeMeeting = (meetingId, expected) => call('diarize_meeting', { meetingId, expected });

export const prepareSummaryModel = () => call('prepare_summary_model');
export const summarizeMeeting = (meetingId) => call('summarize_meeting', { meetingId });
export const readSummary = (meetingId) => call('read_summary', { meetingId });
export const readSummaryCitations = (meetingId, summaryId, references) => call('read_summary_citations', { meetingId, summaryId, references });

export async function subscribeProcessingError(callback) {
  if (!isTauri()) return () => {};
  return listen('grove://processing-error', event => callback(event.payload));
}

export const createFolder = (name) => call('create_folder', { name }).then(accept);
export const renameFolder = (id, name) => call('rename_folder', { id, name }).then(accept);
export const deleteFolder = (id) => call('delete_folder', { id }).then(accept);
export const moveMeetings = (ids, folderId = null) => call('move_meetings', { ids, folderId }).then(accept);
export const trashMeetings = (ids) => call('trash_meetings', { ids }).then(accept);
export const restoreMeetings = (ids) => call('restore_meetings', { ids }).then(accept);
export const purgeMeetings = (ids) => call('purge_meetings', { ids }).then(accept);
export const searchMeetings = (query, includeDeleted = false) => call('search_meetings', { query, includeDeleted });
export const saveSpeakers = (meetingId, expected, speakers, assignments) => call('save_speakers', { meetingId, expected, speakers, assignments });

export const readTranscriptHistory = (meetingId) => call('read_transcript_history', { meetingId });
export const restoreTranscript = (meetingId, expected, target) => call('restore_transcript', { meetingId, expected, target });
