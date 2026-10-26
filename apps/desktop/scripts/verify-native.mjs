import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';

const root = fileURLToPath(new URL('../../../', import.meta.url));
const target = process.env.TAURI_ENV_TARGET_TRIPLE || process.env.CARGO_BUILD_TARGET || '';
const speechTarget = target
  ? (target.startsWith('x86_64-') && target.includes('windows') ? 'windows-x64'
    : target.startsWith('aarch64-') && target.includes('apple-darwin') ? 'macos-arm64' : null)
  : (process.platform === 'win32' && process.arch === 'x64' ? 'windows-x64'
    : process.platform === 'darwin' && process.arch === 'arm64' ? 'macos-arm64' : null);
if (speechTarget) {
  execFileSync(process.env.PYTHON || 'python3', [
    resolve(root, 'scripts/native-diarization/prepare.py'),
    '--verify-staged', resolve(root, 'apps/desktop/.generated/native/diarization'),
    '--target', speechTarget, '--optional',
  ], { cwd: root, stdio: 'inherit', env: { ...process.env, PYTHONDONTWRITEBYTECODE: '1' } });
}
if (process.platform === 'darwin' && (!target || target.includes('apple-darwin'))) {
  execFileSync(process.env.PYTHON || 'python3', [
    resolve(root, 'scripts/native-moss-harness/verify_worker_package.py'),
    'verify-staged', resolve(root, 'apps/desktop/.generated/native/moss'), '--optional',
  ], { cwd: root, stdio: 'inherit', env: { ...process.env, PYTHONDONTWRITEBYTECODE: '1' } });
} else {
  console.log('MOSS package verification skipped: this target does not use the macOS worker.');
}
