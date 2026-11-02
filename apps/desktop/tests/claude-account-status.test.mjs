import assert from 'node:assert/strict';
import test from 'node:test';
import { claudeAccountStatus } from '../src/claude-account-status.js';

test('missing Claude Code offers preparation and login without claiming the user signed out', () => {
  const account = { configured: true, state: 'unknown', runtimeState: 'missing' };
  assert.equal(claudeAccountStatus(account).status, 'Claude Code 설치 필요');
  assert.equal(claudeAccountStatus(account).action, '설치하고 로그인');
  assert.equal(claudeAccountStatus(account, 'waiting').status, '설치 및 로그인 중');
});

test('installed Claude Code requires an explicit login check before readiness', () => {
  const account = { configured: true, state: 'unknown', runtimeState: 'present' };
  assert.equal(claudeAccountStatus(account).action, '로그인 확인');
  assert.equal(claudeAccountStatus(account).status, '로그인 확인 필요');
  assert.equal(claudeAccountStatus({ ...account, state: 'signed_out' }).status, '로그인 필요');
  assert.equal(claudeAccountStatus({ ...account, state: 'ready' }).status, '로그인됨');
  assert.equal(claudeAccountStatus({ ...account, state: 'ready' }).detail, '');
});

test('unavailable or unobserved installation never claims it is absent', () => {
  assert.equal(claudeAccountStatus({ configured: true, state: 'unknown', runtimeState: 'unknown' }).status, '설치 확인 필요');
  assert.equal(claudeAccountStatus({ configured: false, state: 'unavailable' }).status, '사용할 수 없음');
});
