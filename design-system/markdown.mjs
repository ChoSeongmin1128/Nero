import { fromMarkdown } from 'mdast-util-from-markdown';
import { gfm } from 'micromark-extension-gfm';
import { gfmFromMarkdown } from 'mdast-util-gfm';
import remend from 'remend';

// A streamed answer can stop inside a citation such as "[2, 5-"; remend would turn that
// into a placeholder link, so the unfinished citation is dropped until it completes.
const partialCitation = /\[[0-9, \t-]*$/;

function parse(text, { streaming = false } = {}) {
  let source = String(text ?? '');
  if (streaming) source = remend(source.replace(partialCitation, ''));
  return fromMarkdown(source, { extensions: [gfm()], mdastExtensions: [gfmFromMarkdown()] });
}

globalThis.NeroMarkdown = Object.freeze({ parse });
