import { execFileSync, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { copyFile, lstat, readFile } from 'node:fs/promises';
import { join } from 'node:path';

const files = ['claude-manifest.json', 'codex-marketplace.json', 'codex-plugin.json', 'codex-legacy-plugin.json', 'meeting-summary/SKILL.md', 'icon.png'];
const digest = bytes => createHash('sha256').update(bytes).digest('hex');
async function regular(path) {
  const stat = await lstat(path);
  if (!stat.isFile() || stat.isSymbolicLink() || stat.size > 64 * 1024 * 1024) throw Error('Invalid reusable agent file.');
  return readFile(path);
}
function verifySignedApp(app, binary) {
  if (process.platform !== 'darwin') throw Error('Signed agent reuse currently requires macOS.');
  execFileSync('codesign', ['--verify', '--deep', '--strict', app], { stdio: 'pipe' });
  const inspect = path => {
    const result = spawnSync('codesign', ['-d', '--verbose=4', path], { encoding: 'utf8' });
    if (result.error || result.status !== 0) throw Error('The signed agent identity could not be read.');
    const text = result.stdout + result.stderr;
    if (!text.includes('Authority=Developer ID Application:') || !text.includes('runtime')) throw Error('A Developer ID app and agent with hardened runtime are required.');
    return text;
  };
  return inspect(app) + inspect(binary);
}

export async function reuseSignedAgent({ app, destination, metadata, verify = verifySignedApp }) {
  if (!app.endsWith('.app')) throw Error('A signed Nero app is required for agent reuse.');
  const prior = join(app, 'Contents/Resources/agent');
  const previous = JSON.parse(await regular(join(prior, 'agent-build.json')));
  for (const key of ['version', 'integrationVersion', 'apiVersion', 'target', 'binary', 'buildSha256']) {
    if (previous[key] !== metadata[key]) throw Error(`Agent ${key} changed; do not reuse the signed component.`);
  }
  for (const file of files) {
    const before = await regular(join(prior, 'integrations', file));
    const current = await regular(join(destination, 'integrations', file));
    if (!before.equals(current)) throw Error('Agent templates changed; do not reuse the signed component.');
  }
  const executable = join(prior, metadata.binary);
  const signature = await verify(app, executable);
  const teams = [...signature.matchAll(/^TeamIdentifier=([A-Z0-9]{10})$/gm)].map(match => match[1]);
  if (teams.length !== 2 || teams[0] !== teams[1] || !signature.includes('Authority=Developer ID Application:') || !signature.includes('runtime')) throw Error('The reusable agent must have the app Developer ID and hardened runtime.');
  const binary = await regular(executable);
  await copyFile(executable, join(destination, metadata.binary));
  if (digest(await regular(join(destination, metadata.binary))) !== digest(binary)) throw Error('Reusable agent copy changed.');
  return { sha256: digest(binary), teamIdentifier: teams[0] };
}
