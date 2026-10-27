import assert from 'node:assert/strict';
import test from 'node:test';
import { createRequire } from 'node:module';
import { createMeetingSearch, literalOffsets, mountDocumentSearch } from '../src/meeting-search.js';

const UI = createRequire(import.meta.url)('../../../design-system/components.js');
const tick = () => new Promise(setImmediate);
function deferred() { let resolve; const promise = new Promise(yes => { resolve = yes; }); return { promise, resolve }; }

function documentPage({ transcript = [{ row: '0', text: 'Alpha 하나' }, { row: '3', text: 'alpha 둘 alpha 셋' }], summary = ['Alpha 요약'], notes = 'alpha 메모 alpha', select, prepare, ready: initialReady = true } = {}) {
  const nodes = new Map(), visits = [], scrolls = [];
  let ready = initialReady, focused = null, renders = 0, controller;
  let transcriptMarks = [], summaryMarks = [], notesMarks = [];
  const node = id => {
    if (!nodes.has(id)) nodes.set(id, {
      id, value: '', textContent: '', hidden: false, disabled: false, scrollTop: 0, attributes: {}, events: {},
      addEventListener(name, callback) { this.events[name] = callback; },
      setAttribute(name, value) { this.attributes[name] = value; },
      focus() { if (id === 'document-query' && node('document-search').hidden) return; focused = id; },
      setSelectionRange(start, end) { this.selection = [start, end]; },
      querySelectorAll(selector) {
        assert.equal(selector, 'mark.search-match');
        return id === 'transcript-rows' ? transcriptMarks : id === 'summary-content' ? summaryMarks : id === 'notes-search-preview' ? notesMarks : [];
      },
    });
    return nodes.get(id);
  };
  node('document-search').hidden = true;
  node('notes-search-preview').hidden = true;
  node('meeting-notes').value = notes;
  function marks(text, view, row) {
    const html = view === 'summary' ? UI.summaryDocument(text, { searchQuery: controller.query() }) : UI.highlightText(text, controller.query());
    return [...html.matchAll(/<mark class="search-match"([^>]*)>/g)].map((match, ordinal) => {
      const classes = new Set();
      const searchGroup = match[1].match(/data-search-group="([^"]+)"/)?.[1];
      return {
        classes, dataset: searchGroup ? { searchGroup } : {},
        classList: { toggle(name, active) { if (active) classes.add(name); else classes.delete(name); } },
        closest(selector) { assert.equal(selector, '[data-transcript-row]'); return row == null ? null : { dataset: { transcriptRow: row } }; },
        scrollIntoView(options) { scrolls.push({ view, row, ordinal, options }); },
      };
    });
  }
  function renderContent() {
    renders++;
    transcriptMarks = transcript.flatMap(entry => marks(entry.text, 'transcript', entry.row));
    summaryMarks = summary.flatMap(text => marks(text, 'summary'));
    notesMarks = marks(node('meeting-notes').value, 'notes');
    node('meeting-notes').hidden = Boolean(controller.query());
    node('notes-search-preview').hidden = !controller.query();
  }
  controller = mountDocumentSearch({ byId: node, ready: () => ready, renderContent, prepare,
    async selectView(view) {
      visits.push(view);
      const accepted = select ? await select(view) : true;
      if (accepted) renderContent();
      return accepted;
    },
  });
  const open = () => node('document-search-toggle').events.click();
  const input = async value => { node('document-query').value = value; await node('document-query').events.input(); await tick(); };
  const type = async value => {
    const target = node(focused), [start, end] = target.selection || [target.value.length, target.value.length];
    target.value = target.value.slice(0, start) + value + target.value.slice(end);
    if (target.selection) target.selection = [start + value.length, start + value.length];
    await target.events.input?.(); await tick();
  };
  const key = async (value, rest = {}) => {
    let prevented = false;
    node('document-query').events.keydown({ key: value, preventDefault() { prevented = true; }, ...rest });
    await tick();
    return prevented;
  };
  return { controller, node, visits, scrolls, open, input, type, key,
    renderCount: () => renders, focused: () => focused,
    marks: () => [...transcriptMarks, ...summaryMarks, ...notesMarks],
    setReady(value) { ready = value; controller.refresh(); },
  };
}

test('search never accepts an older request after changing query or view', async () => {
  const tasks=[], requests=[], results=[];
  const search=createMeetingSearch({search:(query,deleted)=>new Promise(resolve=>requests.push({query,deleted,resolve})), apply:result=>results.push(result), schedule:task=>{tasks.push(task);return tasks.length;}, clear(){} });
  search.update('회의',false); const first=tasks.pop()();
  search.update('회의',true); const second=tasks.pop()();
  assert.equal(requests[1].deleted,true);
  const hit = { meetingId: 'trash', kind: 'transcript', index: 4, text: '회의 내용' };
  requests[1].resolve({ids:['trash'],hits:[hit],unavailable:0}); await second;
  requests[0].resolve({ids:['active'],hits:[{meetingId:'active',kind:'notes'}],unavailable:0}); await first;
  assert.deepEqual(results.at(-1).ids,['trash']);
  assert.deepEqual(results.at(-1).hits,[hit]);
  search.update(''); assert.equal(results.at(-1).ids,null);
  assert.deepEqual(results.at(-1).hits,[]);
});

test('meeting search clears obsolete hits while pending or failed and tolerates results without hit metadata', async () => {
  const tasks = [], results = [];
  let response = { ids: ['a'], hits: [{ meetingId: 'a', kind: 'speaker', index: 2 }], unavailable: 0 };
  const search = createMeetingSearch({ search: async () => { if (response instanceof Error) throw response; return response; }, apply: result => results.push(result), schedule: task => { tasks.push(task); return tasks.length; }, clear() {} });
  search.update('화자'); await tasks.shift()();
  assert.equal(results.at(-1).hits[0].kind, 'speaker');
  response = Error('검색 실패');
  search.update('다음 검색');
  assert.equal(results.at(-1).pending, true);
  assert.deepEqual(results.at(-1).hits, []);
  await tasks.shift()();
  assert.deepEqual(results.at(-1), { query: '다음 검색', ids: null, hits: [], unavailable: 1, pending: false });
  response = { ids: ['b'], unavailable: 0 };
  search.update('기존 형식'); await tasks.shift()();
  assert.deepEqual(results.at(-1).hits, []);
});

test('reset and clearing the query both invalidate an in-flight meeting response', async () => {
  for (const invalidate of [search => search.reset(), search => search.update(' ')]) {
    const pending = deferred(), tasks = [], results = [];
    const search = createMeetingSearch({ search: () => pending.promise, apply: value => results.push(value), schedule: task => { tasks.push(task); return 1; }, clear() {} });
    search.update('회의'); const request = tasks.shift()();
    invalidate(search);
    const count = results.length;
    pending.resolve({ ids: ['late'], hits: [{ meetingId: 'late', kind: 'summary' }], unavailable: 0 });
    await request;
    assert.equal(results.length, count);
    assert.doesNotMatch(JSON.stringify(results.at(-1)), /late/);
  }
});

test('literal offsets ignore case, preserve UTF-16 positions and never treat input as a pattern', () => {
  assert.deepEqual(literalOffsets('회의 Alpha alpha', ' ALPHA '), [3, 9]);
  assert.deepEqual(literalOffsets('a.b aXb a+b', '.'), [1]);
  assert.deepEqual(literalOffsets('[a] a [a]', '[a]'), [0, 6]);
  assert.deepEqual(literalOffsets('aaaaa', 'aa'), [0, 2]);
  assert.deepEqual(literalOffsets('가\uD801\uDC00 alpha', 'alpha'), [4]);
  assert.deepEqual(literalOffsets('내용', ' '), []);
  assert.deepEqual(literalOffsets('내용', '없음'), []);
});

test('a library hit opens the requested transcript row instead of the first occurrence', async () => {
  const page = documentPage();
  assert.equal(await page.controller.find('alpha', { view: 'transcript', index: 3 }), true);
  assert.deepEqual(page.visits, ['transcript']);
  assert.equal(page.node('document-search').hidden, false);
  assert.equal(page.node('document-query').value, 'alpha');
  assert.equal(page.focused(), 'document-query');
  assert.equal(page.node('document-search-count').textContent, '2 / 6');
  assert.equal(page.node('document-search-count').attributes['aria-label'], '6개 중 2번째');
  assert.deepEqual(page.scrolls.at(-1), { view: 'transcript', row: '3', ordinal: 0, options: { block: 'center', behavior: 'auto' } });
  assert.equal(page.marks().filter(mark => mark.classes.has('is-current')).length, 1);
});

test('a pending view change keeps the target row when an earlier edited row gains matches', async () => {
  const transcript = [{ row: '0', text: '수정 중인 앞 발언' }, { row: '3', text: 'alpha 대상 alpha' }];
  const pending = deferred();
  const page = documentPage({ transcript, summary: [], notes: '', select: () => pending.promise });
  const finding = page.controller.find('alpha', { view: 'transcript', index: 3 });
  assert.equal(page.scrolls.length, 0);
  transcript[0].text = 'alpha 저장된 앞 발언 alpha';
  pending.resolve(true);
  assert.equal(await finding, true);
  assert.deepEqual(page.scrolls.at(-1), { view: 'transcript', row: '3', ordinal: 0, options: { block: 'center', behavior: 'auto' } });
  assert.equal(page.node('document-search-count').textContent, '3 / 4');
  assert.equal(page.node('document-search-count').attributes['aria-label'], '4개 중 3번째');
});

test('typing waits for edit preparation and moves to the first match before Enter advances', async () => {
  const pending = deferred();
  let preparations = 0;
  const page = documentPage({ transcript: [{ row: '0', text: 'alpha 첫 발언' }, { row: '3', text: 'alpha 다음 발언' }], summary: [], notes: '', prepare: () => { preparations++; return preparations === 1 ? true : pending.promise; } });
  await page.open();
  const searching = page.input('alpha');
  assert.equal(preparations, 2);
  assert.equal(page.renderCount(), 0);
  assert.equal(page.scrolls.length, 0);
  pending.resolve(true);
  await searching;
  assert.equal(page.scrolls.length, 1);
  assert.equal(page.scrolls[0].row, '0');
  assert.equal(page.node('document-search-count').textContent, '1 / 2');
  await page.key('Enter');
  assert.equal(page.scrolls.at(-1).row, '3');
  assert.equal(page.node('document-search-count').textContent, '2 / 2');
});

test('next and previous traverse transcript, summary and note matches with wraparound', async () => {
  const page = documentPage({ transcript: [{ row: '0', text: 'alpha' }], summary: ['alpha'], notes: '앞 alpha 뒤 alpha' });
  await page.controller.find('alpha', { view: 'summary' });
  assert.equal(page.node('document-search-count').textContent, '2 / 4');
  assert.equal(await page.key('Enter'), true);
  assert.equal(page.visits.at(-1), 'notes');
  assert.equal(page.focused(), 'document-query');
  assert.deepEqual(page.scrolls.at(-1), { view: 'notes', row: undefined, ordinal: 0, options: { block: 'center', behavior: 'auto' } });
  assert.equal(page.node('document-search-count').textContent, '3 / 4');
  await page.key('Enter');
  assert.equal(page.scrolls.at(-1).view, 'notes');
  assert.equal(page.scrolls.at(-1).ordinal, 1);
  await page.key('Enter');
  assert.equal(page.visits.at(-1), 'transcript');
  assert.equal(page.node('document-search-count').textContent, '1 / 4');
  await page.key('Enter', { shiftKey: true });
  assert.equal(page.visits.at(-1), 'notes');
  assert.equal(page.node('document-search-count').attributes['aria-label'], '4개 중 4번째');
  assert.equal(page.marks().filter(mark => mark.classes.has('is-current')).length, 1);
  assert.equal(page.focused(), 'document-query');
});

test('summary matches split by strong markup count once and highlight every node in the current group', async () => {
  const page = documentPage({ transcript: [], summary: ['**중요** 안건입니다.\n\n다음 중요 안건입니다.'], notes: '' });
  await page.controller.find('중요 안건', { view: 'summary' });
  const groups = () => page.marks().map(mark => mark.dataset.searchGroup);
  const currentGroups = () => page.marks().filter(mark => mark.classes.has('is-current')).map(mark => mark.dataset.searchGroup);
  assert.deepEqual(groups(), ['summary-1', 'summary-1', 'summary-2']);
  assert.equal(page.node('document-search-count').textContent, '1 / 2');
  assert.deepEqual(currentGroups(), ['summary-1', 'summary-1']);
  assert.equal(page.scrolls.at(-1).ordinal, 0);
  await page.key('Enter');
  assert.equal(page.node('document-search-count').textContent, '2 / 2');
  assert.deepEqual(currentGroups(), ['summary-2']);
  assert.equal(page.scrolls.at(-1).ordinal, 2);
  await page.key('Enter', { shiftKey: true });
  assert.equal(page.node('document-search-count').attributes['aria-label'], '2개 중 1번째');
  assert.deepEqual(currentGroups(), ['summary-1', 'summary-1']);
  assert.equal(page.focused(), 'document-query');
});

test('note matches keep search input focused so subsequent typing and navigation cannot overwrite notes', async () => {
  const notes = '앞 alpha 뒤 alpha';
  const page = documentPage({ transcript: [], summary: [], notes });
  await page.open();
  await page.input('alp');
  assert.equal(page.focused(), 'document-query');
  assert.equal(page.node('meeting-notes').hidden, true);
  assert.equal(page.node('notes-search-preview').hidden, false);
  assert.equal(page.scrolls.at(-1).view, 'notes');
  assert.equal(page.scrolls.at(-1).ordinal, 0);
  await page.type('ha');
  assert.equal(page.controller.query(), 'alpha');
  assert.equal(page.node('meeting-notes').value, notes);
  assert.equal(page.scrolls.at(-1).ordinal, 0);
  await page.key('Enter');
  assert.equal(page.focused(), 'document-query');
  assert.equal(page.scrolls.at(-1).ordinal, 1);
  page.node('document-search-next').events.click(); await tick();
  assert.equal(page.focused(), 'document-query');
  assert.equal(page.node('document-search-count').textContent, '1 / 2');
  page.node('document-search-prev').events.click(); await tick();
  assert.equal(page.focused(), 'document-query');
  assert.equal(page.node('document-search-count').textContent, '2 / 2');
  assert.equal(page.node('document-query').value, 'alpha');
  assert.equal(page.node('meeting-notes').value, notes);
  page.node('notes-search-preview').scrollTop = 420;
  await page.key('Escape');
  assert.equal(page.node('meeting-notes').hidden, false);
  assert.equal(page.node('notes-search-preview').hidden, true);
  assert.equal(page.node('meeting-notes').value, notes);
  assert.equal(page.node('meeting-notes').scrollTop, 420);
});

test('finishing a Korean search composition selects the note match without transferring input focus', async () => {
  const notes = '회의 안건 회의 후속';
  const page = documentPage({ transcript: [], summary: [], notes });
  await page.open();
  const field = page.node('document-query');
  field.events.compositionstart();
  await page.input('회');
  await page.input('회의');
  assert.equal(page.focused(), 'document-query');
  assert.equal(page.controller.query(), '');
  field.events.compositionend(); await tick();
  assert.equal(page.controller.query(), '회의');
  assert.equal(page.focused(), 'document-query');
  assert.equal(page.scrolls.at(-1).view, 'notes');
  assert.equal(page.scrolls.at(-1).ordinal, 0);
  await page.key('Enter');
  assert.equal(page.focused(), 'document-query');
  assert.equal(page.scrolls.at(-1).ordinal, 1);
  assert.equal(page.node('meeting-notes').value, notes);
});

test('Korean composition does not search, navigate or close before composition ends', async () => {
  const page = documentPage({ transcript: [{ row: '0', text: '회의 내용' }], summary: [], notes: '' });
  await page.open();
  const field = page.node('document-query');
  field.events.compositionstart();
  await page.input('회의');
  assert.equal(page.controller.query(), '');
  assert.equal(page.renderCount(), 0);
  assert.equal(await page.key('Enter', { isComposing: true }), false);
  assert.equal(await page.key('Escape'), false);
  assert.equal(page.node('document-search').hidden, false);
  assert.deepEqual(page.visits, []);
  field.events.compositionend();
  await tick();
  assert.equal(page.controller.query(), '회의');
  assert.equal(page.scrolls.at(-1).row, '0');
  assert.equal(page.node('document-search-count').textContent, '1 / 1');
});

test('loading and empty results disable movement, while Escape removes highlights and restores focus', async () => {
  const page = documentPage({ ready: false });
  assert.equal(await page.controller.find('alpha', { view: 'summary' }), false);
  assert.equal(page.node('document-search-count').textContent, '불러오는 중');
  assert.equal(page.node('document-search-next').disabled, true);
  assert.deepEqual(page.visits, []);
  page.setReady(true);
  assert.equal(page.node('document-search-next').disabled, false);
  await page.input('일치하지 않는 문장');
  assert.equal(page.node('document-search-count').textContent, '일치하는 내용 없음');
  assert.equal(page.node('document-search-prev').disabled, true);
  await page.input('alpha');
  assert.ok(page.marks().length > 0);
  assert.equal(await page.key('Escape'), true);
  assert.equal(page.controller.query(), '');
  assert.equal(page.node('document-query').value, '');
  assert.equal(page.node('document-search').hidden, true);
  assert.equal(page.focused(), 'inspector-search-toggle');
  assert.equal(page.marks().length, 0);
});

test('reset cancels a pending view transition and a rejected view change never scrolls', async () => {
  const pending = deferred();
  const page = documentPage({ select: () => pending.promise });
  const finding = page.controller.find('alpha', { view: 'transcript', index: 3 });
  page.controller.reset();
  pending.resolve(true);
  assert.equal(await finding, false);
  assert.equal(page.scrolls.length, 0);
  assert.equal(page.controller.query(), '');
  const blocked = documentPage({ select: async () => false });
  assert.equal(await blocked.controller.find('alpha', { view: 'summary' }), false);
  assert.equal(blocked.scrolls.length, 0);
  assert.equal(blocked.node('document-search').hidden, false);
});
