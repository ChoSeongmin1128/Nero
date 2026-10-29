const policies = {
  xrun: { label: '입력 지연 신호', severity: 'info', title: '녹음 진단', message: '입력 지연 신호입니다. 이 신호만으로 음성 누락 여부나 길이를 확인할 수는 없습니다.' },
  routeChanged: { label: '입력 장치 변경', severity: 'warning', title: '녹음 장치가 변경되었습니다', message: '녹음 설정에서 사용할 마이크가 맞는지 확인하세요.' },
  queueOverflow: { label: '음성 저장 지연', severity: 'error', title: '음성 저장 지연으로 녹음이 중단되었습니다', message: '다른 작업을 마친 뒤 새 녹음을 시작하세요.' },
  deviceFailure: { label: '마이크 연결 중단', severity: 'error', title: '마이크 연결이 끊겼습니다', message: '마이크 연결을 확인하고 새 녹음을 시작하세요.' },
  writerFailure: { label: '음성 저장 중단', severity: 'error', title: '음성을 저장하지 못해 녹음이 중단되었습니다', message: '저장 공간과 폴더 접근 권한을 확인한 뒤 새 녹음을 시작하세요.' },
  formatChanged: { label: '입력 형식 변경', severity: 'error', title: '마이크 설정이 바뀌어 녹음이 중단되었습니다', message: '녹음 설정에서 마이크를 확인한 뒤 새 녹음을 시작하세요.' },
  captureInterrupted: { label: '녹음 중단', severity: 'error', title: '녹음이 중단되었습니다', message: '저장된 부분을 확인한 뒤 새 녹음을 시작하세요.' },
};
const priority = { info: 0, warning: 1, error: 2 };

export function recordingQuality(quality, sampleRate, frames) {
  const events = (quality?.events || []).filter(event => Object.hasOwn(policies, event.kind));
  const observed = events.map(event => policies[event.kind]);
  if (quality?.xrunCount > 0) observed.push(policies.xrun);
  if (quality?.routeChangedCount > 0) observed.push(policies.routeChanged);
  const policy = observed.reduce((current, next) => priority[next.severity] > priority[current.severity] ? next : current, observed[0] || policies.xrun);
  const { severity, title, message } = policy;
  const visible = observed.length > 0;
  const locations = [];
  for (const event of events) {
    const position = event.firstSavedFrame;
    const known = Number.isFinite(sampleRate) && sampleRate > 0 && Number.isSafeInteger(position) && position >= 0 && Number.isSafeInteger(frames) && position <= frames;
    const last = locations.at(-1);
    const seconds = known ? position / sampleRate : null;
    if (last && last.kind === event.kind && last.seconds === seconds) continue;
    locations.push({ kind: event.kind, label: policies[event.kind].label, seconds, grouped: Boolean(event.coalesced) });
  }
  return { visible, severity, title, message, locations };
}
