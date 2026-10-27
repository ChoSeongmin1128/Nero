import { spawnSync, execFileSync } from 'node:child_process';
import { mkdir, copyFile, chmod, readFile, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { resolve, join } from 'node:path';
import { prepareAgentPackages } from './agent-packages.mjs';

const root = fileURLToPath(new URL('../../../', import.meta.url));
const args = process.argv.slice(2);
if (args.length && (args.length !== 2 || args[0] !== '--target')) {
  throw Error('Usage: node scripts/prepare-agent.mjs [--target <Rust target triple>]');
}
const requested = args[1] || process.env.TAURI_ENV_TARGET_TRIPLE || process.env.CARGO_BUILD_TARGET;
const host = execFileSync('rustc', ['-vV'], { encoding: 'utf8' }).match(/^host: (.+)$/m)?.[1];
const target = requested || host;
if (!target || !/^[a-z0-9_-]+$/.test(target) || target.includes('universal')) {
  throw Error('Build the Nero agent for one supported Rust target at a time.');
}
const targetDir = resolve(root, process.env.CARGO_TARGET_DIR || '.work/rust-target');
const cargoArgs = ['build', '-p', 'nero-cli', '--release', '--locked', '--target', target, '--target-dir', targetDir];
const command = process.platform === 'win32' ? 'cargo' : 'nice';
const commandArgs = process.platform === 'win32' ? cargoArgs : ['-n', '10', 'cargo', ...cargoArgs];
const result = spawnSync(command, commandArgs, {
  cwd: root,
  stdio: 'inherit',
  env: { ...process.env, CARGO_BUILD_JOBS: '2' },
});
if (result.error) throw result.error;
if (result.status !== 0) process.exit(result.status || 1);
const binary = target.includes('windows') ? 'nero.exe' : 'nero';
const source = join(targetDir, target, 'release', binary);
const directory = join(root, 'apps/desktop/.generated/agent');
await mkdir(directory, { recursive: true });
const destination = join(directory, binary);
await copyFile(source, destination);
if (process.platform !== 'win32') await chmod(destination, 0o755);
const bytes = await readFile(destination);
const version = JSON.parse(await readFile(join(root, 'apps/desktop/package.json'), 'utf8')).version;
const build = JSON.parse(await readFile(join(root, 'apps/desktop/src-tauri/tauri.conf.json'), 'utf8')).bundle.macOS.bundleVersion;
if (!/^[1-9]\d*$/.test(build)) throw Error('Invalid app build for integration packages.');
const integrationVersion = `${version}-nero.${build}`;
await writeFile(join(directory, 'agent-build.json'), JSON.stringify({
  version,
  integrationVersion,
  apiVersion: 1,
  target,
  binary,
  buildSha256: createHash('sha256').update(bytes).digest('hex'),
}, null, 2) + '\n');
await prepareAgentPackages({
  source: join(root, 'apps/desktop/agent'),
  destination: join(directory, 'integrations'),
  icon: join(root, 'apps/desktop/src-tauri/icons/icon.png'),
  version: integrationVersion,
  target,
});
console.log(`Nero read-only agent prepared: ${target}`);
