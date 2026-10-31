const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');
const root = path.resolve(__dirname, '..');
const read = (name) => fs.readFileSync(path.join(root, name), 'utf8');
const { parseCSS } = require('../catalog/build-tokens.cjs');
const desktopRoot = path.resolve(root, '../apps/desktop');
const desktopRead = name => fs.readFileSync(path.join(desktopRoot, name), 'utf8');
const layerRules = parseCSS(read('controls.css'), 'controls.css');
const layers = Object.fromEntries(layerRules.filter(rule => rule.selector === ':root' && rule.property.startsWith('--layer-')).map(rule => [rule.property, Number(rule.value)]));
assert.ok(layers['--layer-popover'] > layers['--layer-dialog-content'] && layers['--layer-dialog-content'] > layers['--layer-dialog-overlay'], 'Nested calendar/select controls must receive pointer input above their modal');
for (const entry of ['catalog/index.html', 'prototype/index.html', 'prototype/plan.html', 'prototype/navigation-review.html', 'windows-preview/index.html']) {
  const html = read(entry);
  for (const file of ['fonts.css', 'typography.css', 'components.css', ...(/(?:plan|navigation-review)\.html$/.test(entry) ? [] : ['controls.css'])]) {
    assert.ok(html.includes('../' + file), `${entry} does not load ${file}`);
  }
}
for (const [script, entry] of [['prototype/app.js', 'prototype/index.html'], ['catalog/catalog.js', 'catalog/index.html']]) {
  const source = read(script);
  assert.ok(read(entry).includes('../components.js'), `${entry} does not load shared views`);
  for (const component of ['meetingRow', 'personRow', 'utterance', 'minutes', 'dialogContent']) {
    assert.ok(source.includes('UI.' + component), `${script} does not use ${component}`);
  }
  for (const duplicated of ['<article class="utterance', '<article class="minutes-document', '<span class="person-voice-status"']) {
    assert.ok(!source.includes(duplicated), `${script} duplicated shared markup: ${duplicated}`);
  }
}
const typography = read('typography.css');
assert.match(typography, /--font-ui:\s*"Pretendard Variable",/);
const fonts = read('fonts.css');
assert.match(fonts, /font-weight:\s*45 920/);
assert.ok(!/https?:|local\(/.test(fonts), 'App fonts must load from bundled assets, independent of the network and installed fonts');
const fontPath = fonts.match(/url\("([^\"]+)"\)/)?.[1];
assert.equal(fontPath, './vendor/pretendard/PretendardVariable.woff2');
const fontSource = JSON.parse(read('vendor/pretendard/SOURCE.json'));
for (const file of fontSource.files) {
  const bytes = fs.readFileSync(path.join(root, 'vendor/pretendard', file.file));
  assert.equal(crypto.createHash('sha256').update(bytes).digest('hex'), file.sha256, `Bundled font provenance mismatch: ${file.file}`);
}
assert.equal(fs.readFileSync(path.join(root, fontPath)).toString('ascii', 0, 4), 'wOF2');
for (const role of ['body', 'label', 'caption', 'aux', 'subtitle', 'reading', 'heading', 'title', 'display', 'code', 'timer']) {
  assert.ok(typography.includes('.type-' + role), `Missing typography role: ${role}`);
}
assert.match(typography, /--font-ui:[^;]*Malgun Gothic[^;]*Apple SD Gothic Neo|--font-ui:[^;]*Apple SD Gothic Neo[^;]*Malgun Gothic/);
assert.match(typography, /font-variant-numeric:\s*tabular-nums/);
for (const file of ['style.css', 'components.css', 'controls.css', 'desktop-preview.css', 'windows-preview/windows.css', 'windows-preview/bridge.css']) {
  const css = read(file);
  assert.ok(!/font-size:\s*\d/.test(css), `${file} gained a raw product text size`);
  assert.ok(!/Avenir|Segoe UI|Malgun Gothic|SFMono-Regular|Consolas/.test(css), `${file} gained a separate font-family contract`);
}
const desktopHTML = desktopRead('src/index.html');
const copyStep = desktopRead('scripts/prepare-shared.mjs');
for (const file of ['appearance.css', 'fonts.css', 'typography.css', 'components.css', 'controls.css']) {
  assert.ok(desktopHTML.includes('/shared/' + file), `Native desktop does not load shared ${file}`);
  assert.ok(copyStep.includes(`'${file}'`), `Native desktop does not package shared ${file}`);
}
assert.ok(copyStep.includes("'appearance.js'"), 'Native desktop must package appearance behavior, not only colors');
assert.ok(copyStep.includes("'vendor/pretendard'"), 'Native desktop must package the font and its OFL notice');
for (const entry of ['src/index.html', 'src/recorder.html', 'src/calendar.html']) {
  const html = desktopRead(entry);
  assert.ok(html.includes('/shared/fonts.css'), `${entry} must load the bundled font`);
  assert.match(html, /data-appearance-storage="nero\.appearance\.v1"/, `${entry} must share the native appearance preference`);
  assert.ok(html.indexOf('/shared/appearance.js') > 0 && html.indexOf('/shared/appearance.js') < html.indexOf('/shared/appearance.css'), `${entry} must apply saved appearance before paint`);
}
for (const name of ['appearance', 'recording', 'agent', 'templates']) assert.ok(desktopHTML.includes(`id="settings-${name}-panel"`), `Native settings missing ${name}`);
assert.ok(!desktopHTML.includes('id="settings-models-panel"'), 'Optional feature preparation belongs in General settings');
for (const [id, variant] of [['meeting-title', 'text-input--title'], ['meeting-notes', 'text-input'], ['search', 'text-input--compact']]) {
  const tag = [...desktopHTML.matchAll(/<(?:input|textarea)\b[^>]*>/g)].map(match => match[0]).find(tag => tag.includes(`id="${id}"`));
  assert.ok(tag?.match(/class="([^"]*)"/)?.[1].split(/\s+/).includes(variant), `${id} must consume shared ${variant}`);
}
assert.match(desktopRead('src/main.js'), /NeroUI\.transcriptRow\(/, 'Native transcript must use the shared reading/editing component');
assert.match(read('components.js'), /<textarea class="text-input"[^;]*data-segment/, 'Transcript editing must use shared text-field states');
const desktopCSS = parseCSS(desktopRead('src/style.css'), 'apps/desktop/src/style.css');
for (const declaration of desktopCSS) {
  if (/(?:#meeting-notes|\.meeting-title|\.search-field\s+input)(?![\w-])/.test(declaration.selector)) {
    assert.ok(!/^(?:border(?:-|$)|background(?:-|$)|padding(?:-|$)|outline(?:-|$))/.test(declaration.property),
      `${declaration.selector} overrides shared text-field appearance with ${declaration.property}`);
  }
}
console.log('PASS: shared foundations, native consumers, text-field ownership, typography and platform fonts.');
