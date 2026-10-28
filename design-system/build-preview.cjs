const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const assert = require('node:assert/strict');
const { execFileSync } = require('node:child_process');
const { build } = require('./catalog/build-tokens.cjs');

const markerName = '.grove-design-system-preview.json';
const manifestName = 'preview-manifest.json';
const marker = { owner: 'grove-design-system-preview', version: 1 };
const source = fs.realpathSync(__dirname);
const hash = (value) => crypto.createHash('sha256').update(value).digest('hex');

function filesAt(folder, prefix = '') {
  return fs.readdirSync(folder, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name)).flatMap((entry) => {
    const relative = prefix ? `${prefix}/${entry.name}` : entry.name;
    const absolute = path.join(folder, entry.name);
    if (entry.isSymbolicLink()) throw new Error(`Symbolic links are not preview inputs or outputs: ${relative}`);
    if (entry.isDirectory()) return filesAt(absolute, relative);
    if (!entry.isFile()) throw new Error(`Unsupported filesystem entry: ${relative}`);
    return [relative];
  });
}

function realDestination(value) {
  let current = path.resolve(value);
  const suffix = [];
  while (!fs.existsSync(current)) {
    suffix.unshift(path.basename(current));
    const parent = path.dirname(current);
    if (parent === current) throw new Error('Cannot resolve output parent.');
    current = parent;
  }
  return path.join(fs.realpathSync(current), ...suffix);
}

function main() {
  const args = process.argv.slice(2);
  if (args.length !== 2 || args[0] !== '--output' || !args[1]) throw new Error('Usage: node design-system/build-preview.cjs --output NEW_OUTPUT_DIRECTORY');
  if (fs.existsSync(path.resolve(args[1])) && fs.lstatSync(path.resolve(args[1])).isSymbolicLink()) throw new Error('Output must not be a symbolic link.');
  const output = realDestination(args[1]);
  const relativeOutput = path.relative(source, output);
  if (!relativeOutput || (!relativeOutput.startsWith(`..${path.sep}`) && relativeOutput !== '..' && !path.isAbsolute(relativeOutput))) throw new Error('Output must be outside the canonical source directory.');
  const currentTokens = build();
  const savedTokens = JSON.parse(fs.readFileSync(path.join(source, 'catalog/tokens.json'), 'utf8'));
  assert.deepEqual(savedTokens, currentTokens, 'Token source drift. Review changes and run node design-system/catalog/build-tokens.cjs first.');
  const files = filesAt(source);
  if (files.includes(markerName) || files.includes(manifestName)) throw new Error('Generated preview metadata must not be stored in the canonical source.');
  if (fs.existsSync(output)) {
    if (!fs.statSync(output).isDirectory()) throw new Error('Output is not a directory.');
    const markerPath = path.join(output, markerName);
    if (!fs.existsSync(markerPath)) throw new Error('Refusing existing output without the owned preview marker.');
    assert.deepEqual(JSON.parse(fs.readFileSync(markerPath, 'utf8')), marker, 'Output is not an owned Grove design-system preview.');
    const allowed = new Set([...files, markerName, manifestName]);
    const unrelated = filesAt(output).filter((file) => !allowed.has(file));
    if (unrelated.length) throw new Error(`Refusing output with unrelated or stale files: ${unrelated.join(', ')}. Choose a new directory.`);
  } else fs.mkdirSync(output, { recursive: true });
  fs.writeFileSync(path.join(output, markerName), JSON.stringify(marker, null, 2) + '\n');
  const manifest = { schemaVersion: 1, files: [] };
  for (const file of files) {
    const input = path.join(source, file);
    const target = path.join(output, file);
    fs.mkdirSync(path.dirname(target), { recursive: true });
    fs.copyFileSync(input, target);
    if (file === 'controls.js') {
      execFileSync(process.execPath, [path.join(source, '../apps/desktop/scripts/build-controls.mjs'), target], { stdio: 'inherit' });
    }
    const bytes = fs.readFileSync(target);
    manifest.files.push({ file, bytes: bytes.length, sha256: hash(bytes), ...(file === 'controls.js' ? { sourceSha256: hash(fs.readFileSync(input)), entry: 'react-controls.jsx' } : {}) });
  }
  fs.writeFileSync(path.join(output, manifestName), JSON.stringify(manifest, null, 2) + '\n');
  console.log(`Built ${files.length} files: ${output}`);
}

try { main(); }
catch (error) { console.error(error.message); process.exitCode = 1; }
