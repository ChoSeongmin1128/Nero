import { build } from 'esbuild';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
import { mkdir } from 'node:fs/promises';

const app = fileURLToPath(new URL('../', import.meta.url));
const options = {
  entryPoints: [resolve(app, '../../design-system/markdown.mjs')],
  bundle: true, minify: true, format: 'iife', target: ['es2022', 'chrome110', 'safari15'],
  nodePaths: [resolve(app, 'node_modules')], legalComments: 'eof',
  // The same bundle runs in Node checks, so entity decoding must not need a DOM.
  conditions: ['worker'],
};

export async function buildMarkdown(output) {
  await mkdir(resolve(output, '..'), { recursive: true });
  await build({ ...options, outfile: output });
}

export async function markdownSource() {
  const result = await build({ ...options, minify: false, write: false });
  return result.outputFiles[0].text;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const output = process.argv[2];
  if (!output) throw new Error('Usage: node build-markdown.mjs OUTPUT_JS');
  await buildMarkdown(resolve(output));
}
