import { copyFile, cp, mkdir, lstat, readFile, readdir, writeFile } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
import { buildControls } from './build-controls.mjs';

const app = fileURLToPath(new URL('..', import.meta.url));
const source = resolve(app, '../../design-system');
const target = resolve(app, '.generated/shared');
await mkdir(target, { recursive: true });
if ((await lstat(target)).isSymbolicLink()) throw new Error('Shared asset output must not be a symlink.');
for (const name of ['appearance.css', 'appearance.js', 'typography.css', 'components.css', 'controls.css', 'desktop-preview.css', 'components.js', 'icons.js']) {
  await copyFile(resolve(source, name), resolve(target, name));
}
await buildControls(resolve(target, 'controls.js'));
const licenses = resolve(target, 'licenses');
await mkdir(licenses, { recursive: true });
await copyFile(resolve(source, 'components/ui/LICENSE.md'), resolve(licenses, 'shadcn-ui.txt'));
const npmArgs = ['ls', '--omit=dev', '--all', '--parseable'];
const runtimePackages = (process.env.npm_execpath
  ? execFileSync(process.execPath, [process.env.npm_execpath, ...npmArgs], { cwd: app, encoding: 'utf8' })
  : process.platform === 'win32'
    ? execFileSync('cmd.exe', ['/d', '/s', '/c', 'npm ls --omit=dev --all --parseable'], { cwd: app, encoding: 'utf8' })
    : execFileSync('npm', npmArgs, { cwd: app, encoding: 'utf8' })).trim().split('\n').slice(1);
const licenseManifest = [];
for (const folder of runtimePackages) {
  const metadata = JSON.parse(await readFile(resolve(folder, 'package.json'), 'utf8'));
  const notices = (await readdir(folder, { withFileTypes: true })).filter(entry => entry.isFile() && /^licen[cs]e(?:[._-].*)?$/i.test(entry.name));
  if (!notices.length) throw new Error(`Missing runtime license notice: ${metadata.name}`);
  const files = [];
  for (const notice of notices) {
    const name = metadata.name.replaceAll('/', '-') + '-' + notice.name;
    await copyFile(resolve(folder, notice.name), resolve(licenses, name)); files.push(name);
  }
  licenseManifest.push({ name: metadata.name, version: metadata.version, license: metadata.license, files });
}
await writeFile(resolve(licenses, 'manifest.json'), JSON.stringify(licenseManifest, null, 2) + '\n');
await cp(resolve(source, 'vendor/lucide'), resolve(target, 'vendor/lucide'), { recursive: true });

await mkdir(resolve(app, '.generated/native'), { recursive: true });
await cp(resolve(app, 'licenses/portable-wav'), resolve(app, '.generated/native/licenses/portable-wav'), { recursive: true });
await mkdir(resolve(app, '.generated/agent'), { recursive: true });

await mkdir(resolve(app, '.generated/brand'), { recursive: true });
await copyFile(resolve(app, 'brand/mark.svg'), resolve(app, '.generated/brand/mark.svg'));

const git = (...args) => execFileSync('git', args, { cwd: resolve(app, '../..'), encoding: 'utf8' }).trim();
const config = JSON.parse(await readFile(resolve(app, 'src-tauri/tauri.conf.json'), 'utf8'));
const release = resolve(app, '.generated/release');
await mkdir(release, { recursive: true });
await writeFile(resolve(release, 'build-info.json'), JSON.stringify({
  formatVersion: 1,
  version: config.version,
  build: config.bundle.macOS.bundleVersion,
  bundleIdentifier: config.identifier,
  sourceCommit: git('rev-parse', 'HEAD'),
  sourceTree: git('rev-parse', 'HEAD^{tree}'),
  sourceDirty: Boolean(git('status', '--porcelain', '--untracked-files=normal')),
  hostPlatform: process.platform,
  hostArchitecture: process.arch,
}, null, 2) + '\n');
