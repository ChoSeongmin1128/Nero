export function recognitionCoverage(document, clock) {
  const report = document?.source?.recognitionReport;
  if (!report) return { message: '', timing: new Set() };
  const timing = new Set(report.timingContextSegments || []);
  const missing = (report.chunks || []).filter(chunk => chunk.state !== 'complete');
  const ranges = [];
  for (const chunk of missing) {
    const previous = ranges.at(-1);
    if (previous?.end === chunk.startFrame) previous.end = chunk.endFrame;
    else ranges.push({ start: chunk.startFrame, end: chunk.endFrame });
  }
  const spans = ranges.map(range => `${clock(range.start * 1000 / report.sampleRate)} - ${clock(range.end * 1000 / report.sampleRate)}`);
  const message = missing.length ? `일부 구간이 누락되었습니다: ${spans.join(', ')}. 정상 결과는 보존되어 있으며 작업 목록에서 실패 구간만 다시 시도할 수 있습니다.`
    : timing.size ? '일부 발언은 정확한 시각 대신 해당 구간 범위로 표시합니다.' : '';
  return { message, timing };
}
