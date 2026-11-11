import test from 'node:test';
import assert from 'node:assert/strict';
import { devSigningPlan, signDevBundle } from '../scripts/dev-signing.mjs';

const development = { environment: 'development' }, appArgs = ['build', '--debug', '--bundles', 'app'];
const hash = 'A'.repeat(40), other = 'B'.repeat(40);
const identities = ` 1) ${hash} "Apple Development: Fixture (FIXTURE)"\n 2) ${other} "Developer ID Application: Fixture (FIXTURE)"\n 2 valid identities found`;
test('stable signing is confined to local development app bundles and discovers no keys for other builds', () => {
  const options = { platform: 'darwin', env: {}, run: () => assert.fail('must not inspect signing keys'), warn() {} };
  assert.equal(devSigningPlan({ environment: 'production' }, appArgs, options), null);
  assert.equal(devSigningPlan(development, ['dev'], options), null);
  assert.equal(devSigningPlan(development, [...appArgs, '--no-sign'], options), null);
  assert.equal(devSigningPlan(development, ['build', '--bundles', 'dmg'], options), null);
  assert.equal(devSigningPlan(development, appArgs, { ...options, platform: 'win32' }), null);
  assert.equal(devSigningPlan(development, appArgs, { ...options, env: { NERO_DEV_SIGN_IDENTITY: '-' } }), null);
});
test('one existing development certificate gives a stable bundle identity, with explicit selection required for ambiguity', () => {
  const options = { platform: 'darwin', env: {}, run: () => identities, warn() {} };
  const plan = devSigningPlan(development, appArgs, options);
  assert.equal(plan.identity, hash);
  assert.ok(plan.app.endsWith('/.work/rust-target/debug/bundle/macos/Nero Dev.app'));
  assert.equal(devSigningPlan(development, appArgs, { ...options, env: { NERO_DEV_SIGN_IDENTITY: other } }).identity, other);
  assert.throws(() => devSigningPlan(development, appArgs, { ...options, env: { NERO_DEV_SIGN_IDENTITY: 'missing' } }));
  assert.throws(() => devSigningPlan(development, appArgs, { ...options, run: () => identities + `\n3) ${'C'.repeat(40)} "Apple Development: Other (OTHER)"` }));
  assert.equal(devSigningPlan(development, appArgs, { ...options, run: () => '0 valid identities found' }), null);
});
test('production identities, linked bundles and shared binaries cannot be signed by the development helper', () => {
  const plan = { app: '/fixture/Nero Dev.app', identity: hash }, calls = [];
  let id = 'io.github.choseongmin1128.nero.desktop', nlink = 1;
  const options = { realpath: value => value, stat: () => ({ isSymbolicLink: () => false, isFile: () => true, get nlink() { return nlink; } }), run: (command, args) => { calls.push([command, args]); if (command.endsWith('plutil')) return args[1] === 'CFBundleIdentifier' ? id : 'nero-desktop'; } };
  assert.throws(() => signDevBundle(plan, options), /non-development/);
  id = 'io.github.choseongmin1128.nero.desktop.dev'; nlink = 2;
  assert.throws(() => signDevBundle(plan, options), /shared/);
  assert.throws(() => signDevBundle(plan, { ...options, realpath: () => '/elsewhere/Nero Dev.app' }), /owned/);
  assert.equal(calls.some(([command]) => command.endsWith('codesign')), false);
});
