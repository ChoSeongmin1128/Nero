const labels = { xrun: '입력 상태 변화', routeChanged: '입력 장치 변경', queueOverflow: '음성 저장 지연', deviceFailure: '마이크 연결 중단', writerFailure: '음성 저장 중단', formatChanged: '입력 형식 변경', captureInterrupted: '녹음 중단' };

export function recordingQuality(quality, sampleRate, frames) {
  const events = (quality?.events || []).filter(event => Object.hasOwn(labels, event.kind));
  const visible = Boolean(events.length || quality?.xrunCount || quality?.routeChangedCount);
  const locations = [];
  for (const event of events) {
    const position = event.firstSavedFrame;
    const known = Number.isFinite(sampleRate) && sampleRate > 0 && Number.isSafeInteger(position) && position >= 0 && Number.isSafeInteger(frames) && position <= frames;
    const last = locations.at(-1);
    const seconds = known ? position / sampleRate : null;
    if (last && last.kind === event.kind && last.seconds === seconds) continue;
    locations.push({ kind: event.kind, label: labels[event.kind], seconds, grouped: Boolean(event.coalesced) });
  }
  return { visible, locations };
}
