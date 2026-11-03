import test from 'node:test';
import assert from 'node:assert/strict';
import { tauriProfile } from '../scripts/tauri-profile.mjs';

const config = ['--config', 'src-tauri/tauri.dev.conf.json'];
test('development commands select matching config and feature before cargo arguments', () => {
  assert.deepEqual(tauriProfile(['dev'], undefined), { args: ['dev', ...config, '--features', 'development'], environment: 'development' });
  assert.deepEqual(tauriProfile(['build', '--debug', '--', '--locked'], undefined), { args: ['build', '--debug', ...config, '--features', 'development', '--', '--locked'], environment: 'development' });
  assert.deepEqual(tauriProfile(['build', '--features', 'development'], undefined), { args: ['build', '--features', 'development', ...config], environment: 'development' });
});
test('production stays unchanged and mismatched or overridden profiles fail closed', () => {
  assert.deepEqual(tauriProfile(['build'], undefined), { args: ['build'], environment: 'production' });
  assert.equal(tauriProfile(['build', '-d']).environment, 'development');
  assert.equal(tauriProfile(['build', '-f', 'other', 'development']).environment, 'development');
  assert.throws(() => tauriProfile(['build'], 'development'));
  assert.throws(() => tauriProfile(['dev'], 'production'));
  assert.throws(() => tauriProfile(['build', '--config', 'other.json']));
  assert.throws(() => tauriProfile(['build', '--', '--features', 'development']));
});
