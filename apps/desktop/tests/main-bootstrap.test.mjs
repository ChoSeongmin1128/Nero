import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { build } from 'esbuild';
import { Window } from 'happy-dom';

const root = fileURLToPath(new URL('../', import.meta.url));
const html = await readFile(new URL('../src/index.html', import.meta.url), 'utf8');
const main = await readFile(new URL('../src/main.js', import.meta.url), 'utf8');
const bridgeFiles = await Promise.all(['bridge.js', 'calendar-bridge.js'].map(name => readFile(new URL(`../src/${name}`, import.meta.url), 'utf8')));
const bridgeNames = [...new Set(bridgeFiles.flatMap(source => [...source.matchAll(/^export (?:async )?(?:const|function) (\w+)/gm)].map(match => match[1])))];
const bundled = await build({
  entryPoints: [root + 'src/main.js'], bundle: true, write: false, format: 'iife', jsx: 'automatic',
  nodePaths: [root + 'node_modules'], define: { 'process.env.NODE_ENV': '"production"' }, loader: { '.css': 'empty' },
  plugins: [{ name: 'test-native-boundary', setup(builder) {
    builder.onResolve({ filter: /^\.\/bridge\.js$/ }, () => ({ path: 'bridge', namespace: 'test-bridge' }));
    builder.onLoad({ filter: /.*/, namespace: 'test-bridge' }, () => ({ contents: bridgeNames.map(name => `export const ${name} = (...args) => globalThis.testBridge.${name}(...args);`).join('\n') }));
    builder.onLoad({ filter: /\/src\/main\.js$/ }, () => {
      const imports = main.match(/^import .*;$/gm).join('\n');
      return { contents: `${imports}\nglobalThis.neroBoot = (async () => {\n${main.replace(/^import .*;\n/gm, '')}\n})();`, loader: 'js' };
    });
  } }],
});

test('the real application HTML boots and opens all settings tabs without stale element bindings', async t => {
  const window = new Window({ url: 'http://nero.test', settings: { enableJavaScriptEvaluation: true, suppressInsecureJavaScriptEnvironmentWarning: true, disableCSSFileLoading: true, disableJavaScriptFileLoading: true } });
  t.after(() => window.happyDOM.abort());
  const errors = [];
  window.addEventListener('error', event => errors.push(event.error || event.message));
  window.document.write(html);
  for (const name of ['appearance', 'icons', 'components']) window.eval(await readFile(new URL(`../../../design-system/${name}.js`, import.meta.url), 'utf8'));
  const calls = [];
  const snapshot = { revision: 1, session: null, meetings: [], folders: [] };
  const stubs = {
    readSnapshot: () => snapshot, listDevices: () => [],
    processingStatus: () => ({ status: 'idle', supported: true, modelReady: true, summarySupported: true, summaryReady: true }),
    setAutoRecorder() {},
    readTerminologySettings: () => ({ available: false }),
    agentConnectionInfo: () => ({ available: false, clients: [] }),
    readCalendarSettings: () => ({ revision: 1, configured: false, connected: false, calendars: [], selectedCalendars: [] }),
    listTemplates: () => [],
  };
  window.testBridge = Object.fromEntries(bridgeNames.map(name => [name, async (...args) => {
    calls.push(name);
    if (name.startsWith('subscribe')) return () => {};
    assert.ok(Object.hasOwn(stubs, name), `Unexpected native call: ${name}`);
    return stubs[name](...args);
  }]));
  window.eval(bundled.outputFiles[0].text);
  await window.neroBoot;
  assert.deepEqual(errors, []);
  assert.equal(window.document.getElementById('error').hidden, true);
  assert.equal(window.document.getElementById('create-meeting').disabled, false);
  const wait = async predicate => {
    for (let attempt = 0; attempt < 100; attempt++) {
      if (predicate()) return;
      await new Promise(resolve => setTimeout(resolve, 5));
    }
    assert.fail('Expected settings state was not rendered');
  };
  window.document.getElementById('settings').click();
  await wait(() => window.document.getElementById('settings-dialog').open);
  for (const tab of window.document.querySelectorAll('#settings-dialog [role="tab"][data-settings-panel]')) {
    tab.click();
    await wait(() => tab.getAttribute('aria-selected') === 'true');
    assert.equal(window.document.getElementById(`settings-${tab.dataset.settingsPanel}-panel`).hidden, false);
  }
  window.document.getElementById('settings-close').click();
  await wait(() => !window.document.getElementById('settings-dialog').open);
  await wait(() => window.document.activeElement.id === 'navigation-toggle');
  assert.equal(window.document.activeElement.id, 'navigation-toggle');
  assert.deepEqual(errors, []);
  assert.equal(window.document.getElementById('error').hidden, true);
  assert.ok(calls.includes('readSnapshot'));
});
