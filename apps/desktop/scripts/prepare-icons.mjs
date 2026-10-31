import { mkdir, copyFile, readFile, writeFile } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const desktop = fileURLToPath(new URL('../', import.meta.url));
const root = path.resolve(desktop, '../..');
const source = path.join(desktop, 'brand/nero-cat.png');
const stage = path.join(root, '.work/nero-brand-icons');
const icons = path.join(desktop, 'src-tauri/icons');
const cli = path.join(desktop, 'node_modules/@tauri-apps/cli/tauri.js');
const args = process.argv.slice(2);
if (args.length && (args.length !== 1 || args[0] !== '--windows-only')) {
  throw Error('Usage: node scripts/prepare-icons.mjs [--windows-only]');
}
const windowsOnly = args.length === 1;
await mkdir(stage, { recursive: true });
await mkdir(icons, { recursive: true });

function generate(output, sizes = [], input = source) {
  execFileSync(process.execPath, [cli, 'icon', input, '--output', output, ...sizes.flatMap(size => ['--png', String(size)])], { cwd: desktop, stdio: 'inherit' });
}

// Windows displays the ICO silhouette directly. Clip only its outer corners;
// keep the shared cat source and macOS assets unchanged.
const image = await readFile(source);
if (!image.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10])) || image.length < 24) {
  throw Error('The shared icon source must be a square PNG.');
}
const width = image.readUInt32BE(16);
const height = image.readUInt32BE(20);
if (!width || width !== height) throw Error('The shared icon source must be a square PNG.');
const windowsSource = path.join(stage, 'windows-rounded.svg');
await writeFile(windowsSource, `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}"><defs><clipPath id="corners"><rect width="${width}" height="${height}" rx="${width * 0.18}"/></clipPath></defs><image width="${width}" height="${height}" href="data:image/png;base64,${image.toString('base64')}" clip-path="url(#corners)"/></svg>`);

if (!windowsOnly) {
  generate(path.join(stage, 'app'));
  generate(path.join(stage, 'png'), [512]);
  await copyFile(path.join(stage, 'app/icon.icns'), path.join(icons, 'icon.icns'));
  await copyFile(path.join(stage, 'png/512x512.png'), path.join(icons, 'icon.png'));
}
generate(path.join(stage, 'windows'), [], windowsSource);
await copyFile(path.join(stage, 'windows/icon.ico'), path.join(icons, 'icon.ico'));
