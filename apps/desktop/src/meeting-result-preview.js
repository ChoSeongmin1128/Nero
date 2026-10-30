import { liveTranscriptRows } from './live-transcript.js';
import { pendingJob } from './processing-state.js';

export function meetingResultPreview({ meeting, session, preview, transcript, processing }) {
  const ownSession = session?.meetingId === meeting.id ? session : null;
  const recording = ['recording', 'paused'].includes(ownSession?.state);
  const sessionId = ownSession?.id || meeting.audio?.sessionId;
  const saved = transcript?.segments || [];
  const rows = transcript ? saved : liveTranscriptRows(preview, meeting.id, sessionId);
  const job = pendingJob(processing, { meetingId: meeting.id, kind: 'transcribe' })
    || pendingJob(processing, { meetingId: meeting.id, kind: 'diarize' });
  return {
    visible: !recording && Boolean(meeting.audio || rows.length || job || ownSession?.state === 'finalizing'),
    busy: Boolean(job || ownSession?.state === 'finalizing'),
    draft: !transcript && rows.length > 0,
    rows,
  };
}
