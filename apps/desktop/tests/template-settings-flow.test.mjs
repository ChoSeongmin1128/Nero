import assert from 'node:assert/strict';
import test from 'node:test';
import { Window } from 'happy-dom';
import { bindTemplateDialogKeys, createTemplateSettingsFlow } from '../src/template-settings-flow.js';

const document = { id: 'meeting', name: '회의', description: '회의 요약', prompt: '기본 지침', revision: 'r1', builtIn: true, customized: false };
const deferred = () => { let resolve; const promise = new Promise(done => { resolve = done; }); return { promise, resolve }; };
const flush = () => new Promise(resolve => setImmediate(resolve));
const setup = async overrides => {
  const bridge = { readTemplateCatalog: async () => ({ revision: 'catalog:1', templates: [document] }), getTemplate: async () => document, ...overrides };
  const flow = createTemplateSettingsFlow({ bridge });
  flow.setActive(true);
  await flush();
  return flow;
};

test('repeated active rendering does not reload and inactive responses are ignored', async () => {
  const pending = deferred();
  let calls = 0;
  const flow = createTemplateSettingsFlow({ bridge: { readTemplateCatalog: () => { calls++; return pending.promise; } } });
  flow.setActive(true);
  flow.setActive(true);
  assert.equal(calls, 1);
  flow.setActive(false);
  pending.resolve({ revision: 'catalog:1', templates: [document] });
  await flush();
  assert.deepEqual(flow.getSnapshot().items, []);
});

test('saving carries exact revision and preserves draft on conflict until explicit reload', async () => {
  let args;
  const flow = await setup({ updateTemplate: async (...values) => { args = values; throw Error('템플릿이 변경되었습니다. 최신 내용을 확인한 뒤 다시 저장해 주세요.'); } });
  await flow.open('meeting');
  flow.edit('prompt', '내 지침');
  await flow.save();
  assert.deepEqual(args, ['meeting', 'r1', { name: '회의', description: '회의 요약', prompt: '내 지침' }]);
  assert.equal(flow.getSnapshot().draft.prompt, '내 지침');
  assert.equal(flow.isDirty(), true);
  assert.match(flow.getSnapshot().error, /최신 내용/);
  flow.setActive(false);
  flow.setActive(true);
  assert.equal(flow.getSnapshot().draft.prompt, '내 지침');
  await flow.reload();
  assert.equal(flow.getSnapshot().draft.prompt, '기본 지침');
});

test('blank drafts do not create persisted templates and duplicate saves are blocked', async () => {
  const pending = deferred();
  let calls = 0;
  const flow = await setup({ createTemplate: async content => { calls++; await pending.promise; return { ...content, id: 'custom', revision: 'new', builtIn: false }; } });
  flow.create();
  await flow.save();
  assert.equal(calls, 0);
  flow.edit('name', '새 템플릿');
  flow.edit('prompt', '내용');
  const save = flow.save();
  await flow.save();
  assert.equal(calls, 1);
  pending.resolve();
  await save;
  assert.equal(flow.isDirty(), false);
  assert.equal(flow.getSnapshot().document.id, 'custom');
});

test('individual reset failure preserves local edits', async () => {
  const flow = await setup({ resetTemplate: async () => { throw Error('복원 실패'); } });
  await flow.open('meeting');
  flow.edit('prompt', '유지할 내용');
  await flow.reset();
  assert.equal(flow.getSnapshot().draft.prompt, '유지할 내용');
});

test('custom deletion uses the current revision and refreshes the single template list', async () => {
  const custom = { ...document, id: 'custom', builtIn: false, revision: 'r2' };
  let items = [document, custom];
  let args;
  const flow = await setup({ readTemplateCatalog: async () => ({ revision: 'catalog:1', templates: items }), getTemplate: async () => custom, deleteTemplate: async (...values) => { args = values; items = [document]; } });
  await flow.open('custom');
  await flow.remove();
  assert.deepEqual(args, ['custom', 'r2']);
  assert.deepEqual(flow.getSnapshot().items, [document]);
  assert.equal(flow.getSnapshot().document, null);
  assert.equal(flow.getSnapshot().draft, null);
  assert.equal('deleted' in flow.getSnapshot(), false);
  assert.equal('showDeleted' in flow, false);
  assert.equal('restore' in flow, false);
});

test('built-in deletion uses the individual revision and can leave the catalog empty', async () => {
  let items = [document], args;
  const flow = await setup({
    readTemplateCatalog: async () => ({ revision: items.length ? 'catalog:1' : 'catalog:2', templates: items }),
    deleteTemplate: async (...values) => { args = values; items = []; },
  });
  await flow.open('meeting'); assert.equal(await flow.remove(), true);
  assert.deepEqual(args, ['meeting', 'r1']);
  assert.deepEqual(flow.getSnapshot().items, []);
  assert.equal(flow.getSnapshot().catalogRevision, 'catalog:2');
});

test('reset-all atomically consumes the observed catalog revision including an empty catalog', async () => {
  let expected;
  const flow = await setup({ readTemplateCatalog: async () => ({ revision: 'empty:1', templates: [] }), resetTemplateCatalog: async revision => { expected = revision; return { revision: 'defaults:2', templates: [document] }; } });
  assert.equal(await flow.resetAll(), true);
  assert.equal(expected, 'empty:1');
  assert.equal(flow.getSnapshot().catalogRevision, 'defaults:2');
  assert.deepEqual(flow.getSnapshot().items, [document]);
});

test('reset-all conflicts retain the observed list and do not silently retry over a concurrent addition', async () => {
  const added = { ...document, id: 'new', builtIn: false };
  let items = [document], revision = 'catalog:1', calls = 0;
  const flow = await setup({ readTemplateCatalog: async () => ({ revision, templates: items }), resetTemplateCatalog: async expected => {
    calls++; if (expected !== revision) throw Error('목록이 변경되었습니다.');
    items = [document]; revision = 'catalog:3'; return { revision, templates: items };
  } });
  items = [document, added]; revision = 'catalog:2';
  assert.equal(await flow.resetAll(), false);
  assert.equal(calls, 1); assert.equal(items.length, 2);
  assert.equal(flow.getSnapshot().catalogRevision, 'catalog:1');
  assert.match(flow.getSnapshot().error, /변경/);
  await flow.reload(); assert.equal(flow.getSnapshot().items.length, 2);
  assert.equal(await flow.resetAll(), true); assert.equal(calls, 2);
});

test('reset-all keeps write admission exclusive and never replaces an open draft', async () => {
  const pending = deferred(); let calls = 0;
  const flow = await setup({ resetTemplateCatalog: async () => { calls++; await pending.promise; return { revision: 'catalog:2', templates: [document] }; } });
  await flow.open('meeting'); flow.edit('prompt', '초안');
  assert.equal(await flow.resetAll(), false); assert.equal(calls, 0);
  flow.discard(); await flow.back();
  const saving = flow.resetAll(); assert.equal(flow.getSnapshot().writing, true);
  assert.equal(await flow.resetAll(), false); assert.equal(calls, 1);
  pending.resolve(); await saving; assert.equal(flow.getSnapshot().writing, false);
});

test('child Escape never reaches the parent and composing Escape does not dismiss', () => {
  const window = new Window();
  const container = window.document.createElement('dialog');
  const input = window.document.createElement('input');
  container.append(input);
  window.document.body.append(container);
  let parent = 0, dismissed = 0;
  const unbind = bindTemplateDialogKeys(container, { isOpen: () => true, cancel: () => dismissed++ });
  container.addEventListener('keydown', () => parent++);
  input.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Escape', isComposing: true, bubbles: true }));
  assert.equal(dismissed, 0);
  input.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
  assert.equal(dismissed, 1);
  assert.equal(parent, 0);
  unbind();
  window.happyDOM.abort();
});

test('failed explicit reload retains the unsaved draft and original CAS revision', async () => {
  let fail = false;
  const flow = await setup({ getTemplate: async () => { if (fail) throw Error('불러오기 실패'); return document; } });
  await flow.open('meeting');
  flow.edit('prompt', '보존할 초안');
  fail = true;
  await flow.reload();
  assert.equal(flow.getSnapshot().draft.prompt, '보존할 초안');
  assert.equal(flow.getSnapshot().document.revision, 'r1');
});
