import assert from 'node:assert/strict';
import test from 'node:test';
import { filterLibraryMeetings, recentLibraryMeetings, mountLibrary } from '../src/library-ui.js';

const meetings = () => [
  { id: 'one', title: '주간 회의', notes: '', folderId: 'project', deletedAt: null },
  { id: 'two', title: '기획 검토', notes: '일정', folderId: null, deletedAt: null },
  { id: 'three', title: '보관할 회의', notes: '', folderId: 'project', deletedAt: 100 },
];
const settle = () => new Promise(resolve => setImmediate(resolve));

function harness(overrides = {}, options = {}) {
  const nodes = new Map(), calls = [], errors = [], dragImages = [], frames = new Map();
  const state = { snapshot: { meetings: meetings(), folders: [{ id: 'project', name: '프로젝트' }], session: null }, selectedId: 'one', connected: true, libraryView: 'all', libraryFilter: 'all', libraryOpen: true, librarySelection: new Set(), libraryBusy: false, processing: { status: 'idle' } };
  let locked = false, owner, frameId = 0;
  class Node {
    constructor(id) { this.id = id; this.listeners = {}; this.dataset = {}; this.value = ''; this.open = false; this.hidden = false; this.disabled = false; this.isConnected = true; this.ownerDocument = owner; this.style = {}; this.children = []; }
    addEventListener(name, listener) { this.listeners[name] = listener; }
    setAttribute(name, value) { this[name] = value; }
    removeAttribute(name) { delete this[name]; }
    focus() { this.focused = true; }
    select() { this.selected = true; }
    showModal() { this.open = true; }
    close() { this.open = false; }
    appendChild(node) { this.children.push(node); node.parentNode = this; node.isConnected = this.isConnected; return node; }
    remove() { if (this.parentNode) this.parentNode.children = this.parentNode.children.filter(node => node !== this); this.parentNode = null; this.isConnected = false; }
    getBoundingClientRect() { return { width: Math.min(280, this.children.reduce((sum, node) => sum + (node.textContent?.length || 0) * 8 + 8, 20)), height: 34 }; }
    set innerHTML(value) {
      this.html = value;
      if (this.id === 'library-dialog-body') {
        for (const match of value.matchAll(/<input[^>]*id="([^"]+)"[^>]*>/g)) {
          const node = byId(match[1]);
          node.value = /value="([^"]*)"/.exec(match[0])?.[1]?.replace(/&quot;/g, '"') || '';
          this.autofocus = node;
        }
        this.checkedRadio = /<input[^>]*name="library-destination"[^>]*value="([^"]*)"[^>]* checked/.exec(value)?.[1];
      }
    }
    get innerHTML() { return this.id === 'folder-list' ? ['library-system-views', 'library-user-folders', 'library-trash-view'].map(id => byId(id).innerHTML).join('') : this.html || ''; }
    querySelector(selector) {
      if (selector === '[autofocus]') return this.autofocus || null;
      if (selector === '[data-library-confirm]') return byId('confirm');
      if (selector.includes('library-destination')) return this.checkedRadio == null ? null : { value: this.checkedRadio };
      return null;
    }
    querySelectorAll() { return []; }
  }
  owner = { activeElement: null, listeners: {},
    addEventListener(name, listener) { (this.listeners[name] ||= []).push(listener); },
    createElement(tag) { const node = new Node(tag); node.isConnected = false; return node; },
    defaultView: { innerWidth: 1000, innerHeight: 700, addEventListener() {},
      requestAnimationFrame(action) { frames.set(++frameId, action); return frameId; },
      cancelAnimationFrame(id) { frames.delete(id); } },
  };
  owner.body = new Node('document-body');
  const byId = id => { if (!nodes.has(id)) nodes.set(id, new Node(id)); return nodes.get(id); };
  const snapshot = () => structuredClone(state.snapshot);
  const record = (name, action = () => snapshot()) => async (...args) => { calls.push({ name, args, locked }); return action(...args); };
  const bridge = {
    readSnapshot: record('read'),
    createFolder: record('create'), renameFolder: record('renameFolder'), deleteFolder: record('deleteFolder'), updateMeeting: record('renameMeeting'), moveMeetings: record('move'), trashMeetings: record('trash'), restoreMeetings: record('restore'), purgeMeetings: record('purge'),
    ...overrides,
  };
  let controller;
  const render = () => controller?.render();
  controller = mountLibrary({ bridge, getState: () => state, byId,
    finishEditing: async action => { locked = true; calls.push({ name: 'flush' }); try { return await action(); } finally { locked = false; } },
    acceptSnapshot: value => { state.snapshot = value; }, selectMeeting() {}, render,
    showError: error => errors.push(String(error)), escape: value => String(value ?? '').replace(/[&<>"]/g, character => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[character]), icon: name => `<svg data-icon="${name}"></svg>`,
    ...options,
  });
  controller.render();
  const click = (id, target = { closest: () => null }, event = {}) => byId(id).listeners.click({ target, preventDefault() {}, stopPropagation() {}, ...event });
  const target = (dataset, selectors) => ({ dataset, closest: selector => selectors.some(part => selector.includes(part)) ? targetNode : null });
  let targetNode;
  const delegated = (id, dataset, selectors, event = {}) => { targetNode = target(dataset, selectors); click(id, targetNode, event); };
  const submit = async () => { byId('library-dialog').listeners.submit({ preventDefault() {} }); await settle(); };
  const drag = (id, { image = 'supported', failData = false, clientX = 500, clientY = 350 } = {}) => {
    const payload = new Map();
    const dataTransfer = { types: [], effectAllowed: '', dropEffect: '', setData(type, value) { if (failData) throw new Error('transfer failed'); payload.set(type, value); this.types = [...payload.keys()]; } };
    if (image !== 'missing') dataTransfer.setDragImage = (node, x, y) => {
      dragImages.push({ node, x, y, connectedAtCall: node.isConnected });
      if (image === 'throws') throw new Error('drag bitmap unavailable');
    };
    const row = { dataset: { meetingId: id } };
    let prevented = false;
    byId('meeting-list').listeners.dragstart({ target: { closest: selector => selector === '[data-meeting-id]' ? row : null }, dataTransfer, clientX, clientY, preventDefault() { prevented = true; } });
    return { dataTransfer, payload, prevented, row };
  };
  const destination = filter => {
    const classes = new Set();
    const target = { dataset: { libraryFilter: filter }, isConnected: true, disabled: false, contains: value => value === target,
      classList: { add: name => classes.add(name), remove: name => classes.delete(name) } };
    target.closest = selector => selector === '[data-library-filter]' ? target : null;
    return { target, classes };
  };
  const dragEvent = (type, transfer, destination) => {
    let prevented = false;
    byId('folder-list').listeners[type]({ target: destination.target, dataTransfer: transfer.dataTransfer, preventDefault() { prevented = true; } });
    return prevented;
  };
  const documentEvent = (name, event = {}) => { for (const listener of owner.listeners[name] || []) listener(event); };
  const paint = () => { const pending = [...frames.values()]; frames.clear(); pending.forEach(action => action()); };
  return { state, nodes, byId, calls, errors, controller, bridge, click, delegated, submit, snapshot, drag, destination, dragEvent, owner, dragImages, documentEvent, paint };
}

test('a single meeting uses its title-only rendered drag image instead of the full meeting row', () => {
  const app = harness();
  app.state.snapshot.meetings[1].audio = { frames: 16000, sampleRate: 16000 };
  const transfer = app.drag('two');
  const image = app.dragImages[0];
  assert.equal(image.connectedAtCall, true);
  assert.notEqual(image.node, transfer.row);
  assert.equal(image.node.className, 'meeting-drag-preview');
  assert.equal(image.node['aria-hidden'], 'true');
  assert.deepEqual(image.node.children.map(node => node.textContent), ['기획 검토']);
  assert.equal(image.node.children[0].className, 'meeting-drag-preview-title');
  assert.equal(image.x, 12);
  assert.equal(image.y, 17);
  assert.equal(transfer.payload.get('application/x-nero-meetings'), '["two"]');
  app.paint();
  assert.equal(image.node.isConnected, false);
  assert.equal(app.owner.body.children.length, 0);
  assert.equal(transfer.dataTransfer.effectAllowed, 'move');
});

test('a selected drag shows the actual grabbed title and total count while preserving selected ID provenance', () => {
  const app = harness();
  app.state.librarySelection = new Set(['one', 'two']);
  const transfer = app.drag('two');
  const image = app.dragImages[0].node;
  assert.deepEqual(image.children.map(node => node.textContent), ['기획 검토', '2개']);
  assert.equal(image.children[1].className, 'meeting-drag-preview-count');
  assert.equal(transfer.payload.get('application/x-nero-meetings'), '["one","two"]');
  assert.deepEqual([...app.state.librarySelection], ['one', 'two']);
});

test('a long title remains safe text in the ellipsis label and its drag image stays inside the viewport', () => {
  const app = harness();
  const title = '긴 프로젝트 운영과 다음 분기 실행 계획을 검토하는 회의 '.repeat(10) + '<img src=x onerror=alert(1)>';
  app.state.snapshot.meetings[1].title = title;
  app.drag('two', { clientX: 999, clientY: 699 });
  const { node, x, y } = app.dragImages[0];
  assert.equal(node.children[0].textContent, title);
  assert.equal(node.children[0].innerHTML, '');
  assert.equal(node.children[0].className, 'meeting-drag-preview-title');
  const bounds = node.getBoundingClientRect();
  assert.ok(Number.parseFloat(node.style.left) >= 0);
  assert.ok(Number.parseFloat(node.style.left) + bounds.width <= app.owner.defaultView.innerWidth);
  assert.ok(Number.parseFloat(node.style.top) + bounds.height <= app.owner.defaultView.innerHeight);
  assert.ok(x < bounds.width && y < bounds.height);
});

for (const end of ['document-dragend', 'row-dragend', 'escape', 'next-drag']) {
  test(`a drag preview is removed on ${end} without changing meetings`, () => {
    const app = harness();
    const before = app.snapshot();
    app.drag('two');
    const preview = app.dragImages[0].node;
    if (end === 'document-dragend') app.documentEvent('dragend');
    if (end === 'row-dragend') app.byId('meeting-list').listeners.dragend();
    if (end === 'escape') app.documentEvent('keydown', { key: 'Escape' });
    if (end === 'next-drag') app.drag('one', { image: 'missing' });
    assert.equal(preview.isConnected, false);
    assert.equal(app.owner.body.children.length, 0);
    assert.deepEqual(app.snapshot(), before);
    assert.equal(app.calls.length, 0);
  });
}

test('document drop cleanup preserves source validation until the folder drop handler consumes it', async () => {
  const app = harness();
  const transfer = app.drag('two');
  app.documentEvent('drop');
  assert.equal(app.owner.body.children.length, 0);
  assert.equal(app.dragEvent('drop', transfer, app.destination('project')), true);
  await settle();
  assert.deepEqual(app.calls.find(call => call.name === 'move').args, [['two'], 'project']);
});

test('Escape cancels the local drag and leaves no source IDs available for a later synthetic drop', async () => {
  const app = harness();
  const transfer = app.drag('two');
  app.documentEvent('keydown', { key: 'Escape' });
  assert.equal(app.dragEvent('drop', transfer, app.destination('project')), false);
  await settle();
  assert.equal(app.calls.some(call => call.name === 'move'), false);
});

for (const image of ['missing', 'throws']) {
  test(`a ${image} native drag-image API safely keeps the original move payload without leaking preview nodes`, async () => {
    const app = harness();
    const transfer = app.drag('two', { image });
    assert.equal(transfer.prevented, false);
    assert.equal(transfer.payload.get('application/x-nero-meetings'), '["two"]');
    assert.equal(app.owner.body.children.length, 0);
    assert.equal(app.dragEvent('drop', transfer, app.destination('project')), true);
    await settle();
    assert.deepEqual(app.calls.find(call => call.name === 'move').args, [['two'], 'project']);
    assert.equal(app.errors.length, 0);
  });
}

test('a failed data transfer cancels the drag and clears prior preview and source state', () => {
  const app = harness();
  app.drag('one');
  const previous = app.dragImages[0].node;
  const failed = app.drag('two', { failData: true });
  assert.equal(failed.prevented, true);
  assert.equal(previous.isConnected, false);
  assert.equal(app.owner.body.children.length, 0);
  assert.equal(app.dragEvent('dragover', failed, app.destination('project')), false);
  assert.equal(app.calls.length, 0);
});

test('dragging a meeting onto a folder flushes edits and moves its ID without touching recording or processing', async () => {
  const app = harness();
  app.state.snapshot.meetings[1].audio = { sessionId: 'audio', relativePath: 'original.wav' };
  app.state.snapshot.session = { id: 'capture', meetingId: 'two', state: 'recording' };
  app.state.processing = { jobs: [{ id: 'queued', kind: 'summary', meetingId: 'two', status: 'queued' }] };
  const transfer = app.drag('two'), folder = app.destination('project');
  assert.equal(transfer.prevented, false);
  assert.equal(transfer.payload.get('application/x-nero-meetings'), '["two"]');
  assert.equal(app.dragEvent('dragover', transfer, folder), true);
  assert.equal(folder.classes.has('is-drop-target'), true);
  assert.equal(app.dragEvent('drop', transfer, folder), true);
  await settle();
  const move = app.calls.find(call => call.name === 'move');
  assert.deepEqual(move.args, [['two'], 'project']);
  assert.equal(move.locked, false);
  assert.equal(app.calls[0].name, 'flush');
  assert.equal(folder.classes.has('is-drop-target'), false);
  assert.equal(app.state.snapshot.session.id, 'capture');
  assert.equal(app.state.processing.jobs[0].status, 'queued');
  assert.equal(app.state.snapshot.meetings[1].audio.relativePath, 'original.wav');
  assert.equal(app.calls.some(call => ['trash', 'purge'].includes(call.name)), false);
});

test('selected meetings drag together while all/same-folder/external drops remain invalid', async () => {
  const app = harness();
  app.state.librarySelection = new Set(['one', 'two']);
  const transfer = app.drag('one');
  assert.equal(app.dragEvent('dragover', transfer, app.destination('all')), false);
  const folder = app.destination('project');
  app.dragEvent('drop', transfer, folder); await settle();
  assert.deepEqual(app.calls.find(call => call.name === 'move').args, [['one', 'two'], 'project']);
  const noMove = app.drag('one');
  assert.equal(app.dragEvent('dragover', noMove, folder), false);
  assert.equal(app.dragEvent('dragover', { dataTransfer: { types: ['Files'] } }, app.destination('unfiled')), false);
});

test('failed edit persistence never executes a drag move and retains the selection', async () => {
  const app = harness({}, { finishEditing: async () => { throw new Error('메모 저장 실패'); } });
  app.state.librarySelection.add('two');
  app.dragEvent('drop', app.drag('two'), app.destination('project')); await settle();
  assert.equal(app.calls.some(call => call.name === 'move'), false);
  assert.equal(app.state.librarySelection.has('two'), true);
  assert.match(app.errors[0], /메모 저장 실패/);
});

for (const change of ['folder-removed', 'folder-renamed', 'source-moved', 'source-trashed']) {
  test(`a drag move rejects ${change} while edits are being saved`, async () => {
    let release;
    const saved = new Promise(resolve => { release = resolve; });
    const app = harness({}, { finishEditing: async action => { await saved; return action(); } });
    app.dragEvent('drop', app.drag('two'), app.destination('project'));
    if (change === 'folder-removed') app.state.snapshot.folders = [];
    if (change === 'folder-renamed') app.state.snapshot.folders[0].name = '다른 이름';
    if (change === 'source-moved') app.state.snapshot.meetings[1].folderId = 'project';
    if (change === 'source-trashed') app.state.snapshot.meetings[1].deletedAt = 100;
    release(); await settle();
    assert.equal(app.calls.some(call => call.name === 'move'), false);
    assert.match(app.errors[0], /변경되었습니다/);
  });
}

test('a stale or disconnected drag cannot highlight or move, and trash still asks for confirmation', () => {
  const app = harness();
  const transfer = app.drag('two'), folder = app.destination('project');
  folder.target.isConnected = false;
  assert.equal(app.dragEvent('dragover', transfer, folder), false);
  app.state.connected = false;
  assert.equal(app.drag('two').prevented, true);
  app.state.connected = true;
  app.dragEvent('drop', app.drag('two'), app.destination('trash'));
  assert.equal(app.calls.some(call => call.name === 'trash'), false);
  assert.equal(app.byId('library-dialog').open, true);
});

test('library scopes exclude trash and do not confuse unfiled with a folder', () => {
  const input = meetings();
  assert.deepEqual(filterLibraryMeetings(input).map(meeting => meeting.id), ['one', 'two']);
  assert.deepEqual(filterLibraryMeetings(input, 'project').map(meeting => meeting.id), ['one']);
  assert.deepEqual(filterLibraryMeetings(input, 'unfiled').map(meeting => meeting.id), ['two']);
  assert.deepEqual(filterLibraryMeetings(input, 'trash').map(meeting => meeting.id), ['three']);
  assert.deepEqual(filterLibraryMeetings(input, 'all', '일정').map(meeting => meeting.id), ['two']);
});

test('folder navigation flushes edits and clears the previous selection', async () => {
  const app = harness();
  app.state.librarySelection.add('one');
  await app.controller.show('unfiled');
  assert.equal(app.calls[0].name, 'flush');
  assert.equal(app.state.libraryFilter, 'unfiled');
  assert.equal(app.state.librarySelection.size, 0);
  assert.equal(app.byId('library-batch').hidden, true);
  assert.equal(app.byId('library-title').textContent, '미분류');
});

test('detail selection moves from the preserved navigation scope to the open recent meeting', () => {
  const app = harness();
  app.state.snapshot.meetings[0].createdAt = new Date(2026, 0, 1, 9, 0).getTime();
  app.state.libraryFilter = 'all';
  app.controller.render();
  const navigation = app.byId('folder-list').innerHTML;
  app.state.libraryOpen = false;
  app.controller.render();
  assert.match(navigation, /data-library-filter="all"[^>]*aria-current="page"/);
  assert.doesNotMatch(app.byId('folder-list').innerHTML, /aria-current/);
  assert.match(app.byId('recent-meetings').innerHTML, /data-recent-meeting="one"[^>]*aria-current="page"/);
  assert.match(app.byId('recent-meetings').innerHTML, /aria-label="주간 회의, 프로젝트, 1\. 1\. 09:00"/);
  assert.equal(app.state.libraryFilter, 'all');
  assert.equal(app.byId('meeting-folder-context').textContent, '프로젝트');
  assert.equal(app.byId('meeting-folder-context').title, '프로젝트');
  app.state.snapshot.meetings[0].folderId = null;
  app.controller.render();
  assert.equal(app.byId('meeting-folder-context').textContent, '미분류');
  assert.equal(app.state.libraryFilter, 'all');
  app.state.libraryFilter = 'project';
  app.controller.render();
  assert.doesNotMatch(app.byId('folder-list').innerHTML, /aria-current/);
  assert.equal(app.byId('meeting-folder-context').textContent, '미분류');
  app.state.libraryOpen = true;
  app.controller.render();
  assert.match(app.byId('folder-list').innerHTML, /data-library-filter="project"[^>]*aria-current="page"/);
  assert.doesNotMatch(app.byId('recent-meetings').innerHTML, /aria-current/);
});

test('renamed folders update detail context and long names keep an escaped full title', () => {
  const app = harness();
  const name = '이름이 긴 프로젝트와 반복 회의를 함께 정리하는 폴더 "검토"';
  app.state.snapshot.folders[0].name = name;
  app.controller.render();
  const html = app.byId('folder-list').innerHTML;
  assert.match(html, /class="library-folder-name" title="이름이 긴 프로젝트/);
  assert.match(html, /폴더 &quot;검토&quot;/);
  assert.equal(app.byId('meeting-folder-context').textContent, name);
  assert.equal(app.byId('meeting-folder-context').title, name);
  app.state.snapshot.folders[0].name = '변경한 폴더';
  app.controller.render();
  assert.equal(app.byId('meeting-folder-context').textContent, '변경한 폴더');
  app.state.selectedId = null;
  app.controller.render();
  assert.equal(app.byId('meeting-folder-context').hidden, true);
});

test('select all respects full-text search IDs and the current folder scope', () => {
  const app = harness();
  app.byId('search').value = '본문에서만 찾을 표현';
  app.state.librarySearchIds = new Set(['two', 'three']);
  app.click('library-select-all');
  assert.deepEqual(app.controller.selectedIds(), ['two']);
  app.state.libraryFilter = 'project';
  app.controller.render();
  assert.deepEqual(app.controller.selectedIds(), []);
});

test('folder creation waits for edit persistence and releases edit locks before native work', async () => {
  const app = harness();
  app.click('new-folder');
  assert.equal(app.byId('library-dialog').open, true);
  app.byId('library-folder-name').value = '  제품  ';
  await app.submit();
  const call = app.calls.find(call => call.name === 'create');
  assert.deepEqual(call.args, ['제품']);
  assert.equal(call.locked, false);
  assert.equal(app.byId('library-dialog').open, false);
  assert.equal(app.state.libraryBusy, false);
});

test('meeting rename uses the saved current notes and rejects an empty title', async () => {
  const app = harness();
  app.delegated('meeting-list', { libraryAction: 'rename', libraryTarget: 'one' }, ['[data-library-action]']);
  app.byId('library-meeting-name').value = '  ';
  await app.submit();
  assert.equal(app.calls.some(call => call.name === 'renameMeeting'), false);
  assert.match(app.byId('library-dialog-error').textContent, /회의 이름/);
  app.byId('library-meeting-name').value = '새 제목';
  app.state.snapshot.meetings[0].notes = '저장된 최신 메모';
  await app.submit();
  assert.deepEqual(app.calls.find(call => call.name === 'renameMeeting').args, ['one', '새 제목', '저장된 최신 메모']);
});

test('trash confirmation explains cancelling the affected processing job', () => {
  const app = harness();
  app.state.processing = { status: 'running', meetingId: 'one' };
  app.click('meeting-trash');
  assert.doesNotMatch(app.byId('library-dialog-body').innerHTML, /휴지통에서 복원/);
  assert.match(app.byId('library-dialog-body').innerHTML, /처리 취소 후 휴지통/);
  assert.equal(app.calls.some(call => call.name === 'trash'), false);
});

test('active recording cannot open a trash confirmation', () => {
  const app = harness();
  app.state.snapshot.session = { state: 'recording', meetingId: 'one' };
  app.click('meeting-trash');
  assert.equal(app.byId('library-dialog').open, false);
  assert.match(app.errors[0], /녹음을 종료/);
});

test('purge failure refreshes its committed deletion and keeps the recovery error visible', async () => {
  const app = harness();
  app.state.libraryFilter = 'trash';
  app.state.selectedId = 'three';
  app.state.librarySelection.add('three');
  app.controller.render();
  app.bridge.purgeMeetings = async ids => { app.state.snapshot.meetings = app.state.snapshot.meetings.filter(meeting => !ids.includes(meeting.id)); throw new Error('파일 정리를 재시도합니다.'); };
  app.click('library-purge');
  assert.match(app.byId('library-dialog-body').innerHTML, /영구 삭제/);
  await app.submit();
  assert.equal(app.state.snapshot.meetings.some(meeting => meeting.id === 'three'), false);
  assert.equal(app.calls.at(-1).name, 'read');
  assert.equal(app.state.librarySelection.size, 0);
  assert.equal(app.state.libraryBusy, false);
  assert.equal(app.byId('library-dialog').open, false);
  assert.match(app.errors.at(-1), /재시도/);
});

test('slow restore keeps only library operations busy and does not hold edit locks', async () => {
  let resolve;
  const app = harness();
  app.state.libraryFilter = 'trash';
  app.state.librarySelection.add('three');
  app.bridge.restoreMeetings = () => new Promise(done => { resolve = done; });
  app.click('library-restore');
  await settle();
  assert.equal(app.state.libraryBusy, true);
  assert.equal(app.state.busy, undefined);
  assert.equal(app.byId('library-restore').disabled, true);
  resolve(app.snapshot());
  await settle();
  assert.equal(app.state.libraryBusy, false);
  assert.equal(app.state.librarySelection.size, 0);
});

test('failed folder save keeps the dialog input and returns focus for retry', async () => {
  const app = harness({ createFolder: async () => { throw new Error('다시 시도해 주세요.'); } });
  app.click('new-folder');
  app.byId('library-folder-name').value = '보존할 입력';
  await app.submit();
  assert.equal(app.byId('library-dialog').open, true);
  assert.equal(app.byId('library-folder-name').value, '보존할 입력');
  assert.equal(app.byId('library-folder-name').focused, true);
  assert.match(app.byId('library-dialog-error').textContent, /다시 시도/);
});

test('slow trash closes confirmation and preserves focus in another editor', async () => {
  let resolve;
  const app = harness({ trashMeetings: () => new Promise(done => { resolve = done; }) });
  app.click('meeting-trash');
  app.byId('library-dialog').listeners.submit({ preventDefault() {} });
  await settle();
  assert.equal(app.byId('library-dialog').open, false);
  assert.equal(app.state.libraryBusy, true);
  app.state.selectedId = 'two';
  app.state.libraryOpen = false;
  const editor = app.byId('other-meeting-notes');
  app.byId('library-dialog').ownerDocument.activeElement = editor;
  resolve(app.snapshot());
  await settle();
  assert.equal(app.state.libraryOpen, false);
  assert.equal(app.byId('new-folder').focused, undefined);
  assert.equal(app.byId('library-title').focused, undefined);
});

test('view callback only runs for an actual filter change, never during rendering', async () => {
  const changes = [];
  const app = harness({}, { onViewChanged: filter => changes.push(filter) });
  app.controller.render();
  await app.controller.show('all');
  await app.controller.show('trash');
  app.controller.render();
  assert.deepEqual(changes, ['trash']);
  app.byId('search').value = '수정한 회의 내용';
  await app.controller.show('trash');
  assert.deepEqual(changes, ['trash', 'trash']);
});

test('renaming a meeting invalidates content search without changing its scope', async () => {
  const changes = [];
  const app = harness({}, { onViewChanged: filter => changes.push(filter) });
  app.state.libraryFilter = 'project';
  app.byId('search').value = '주간';
  app.delegated('meeting-list', { libraryAction: 'rename', libraryTarget: 'one' }, ['[data-library-action]']);
  app.byId('library-meeting-name').value = '제품 검토';
  await app.submit();
  assert.deepEqual(changes, ['project']);
  assert.equal(app.calls.find(call => call.name === 'renameMeeting').args[1], '제품 검토');
});

function showTrashPreview(app) {
  const sidebar = app.byId('navigation-sidebar');
  sidebar.dataset.sidebarView = 'trash';
  sidebar.inert = false;
  app.byId('sidebar-empty-trash').hidden = false;
}

test('sidebar empty trash confirms the whole trash without navigating the current meeting', async () => {
  const app = harness();
  app.state.libraryFilter = 'project';
  app.state.libraryOpen = false;
  app.state.snapshot.meetings.push({ id: 'four', title: '또 다른 회의', deletedAt: 200 });
  app.byId('search').value = '검색에 없는 제목';
  app.state.librarySearchIds = [];
  showTrashPreview(app);
  app.controller.render();
  app.state.librarySelection.add('one');
  app.click('library-empty-trash');
  assert.equal(app.byId('library-dialog').open, false);
  app.click('sidebar-empty-trash');
  assert.match(app.byId('library-dialog-body').innerHTML, /회의 2개/);
  assert.equal(app.byId('library-dialog').role, 'alertdialog');
  assert.equal(app.calls.length, 0);
  app.bridge.purgeMeetings = async ids => {
    app.calls.push({ name: 'purge', args: [ids] });
    app.state.snapshot.meetings = app.state.snapshot.meetings.filter(meeting => !ids.includes(meeting.id));
    return app.snapshot();
  };
  await app.submit();
  assert.deepEqual(app.calls.find(call => call.name === 'purge').args, [['three', 'four']]);
  assert.deepEqual(app.calls.map(call => call.name), ['flush', 'read', 'purge']);
  assert.equal(app.state.libraryFilter, 'project');
  assert.equal(app.state.libraryOpen, false);
  assert.equal(app.state.selectedId, 'one');
  assert.equal(app.byId('sidebar-empty-trash').disabled, true);
});

for (const [name, change] of [
  ['another preview', app => { app.byId('navigation-sidebar').dataset.sidebarView = 'all'; }],
  ['hidden action', app => { app.byId('sidebar-empty-trash').hidden = true; }],
  ['hidden ancestor', app => { app.byId('sidebar-empty-trash').closest = () => ({}); }],
  ['hidden sidebar', app => { app.byId('navigation-sidebar').hidden = true; }],
  ['collapsed sidebar', app => { app.byId('navigation-sidebar').inert = true; }],
  ['busy library', app => { app.state.libraryBusy = true; }],
  ['disconnected library', app => { app.state.connected = false; }],
]) {
  test(`sidebar empty trash rejects an unavailable entry: ${name}`, () => {
    const app = harness();
    app.state.libraryFilter = 'trash';
    showTrashPreview(app);
    change(app);
    app.click('sidebar-empty-trash');
    assert.equal(app.byId('library-dialog').open, false);
    assert.equal(app.calls.length, 0);
  });
}

test('both empty-trash actions share availability across connection, busy and trash changes', () => {
  const app = harness();
  const check = expected => {
    app.controller.render();
    for (const id of ['library-empty-trash', 'sidebar-empty-trash']) assert.equal(app.byId(id).disabled, expected);
  };
  check(false);
  app.state.connected = false; check(true);
  app.state.connected = true; app.state.libraryBusy = true; check(true);
  app.state.libraryBusy = false; check(false);
  app.state.snapshot.meetings.pop(); check(true);
});

test('sidebar empty-trash cancellation preserves meetings and returns focus to its action', () => {
  const app = harness();
  showTrashPreview(app);
  app.owner.activeElement = app.byId('sidebar-empty-trash');
  app.click('sidebar-empty-trash');
  app.byId('library-dialog').listeners.cancel({ preventDefault() {} });
  assert.equal(app.calls.length, 0);
  assert.equal(app.state.snapshot.meetings.length, 3);
  assert.equal(app.byId('sidebar-empty-trash').focused, true);
});

for (const [name, change, message] of [
  ['restored target', app => { app.state.snapshot.meetings[2].deletedAt = null; }, /다시 확인/],
  ['trashed-again target', app => { app.state.snapshot.meetings[2].deletedAt = 300; }, /다시 확인/],
  ['recording target', app => { app.state.snapshot.session = { state: 'recording', meetingId: 'three' }; }, /녹음을 종료/],
  ['unreadable snapshot', app => { app.bridge.readSnapshot = async () => { throw new Error('목록을 읽지 못했습니다.'); }; }, /목록을 읽지 못했습니다/],
]) {
  test(`sidebar empty trash preserves data after a confirmation failure: ${name}`, async () => {
    const app = harness();
    showTrashPreview(app);
    app.click('sidebar-empty-trash');
    change(app);
    await app.submit();
    assert.equal(app.calls.some(call => call.name === 'purge'), false);
    assert.equal(app.state.snapshot.meetings.length, 3);
    assert.equal(app.byId('library-dialog').open, true);
    assert.match(app.byId('library-dialog-error').textContent, message);
    assert.equal(app.state.libraryBusy, false);
  });
}

test('empty trash is scoped to trash, disables for an empty trash and ignores search and selection', async () => {
  const app = harness();
  assert.equal(app.byId('library-empty-trash').hidden, true);
  app.state.libraryFilter = 'trash';
  app.state.snapshot.meetings.push({ id: 'four', title: '또 다른 회의', deletedAt: 200 });
  app.byId('search').value = '검색에 없는 제목';
  app.state.librarySearchIds = new Set();
  app.controller.render();
  assert.equal(app.controller.visibleMeetings().length, 0);
  assert.equal(app.byId('library-empty-trash').hidden, false);
  assert.equal(app.byId('library-empty-trash').disabled, false);
  app.click('library-empty-trash');
  assert.match(app.byId('library-dialog-body').innerHTML, /회의 2개/);
  assert.match(app.byId('library-dialog-body').innerHTML, /회의 2개/);
  assert.match(app.byId('library-dialog-body').innerHTML, /class="danger"/);
  assert.equal(app.byId('library-dialog').role, 'alertdialog');
  assert.equal(app.calls.some(call => call.name === 'purge'), false);
  app.bridge.purgeMeetings = async ids => {
    app.calls.push({ name: 'purge', args: [ids] });
    app.state.snapshot.meetings = app.state.snapshot.meetings.filter(meeting => !ids.includes(meeting.id));
    return app.snapshot();
  };
  await app.submit();
  assert.deepEqual(app.calls.find(call => call.name === 'purge').args, [['three', 'four']]);
  assert.equal(app.byId('library-empty-trash').disabled, true);
  assert.deepEqual(app.state.snapshot.meetings.map(meeting => meeting.id), ['one', 'two']);
  app.click('library-empty-trash');
  assert.equal(app.byId('library-dialog').open, false);
});

test('empty trash cancel preserves meetings and restores its trigger focus', () => {
  const app = harness();
  app.state.libraryFilter = 'trash';
  app.controller.render();
  app.byId('library-dialog').ownerDocument.activeElement = app.byId('library-empty-trash');
  app.click('library-empty-trash');
  app.byId('library-dialog').listeners.cancel({ preventDefault() {} });
  assert.equal(app.calls.length, 0);
  assert.equal(app.state.snapshot.meetings.length, 3);
  assert.equal(app.byId('library-empty-trash').focused, true);
});

test('empty trash deletes only confirmed IDs even if new meetings enter trash', async () => {
  const app = harness();
  app.state.libraryFilter = 'trash';
  app.click('library-empty-trash');
  app.state.snapshot.meetings.push({ id: 'four', title: '추가한 회의', deletedAt: 200 });
  app.click('library-empty-trash');
  await app.submit();
  assert.deepEqual(app.calls.find(call => call.name === 'purge').args, [['three']]);
});

for (const change of ['restored', 'removed', 'trashed-again']) {
  test(`empty trash refuses a changed confirmed target: ${change}`, async () => {
    const app = harness();
    app.state.libraryFilter = 'trash';
    app.click('library-empty-trash');
    if (change === 'removed') app.state.snapshot.meetings.pop();
    else app.state.snapshot.meetings[2].deletedAt = change === 'restored' ? null : 300;
    await app.submit();
    assert.equal(app.calls.some(call => call.name === 'purge'), false);
    assert.equal(app.byId('library-dialog').open, true);
    assert.match(app.byId('library-dialog-error').textContent, /다시 확인/);
  });
}

test('empty trash prevents duplicate submissions and cancellation while purging', async () => {
  const app = harness();
  let resolve;
  app.state.libraryFilter = 'trash';
  app.state.librarySelection.add('three');
  app.byId('library-dialog').ownerDocument.activeElement = app.byId('library-empty-trash');
  app.bridge.purgeMeetings = ids => {
    app.calls.push({ name: 'purge', args: [ids] });
    return new Promise(done => { resolve = done; });
  };
  app.click('library-empty-trash');
  await app.submit();
  assert.equal(app.state.libraryBusy, true);
  assert.equal(app.byId('library-empty-trash').disabled, true);
  await app.submit();
  app.click('library-empty-trash');
  app.byId('library-dialog').listeners.cancel({ preventDefault() {} });
  assert.equal(app.byId('library-dialog').open, true);
  assert.equal(app.calls.filter(call => call.name === 'purge').length, 1);
  const result = app.snapshot();
  result.meetings = result.meetings.filter(meeting => meeting.id !== 'three');
  resolve(result);
  await settle();
  assert.equal(app.state.libraryBusy, false);
  assert.equal(app.state.librarySelection.size, 0);
  assert.equal(app.byId('library-title').focused, true);
});

test('empty trash preserves failed targets and selection, and refreshes partial deletion', async () => {
  const app = harness();
  app.state.libraryFilter = 'trash';
  app.state.snapshot.meetings.push({ id: 'four', title: '남는 회의', deletedAt: 200 });
  app.state.librarySelection = new Set(['three', 'four']);
  app.bridge.purgeMeetings = async () => {
    app.state.snapshot.meetings = app.state.snapshot.meetings.filter(meeting => meeting.id !== 'three');
    throw new Error('파일을 정리하지 못했습니다.');
  };
  app.click('library-empty-trash');
  await app.submit();
  assert.equal(app.calls.at(-1).name, 'read');
  assert.deepEqual(app.controller.selectedIds(), ['four']);
  assert.equal(app.state.libraryBusy, false);
  assert.equal(app.byId('library-dialog').open, true);
  assert.match(app.byId('library-dialog-error').textContent, /파일을 정리하지 못했습니다/);
  assert.equal(app.byId('library-empty-trash').disabled, false);
});

test('empty trash cannot purge when its latest snapshot cannot be read', async () => {
  const app = harness({ readSnapshot: async () => { throw new Error('목록을 읽지 못했습니다.'); } });
  app.state.libraryFilter = 'trash';
  app.click('library-empty-trash');
  await app.submit();
  assert.equal(app.calls.some(call => call.name === 'purge'), false);
  assert.equal(app.state.snapshot.meetings.length, 3);
  assert.match(app.byId('library-dialog-error').textContent, /목록을 읽지 못했습니다/);
});

test('empty trash respects recording protection before opening and at confirmation', async () => {
  const app = harness();
  app.state.libraryFilter = 'trash';
  app.state.snapshot.session = { state: 'paused', meetingId: 'three' };
  app.click('library-empty-trash');
  assert.equal(app.byId('library-dialog').open, false);
  app.state.snapshot.session = null;
  app.click('library-empty-trash');
  app.state.snapshot.session = { state: 'recording', meetingId: 'three' };
  await app.submit();
  assert.equal(app.calls.some(call => call.name === 'purge'), false);
  assert.match(app.byId('library-dialog-error').textContent, /녹음을 종료/);
});


test('recent meetings exclude trash, use recording chronology and do not reorder the source', () => {
  const source = Array.from({length: 8}, (_, index) => ({id: String(index), title: '회의', createdAt: index * 10, updatedAt: 999 - index, deletedAt: index === 7 ? 1 : null}));
  source[0].recordedAt = 65;
  const before = JSON.stringify(source);
  assert.deepEqual(recentLibraryMeetings(source).map(item => item.id), ['0', '6', '5', '4', '3']);
  assert.equal(JSON.stringify(source), before);
});

test('recent navigation opens current meetings and ignores a deleted target without changing filters', async () => {
  const opened = [];
  const app = harness({}, {selectMeeting(id) { opened.push(id); }});
  const target = {dataset: {recentMeeting: 'two'}};
  app.byId('recent-meetings').listeners.click({target: {closest: selector => selector === '[data-recent-meeting]' ? target : null}});
  await settle();
  assert.deepEqual(opened, ['two']);
  assert.equal(app.state.libraryFilter, 'all');
  target.dataset.recentMeeting = 'three';
  app.byId('recent-meetings').listeners.click({target: {closest: selector => selector === '[data-recent-meeting]' ? target : null}});
  app.state.connected = false;
  target.dataset.recentMeeting = 'one';
  app.byId('recent-meetings').listeners.click({target: {closest: selector => selector === '[data-recent-meeting]' ? target : null}});
  await settle();
  assert.deepEqual(opened, ['two']);
});


function listKey(app, key, options = {}) {
  const event = { key, target: { closest: () => null }, defaultPrevented: false, propagationStopped: false,
    preventDefault() { this.defaultPrevented = true; }, stopPropagation() { this.propagationStopped = true; }, ...options };
  app.documentEvent('keydown', event);
  if (!event.propagationStopped) app.byId('meeting-list').listeners.keydown(event);
  return event;
}
function orderedMeetings(app) {
  app.state.snapshot.meetings = [
    { id: 'early', title: '이전 회의', createdAt: 1, recordedAt: 10 },
    { id: 'late', title: '최근 회의', createdAt: 2, recordedAt: 40 },
    { id: 'middle', title: '중간 회의', createdAt: 3, recordedAt: 20 },
    { id: 'tie', title: '동일 시각 회의', createdAt: 4, recordedAt: 40 },
  ];
  app.controller.render();
}
test('selection uses no mode and bulk actions exist only when there is a selection', () => {
  const app = harness();
  assert.equal(app.nodes.has('library-select-toggle'), false);
  assert.equal(app.byId('library-batch').hidden, true);
  assert.equal(app.controller.handleRowSelection('one', {}), false);
  assert.deepEqual(app.controller.selectedIds(), []);
  assert.equal(app.controller.handleRowSelection('one', { metaKey: true }), true);
  assert.equal(app.controller.handleRowSelection('two', { ctrlKey: true }), true);
  assert.deepEqual(app.controller.selectedIds(), ['one', 'two']);
  assert.equal(app.byId('library-batch').hidden, false);
  assert.equal(app.byId('library-selected-count').textContent, '2개 선택');
  app.click('library-selection-clear');
  assert.equal(app.byId('library-batch').hidden, true);
  assert.equal(app.calls.some(call => ['trash', 'purge', 'move'].includes(call.name)), false);
});
test('shift range follows visible recorded timestamps with creation tie-breaking', () => {
  const app = harness(); orderedMeetings(app);
  assert.deepEqual(app.controller.visibleMeetings().map(meeting => meeting.id), ['tie', 'late', 'middle', 'early']);
  app.controller.toggleSelection('late');
  app.controller.handleRowSelection('early', { shiftKey: true });
  assert.deepEqual(app.controller.selectedIds(), ['late', 'middle', 'early']);
  app.controller.handleRowSelection('middle', { shiftKey: true });
  assert.deepEqual(app.controller.selectedIds(), ['late', 'middle']);
  app.controller.handleRowSelection('tie', { shiftKey: true, ctrlKey: true });
  assert.deepEqual(new Set(app.controller.selectedIds()), new Set(['tie', 'late', 'middle']));
});
test('checkbox shift-click supports ranges and clearing selection resets its anchor', () => {
  const app = harness(); orderedMeetings(app);
  app.delegated('meeting-list', { libraryToggle: 'late' }, ['[data-library-toggle]']);
  app.delegated('meeting-list', { libraryToggle: 'early' }, ['[data-library-toggle]'], { shiftKey: true });
  assert.deepEqual(app.controller.selectedIds(), ['late', 'middle', 'early']);
  app.controller.resetSelection();
  app.controller.handleRowSelection('early', { shiftKey: true });
  assert.deepEqual(app.controller.selectedIds(), ['early']);
});
test('search pruning clears an unavailable anchor and selected trash hides empty-all', () => {
  const app = harness(); orderedMeetings(app); app.controller.toggleSelection('late');
  app.byId('search').value = '기록에서 찾기'; app.state.librarySearchIds = ['middle', 'early'];
  app.controller.render();
  assert.deepEqual(app.controller.selectedIds(), []);
  assert.equal(app.byId('library-batch').hidden, true);
  app.controller.handleRowSelection('early', { shiftKey: true });
  assert.deepEqual(app.controller.selectedIds(), ['early']);
  app.state.snapshot.meetings.find(meeting => meeting.id === 'early').deletedAt = 100;
  app.controller.render(); assert.deepEqual(app.controller.selectedIds(), []);
  app.byId('search').value = ''; app.state.libraryFilter = 'trash'; app.controller.render();
  assert.equal(app.byId('library-empty-trash').hidden, false);
  app.controller.toggleSelection('early');
  assert.equal(app.byId('library-empty-trash').hidden, false);
  app.click('library-selection-clear');
  assert.equal(app.byId('library-empty-trash').hidden, false);
});
test('list shortcuts preserve editor, composition and modal ownership', () => {
  const app = harness();
  assert.equal(listKey(app, 'a', { metaKey: true }).defaultPrevented, true);
  assert.deepEqual(app.controller.selectedIds(), ['one', 'two']);
  for (const options of [{ isComposing: true }, { keyCode: 229 }, { target: { closest: () => ({}) } }]) {
    assert.equal(listKey(app, 'Escape', options).defaultPrevented, false);
    assert.equal(app.state.librarySelection.size, 2);
  }
  app.byId('library-dialog').open = true;
  assert.equal(listKey(app, 'Escape').defaultPrevented, false);
  assert.equal(app.state.librarySelection.size, 2);
  app.byId('library-dialog').open = false;
  assert.equal(listKey(app, 'Escape').defaultPrevented, true);
  assert.equal(app.byId('library-batch').hidden, true);
  assert.equal(listKey(app, 'Escape').defaultPrevented, false);
  app.state.libraryOpen = false;
  assert.equal(listKey(app, 'a', { ctrlKey: true }).defaultPrevented, false);
  assert.deepEqual(app.controller.selectedIds(), []);
});
test('Escape consumes the open menu first and a second Escape clears the selection', () => {
  const app = harness(); app.controller.toggleSelection('one');
  const trigger = { focus() {}, getBoundingClientRect: () => ({ right: 200, top: 30, bottom: 50 }) };
  const content = { style: {}, getBoundingClientRect: () => ({ width: 180, height: 80 }) };
  const menu = { open: true, isConnected: true, matches: () => true, classList: { add() {}, remove() {} },
    querySelector: selector => selector === ':scope > div' ? content : trigger };
  app.documentEvent('toggle', { target: menu });
  assert.equal(listKey(app, 'Escape').defaultPrevented, true);
  assert.equal(menu.open, false);
  assert.deepEqual(app.controller.selectedIds(), ['one']);
  listKey(app, 'Escape');
  assert.deepEqual(app.controller.selectedIds(), []);
});
test('disconnected or busy libraries ignore selection changes and scoped shortcuts', () => {
  const app = harness(); app.state.connected = false;
  app.controller.toggleSelection('one'); app.click('library-select-all');
  assert.deepEqual(app.controller.selectedIds(), []);
  assert.equal(listKey(app, 'a', { ctrlKey: true }).defaultPrevented, false);
  app.state.connected = true; app.controller.toggleSelection('one'); app.state.libraryBusy = true;
  app.controller.handleRowSelection('two', { ctrlKey: true }); app.click('library-selection-clear');
  assert.deepEqual(app.controller.selectedIds(), ['one']);
  assert.equal(listKey(app, 'Escape').defaultPrevented, false);
});


test('bulk toolbar focus retains list-wide select-all and Escape behavior', () => {
  const app = harness(); app.controller.toggleSelection('one');
  const key = (value, options = {}) => {
    const event = { key: value, target: { closest: () => null }, defaultPrevented: false,
      preventDefault() { this.defaultPrevented = true; }, stopPropagation() {}, ...options };
    app.byId('library-batch').listeners.keydown(event);
    return event;
  };
  assert.equal(key('a', { ctrlKey: true }).defaultPrevented, true);
  assert.deepEqual(app.controller.selectedIds(), ['one', 'two']);
  assert.equal(key('Escape', { isComposing: true }).defaultPrevented, false);
  assert.equal(key('Escape', { target: { closest: () => ({}) } }).defaultPrevented, false);
  assert.equal(app.byId('library-batch').hidden, false);
  assert.equal(key('Escape').defaultPrevented, true);
  assert.deepEqual(app.controller.selectedIds(), []);
  assert.equal(app.byId('library-batch').hidden, true);
});


test('recent duration updates after finalization without navigation or title changes', () => {
  const h = harness({}, { duration: value => `${value}ms` });
  const before = h.byId('recent-meetings').innerHTML;
  h.state.snapshot.meetings[0].audio = { sampleRate: 16000, frames: 32000 };
  h.controller.render();
  assert.notEqual(h.byId('recent-meetings').innerHTML, before);
  assert.match(h.byId('recent-meetings').innerHTML, /녹음 길이 2초/);
  h.state.snapshot.meetings[0].audio.frames = 48000;
  h.controller.render();
  assert.match(h.byId('recent-meetings').innerHTML, /녹음 길이 3초/);
  assert.doesNotMatch(h.byId('recent-meetings').innerHTML, /녹음 길이 2초/);
});

 test('built-in views, user folders and trash occupy separate navigation groups', () => {
  const app = harness();
  const views = app.byId('library-system-views').innerHTML;
  const folders = app.byId('library-user-folders').innerHTML;
  const trash = app.byId('library-trash-view').innerHTML;
  assert.match(views, /data-library-filter="all"/);
  assert.match(views, /data-library-filter="unfiled"/);
  assert.doesNotMatch(views, /data-library-filter="project"|data-library-filter="trash"/);
  assert.match(folders, /data-library-filter="project"/);
  assert.doesNotMatch(folders, /data-library-filter="all"|data-library-filter="trash"/);
  assert.match(trash, /data-library-filter="trash"/);
  app.state.snapshot.folders = [];
  app.controller.render();
  assert.equal(app.byId('library-user-folders').innerHTML, '');
  assert.match(app.byId('library-system-views').innerHTML, /data-library-filter="all"/);
});


test('folder deletion defaults to unfiled and freezes the confirmed membership', async () => {
  const app = harness();
  const folder = app.state.snapshot.folders[0];
  app.delegated('folder-list', { folderDelete: folder.id }, ['data-folder-delete']);
  assert.match(app.byId('library-dialog-body').innerHTML, /회의 1개도 휴지통/);
  await app.submit();
  const call = app.calls.find(call => call.name === 'deleteFolder');
  assert.equal(call.args[1].trashMeetings, false);
  assert.deepEqual(call.args[1].folder, folder);
  assert.deepEqual(call.args[1].meetings, app.state.snapshot.meetings.filter(meeting => meeting.folderId === folder.id).map(({ id, deletedAt }) => ({ id, deletedAt: deletedAt ?? null })));
});

test('folder deletion includes trash only after explicit selection and protects recording', async () => {
  const app = harness();
  const folder = app.state.snapshot.folders[0];
  app.delegated('folder-list', { folderDelete: folder.id }, ['data-folder-delete']);
  app.byId('library-folder-trash').checked = true;
  await app.submit();
  assert.equal(app.calls.find(call => call.name === 'deleteFolder').args[1].trashMeetings, true);
  const recording = harness();
  recording.delegated('folder-list', { folderDelete: folder.id }, ['data-folder-delete']);
  recording.byId('library-folder-trash').checked = true;
  recording.state.snapshot.session = { state: 'recording', meetingId: 'one' };
  await recording.submit();
  assert.equal(recording.calls.some(call => call.name === 'deleteFolder'), false);
});

test('trash header replaces create and record with the empty-trash action', () => {
  const app = harness();
  app.state.libraryFilter = 'trash'; app.controller.render();
  assert.equal(app.byId('library-create').hidden, true);
  assert.equal(app.byId('library-record').hidden, true);
  assert.equal(app.byId('library-empty-trash').hidden, false);
  app.state.libraryFilter = 'all'; app.controller.render();
  assert.equal(app.byId('library-create').hidden, false);
  assert.equal(app.byId('library-record').hidden, false);
});
