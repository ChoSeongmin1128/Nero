import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { createUpdateFeed, releaseIdentity } from '../scripts/app-update-feed.mjs';

const sha = bytes => createHash('sha256').update(bytes).digest('hex');
const json = (file, value) => writeFile(file, JSON.stringify(value));

async function fixture(t) {
  const directory = await mkdtemp(path.join(tmpdir(), 'nero-feed-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const artifacts = [];
  for (const [platform, hostPlatform, hostArchitecture, filename] of [
    ['darwin-aarch64', 'darwin', 'arm64', 'Nero-build41.app.tar.gz'],
    ['windows-x86_64', 'win32', 'x64', 'Nero-build41-setup.exe'],
  ]) {
    const file = path.join(directory, filename), signature = `${file}.sig`, provenance = `${file}.json`, receipt = `${file}.receipt.json`;
    const bytes = Buffer.from(`synthetic ${platform} package`);
    const source = { version: '0.0.1', build: '41', sourceCommit: 'a'.repeat(40), sourceTree: 'b'.repeat(40), sourceDirty: false };
    await writeFile(file, bytes);
    await writeFile(signature, 'synthetic signature');
    await json(provenance, { formatVersion: 1, ...source, bundleIdentifier: 'io.github.choseongmin1128.nero.desktop', hostPlatform, hostArchitecture });
    await json(receipt, { schemaVersion: 1, platform, ...source, artifact: { filename, bytes: bytes.length, sha256: sha(bytes) } });
    artifacts.push({ platform, file, signature, provenance, receipt });
  }
  const calls = [];
  // Real Minisign verification belongs to the native verifier tests. This double
  // isolates whether packaging evidence is bound before invoking that verifier.
  const verify = async (file, signature, wire) => { calls.push([file, signature, wire]); return sha(await readFile(file)); };
  return { directory, calls, options: { version: '0.0.1', build: '41', tag: 'nero-build41', artifacts, verify } };
}

test('feed build is an exact canonical integer, including beyond JS safe integers', () => {
  assert.equal(releaseIdentity('0.0.1', '9007199254740993'), '0.0.1+9007199254740993');
  for (const value of ['0', '041', '-1', '1e3', '18446744073709551616', 41]) assert.throws(() => releaseIdentity('0.0.1', value));
  assert.throws(() => releaseIdentity('0.0.1-beta', '41'));
});

test('feed binds both clean platform packages to their packaging receipts and authenticated hashes', async t => {
  const { directory, calls, options } = await fixture(t);
  const result = await createUpdateFeed(options);
  assert.equal(calls.length, 2);
  assert.equal(calls[0][2], '0.0.1+41');
  assert.equal(result.manifest.version, '0.0.1+41');
  assert.equal(result.receipt.published, false);
  assert.equal(result.receipt.sourceCommit, 'a'.repeat(40));
  assert.equal(result.receipt.sourceTree, 'b'.repeat(40));
  assert.equal(result.receipt.artifacts.length, 2);
  assert(!JSON.stringify(result.manifest).includes(directory));
  assert(!JSON.stringify(result.manifest).includes('sourceCommit'));
  for (const artifact of options.artifacts) {
    const packaging = JSON.parse(await readFile(artifact.receipt, 'utf8'));
    assert.deepEqual(result.receipt.artifacts.find(value => value.platform === artifact.platform), { platform: artifact.platform, ...packaging.artifact });
  }
  await assert.rejects(createUpdateFeed({ ...options, verify: async () => { throw Error('invalid signature'); } }), /invalid signature/);
  await assert.rejects(createUpdateFeed({ ...options, verify: async () => '0'.repeat(64) }), /changed during/);
  await assert.rejects(createUpdateFeed({ ...options, artifacts: options.artifacts.slice(0, 1) }), /Both/);
});

test('a different package cannot borrow matching clean build provenance', async t => {
  const { options, calls } = await fixture(t);
  const artifact = options.artifacts[0];
  const original = await readFile(artifact.file);
  const substituted = Buffer.from(original);
  substituted[0] ^= 1;
  await writeFile(artifact.file, substituted);
  await assert.rejects(createUpdateFeed(options), /hash differs from its packaging receipt/);
  assert.equal(calls.length, 0, 'a valid signature alone must not replace package provenance');
});

test('packaging receipt requires the actual filename, size and SHA256', async t => {
  const { options } = await fixture(t);
  const artifact = options.artifacts[0], original = JSON.parse(await readFile(artifact.receipt, 'utf8'));
  for (const change of [{ filename: 'another.app.tar.gz' }, { bytes: original.artifact.bytes + 1 }, { bytes: String(original.artifact.bytes) }, { sha256: '0'.repeat(64) }]) {
    await json(artifact.receipt, { ...original, artifact: { ...original.artifact, ...change } });
    await assert.rejects(createUpdateFeed(options), /differs from its packaging receipt/);
  }
});

test('packaging receipt identity must match its platform and packaged source metadata', async t => {
  const { options } = await fixture(t);
  const artifact = options.artifacts[0], original = JSON.parse(await readFile(artifact.receipt, 'utf8'));
  for (const change of [{ schemaVersion: 2 }, { platform: 'windows-x86_64' }, { version: '0.0.2' }, { build: '42' }, { build: 41 }, { sourceCommit: 'c'.repeat(40) }, { sourceTree: 'c'.repeat(40) }, { sourceDirty: true }]) {
    await json(artifact.receipt, { ...original, ...change });
    await assert.rejects(createUpdateFeed(options), /packaging receipt differs from packaged build provenance/);
  }
  await assert.rejects(createUpdateFeed({ ...options, artifacts: options.artifacts.map(({ receipt, ...value }) => value) }), /packaging receipt is required/);
});

test('both platforms must identify the same commit and tree even with coherent individual receipts', async t => {
  const { options } = await fixture(t);
  const artifact = options.artifacts[1];
  const provenance = JSON.parse(await readFile(artifact.provenance, 'utf8')), receipt = JSON.parse(await readFile(artifact.receipt, 'utf8'));
  for (const field of ['sourceCommit', 'sourceTree']) {
    await json(artifact.provenance, { ...provenance, [field]: 'c'.repeat(40) });
    await json(artifact.receipt, { ...receipt, [field]: 'c'.repeat(40) });
    await assert.rejects(createUpdateFeed(options), /Platform source (commits|trees) differ/);
  }
});

test('packaged provenance must identify the actual application and supported build target', async t => {
  const { options } = await fixture(t);
  const artifact = options.artifacts[0], original = JSON.parse(await readFile(artifact.provenance, 'utf8'));
  for (const change of [{ formatVersion: 2 }, { build: 41 }, { sourceDirty: true }, { sourceTree: undefined }, { sourceCommit: 'not-a-commit' }, { bundleIdentifier: 'another.application' }, { hostPlatform: 'win32' }, { hostArchitecture: 'x64' }]) {
    await json(artifact.provenance, { ...original, ...change });
    await assert.rejects(createUpdateFeed(options), /Clean, matching packaged build provenance and target/);
  }
});

test('feed cannot embed a signature replaced while its native verification runs', async t => {
  const { options } = await fixture(t);
  await assert.rejects(createUpdateFeed({ ...options, verify: async (file, signature) => {
    await writeFile(signature, 'replacement signature');
    return sha(await readFile(file));
  } }), /Signature changed during native verification/);
});

test('CLI requires both packaging receipt options before verifying or writing a feed', async t => {
  const { options, directory } = await fixture(t);
  const [mac, windows] = options.artifacts;
  const result = spawnSync(process.execPath, [fileURLToPath(new URL('../scripts/app-update-feed.mjs', import.meta.url)),
    '--version', options.version, '--build', options.build, '--tag', options.tag, '--verifier', process.execPath,
    '--mac', mac.file, '--mac-provenance', mac.provenance,
    '--windows', windows.file, '--windows-provenance', windows.provenance,
    '--out', path.join(directory, 'latest.json')], { encoding: 'utf8', timeout: 10_000 });
  assert.equal(result.error, undefined);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /Required:.*--mac-receipt.*--windows-receipt/);
  await assert.rejects(readFile(path.join(directory, 'latest.json')), { code: 'ENOENT' });
  await assert.rejects(readFile(path.join(directory, 'latest.json.receipt.json')), { code: 'ENOENT' });
});
