import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { Window } from 'happy-dom';
import config from '../vite.config.js';

test('only the native macOS main document reserves the system window controls', () => {
  const transform = config.plugins.find(plugin => plugin.name === 'nero-native-window-chrome').transformIndexHtml;
  const previous = process.env.TAURI_ENV_TARGET_TRIPLE;
  try {
    for (const target of ['aarch64-apple-darwin', 'x86_64-pc-windows-msvc', '']) {
      process.env.TAURI_ENV_TARGET_TRIPLE = target;
      for (const entry of ['index', 'recorder', 'calendar']) {
        const native = fileURLToPath(new URL(`../src/${entry}.html`, import.meta.url));
        for (const filename of new Set([native, native.replaceAll('\\', '/')])) {
          const result = transform('<html lang="ko"><body></body></html>', { filename });
          assert.equal(result.includes('data-window-chrome="macos-overlay"'), target.endsWith('-apple-darwin') && entry === 'index');
        }
      }
    }
  } finally {
    if (previous === undefined) delete process.env.TAURI_ENV_TARGET_TRIPLE;
    else process.env.TAURI_ENV_TARGET_TRIPLE = previous;
  }
});

test('native window controls survive and toolbar actions are outside the drag target', async t => {
  const native = JSON.parse(await readFile(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'));
  const main = native.app.windows.find(window => window.label === 'main');
  assert.equal(main.titleBarStyle, 'Overlay');
  assert.equal(main.hiddenTitle, true);
  assert.notEqual(main.decorations, false);
  const permission = JSON.parse(await readFile(new URL('../src-tauri/capabilities/macos-window.json', import.meta.url), 'utf8'));
  assert.deepEqual(permission.platforms, ['macOS']);
  assert.deepEqual(permission.windows, ['main']);
  assert.deepEqual(permission.permissions, ['core:window:allow-internal-toggle-maximize', 'core:window:allow-is-fullscreen']);
  const window = new Window({ settings: { disableCSSFileLoading: true, disableJavaScriptFileLoading: true } });
  t.after(() => window.happyDOM.abort());
  window.document.write(await readFile(new URL('../src/index.html', import.meta.url), 'utf8'));
  const header = window.document.querySelector('.application-toolbar');
  assert.equal(header.hasAttribute('data-tauri-drag-region'), true);
  assert.equal(header.querySelectorAll('button[data-tauri-drag-region]').length, 0);
});

test('native controls inset matches the platform and fullscreen state', async t => {
  const css = await readFile(new URL('../../../design-system/navigation.css', import.meta.url), 'utf8');
  for (const [attributes, expected] of [
    ['data-window-chrome="macos-overlay"', '88px'],
    ['data-window-chrome="macos-overlay" data-window-fullscreen="true"', '12px'],
    ['data-window-chrome="macos-overlay" data-window-fullscreen="false"', '88px'],
    ['', '12px'],
  ]) {
    const window = new Window();
    t.after(() => window.happyDOM.abort());
    window.document.write(`<html ${attributes}><head><style>${css}</style></head><body class="application-shell"><header class="application-toolbar"></header></body></html>`);
    assert.equal(window.getComputedStyle(window.document.querySelector('.application-toolbar')).paddingLeft, expected);
  }
});
