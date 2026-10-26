import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';

const root = fileURLToPath(new URL('../../../', import.meta.url));
const target = process.env.TAURI_ENV_TARGET_TRIPLE || process.env.CARGO_BUILD_TARGET || '';
if (process.platform === 'darwin' && (!target || target.includes('apple-darwin'))) {
  execFileSync(process.env.PYTHON || 'python3', [
    resolve(root, 'scripts/native-moss-harness/verify_worker_package.py'),
    'verify-staged', resolve(root, 'apps/desktop/.generated/native/moss'), '--optional',
  ], { cwd: root, stdio: 'inherit', env: { ...process.env, PYTHONDONTWRITEBYTECODE: '1' } });
} else {
  console.log('MOSS package verification skipped: this target does not use the macOS worker.');
}
