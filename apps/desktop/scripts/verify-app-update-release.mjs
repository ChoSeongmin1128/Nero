import { mkdtemp, open, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { createHash } from 'node:crypto';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { releaseIdentity, verifyNative } from './app-update-feed.mjs';

const REPOSITORY = 'https://github.com/ChoSeongmin1128/Nero';
const PLATFORMS = ['darwin-aarch64', 'windows-x86_64'];
const MAX_FEED = 64 * 1024;
const MAX_PACKAGE = 512 * 1024 * 1024;
const SHA = /^[0-9a-f]{64}$/;
const HASH = /^[0-9a-f]{40}$/;
const NAME = /^[A-Za-z0-9][A-Za-z0-9._-]+$/;
const digest = bytes => createHash('sha256').update(bytes).digest('hex');

function httpsURL(value, redirect = false) {
  const url = new URL(value);
  if (url.protocol !== 'https:' || url.username || url.password || url.hash || (!redirect && url.search)) throw Error('Unsafe public URL');
  return url;
}

export function validatePublicURL(value, expected) {
  const url = httpsURL(value);
  if (url.href !== expected) throw Error('Public URL must match the official repository release');
  return url.href;
}

export async function downloadPublicFile({ url, destination, maxBytes, fetchImpl = fetch, timeoutMs = 15 * 60 * 1000, maxRedirects = 5 }) {
  let current = httpsURL(url).href;
  const abort = new AbortController();
  const timer = setTimeout(() => abort.abort(), timeoutMs);
  let file;
  try {
    for (let redirects = 0; ; redirects++) {
      const response = await fetchImpl(current, { redirect: 'manual', signal: abort.signal, credentials: 'omit', referrerPolicy: 'no-referrer' });
      if ([301, 302, 303, 307, 308].includes(response.status)) {
        await response.body?.cancel();
        if (redirects >= maxRedirects) throw Error('Redirect limit exceeded');
        const location = response.headers.get('location');
        if (!location) throw Error('Missing redirect destination');
        // GitHub CDN redirects include short-lived signature queries; never record them.
        current = httpsURL(new URL(location, current).href, true).href;
        continue;
      }
      if (response.status !== 200 || !response.body) { await response.body?.cancel(); throw Error('Public download did not return a file'); }
      const declared = response.headers.get('content-length');
      if (declared !== null && (!/^\d+$/.test(declared) || Number(declared) > maxBytes)) { await response.body.cancel(); throw Error('Download exceeds size limit'); }
      file = await open(destination, 'wx');
      const hash = createHash('sha256'); let bytes = 0;
      for await (const chunk of response.body) {
        if (abort.signal.aborted) throw Error('Download timed out');
        bytes += chunk.length;
        if (bytes > maxBytes) throw Error('Download exceeds size limit');
        hash.update(chunk); await file.writeFile(chunk);
      }
      if (!bytes) throw Error('Empty public download');
      if (abort.signal.aborted) throw Error('Download timed out');
      return { bytes, sha256: hash.digest('hex') };
    }
  } finally { clearTimeout(timer); await file?.close(); }
}

export async function verifyAppUpdateRelease({ feed, receipt, feedUrl, verifier, fetchImpl = fetch, verify = verifyNative, now = () => new Date().toISOString() }) {
  const local = await readFile(feed);
  if (!local.length || local.length > MAX_FEED) throw Error('Local feed exceeds size limit');
  const receiptBytes = await readFile(receipt);
  if (receiptBytes.length > MAX_FEED) throw Error('Local receipt exceeds size limit');
  const expected = JSON.parse(receiptBytes), manifest = JSON.parse(local);
  const wire = releaseIdentity(expected.version, expected.build);
  if (!HASH.test(expected.sourceCommit || '') || !HASH.test(expected.sourceTree || '') || !/^[A-Za-z0-9][A-Za-z0-9._-]{0,99}$/.test(expected.tag || '') || expected.published !== false) throw Error('Invalid local feed provenance');
  if (manifest.version !== wire || !manifest.platforms || Object.keys(manifest.platforms).sort().join() !== [...PLATFORMS].sort().join()) throw Error('Feed must contain both supported platforms and the exact version');
  if (!Array.isArray(expected.artifacts) || expected.artifacts.length !== 2 || PLATFORMS.some(platform => expected.artifacts.filter(item => item.platform === platform).length !== 1)) throw Error('Receipt must contain both supported platforms');
  const tagURL = `${REPOSITORY}/releases/download/${expected.tag}/latest.json`;
  const latestURL = `${REPOSITORY}/releases/latest/download/latest.json`;
  if (feedUrl !== tagURL && feedUrl !== latestURL) throw Error('Feed URL must select the prepared release or the latest endpoint');
  validatePublicURL(feedUrl, feedUrl);
  for (const platform of PLATFORMS) {
    const artifact = expected.artifacts.find(item => item.platform === platform);
    const entry = manifest.platforms[platform];
    if (!NAME.test(artifact.filename || '') || !(platform === 'darwin-aarch64' ? artifact.filename.endsWith('.app.tar.gz') : artifact.filename.endsWith('-setup.exe')) || !Number.isSafeInteger(artifact.bytes) || artifact.bytes <= 0 || artifact.bytes > MAX_PACKAGE || !SHA.test(artifact.sha256 || '')) throw Error('Invalid artifact receipt');
    validatePublicURL(entry.url, `${REPOSITORY}/releases/download/${expected.tag}/${artifact.filename}`);
    if (typeof entry.signature !== 'string' || !entry.signature.trim() || Buffer.byteLength(entry.signature) > 16 * 1024) throw Error('Invalid signature in feed');
  }
  const temporary = await mkdtemp(path.join(tmpdir(), 'nero-public-update-'));
  try {
    const downloadedFeed = path.join(temporary, 'latest.json');
    await downloadPublicFile({ url: feedUrl, destination: downloadedFeed, maxBytes: MAX_FEED, fetchImpl });
    if (!(await readFile(downloadedFeed)).equals(local)) throw Error('Published feed bytes differ from the prepared feed');
    const artifacts = [];
    for (const platform of PLATFORMS) {
      const artifact = expected.artifacts.find(item => item.platform === platform), entry = manifest.platforms[platform];
      const file = path.join(temporary, artifact.filename), signature = `${file}.sig`;
      const downloaded = await downloadPublicFile({ url: entry.url, destination: file, maxBytes: MAX_PACKAGE, fetchImpl });
      if (downloaded.bytes !== artifact.bytes || downloaded.sha256 !== artifact.sha256) throw Error('Published artifact bytes or hash differ from the prepared receipt');
      const publicSignatureURL = `${entry.url}.sig`;
      validatePublicURL(publicSignatureURL, `${REPOSITORY}/releases/download/${expected.tag}/${artifact.filename}.sig`);
      const downloadedSignature = await downloadPublicFile({ url: publicSignatureURL, destination: signature, maxBytes: 16 * 1024, fetchImpl });
      if ((await readFile(signature, 'utf8')).trim() !== entry.signature.trim()) throw Error('Published signature differs from the prepared feed');
      if (await verify(verifier, file, signature, wire) !== downloaded.sha256) throw Error('Native verifier hash differs from the downloaded artifact');
      artifacts.push({ platform, filename: artifact.filename, bytes: downloaded.bytes, sha256: downloaded.sha256, publicURL: entry.url, publicSignatureURL, signatureSha256: downloadedSignature.sha256, signatureVerified: true });
    }
    return { schemaVersion: 1, verifiedAt: now(), version: expected.version, build: expected.build, wireVersion: wire, sourceCommit: expected.sourceCommit, sourceTree: expected.sourceTree, sourceDirty: false, tag: expected.tag, publicURL: feedUrl, feedSha256: digest(local), feedBytesMatch: true, artifacts, signatureVerified: true, installationVerified: false, scope: 'Public delivery and production signature/build verification only; no installation performed.' };
  } finally { await rm(temporary, { recursive: true, force: true }); }
}

async function main(args) {
  const names = ['feed', 'receipt', 'feed-url', 'verifier', 'out'], options = {};
  for (let index = 0; index < args.length; index += 2) {
    const name = args[index]?.replace(/^--/, '');
    if (!args[index]?.startsWith('--') || !names.includes(name) || options[name] || !args[index + 1]) throw Error('Invalid or duplicate option');
    options[name] = args[index + 1];
  }
  if (names.some(name => !options[name])) throw Error(`Required: ${names.map(name => `--${name}`).join(' ')}`);
  const result = await verifyAppUpdateRelease({ feed: options.feed, receipt: options.receipt, feedUrl: options['feed-url'], verifier: path.resolve(options.verifier) });
  await writeFile(options.out, JSON.stringify(result, null, 2) + '\n', { flag: 'wx' });
  console.log('Public delivery and signature verified. No installation performed.');
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2)).catch(() => { console.error('Public release verification failed; this run is not acceptance.'); process.exitCode = 1; });
}
