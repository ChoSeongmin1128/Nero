import { readFile, stat, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const MAX_PACKAGE = 512 * 1024 * 1024;
const TARGETS = { 'darwin-aarch64': ['darwin', 'arm64'], 'windows-x86_64': ['win32', 'x64'] };
const BUNDLE_IDENTIFIER = 'io.github.choseongmin1128.nero.desktop';
const SOURCE_FIELDS = ['version', 'build', 'sourceCommit', 'sourceTree', 'sourceDirty'];
export function releaseIdentity(version, build) {
  if (typeof version !== 'string' || typeof build !== 'string' || !/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(version) || !/^[1-9]\d*$/.test(build) || BigInt(build) > 18446744073709551615n) throw Error('Invalid product version or exact integer build');
  return `${version}+${build}`;
}

export function verifyNative(verifier, file, signature, wire) {
  const result = spawnSync(verifier, ['--nero-verify-app-update', file, signature, wire], { shell: false, windowsHide: true, timeout: 120_000, encoding: 'utf8' });
  if (result.error || result.status !== 0) throw Error('Native signature/build verification failed');
  const digest = result.stdout.trim();
  if (!/^[0-9a-f]{64}$/.test(digest)) throw Error('Native verifier did not return its authenticated payload hash');
  return digest;
}

export async function createUpdateFeed({ version, build, tag, artifacts, verify }) {
  const wire = releaseIdentity(version, build);
  if (!/^[A-Za-z0-9][A-Za-z0-9._-]{0,99}$/.test(tag || '')) throw Error('Invalid release tag');
  if (typeof verify !== 'function') throw Error('Signature verifier is required');
  const expected = Object.keys(TARGETS);
  if (artifacts.length !== 2 || expected.some(platform => artifacts.filter(a => a.platform === platform).length !== 1)) throw Error('Both macOS arm64 and Windows x64 artifacts are required');
  const platforms = {}, evidence = [];
  let sourceCommit, sourceTree;
  for (const artifact of [...artifacts].sort((a, b) => a.platform.localeCompare(b.platform))) {
    const file = path.resolve(artifact.file), signatureFile = path.resolve(artifact.signature);
    const filename = path.basename(file);
    if (!/^[A-Za-z0-9][A-Za-z0-9._-]+$/.test(filename)) throw Error('Artifact filename must be URL-safe ASCII');
    if (artifact.platform === 'darwin-aarch64' ? !filename.endsWith('.app.tar.gz') : !filename.endsWith('-setup.exe')) throw Error('Incorrect updater artifact format');
    const size = (await stat(file)).size;
    if (size === 0 || size > MAX_PACKAGE) throw Error('Artifact is empty or exceeds size limit');
    if ((await stat(signatureFile)).size > 16 * 1024) throw Error('Signature exceeds size limit');
    const signature = (await readFile(signatureFile, 'utf8')).trim();
    const provenance = JSON.parse(await readFile(artifact.provenance, 'utf8'));
    const [hostPlatform, hostArchitecture] = TARGETS[artifact.platform];
    if (!provenance || provenance.formatVersion !== 1 || provenance.version !== version || provenance.build !== build || provenance.sourceDirty !== false || !/^[0-9a-f]{40}$/.test(provenance.sourceCommit || '') || !/^[0-9a-f]{40}$/.test(provenance.sourceTree || '') || provenance.bundleIdentifier !== BUNDLE_IDENTIFIER || provenance.hostPlatform !== hostPlatform || provenance.hostArchitecture !== hostArchitecture) throw Error('Clean, matching packaged build provenance and target are required');
    if (sourceCommit && sourceCommit !== provenance.sourceCommit) throw Error('Platform source commits differ');
    if (sourceTree && sourceTree !== provenance.sourceTree) throw Error('Platform source trees differ');
    sourceCommit = provenance.sourceCommit;
    sourceTree = provenance.sourceTree;
    if (typeof artifact.receipt !== 'string') throw Error('Artifact packaging receipt is required');
    const packaging = JSON.parse(await readFile(artifact.receipt, 'utf8'));
    if (!packaging || packaging.schemaVersion !== 1 || packaging.platform !== artifact.platform || SOURCE_FIELDS.some(field => packaging[field] !== provenance[field])) throw Error('Artifact packaging receipt differs from packaged build provenance');
    const bytes = await readFile(file);
    const sha256 = createHash('sha256').update(bytes).digest('hex');
    // The receipt is trusted local packaging evidence, not an independent signature.
    if (bytes.length !== size || !packaging.artifact || packaging.artifact.filename !== filename || packaging.artifact.bytes !== bytes.length || packaging.artifact.sha256 !== sha256) throw Error('Artifact filename, size or hash differs from its packaging receipt');
    if (await verify(file, signatureFile, wire) !== sha256) throw Error('Artifact changed during native verification');
    if ((await readFile(signatureFile, 'utf8')).trim() !== signature) throw Error('Signature changed during native verification');
    platforms[artifact.platform] = { url: `https://github.com/ChoSeongmin1128/Nero/releases/download/${tag}/${filename}`, signature };
    evidence.push({ platform: artifact.platform, filename, bytes: size, sha256 });
  }
  return { manifest: { version: wire, platforms }, receipt: { version, build, sourceCommit, sourceTree, tag, artifacts: evidence, published: false } };
}

async function main(args) {
  const names = ['version', 'build', 'tag', 'verifier', 'mac', 'mac-provenance', 'mac-receipt', 'windows', 'windows-provenance', 'windows-receipt', 'out'];
  const options = {};
  for (let index = 0; index < args.length; index += 2) {
    const name = args[index]?.replace(/^--/, '');
    if (!args[index]?.startsWith('--') || !names.includes(name) || options[name] || !args[index + 1]) throw Error('Invalid or duplicate option');
    options[name] = args[index + 1];
  }
  if (names.some(name => !options[name])) throw Error(`Required: ${names.map(name => `--${name}`).join(' ')}`);
  const verifier = path.resolve(options.verifier);
  const { manifest, receipt } = await createUpdateFeed({ version: options.version, build: options.build, tag: options.tag,
    artifacts: [{ platform: 'darwin-aarch64', file: options.mac, signature: `${options.mac}.sig`, provenance: options['mac-provenance'], receipt: options['mac-receipt'] }, { platform: 'windows-x86_64', file: options.windows, signature: `${options.windows}.sig`, provenance: options['windows-provenance'], receipt: options['windows-receipt'] }],
    verify: (file, signature, wire) => verifyNative(verifier, file, signature, wire) });
  await writeFile(`${options.out}.receipt.json`, JSON.stringify(receipt, null, 2) + '\n', { flag: 'wx' });
  await writeFile(options.out, JSON.stringify(manifest, null, 2) + '\n', { flag: 'wx' });
  console.log('Verified local feed created. Nothing was published.');
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2)).catch(error => { console.error(error.message); process.exitCode = 1; });
}
