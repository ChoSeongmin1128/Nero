import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { createUpdateFeed, releaseIdentity } from '../scripts/app-update-feed.mjs';

test('feed build is an exact canonical integer, including beyond JS safe integers', () => {
  assert.equal(releaseIdentity('0.0.1', '9007199254740993'), '0.0.1+9007199254740993');
  for (const value of ['0', '041', '-1', '1e3', '18446744073709551616', 41]) assert.throws(() => releaseIdentity('0.0.1', value));
  assert.throws(() => releaseIdentity('0.0.1-beta', '41'));
});

test('feed requires both clean platform builds and a native authenticated payload hash', async () => {
  const directory = await mkdtemp(path.join(tmpdir(), 'nero-feed-'));
  try {
    const artifacts = [];
    for (const [platform, filename] of [['darwin-aarch64', 'Nero-build41.app.tar.gz'], ['windows-x86_64', 'Nero-build41-setup.exe']]) {
      const file = path.join(directory, filename), signature = `${file}.sig`, provenance = `${file}.json`;
      await writeFile(file, 'synthetic package');
      await writeFile(signature, 'synthetic signature');
      await writeFile(provenance, JSON.stringify({ version: '0.0.1', build: '41', sourceCommit: 'a'.repeat(40), sourceDirty: false }));
      artifacts.push({ platform, file, signature, provenance });
    }
    const calls = [];
    const verify = async (file, signature, wire) => { calls.push([file, signature, wire]); return createHash('sha256').update(await readFile(file)).digest('hex'); };
    const options = { version: '0.0.1', build: '41', tag: 'nero-build41', artifacts, verify };
    const result = await createUpdateFeed(options);
    assert.equal(calls.length, 2);
    assert.equal(calls[0][2], '0.0.1+41');
    assert.equal(result.manifest.version, '0.0.1+41');
    assert.equal(result.receipt.published, false);
    assert.equal(result.receipt.artifacts.length, 2);
    assert(!JSON.stringify(result.manifest).includes(directory));
    await assert.rejects(createUpdateFeed({ ...options, verify: async () => { throw Error('invalid signature'); } }), /invalid signature/);
    await assert.rejects(createUpdateFeed({ ...options, verify: async () => '0'.repeat(64) }), /changed during/);
    await assert.rejects(createUpdateFeed({ ...options, artifacts: artifacts.slice(0, 1) }), /Both/);
    await writeFile(artifacts[0].provenance, JSON.stringify({ version: '0.0.1', build: '41', sourceCommit: 'a'.repeat(40), sourceDirty: true }));
    await assert.rejects(createUpdateFeed(options), /Clean/);
  } finally { await rm(directory, { recursive: true, force: true }); }
});
