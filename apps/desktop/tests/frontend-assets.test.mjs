import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { build } from 'vite';
import config from '../vite.config.js';

const desktop = fileURLToPath(new URL('..', import.meta.url));

async function inventory(directory, prefix = '') {
  const result = [];
  for (const item of await readdir(directory, { withFileTypes: true })) {
    const name = prefix + item.name;
    if (item.isDirectory()) result.push(...await inventory(join(directory, item.name), name + '/'));
    else result.push(name);
  }
  return result.sort();
}

test('frontend build includes public assets without embedding native workers, agent or release metadata', async t => {
  const fixture = await mkdtemp(join(tmpdir(), 'nero-frontend-assets-'));
  t.after(() => rm(fixture, { recursive: true, force: true }));
  const files = {
    'src/index.html': '<!doctype html><link rel="stylesheet" href="/shared/theme.css"><img src="/brand/logo.svg"><script src="/shared/control.js"></script>',
    '.generated/public/shared/theme.css': 'body { color: black; }',
    '.generated/public/shared/control.js': 'window.neroFixture = true;',
    '.generated/public/shared/licenses/example.txt': 'Example license',
    '.generated/public/brand/logo.svg': '<svg xmlns="http://www.w3.org/2000/svg"/>',
    '.generated/native/worker.bin': 'native-worker-sentinel',
    '.generated/agent/nero.exe': 'local-agent-sentinel',
    '.generated/release/build-info.json': '{"source":"release-only-sentinel"}',
    '.generated/shared/stale.js': 'stale-generated-sentinel',
  };
  for (const [name, bytes] of Object.entries(files)) {
    await mkdir(resolve(fixture, name, '..'), { recursive: true });
    await writeFile(resolve(fixture, name), bytes);
  }
  const output = resolve(fixture, relative(desktop, config.build.outDir));
  await build({
    ...config,
    configFile: false,
    logLevel: 'silent',
    root: resolve(fixture, relative(desktop, config.root)),
    publicDir: resolve(fixture, relative(desktop, config.publicDir)),
    build: {
      ...config.build,
      outDir: output,
      rollupOptions: { input: resolve(fixture, 'src/index.html') },
    },
  });
  assert.deepEqual(await inventory(output), [
    'brand/logo.svg', 'index.html', 'shared/control.js', 'shared/licenses/example.txt', 'shared/theme.css',
  ]);
  for (const name of ['shared/control.js', 'shared/licenses/example.txt', 'shared/theme.css', 'brand/logo.svg']) {
    assert.equal(await readFile(join(output, name), 'utf8'), files['.generated/public/' + name]);
  }
  const html = await readFile(join(output, 'index.html'), 'utf8');
  assert.match(html, /\/shared\/theme\.css/);
  assert.match(html, /\/shared\/control\.js/);
  assert.match(html, /\/brand\/logo\.svg/);
});
