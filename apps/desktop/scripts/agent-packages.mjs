import { readFile, writeFile, mkdir, copyFile } from 'node:fs/promises';
import { join } from 'node:path';

const documents = ['claude-manifest', 'codex-marketplace', 'codex-plugin', 'codex-legacy-plugin'];

export function targetPlatform(target) {
  if (target.endsWith('-apple-darwin')) return 'darwin';
  if (target.includes('-windows-')) return 'win32';
  if (target.includes('-linux-')) return 'linux';
  throw Error('Unsupported agent package target.');
}

export async function prepareAgentPackages({ source, destination, icon, version, target }) {
  if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version)) throw Error('Invalid agent package version.');
  const platform = targetPlatform(target);
  const binary = platform === 'win32' ? 'nero.exe' : 'nero';
  await mkdir(join(destination, 'meeting-summary'), { recursive: true });
  for (const name of documents) {
    const value = JSON.parse(await readFile(join(source, `${name}.template.json`), 'utf8'));
    if (name !== 'codex-marketplace') value.version = version;
    if (name === 'claude-manifest') {
      value.server.entry_point = `server/${binary}`;
      value.server.mcp_config.command = '${__dirname}/server/' + binary;
      value.compatibility.platforms = [platform];
    }
    await writeFile(join(destination, `${name}.json`), JSON.stringify(value, null, 2) + '\n');
  }
  await copyFile(join(source, 'meeting-summary/SKILL.md'), join(destination, 'meeting-summary/SKILL.md'));
  await copyFile(icon, join(destination, 'icon.png'));
}
