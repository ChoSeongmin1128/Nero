import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { copyFile, mkdir, mkdtemp, readFile, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';

const tracked = [
  'apps/desktop/package.json', 'apps/desktop/package-lock.json',
  'apps/desktop/src-tauri/tauri.conf.json', 'apps/desktop/src-tauri/Cargo.toml',
  'crates/nero-core/Cargo.toml', 'Cargo.lock',
  'crates/nero-cli/Cargo.toml',
];

async function fixture() {
  const root = await mkdtemp(path.join(os.tmpdir(), 'nero-version-test-'));
  const script = path.join(root, 'apps/desktop/scripts/version.mjs');
  await mkdir(path.dirname(script), { recursive: true });
  await copyFile(new URL('../scripts/version.mjs', import.meta.url), script);
  const json = value => `${JSON.stringify(value, null, 2)}\n`;
  const files = [
    json({ name: 'nero-desktop', version: '0.0.1', private: true }),
    json({ version: '0.0.1', packages: { '': { version: '0.0.1' }, 'node_modules/example': { version: '0.1.0' } } }),
    json({ version: '0.0.1', identifier: 'stable-app-id', bundle: { macOS: { bundleVersion: '1' } } }),
    '[package]\nname = "nero-desktop"\nversion = "0.0.1"\n\n[dependencies]\nexample = "0.1.0"\n',
    '[package]\nname = "nero-core"\nversion = "0.0.1"\n\n[dependencies]\nexample = "0.1.0"\n',
    'version = 4\n\n[[package]]\nname = "example"\nversion = "0.1.0"\nsource = "registry+example"\n\n[[package]]\nname = "nero-desktop"\nversion = "0.0.1"\ndependencies = ["example"]\n\n[[package]]\nname = "nero-core"\nversion = "0.0.1"\n\n[[package]]\nname = "nero-cli"\nversion = "0.0.1"\n',
    '[package]\nname = "nero-cli"\nversion = "0.0.1"\n\n[dependencies]\nexample = "0.1.0"\n',
  ];
  for (let index = 0; index < tracked.length; index++) {
    const target = path.join(root, tracked[index]);
    await mkdir(path.dirname(target), { recursive: true });
    await writeFile(target, files[index]);
  }
  return {
    root,
    run: (...args) => spawnSync(process.execPath, [script, ...args], { encoding: 'utf8' }),
    files: () => Promise.all(tracked.map(file => readFile(path.join(root, file), 'utf8'))),
  };
}

test('version CLI checks every product mirror and synchronizes release version without changing dependencies or identity', async () => {
  const project = await fixture();
  assert.equal(project.run('check').status, 0);
  const result = project.run('set', '0.0.2', '--build', '2');
  assert.equal(result.status, 0, result.stderr);
  assert.equal(project.run('check').status, 0);
  const files = await project.files();
  assert.equal(JSON.parse(files[0]).version, '0.0.2');
  assert.equal(JSON.parse(files[1]).packages['node_modules/example'].version, '0.1.0');
  assert.equal(JSON.parse(files[2]).identifier, 'stable-app-id');
  assert.equal(JSON.parse(files[2]).bundle.macOS.bundleVersion, '2');
  assert.match(files[3], /example = "0\.1\.0"/);
  assert.match(files[5], /name = "example"\nversion = "0\.1\.0"/);
  assert.match(files[5], /name = "nero-cli"\nversion = "0\.0\.2"/);
  assert.match(files[6], /version = "0\.0\.2"/);
});

test('version CLI allows rebuilds but rejects malformed versions, rollback and reused build numbers before writing', async () => {
  const project = await fixture();
  const before = await project.files();
  for (const args of [
    ['set', '0.0', '--build', '2'], ['set', '0.0.2-beta', '--build', '2'],
    ['set', '00.0.2', '--build', '2'], ['set', '0.0.0', '--build', '2'],
    ['set', '0.0.2', '--build', '1'], ['set', '0.0.2', '--build', '0'],
  ]) {
    assert.notEqual(project.run(...args).status, 0, args.join(' '));
    assert.deepEqual(await project.files(), before);
  }
  assert.equal(project.run('set', '0.0.1', '--build', '2').status, 0);
});

test('a mismatched late manifest blocks check and all release writes', async () => {
  const project = await fixture();
  const target = path.join(project.root, 'Cargo.lock');
  await writeFile(target, (await readFile(target, 'utf8')).replace('name = "nero-core"\nversion = "0.0.1"', 'name = "nero-core"\nversion = "0.0.3"'));
  const before = await project.files();
  assert.notEqual(project.run('check').status, 0);
  assert.notEqual(project.run('set', '0.0.2', '--build', '2').status, 0);
  assert.deepEqual(await project.files(), before);
});
