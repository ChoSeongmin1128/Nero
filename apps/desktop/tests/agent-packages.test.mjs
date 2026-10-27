import assert from 'node:assert/strict';
import { readFile, mkdtemp } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { prepareAgentPackages, targetPlatform } from '../scripts/agent-packages.mjs';

const root = fileURLToPath(new URL('../../../', import.meta.url));
const source = join(root, 'apps/desktop/agent');
const icon = join(root, 'apps/desktop/src-tauri/icons/icon.png');
const json = async path => JSON.parse(await readFile(path, 'utf8'));

for (const [target, platform, binary] of [
  ['aarch64-apple-darwin', 'darwin', 'nero'],
  ['x86_64-pc-windows-msvc', 'win32', 'nero.exe'],
]) {
  test(`packages ${target} without a developer runtime or machine-specific paths`, async () => {
    const destination = await mkdtemp(join(tmpdir(), 'nero-agent-package-'));
    await prepareAgentPackages({ source, destination, icon, version: '0.0.1-nero.19', target });
    const claude = await json(join(destination, 'claude-manifest.json'));
    assert.equal(claude.manifest_version, '0.3');
    assert.equal(claude.version, '0.0.1-nero.19');
    assert.equal(claude.server.type, 'binary');
    assert.equal(claude.server.entry_point, `server/${binary}`);
    assert.equal(claude.server.mcp_config.command, '${__dirname}/server/' + binary);
    assert.deepEqual(claude.server.mcp_config.args, ['mcp']);
    assert.deepEqual(claude.server.mcp_config.env, {});
    assert.deepEqual(claude.compatibility, { platforms: [platform] });
    assert.equal(claude.user_config, undefined);
    const cli = await readFile(join(root, 'crates/nero-cli/src/lib.rs'), 'utf8');
    const methods = [...cli.match(/pub const METHODS[^=]*=\s*\[([^\]]+)\]/s)[1].matchAll(/"([a-z_]+)"/g)].map(match => match[1]);
    assert.deepEqual(claude.tools.map(tool => tool.name), methods);
    const portable = await json(join(destination, 'codex-plugin.json'));
    assert.equal(portable.$schema, 'https://agent-plugins.org/schemas/1.0.0/plugin.schema.json');
    assert.equal(portable.name, 'nero');
    assert.equal(portable.version, '0.0.1-nero.19');
    assert.equal(portable.mcpServers, undefined);
    assert.equal(portable.extensions['com.openai'].interface.displayName, 'Nero');
    const legacy = await json(join(destination, 'codex-legacy-plugin.json'));
    assert.equal(legacy.skills, './skills/');
    assert.equal(legacy.mcpServers, './.mcp.json');
    const marketplace = await json(join(destination, 'codex-marketplace.json'));
    assert.deepEqual(marketplace.plugins[0].source, { source: 'local', path: './plugins/nero' });
    const texts = await Promise.all(['claude-manifest', 'codex-plugin', 'codex-legacy-plugin', 'codex-marketplace'].map(name => readFile(join(destination, `${name}.json`), 'utf8')));
    for (const text of texts) assert.doesNotMatch(text, /\/Users\/|[A-Z]:\\|python|node_modules|https?:\/\/(?!agent-plugins\.org)/);
    assert.deepEqual(await readFile(join(destination, 'icon.png')), await readFile(icon));
    const skill = await readFile(join(destination, 'meeting-summary/SKILL.md'), 'utf8');
    assert.match(skill, /pagination\.hasMore/);
    assert.match(skill, /sourceVersion/);
    assert.match(skill, /읽기 전용/);
    assert.doesNotMatch(skill, /글로랑|AX팀/);
    assert.equal(resolve(destination, claude.server.entry_point), join(destination, 'server', binary));
  });
}

test('rejects unknown targets and invalid versions before package creation', async () => {
  assert.throws(() => targetPlatform('universal-apple'), /Unsupported/);
  await assert.rejects(prepareAgentPackages({ source, destination: '/not-used', icon, version: 'invalid', target: 'aarch64-apple-darwin' }), /Invalid/);
});
