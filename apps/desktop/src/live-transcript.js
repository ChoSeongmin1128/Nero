// A provisional view only. These rows never become saved transcript identities.
export function liveTranscriptRows(preview, meetingId, sessionId) {
  if (!preview || !meetingId || !sessionId || preview.meetingId !== meetingId || preview.sessionId !== sessionId || !Array.isArray(preview.words)) return [];
  // The pinned C ABI supplies word offsets only at finalization. Never invent
  // offsets or speaker assignments for its interim transcript.
  if (!preview.words.length && typeof preview.text === 'string' && preview.text.trim()) {
    return [{ start: null, end: null, text: preview.text, speaker: null }];
  }
  const rows = [];
  for (const word of preview.words) {
    if (typeof word.word !== 'string' || !word.word.trim() || !Number.isFinite(word.start) || !Number.isFinite(word.end) || word.start < 0 || word.end < word.start) continue;
    const speaker = Number.isInteger(word.speaker) && word.speaker >= 1 && word.speaker <= 8 ? word.speaker : null;
    let row = rows.at(-1);
    if (!row || row.speaker !== speaker || word.start - row.end >= 0.75 || /[.!?]$/.test(row.text) || word.end - row.start > 180) {
      row = { start: word.start, end: word.end, text: '', speaker };
      rows.push(row);
    }
    row.text = row.text ? `${row.text} ${word.word}` : word.word;
    row.end = word.end;
  }
  return rows;
}
