import assert from 'node:assert/strict';
import { readFile, mkdtemp, mkdir, cp, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { prepareAgentPackages, targetPlatform } from '../scripts/agent-packages.mjs';
import { reuseSignedAgent } from '../scripts/reuse-signed-agent.mjs';

const root = fileURLToPath(new URL('../../../', import.meta.url));
const source = join(root, 'apps/desktop/agent');
const icon = join(root, 'apps/desktop/src-tauri/icons/icon.png');
const json = async path => JSON.parse(await readFile(path, 'utf8'));
const { integrationVersion } = await json(join(source, 'release.json'));

for (const [target, platform, binary] of [
  ['aarch64-apple-darwin', 'darwin', 'nero'],
  ['x86_64-pc-windows-msvc', 'win32', 'nero.exe'],
]) {
  test(`packages ${target} without a developer runtime or machine-specific paths`, async () => {
    const destination = await mkdtemp(join(tmpdir(), 'nero-agent-package-'));
    await prepareAgentPackages({ source, destination, icon, version: integrationVersion, target });
    const claude = await json(join(destination, 'claude-manifest.json'));
    assert.equal(claude.manifest_version, '0.3');
    assert.equal(claude.version, integrationVersion);
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
    assert.equal(portable.version, integrationVersion);
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

async function reusableAgent() {
  const directory = await mkdtemp(join(tmpdir(), 'nero-agent-reuse-'));
  const destination = join(directory, 'current');
  const app = join(directory, 'Nero.app');
  const prior = join(app, 'Contents/Resources/agent');
  await prepareAgentPackages({source, destination:join(destination,'integrations'), icon, version:integrationVersion, target:'aarch64-apple-darwin'});
  await mkdir(prior,{recursive:true});
  await cp(join(destination,'integrations'),join(prior,'integrations'),{recursive:true});
  const metadata = {version:'0.0.1',integrationVersion,apiVersion:1,target:'aarch64-apple-darwin',binary:'nero',buildSha256:'a'.repeat(64)};
  await writeFile(join(prior,'agent-build.json'),JSON.stringify(metadata));
  await writeFile(join(prior,'nero'),'signed executable fixture');
  await writeFile(join(destination,'nero'),'fresh executable fixture');
  const signature = 'Authority=Developer ID Application: Fixture\nTeamIdentifier=ABCDEFGHIJ\nflags=0x10000(runtime)\n';
  return {app,destination,metadata,prior,verify:async()=>signature.repeat(2)};
}

test('unchanged signed agent reuse preserves exact client generation inputs', async () => {
  const fixture = await reusableAgent();
  const receipt = await reuseSignedAgent(fixture);
  const binary = await readFile(join(fixture.destination,'nero'));
  assert.deepEqual(binary,await readFile(join(fixture.prior,'nero')));
  assert.equal(receipt.sha256,createHash('sha256').update(binary).digest('hex'));
  assert.equal(receipt.teamIdentifier,'ABCDEFGHIJ');
});

test('changed executable build or integration templates reject component reuse', async () => {
  const fixture = await reusableAgent();
  await assert.rejects(reuseSignedAgent({...fixture,metadata:{...fixture.metadata,buildSha256:'b'.repeat(64)}}),/buildSha256 changed/);
  await assert.rejects(reuseSignedAgent({...fixture,metadata:{...fixture.metadata,integrationVersion:'0.0.1-nero.19'}}),/integrationVersion changed/);
  await writeFile(join(fixture.destination,'integrations/meeting-summary/SKILL.md'),'changed instructions');
  await assert.rejects(reuseSignedAgent(fixture),/templates changed/);
  assert.equal(await readFile(join(fixture.destination,'nero'),'utf8'),'fresh executable fixture');
});

test('invalid signatures and different signing teams cannot reuse a component', async () => {
  const fixture = await reusableAgent();
  await assert.rejects(reuseSignedAgent({...fixture,verify:async()=>{throw Error('signature invalid');}}),/signature invalid/);
  const wrongTeam = 'Authority=Developer ID Application: Fixture\nTeamIdentifier=ABCDEFGHIJ\nflags=runtime\nTeamIdentifier=1234567890\n';
  await assert.rejects(reuseSignedAgent({...fixture,verify:async()=>wrongTeam}),/Developer ID/);
  assert.equal(await readFile(join(fixture.destination,'nero'),'utf8'),'fresh executable fixture');
});
