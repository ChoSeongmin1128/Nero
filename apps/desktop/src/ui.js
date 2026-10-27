export const ui = globalThis.GroveUI;
export const icon = (name) => globalThis.GroveIcons?.(name) || '';
export const escape = ui.escape;

export function button(text, action, iconName, className = '', disabled = false) {
  return ui.button({ html: `${iconName ? icon(iconName) : ''}<span>${escape(text)}</span>`, action, withIcon: Boolean(iconName), className, disabled });
}

export function duration(milliseconds = 0) {
  const total = Math.max(0, Math.floor(milliseconds / 1000));
  const seconds = String(total % 60).padStart(2, '0');
  const minutes = String(Math.floor(total / 60) % 60).padStart(2, '0');
  return total >= 3600 ? `${Math.floor(total / 3600)}:${minutes}:${seconds}` : `${minutes}:${seconds}`;
}

export function recordingStatus(state) {
  return ({ draft: '녹음 전', idle: '녹음 전', recording: '녹음 중', paused: '일시정지', finalizing: '저장 중', interrupted: '녹음 중단', completed: '녹음 완료' })[state] || '녹음 전';
}

export function statusMarkup(state) {
  if (state === 'interrupted') return ui.badge(recordingStatus(state), true);
  const phase = ['recording', 'paused'].includes(state) ? state : ['completed', 'finalizing'].includes(state) ? 'finished' : 'ready';
  return ui.status(phase, recordingStatus(state));
}

export function sessionControls(session, busy = false) {
  if (session.state === 'finalizing') return '<span class="processing-inline muted type-caption" role="status"><span class="activity-spinner" aria-hidden="true"></span>녹음 저장 중</span>';
  return button(session.state === 'paused' ? '계속 녹음' : '일시정지', session.state === 'paused' ? 'resume' : 'pause', session.state === 'paused' ? 'play' : 'pause', '', busy) + button('녹음 종료', 'stop', 'square', 'stop-recording', busy);
}

export function applySystemAppearance() {
  globalThis.GroveAppearance?.apply();
}

export function documentExport(meeting, transcript, summary, kind, format = 'txt') {
  if (!['transcript', 'summary'].includes(kind) || !['txt', 'md'].includes(format)) throw new Error('지원하지 않는 파일 형식입니다.');
  const heading = kind === 'transcript' ? '음성 기록' : '회의록';
  const title = meeting.title.trim() || '제목 없는 회의';
  const safeName = title.replace(/[\\/:*?"<>|\u0000-\u001f]/g, '_').replace(/[. ]+$/g, '').slice(0, 80) || '회의';
  const markdown = text => text.replace(/[\\`*_{}\[\]<>#!|]/g, '\\$&');
  let body;
  if (kind === 'transcript') {
    if (!transcript?.segments?.length) throw new Error('내보낼 음성 기록이 없습니다.');
    body = transcript.segments.map(segment => {
      const timestamp = duration(segment.start * 1000);
      const name = transcript.speakers?.find(speaker => speaker.id === segment.speakerId)?.name;
      const label = name ? `${timestamp} ${name}` : timestamp;
      return format === 'md' ? `**${markdown(label)}**\n\n${markdown(segment.text)}` : `[${label}] ${segment.text}`;
    }).join('\n\n');
  } else {
    if (!summary?.document?.text) throw new Error('내보낼 요약이 없습니다.');
    body = summary.document.text;
    if (summary.stale) body = `이전 내용으로 만든 요약\n\n${body}`;
  }
  const header = format === 'md' ? `# ${markdown(title)}\n\n## ${heading}` : `${title}\n${heading}`;
  return { name: `${safeName}-${heading}.${format}`, text: `${header}\n\n${body}\n` };
}
