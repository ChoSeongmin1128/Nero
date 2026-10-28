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
