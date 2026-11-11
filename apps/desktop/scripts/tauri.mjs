import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { tauriProfile } from './tauri-profile.mjs';
import { devSigningPlan, signDevBundle } from './dev-signing.mjs';

const profile = tauriProfile(process.argv.slice(2));
const signing = devSigningPlan(profile, process.argv.slice(2));
const cli = fileURLToPath(new URL('../node_modules/@tauri-apps/cli/tauri.js', import.meta.url));
const result = spawnSync(process.execPath, [cli, ...profile.args], {
  stdio: 'inherit',
  env: { ...process.env, ...(profile.environment ? { NERO_BUILD_ENV: profile.environment } : {}) },
});
if (result.error) throw result.error;
if (result.status === 0) signDevBundle(signing);
if (result.signal) process.kill(process.pid, result.signal);
else process.exit(result.status ?? 1);
