import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { Window } from 'happy-dom';

test('application chrome suppresses selection while content, prompts and editable fields retain it', async t => {
  const window = new Window();
  t.after(() => window.happyDOM.abort());
  const document = window.document;
  document.write(`<html><head><style>${await readFile(new URL('../../../design-system/components.css', import.meta.url), 'utf8')}</style></head><body class="application-shell">
    <h1>Library</h1><button>New meeting</button><label>Name<input value="Editable"></label><textarea>Notes</textarea>
    <article class="summary-document"><p>Summary</p><button>Citation</button></article>
    <div class="transcript-content">Transcript</div><div class="transcript-preview">Provisional text</div>
    <div class="summary-evidence-entry">Source passage</div><div class="notes-search-preview">Notes match</div>
    <div class="notice">Failure details<button>Retry</button></div><pre><code>Agent prompt</code></pre>
    <div contenteditable="true">Editable content</div><div contenteditable="false">Chrome</div>
  </body></html>`);
  const selection = selector => window.getComputedStyle(document.querySelector(selector));
  assert.equal(selection('body').userSelect, 'none');
  for (const selector of ['button', 'label', '.summary-document button', '.notice button']) {
    assert.equal(selection(selector).userSelect, 'none', selector);
  }
  for (const selector of ['input', 'textarea', '.summary-document', '.transcript-content', '.transcript-preview', '.summary-evidence-entry', '.notes-search-preview', '.notice', 'pre', 'code', '[contenteditable="true"]']) {
    assert.equal(selection(selector).userSelect, 'text', selector);
  }
});
