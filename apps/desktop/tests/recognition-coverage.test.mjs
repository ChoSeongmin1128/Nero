import test from 'node:test';
import assert from 'node:assert/strict';
import { recognitionCoverage } from '../src/recognition-coverage.js';
import { canRetryProcessing } from '../src/processing-feedback.js';
test('partial coverage merges missing ranges and retains timing uncertainty', () => {
  const report = { sampleRate: 16000, timingContextSegments: [4], chunks: [
    {startFrame:0,endFrame:960000,state:'complete'},
    {startFrame:960000,endFrame:1920000,state:'failed'},
    {startFrame:1920000,endFrame:2880000,state:'notProcessed'},
  ]};
  const coverage = recognitionCoverage({source:{recognitionReport:report}}, ms => String(ms/1000));
  assert.match(coverage.message,/60 - 180/);
  assert.ok(coverage.timing.has(4));
  assert.ok(canRetryProcessing({status:'needsReview',phase:'mossPartial',retryable:true}));
  assert.equal(canRetryProcessing({status:'needsReview',phase:'mossTimingContext',retryable:false}),false);
  assert.equal(recognitionCoverage(null,String).message,'');
});

test('a bounded end correction does not show missing coverage or uncertain timing', () => {
  const document = {source:{recognitionReport:{sampleRate:16000, inputFrames:960000,
    chunks:[{startFrame:0,endFrame:960000,state:'complete'}],
    timingContextSegments:[], normalizedEndSegments:[0]}}};
  const coverage = recognitionCoverage(document, String);
  assert.equal(coverage.message, '');
  assert.equal(coverage.timing.size, 0);
});
