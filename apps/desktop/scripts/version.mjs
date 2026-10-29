import { readFile, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const root = fileURLToPath(new URL('../../../', import.meta.url));
const paths = [
  'apps/desktop/package.json',
  'apps/desktop/package-lock.json',
  'apps/desktop/src-tauri/tauri.conf.json',
  'apps/desktop/src-tauri/Cargo.toml',
  'crates/nero-core/Cargo.toml',
  'Cargo.lock',
  'crates/nero-cli/Cargo.toml',
];

function version(value) {
  if (typeof value !== 'string' || !/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(value)) {
    throw Error('Version must contain three numeric components: x.y.z');
  }
  return value.split('.').map(BigInt);
}

function build(value) {
  if (typeof value !== 'string' || !/^[1-9]\d*$/.test(value)) {
    throw Error('macOS bundleVersion must be a positive integer');
  }
  return BigInt(value);
}

function packageSection(text, label, replace) {
  const sections = [...text.matchAll(/^\[package\][^\S\r\n]*\r?\n[\s\S]*?(?=^\[|$(?![\s\S]))/gm)];
  if (sections.length !== 1) throw Error(`Expected one [package] section in ${label}`);
  const section = sections[0];
  const versions = [...section[0].matchAll(/^version\s*=\s*"([^"]+)"/gm)];
  if (versions.length !== 1) throw Error(`Expected one package version in ${label}`);
  if (replace === undefined) return versions[0][1];
  return text.slice(0, section.index) + section[0].replace(/^(version\s*=\s*")[^"]+(".*)$/m, (_, prefix, suffix) => `${prefix}${replace}${suffix}`) + text.slice(section.index + section[0].length);
}

function lockedPackage(text, name, replace) {
  const sections = [...text.matchAll(/^\[\[package\]\][^\S\r\n]*\r?\n[\s\S]*?(?=^\[\[package\]\]|$(?![\s\S]))/gm)]
    .filter(section => new RegExp(`^name\\s*=\\s*"${name}"\\s*$`, 'm').test(section[0]));
  if (sections.length !== 1) throw Error(`Expected one ${name} entry in Cargo.lock`);
  const section = sections[0];
  const versions = [...section[0].matchAll(/^version\s*=\s*"([^"]+)"/gm)];
  if (versions.length !== 1 || /^source\s*=/m.test(section[0])) throw Error(`Invalid local ${name} entry in Cargo.lock`);
  if (replace === undefined) return versions[0][1];
  return text.slice(0, section.index) + section[0].replace(/^(version\s*=\s*")[^"]+(".*)$/m, (_, prefix, suffix) => `${prefix}${replace}${suffix}`) + text.slice(section.index + section[0].length);
}

function inspect(files) {
  const pkg = JSON.parse(files[0]);
  const lock = JSON.parse(files[1]);
  const tauri = JSON.parse(files[2]);
  version(pkg.version);
  build(tauri.bundle?.macOS?.bundleVersion);
  const mirrors = [
    ['package-lock.json', lock.version],
    ['package-lock.json root package', lock.packages?.['']?.version],
    ['tauri.conf.json', tauri.version],
    [paths[3], packageSection(files[3], paths[3])],
    [paths[4], packageSection(files[4], paths[4])],
    ['Cargo.lock nero-desktop', lockedPackage(files[5], 'nero-desktop')],
    ['Cargo.lock nero-core', lockedPackage(files[5], 'nero-core')],
    [paths[6], packageSection(files[6], paths[6])],
    ['Cargo.lock nero-cli', lockedPackage(files[5], 'nero-cli')],
  ];
  for (const [label, value] of mirrors) {
    if (value !== pkg.version) throw Error(`${label} version is ${String(value)}; expected ${pkg.version}`);
  }
  return { pkg, lock, tauri };
}

async function main(args) {
  if (!(args.length === 1 && args[0] === 'check') && !(args.length === 4 && args[0] === 'set' && args[2] === '--build')) {
    throw Error('Usage: node scripts/version.mjs check | set <x.y.z> --build <positive integer>');
  }
  const absolute = paths.map(file => path.join(root, file));
  const files = await Promise.all(absolute.map(file => readFile(file, 'utf8')));
  const { pkg, lock, tauri } = inspect(files);
  if (args[0] === 'check') {
    console.log(`Version ${pkg.version}, build ${tauri.bundle.macOS.bundleVersion}: all manifests agree`);
    return;
  }
  const next = version(args[1]);
  const current = version(pkg.version);
  const difference = next.findIndex((value, index) => value !== current[index]);
  if (difference >= 0 && next[difference] < current[difference]) throw Error('Version rollback is not allowed');
  if (build(args[3]) <= build(tauri.bundle.macOS.bundleVersion)) throw Error('Build number must increase');
  pkg.version = lock.version = lock.packages[''].version = tauri.version = args[1];
  tauri.bundle.macOS.bundleVersion = args[3];
  const stringify = value => `${JSON.stringify(value, null, 2)}\n`;
  const planned = [
    stringify(pkg), stringify(lock), stringify(tauri),
    packageSection(files[3], paths[3], args[1]),
    packageSection(files[4], paths[4], args[1]),
    lockedPackage(lockedPackage(lockedPackage(files[5], 'nero-desktop', args[1]), 'nero-core', args[1]), 'nero-cli', args[1]),
    packageSection(files[6], paths[6], args[1]),
  ];
  inspect(planned);
  const currentFiles = await Promise.all(absolute.map(file => readFile(file, 'utf8')));
  if (currentFiles.some((value, index) => value !== files[index])) throw Error('Version files changed during validation; no files written');
  for (let index = 0; index < absolute.length; index++) {
    if (planned[index] !== files[index]) await writeFile(absolute[index], planned[index]);
  }
  console.log(`Set version ${pkg.version}, build ${tauri.bundle.macOS.bundleVersion}`);
}

main(process.argv.slice(2)).catch(error => {
  console.error(error.message);
  process.exitCode = 1;
});
