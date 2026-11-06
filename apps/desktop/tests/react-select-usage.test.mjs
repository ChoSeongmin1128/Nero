import assert from 'node:assert/strict';
import test from 'node:test';
import { readdir, readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

const source = fileURLToPath(new URL('../src/', import.meta.url));

// Vanilla screens use native selects that NeroControls enhances; React never sees that
// enhancement, so React screens must use the shared Select component instead.
test('React screens use the shared Select instead of native select menus', async () => {
  const files = (await readdir(source)).filter(name => name.endsWith('.jsx'));
  assert.ok(files.length > 0);
  const offenders = [];
  for (const name of files) if (/<select[\s>]/.test(await readFile(source + name, 'utf8'))) offenders.push(name);
  assert.deepEqual(offenders, []);
});
