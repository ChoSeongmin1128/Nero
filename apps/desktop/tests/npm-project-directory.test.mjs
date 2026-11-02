import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, symlink, rm, realpath } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import test from 'node:test';
import { npmProjectDirectory } from '../scripts/npm-project-directory.mjs';

async function fixture(t) {
  const root = await realpath(await mkdtemp(resolve(tmpdir(), 'nero-npm-directory-')));
  t.after(() => rm(root, { recursive: true, force: true }));
  const owner = resolve(root, 'owner'), consumer = resolve(root, 'consumer');
  await Promise.all([mkdir(resolve(owner, 'node_modules'), { recursive: true }), mkdir(consumer)]);
  const manifest = { name: 'nero-desktop', dependencies: { react: '19.3.1', clsx: '2.1.1' } };
  await Promise.all([owner, consumer].map(path => writeFile(resolve(path, 'package.json'), JSON.stringify(manifest))));
  await Promise.all([owner, consumer].map(path => writeFile(resolve(path, 'package-lock.json'), JSON.stringify({ packages: { '': manifest, 'node_modules/react': { version: '19.3.1', integrity: 'example-integrity' } } }))));
  return { owner, consumer, manifest };
}

test('physical dependency tree keeps its project directory', async t => {
  const { owner } = await fixture(t);
  assert.equal(await npmProjectDirectory(owner), owner);
});

test('shared dependency tree resolves to its owner without depending on object order', async t => {
  const { owner, consumer, manifest } = await fixture(t);
  await symlink(resolve(owner, 'node_modules'), resolve(consumer, 'node_modules'), process.platform === 'win32' ? 'junction' : 'dir');
  await writeFile(resolve(consumer, 'package.json'), JSON.stringify({ ...manifest, version: '0.0.2', dependencies: { clsx: '2.1.1', react: '19.3.1' } }));
  assert.equal(await npmProjectDirectory(consumer), owner);
});

test('shared dependency tree with different runtime requirements is rejected', async t => {
  const { owner, consumer, manifest } = await fixture(t);
  await symlink(resolve(owner, 'node_modules'), resolve(consumer, 'node_modules'), process.platform === 'win32' ? 'junction' : 'dir');
  await writeFile(resolve(consumer, 'package.json'), JSON.stringify({ ...manifest, dependencies: { react: '19.4.0' } }));
  await assert.rejects(npmProjectDirectory(consumer), /does not match/);
});

test('shared dependency tree rejects changed transitive identities with the same manifest', async t => {
  const { owner, consumer, manifest } = await fixture(t);
  await symlink(resolve(owner, 'node_modules'), resolve(consumer, 'node_modules'), process.platform === 'win32' ? 'junction' : 'dir');
  await writeFile(resolve(consumer, 'package-lock.json'), JSON.stringify({ packages: { '': manifest, 'node_modules/react': { version: '19.3.1', integrity: 'different-integrity' } } }));
  await assert.rejects(npmProjectDirectory(consumer), /does not match package-lock/);
});

test('shared dependency tree cannot substitute another tree for npm inspection', async t => {
  const { owner, consumer } = await fixture(t);
  const cached = resolve(owner, 'cached-modules');
  await mkdir(cached);
  await symlink(cached, resolve(consumer, 'node_modules'), process.platform === 'win32' ? 'junction' : 'dir');
  await assert.rejects(npmProjectDirectory(consumer), /no matching npm project/);
});
