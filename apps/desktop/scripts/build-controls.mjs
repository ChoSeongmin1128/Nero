import { build } from 'esbuild';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
import { mkdir } from 'node:fs/promises';

const app = fileURLToPath(new URL('../', import.meta.url));
export async function buildControls(output) {
  await mkdir(resolve(output, '..'), { recursive: true });
  await build({
    entryPoints: [resolve(app, '../../design-system/controls.js')], outfile: output,
    bundle: true, minify: true, format: 'iife', target: ['es2022', 'chrome110', 'safari15'],
    nodePaths: [resolve(app, 'node_modules')], define: { 'process.env.NODE_ENV': '"production"' },
    jsx: 'automatic', legalComments: 'eof',
  });
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const output = process.argv[2];
  if (!output) throw new Error('Usage: node build-controls.mjs OUTPUT_JS');
  await buildControls(resolve(output));
}
