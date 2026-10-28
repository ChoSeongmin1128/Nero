import assert from 'node:assert/strict';
import test from 'node:test';
import { Window } from 'happy-dom';
import { bindTemplateDialogKeys, createTemplateSettingsFlow } from '../src/template-settings-flow.js';

const document = { id: 'meeting', name: '회의', description: '회의 요약', prompt: '기본 지침', revision: 'r1', builtIn: true, customized: false };
const deferred = () => { let resolve; const promise = new Promise(done => { resolve = done; }); return { promise, resolve }; };
const flush = () => new Promise(resolve => setImmediate(resolve));
const setup = async overrides => {
  const bridge = { listTemplates: async () => [document], getTemplate: async () => document, ...overrides };
  const flow = createTemplateSettingsFlow({ bridge });
  flow.setActive(true);
  await flush();
  return flow;
};

test('repeated active rendering does not reload and inactive responses are ignored', async () => {
  const pending = deferred();
  let calls = 0;
  const flow = createTemplateSettingsFlow({ bridge: { listTemplates: () => { calls++; return pending.promise; } } });
  flow.setActive(true);
  flow.setActive(true);
  assert.equal(calls, 1);
  flow.setActive(false);
  pending.resolve([document]);
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

test('built-ins cannot be deleted; reset failure preserves local edits', async () => {
  let deleted = false;
  const flow = await setup({ deleteTemplate: async () => { deleted = true; }, resetTemplate: async () => { throw Error('복원 실패'); } });
  await flow.open('meeting');
  flow.edit('prompt', '유지할 내용');
  await flow.remove();
  await flow.reset();
  assert.equal(deleted, false);
  assert.equal(flow.getSnapshot().draft.prompt, '유지할 내용');
});

test('deleted list filters active entries and restores with tombstone revision', async () => {
  const deleted = { id: 'custom', name: '사용자 템플릿', deleted: true, revision: 'deleted-r2' };
  let args;
  const flow = await setup({ listTemplates: async () => [document, deleted], restoreTemplate: async (...values) => { args = values; return deleted; } });
  await flow.showDeleted(true);
  assert.deepEqual(flow.getSnapshot().items, [deleted]);
  await flow.restore(deleted);
  assert.deepEqual(args, ['custom', 'deleted-r2']);
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
