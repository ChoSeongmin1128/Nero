const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');

const root = path.resolve(__dirname, '..');
const inputs = ['appearance.css', 'fonts.css', 'typography.css', 'components.css', 'style.css', 'controls.css', 'desktop-preview.css', 'windows-preview/windows.css', 'windows-preview/bridge.css', 'catalog/catalog.css', 'appearance.js', 'components.js', 'controls.js', 'windows-preview/windows.js', 'windows-preview/bridge.js'];
const output = path.join(__dirname, 'tokens.json');
const normalize = (value) => value.replace(/\s+/g, ' ').trim();
const hash = (value) => crypto.createHash('sha256').update(value).digest('hex');

function parseCSS(source, file) {
  const clean = source.replace(/\/\*[\s\S]*?\*\//g, (comment) => comment.replace(/[^\n]/g, ' '));
  const declarations = [];
  const stack = [];
  let start = 0;
  let quote = '';
  let round = 0;
  let square = 0;
  function declaration(end) {
    const raw = clean.slice(start, end);
    const match = raw.match(/^\s*([\w-]+)\s*:\s*([\s\S]*?)\s*$/);
    const block = stack.at(-1);
    if (!match || !block || block.header.startsWith('@')) return;
    const offset = start + raw.search(/\S/);
    const before = source.slice(0, offset);
    const line = before.split('\n').length;
    const column = offset - before.lastIndexOf('\n');
    declarations.push({
      id: `${file}:${line}:${column}:${match[1]}`,
      property: match[1],
      value: normalize(match[2]),
      file, line, column,
      selector: block.header,
      conditions: stack.filter((entry) => entry.header.startsWith('@')).map((entry) => entry.header),
    });
  }
  for (let index = 0; index < clean.length; index += 1) {
    const ch = clean[index];
    if (quote) {
      if (ch === '\\') index += 1;
      else if (ch === quote) quote = '';
      continue;
    }
    if (ch === '"' || ch === "'") { quote = ch; continue; }
    if (ch === '(') round += 1;
    if (ch === ')') round -= 1;
    if (ch === '[') square += 1;
    if (ch === ']') square -= 1;
    if (round || square) continue;
    if (ch === '{') {
      stack.push({ header: normalize(clean.slice(start, index)) });
      start = index + 1;
    } else if (ch === ';') {
      declaration(index);
      start = index + 1;
    } else if (ch === '}') {
      declaration(index);
      if (!stack.length) throw new Error(`Unmatched CSS close brace: ${file}`);
      stack.pop();
      start = index + 1;
    }
  }
  if (stack.length || quote || round || square) throw new Error(`Unclosed CSS syntax: ${file}`);
  return declarations;
}

function metricCategory(property) {
  if (/^(font|line-height|letter-spacing|word-spacing|text-)/.test(property)) return 'typography';
  if (/^(gap|row-gap|column-gap|padding|margin)/.test(property)) return 'spacing';
  if (/radius/.test(property)) return 'radius';
  if (/^(transition|animation)/.test(property)) return 'motion';
  if (/^(width|height|min-width|min-height|max-width|max-height|grid-template|flex-basis)/.test(property)) return 'density';
  if (/^(outline|box-shadow|stroke-width)/.test(property)) return 'state';
  return null;
}

function tokenType(name, value) {
  if (['--body', '--display', '--mono', '--font-ui', '--font-code'].includes(name)) return 'fontFamily';
  if (/radius|width|size|tracking/.test(name)) return 'dimension';
  if (name.startsWith('--weight-')) return 'fontWeight';
  if (name.startsWith('--leading-')) return 'lineHeight';
  if (/^#[\da-f]+$/i.test(value)) return 'color';
  if (value.includes('var(')) return 'reference';
  return 'string';
}

function rootMatches(selector, color, mode) {
  return selector.split(',').some((branch) => {
    const text = branch.trim();
    if (!/^:root(?:\[[^\]]+\])*$/.test(text)) return false;
    const foundColor = text.match(/\[data-color="([^"]+)"\]/)?.[1];
    const foundMode = text.match(/\[data-mode="([^"]+)"\]/)?.[1];
    return (!foundColor || foundColor === color) && (!foundMode || foundMode === mode);
  });
}

function resolveReferences(values) {
  const resolved = {};
  function resolve(name, chain = []) {
    if (chain.includes(name)) throw new Error(`Variable cycle: ${[...chain, name].join(' -> ')}`);
    if (!(name in values)) throw new Error(`Missing root variable: ${name}`);
    return values[name].replace(/var\((--[\w-]+)\)/g, (_, ref) => resolve(ref, [...chain, name]));
  }
  for (const name of Object.keys(values)) resolved[name] = resolve(name);
  return resolved;
}

function build() {
  const sources = inputs.map((file) => {
    const text = fs.readFileSync(path.join(root, file), 'utf8');
    return { file, text, sha256: hash(text), bytes: Buffer.byteLength(text) };
  });
  const declarations = sources.filter((source) => source.file.endsWith('.css')).flatMap((source) => parseCSS(source.text, source.file));
  const appearanceJS = sources.find((source) => source.file === 'appearance.js').text;
  const setValues = (name) => {
    const match = appearanceJS.match(new RegExp(`const ${name} = new Set\\(\\[([^\\]]+)\\]\\)`));
    if (!match) throw new Error(`Missing appearance set: ${name}`);
    return [...match[1].matchAll(/"([^"]+)"/g)].map((entry) => entry[1]);
  };
  const colorIds = ['mono'];
  const preferenceModes = setValues('modes');
  const labels = { mono: 'Nero' };
  const tokens = declarations.filter((item) => item.property.startsWith('--')).map(({ property, ...item }) => ({
    ...item,
    name: property,
    type: tokenType(property, item.value),
    level: item.file === 'desktop-preview.css' || item.file.startsWith('windows-preview/') ? 'component' : item.file === 'typography.css' || ['--body', '--display', '--mono', '--radius'].includes(property) ? 'primitive' : 'semantic',
  }));
  const defined = new Set(tokens.map((token) => token.name));
  const references = declarations.flatMap((item) => [...item.value.matchAll(/var\((--[\w-]+)(?:,([^)]*))?\)/g)].map((match) => ({
    name: match[1], fallback: match[2] ? normalize(match[2]) : null,
    declaration: item.id, file: item.file, line: item.line, selector: item.selector,
  })));
  const unresolvedReferences = references.filter((ref) => !defined.has(ref.name) && !ref.fallback);
  const resolved = colorIds.flatMap((color) => ['light', 'dark'].map((mode) => {
    const values = {};
    const provenance = {};
    for (const token of tokens) {
      if (!token.conditions.length && rootMatches(token.selector, color, mode)) {
        values[token.name] = token.value;
        provenance[token.name] = token.id;
      }
    }
    return { id: `${color}-${mode}`, color, mode, values, resolvedValues: resolveReferences(values), provenance };
  }));
  const duplicateGroups = new Map();
  for (const item of declarations) {
    const key = JSON.stringify([item.selector, item.conditions, item.property]);
    if (!duplicateGroups.has(key)) duplicateGroups.set(key, []);
    duplicateGroups.get(key).push(item);
  }
  const overrides = [...duplicateGroups.values()].filter((group) => group.length > 1).map((group) => ({
    selector: group[0].selector, conditions: group[0].conditions, property: group[0].property,
    declarations: group.map(({ id, value, file, line }) => ({ id, value, file, line })),
  }));
  const statusNames = ['--warning-text', '--warning-surface', '--recording-text', '--recording-surface', '--ready-text', '--ready-surface'];
  const independentStatus = statusNames.every((name) => ['light', 'dark'].every((mode) => new Set(resolved.filter((theme) => theme.mode === mode).map((theme) => theme.values[name])).size === 1));
  return {
    meta: {
      schemaVersion: 1,
      scope: 'Canonical browser design system with shared typography roles, controls and views. Native OS behavior requires separate verification.',
      sourceOrder: inputs,
      sources: sources.map(({ text, ...source }) => source),
      generate: 'node design-system/catalog/build-tokens.cjs',
      check: 'node design-system/catalog/check-tokens.cjs',
      declarationCount: declarations.length,
      tokenDeclarationCount: tokens.length,
      uniqueTokenCount: defined.size,
      platforms: {
        common: ['appearance tokens', 'text hierarchy', 'list density', 'controls', 'content states'],
        macOSPrototype: ['notch placement', 'menu bar', 'floating recorder', 'desktop sample chrome'],
        windows: {
          browserReference: ['shared product iframe', 'title bar', 'taskbar and tray', 'floating recorder', 'notification', 'focus return'],
          nativeImplementation: 'unverified',
          sizing: 'Browser reference values only; not verified Windows shell metrics.',
        },
      },
    },
    themes: {
      colors: colorIds.map((id) => ({ id, label: labels[id] || id })),
      preferenceModes,
      resolvedModes: ['light', 'dark'],
      status: {
        names: statusNames, independentOfColor: independentStatus, modeDependent: true,
        roles: statusNames.map((name) => ({
          name,
          independentOfColor: ['light', 'dark'].every((mode) => new Set(resolved.filter((theme) => theme.mode === mode).map((theme) => theme.values[name])).size === 1),
          values: Object.fromEntries(resolved.map((theme) => [theme.id, theme.values[name]])),
        })),
        exception: null,
      },
      resolved,
      desktop: {
        palette: 'neutral, depends on light/dark only',
        chromeTokens: tokens.filter((item) => item.name.startsWith('--desktop-')).map((item) => item.id),
        overlay: 'neutral dark surface for all color/mode combinations',
        overlayTokens: tokens.filter((item) => item.name.startsWith('--overlay-')).map((item) => item.id),
      },
      windows: {
        palette: 'Browser shell neutral colors depend on light/dark, not brand color.',
        tokens: tokens.filter((item) => item.name.startsWith('--win-')).map((item) => item.id),
        typography: 'Bundled Pretendard Variable with shared typography.css roles for both platform previews and the common product iframe.',
        nativeImplementation: 'unverified',
      },
    },
    tokens,
    metrics: declarations.filter((item) => !item.property.startsWith('--') && metricCategory(item.property)).map((item) => ({ ...item, category: metricCategory(item.property) })),
    rawValues: {
      description: 'Literal color inventory. Not a named primitive palette or a proposed numeric scale.',
      colors: [...new Set(declarations.flatMap((item) => [...item.value.matchAll(/#[\da-f]{3,8}\b/gi)].map((match) => match[0])))].sort(),
      componentColorLiterals: declarations.filter((item) => !item.property.startsWith('--') && /#[\da-f]{3,8}\b/i.test(item.value)),
    },
    audit: {
      unresolvedReferences,
      overrides,
      naming: 'Semantic color and typography roles are shared. Platform aliases remain component tokens. Component geometry remains with its owner; raw values are an inventory rather than a universal scale.',
      unresolvedDesignDecisions: [
        'Typography roles are applied; a universal spacing/radius scale is intentionally not asserted for unrelated component geometry.',
        'Different control heights and focus widths remain actual per-component decisions, not a unified standard.',
        'Windows browser shell is a reference. Native Windows panels, notifications and accessibility behavior remain unverified.',
        'Native macOS accessibility and shell behavior are not established by this web prototype.',
      ],
    },
  };
}

if (require.main === module) {
  const result = build();
  const expected = JSON.stringify(result, null, 2) + '\n';
  if (process.argv.includes('--check')) {
    if (!fs.existsSync(output) || fs.readFileSync(output, 'utf8') !== expected) {
      console.error('tokens.json differs from current source. Run node design-system/catalog/build-tokens.cjs after reviewing source changes.');
      process.exitCode = 1;
    } else console.log('PASS: tokens.json matches current source.');
  } else {
    fs.writeFileSync(output, expected);
    console.log(`Generated tokens.json: ${result.tokens.length} declarations, ${result.meta.uniqueTokenCount} names, ${result.themes.resolved.length} resolved themes.`);
  }
}

module.exports = { build, parseCSS, rootMatches, resolveReferences };
