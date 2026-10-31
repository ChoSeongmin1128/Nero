import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, readFile, rm, access } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { verifyAppUpdateRelease, downloadPublicFile } from '../scripts/verify-app-update-release.mjs';

const sha = bytes => createHash('sha256').update(bytes).digest('hex');
const repo = 'https://github.com/ChoSeongmin1128/Nero';
async function fixture(t, mutate = () => {}) {
  const directory = await mkdtemp(path.join(tmpdir(), 'nero-public-test-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const objects = new Map(), verified = [], requested = [];
  const receipt = { version: '0.0.1', build: '44', sourceCommit: 'a'.repeat(40), sourceTree: 'b'.repeat(40), tag: 'nero-build44', published: false, artifacts: [] };
  const manifest = { version: '0.0.1+44', platforms: {} };
  for (const [platform, filename] of [['darwin-aarch64', 'Nero.app.tar.gz'], ['windows-x86_64', 'Nero-setup.exe']]) {
    const bytes = Buffer.from(`synthetic ${platform}`), url = `${repo}/releases/download/${receipt.tag}/${filename}`;
    receipt.artifacts.push({ platform, filename, bytes: bytes.length, sha256: sha(bytes) });
    manifest.platforms[platform] = { url, signature: 'synthetic signature' }; objects.set(url, bytes); objects.set(`${url}.sig`, Buffer.from('synthetic signature'));
  }
  mutate({ receipt, manifest });
  const feed = path.join(directory, 'latest.json'), receiptFile = `${feed}.receipt.json`;
  await writeFile(feed, JSON.stringify(manifest)); await writeFile(receiptFile, JSON.stringify(receipt));
  const feedUrl = `${repo}/releases/download/${receipt.tag}/latest.json`;
  objects.set(feedUrl, await readFile(feed)); objects.set(`${repo}/releases/latest/download/latest.json`, await readFile(feed));
  const options = { feed, receipt: receiptFile, feedUrl, verifier: '/synthetic/verifier',
    fetchImpl: async (url, init) => { requested.push(url); assert.equal(init.redirect, 'manual'); assert.equal(init.credentials, 'omit'); assert(objects.has(url)); return new Response(objects.get(url)); },
    verify: async (verifier, file, signature, wire) => { verified.push(file); assert.equal(verifier, '/synthetic/verifier'); assert.equal(wire, '0.0.1+44'); assert.equal(await readFile(signature, 'utf8'), 'synthetic signature'); return sha(await readFile(file)); },
    now: () => '2026-10-31T00:00:00.000Z',
  };
  return { directory, options, objects, verified, requested, manifest };
}

test('public tag and latest endpoints verify both downloaded payloads and clean temporary files', async t => {
  const f = await fixture(t);
  for (const feedUrl of [f.options.feedUrl, `${repo}/releases/latest/download/latest.json`]) {
    const receipt = await verifyAppUpdateRelease({ ...f.options, feedUrl });
    assert.equal(receipt.publicURL, feedUrl); assert.equal(receipt.sourceTree, 'b'.repeat(40));
    assert.equal(receipt.signatureVerified, true); assert.equal(receipt.installationVerified, false);
    assert.equal(receipt.artifacts.length, 2);
    for (const artifact of receipt.artifacts) {
      assert.equal(artifact.publicSignatureURL, `${artifact.publicURL}.sig`);
      assert.equal(artifact.signatureSha256, sha('synthetic signature'));
    }
    assert(!JSON.stringify(receipt).includes(f.directory));
  }
  assert.equal(f.verified.length, 4);
  for (const file of f.verified) await assert.rejects(access(file));
});

test('changed published feed fails before downloading any package', async t => {
  const f = await fixture(t); f.objects.set(f.options.feedUrl, Buffer.from(JSON.stringify(f.manifest) + '\n'));
  await assert.rejects(verifyAppUpdateRelease(f.options), /feed bytes differ/);
  assert.equal(f.requested.length, 1); assert.equal(f.verified.length, 0);
});

test('tampered package or wrong authenticated hash fails verification', async t => {
  const f = await fixture(t); const url = f.manifest.platforms['darwin-aarch64'].url;
  const original = f.objects.get(url); f.objects.set(url, Buffer.from('tampered'));
  await assert.rejects(verifyAppUpdateRelease(f.options), /artifact bytes or hash/);
  assert.equal(f.verified.length, 0);
  f.objects.set(url, original);
  await assert.rejects(verifyAppUpdateRelease({ ...f.options, verify: async () => '0'.repeat(64) }), /Native verifier hash/);
  await assert.rejects(verifyAppUpdateRelease({ ...f.options, verify: async () => { throw Error('bad signature'); } }), /bad signature/);
});

test('missing platform and malicious asset URLs fail before the first network call', async t => {
  for (const mutate of [
    ({ manifest }) => { delete manifest.platforms['windows-x86_64']; },
    ({ receipt }) => { receipt.artifacts.pop(); },
    ...['http://github.com/ChoSeongmin1128/Nero/releases/download/nero-build44/Nero.app.tar.gz', 'https://user@github.com/ChoSeongmin1128/Nero/releases/download/nero-build44/Nero.app.tar.gz', `${repo}/releases/download/nero-build44/Nero.app.tar.gz?token=x`, `${repo}/releases/download/nero-build44/Nero.app.tar.gz#x`, 'https://evil.test/package'].map(url => ({ manifest }) => { manifest.platforms['darwin-aarch64'].url = url; }),
  ]) {
    const f = await fixture(t, mutate);
    await assert.rejects(verifyAppUpdateRelease(f.options)); assert.equal(f.requested.length, 0);
  }
});

test('redirects allow signed HTTPS CDN URLs but reject downgrade, credentials, fragments and loops', async t => {
  const directory = await mkdtemp(path.join(tmpdir(), 'nero-download-test-')); t.after(() => rm(directory, { recursive: true, force: true }));
  for (const [index, location] of ['http://cdn.test/file', 'https://user:pass@cdn.test/file', 'https://cdn.test/file#x'].entries()) {
    let calls = 0;
    await assert.rejects(downloadPublicFile({ url: `${repo}/file`, destination: path.join(directory, String(index)), maxBytes: 10, fetchImpl: async () => { calls++; return new Response(null, { status: 302, headers: { location } }); } }), /Unsafe/);
    assert.equal(calls, 1);
  }
  let calls = 0;
  const result = await downloadPublicFile({ url: `${repo}/file`, destination: path.join(directory, 'valid'), maxBytes: 10, fetchImpl: async url => ++calls === 1 ? new Response(null, { status: 302, headers: { location: 'https://cdn.test/file?signature=secret' } }) : (assert.equal(url, 'https://cdn.test/file?signature=secret'), new Response('ok')) });
  assert.deepEqual(result, { bytes: 2, sha256: sha('ok') });
  await assert.rejects(downloadPublicFile({ url: `${repo}/file`, destination: path.join(directory, 'loop'), maxBytes: 10, maxRedirects: 2, fetchImpl: async () => new Response(null, { status: 302, headers: { location: 'https://cdn.test/file' } }) }), /Redirect limit/);
});

test('streaming byte caps and timeout remain enforced without Content-Length', async t => {
  const directory = await mkdtemp(path.join(tmpdir(), 'nero-download-test-')); t.after(() => rm(directory, { recursive: true, force: true }));
  await assert.rejects(downloadPublicFile({ url: `${repo}/file`, destination: path.join(directory, 'large'), maxBytes: 2, fetchImpl: async () => new Response('large') }), /size limit/);
  await assert.rejects(downloadPublicFile({ url: `${repo}/file`, destination: path.join(directory, 'timeout'), maxBytes: 10, timeoutMs: 5, fetchImpl: (_, { signal }) => new Promise((resolve, reject) => signal.addEventListener('abort', () => reject(Error('timed out')), { once: true })) }), /timed out/);
});


test('missing or tampered public signature fails before native verification', async t => {
  const f = await fixture(t);
  const signatureURL = `${f.manifest.platforms['darwin-aarch64'].url}.sig`;
  await assert.rejects(verifyAppUpdateRelease({ ...f.options, fetchImpl: (url, init) => url === signatureURL ? Promise.resolve(new Response('Not found', { status: 404 })) : f.options.fetchImpl(url, init) }), /did not return a file/);
  assert.equal(f.verified.length, 0);
  f.objects.set(signatureURL, Buffer.from('different signature'));
  await assert.rejects(verifyAppUpdateRelease(f.options), /Published signature differs/);
  assert.equal(f.verified.length, 0);
});
