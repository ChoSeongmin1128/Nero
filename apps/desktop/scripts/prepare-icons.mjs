import { mkdir, copyFile } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const desktop = fileURLToPath(new URL('../', import.meta.url));
const root = path.resolve(desktop, '../..');
const source = path.join(desktop, 'brand/nero-cat.png');
const stage = path.join(root, '.work/nero-brand-icons');
const icons = path.join(desktop, 'src-tauri/icons');
const cli = path.join(desktop, 'node_modules/@tauri-apps/cli/tauri.js');
await mkdir(stage, { recursive: true });
await mkdir(icons, { recursive: true });

function generate(output, sizes = []) {
  execFileSync(process.execPath, [cli, 'icon', source, '--output', output, ...sizes.flatMap(size => ['--png', String(size)])], { cwd: desktop, stdio: 'inherit' });
}

generate(path.join(stage, 'app'));
generate(path.join(stage, 'png'), [512]);
await copyFile(path.join(stage, 'app/icon.icns'), path.join(icons, 'icon.icns'));
await copyFile(path.join(stage, 'app/icon.ico'), path.join(icons, 'icon.ico'));
await copyFile(path.join(stage, 'png/512x512.png'), path.join(icons, 'icon.png'));
