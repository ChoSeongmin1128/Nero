import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { Window } from 'happy-dom';
import { mountLibrary } from '../src/library-ui.js';

globalThis.NeroUI ??= createRequire(import.meta.url)('../../../design-system/components.js');
const { escape, duration, recordingStatus, statusMarkup } = await import('../src/ui.js');
const html = readFileSync(new URL('../src/index.html', import.meta.url), 'utf8');
const at = (day, hour, minute) => new Date(2026, 9, day, hour, minute).getTime();

function createList(meetings, folders = [], { selectMeeting = () => {} } = {}) {
  const window = new Window({ url: 'http://nero.test', settings: { disableCSSFileLoading: true, disableJavaScriptFileLoading: true } });
  const document = window.document;
  document.write(html);
  const state = { snapshot: { meetings, folders, session: null }, librarySelection: new Set(), librarySearchHits: [], selectedId: null, connected: true, libraryBusy: false, libraryFilter: 'all', libraryView: 'all', libraryOpen: true };
  let library;
  library = mountLibrary({ bridge: {}, getState: () => state, byId: id => document.getElementById(id), finishEditing: async action => action(), acceptSnapshot() {}, selectMeeting,
    render: () => library.render(), showError(error) { throw error; }, escape, icon: () => '<svg aria-hidden="true"></svg>', duration, recordingStatus });
  const render = () => library.render();
  render();
  return { window, document, state, render };
}

test('completed list rows keep metadata, actions and selection focus without a status placeholder', () => {
  const meetings = [{ id: 'one', title: '검토 회의', status: 'completed', folderId: 'team', audio: { frames: 75, sampleRate: 1 }, notes: '', createdAt: at(28, 10, 0), deletedAt: null }];
  const { window, document, state, render } = createList(meetings, [{ id: 'team', name: '제품팀' }]);
  const row = document.querySelector('[data-meeting-id="one"]');
  assert.equal(row.querySelector('.meeting-item-state'), null);
  assert.equal(row.querySelector('.meeting-item').children.length, 1);
  assert.equal(row.querySelector('strong').textContent, '검토 회의');
  assert.deepEqual([...row.querySelector('.meeting-item-meta').children].map(node => node.textContent), ['10. 28. 10:00', '01:15', '제품팀']);
  assert.deepEqual([...row.querySelectorAll('[data-library-action]')].map(node => node.dataset.libraryAction), ['rename', 'move', 'trash']);
  const checkbox = document.querySelector('[data-library-toggle="one"]');
  assert.equal(checkbox.hidden, false);
  checkbox.focus();
  state.librarySelection.add('one');
  render();
  assert.equal(document.activeElement.dataset.libraryToggle, 'one');
  assert.equal(document.activeElement.checked, true);
  assert.equal(document.getElementById('meeting-list').classList.contains('has-selection'), true);
  document.querySelector('#meeting-list summary').focus();
  state.librarySelection.clear();
  render();
  assert.equal(document.activeElement.tagName, 'SUMMARY');
  assert.equal(document.getElementById('meeting-list').classList.contains('has-selection'), false);
  window.happyDOM.abort();
});

test('other list states and the shared completed status remain visible', () => {
  const labels = { recording: '녹음 중', paused: '일시정지', finalizing: '저장 중', interrupted: '녹음 중단', draft: '녹음 전', idle: '녹음 전' };
  const meetings = Object.keys(labels).map(status => ({ id: status, title: status, status, createdAt: 1, deletedAt: null }));
  const { window, document } = createList(meetings);
  for (const [phase, label] of Object.entries(labels)) {
    const status = document.querySelector(`[data-meeting-id="${phase}"] .meeting-item-state`);
    assert.equal(status.textContent, label);
    assert.equal(status.classList.contains('recording'), ['recording', 'paused'].includes(phase));
  }
  const detail = document.createElement('div');
  detail.innerHTML = statusMarkup('completed');
  assert.equal(detail.querySelector('.status-chip.finished').textContent, '녹음 완료');
  detail.innerHTML = statusMarkup('completed', { imported: true });
  assert.equal(detail.querySelector('.status-chip.finished').textContent, '가져온 음성');
  detail.innerHTML = statusMarkup('finalizing', { imported: true });
  assert.equal(detail.querySelector('.status-chip.finished').textContent, '저장 중');
  window.happyDOM.abort();
});

test('a refreshed recording row removes only its completed status and can show interruption again', () => {
  const meeting = { id: 'one', title: '검토 회의', status: 'recording', createdAt: 1, deletedAt: null };
  const { window, document, render } = createList([meeting]);
  assert.equal(document.querySelector('.meeting-item-state').textContent, '녹음 중');
  meeting.status = 'completed';
  render();
  assert.equal(document.querySelector('.meeting-item-state'), null);
  assert.equal(document.querySelector('[data-open-meeting="one"]').textContent.includes('검토 회의'), true);
  assert.ok(document.querySelector('[data-meeting-menu="one"]'));
  meeting.status = 'interrupted';
  render();
  assert.equal(document.querySelector('.meeting-item-state').textContent, '녹음 중단');
  window.happyDOM.abort();
});

test('meeting list shows local recording time and refreshes it when recording starts', () => {
  const meeting = { id: 'a', title: '회의 A', notes: '', status: 'draft', createdAt: at(20, 9, 5), meetingDate: '2026-10-01', deletedAt: null };
  const { window, document, state, render } = createList([meeting]);
  const list = document.getElementById('meeting-list');
  assert.match(list.innerHTML, />10\. 20\. 09:05<\/span>/);
  assert.match(list.innerHTML, /title="생성 /);
  list.scrollTop = 80;
  state.snapshot = { ...state.snapshot, meetings: [{ ...meeting, recordedAt: at(20, 18, 41) }] };
  render();
  assert.match(list.innerHTML, />10\. 20\. 18:41<\/span>/);
  assert.match(list.innerHTML, /title="녹음 시작 /);
  assert.match(list.innerHTML, /회의 날짜 2026-10-01/);
  assert.doesNotMatch(list.innerHTML, />10\. 1\. 18:41/);
  assert.equal(list.scrollTop, 80);
  window.happyDOM.abort();
});

test('meeting list orders by the displayed timestamp without inventing imported recording time', () => {
  const base = { title: '회의 A', notes: '', status: 'draft', deletedAt: null };
  const recorded = { ...base, id: 'recorded', createdAt: at(20, 8, 0), recordedAt: at(20, 18, 41), meetingDate: '2026-10-01', audio: {} };
  const imported = { ...base, id: 'imported', createdAt: at(20, 14, 5), audio: {} };
  const midnight = { ...base, id: 'midnight', createdAt: at(20, 0, 0) };
  const { window, document } = createList([midnight, imported, recorded]);
  const rows = document.getElementById('meeting-list').innerHTML;
  assert.ok(rows.indexOf('data-meeting-id="recorded"') < rows.indexOf('data-meeting-id="imported"'));
  assert.ok(rows.indexOf('data-meeting-id="imported"') < rows.indexOf('data-meeting-id="midnight"'));
  assert.match(rows, />10\. 20\. 00:00<\/span>/);
  const importedRow = rows.slice(rows.indexOf('data-meeting-id="imported"'), rows.indexOf('data-meeting-id="midnight"'));
  assert.match(importedRow, /title="생성 /);
  assert.doesNotMatch(importedRow, /녹음 시작/);
  window.happyDOM.abort();
});

test('opening a row passes its content search hit while a trashed row only toggles selection', () => {
  const opened = [];
  const meetings = [{ id: 'live', title: '회의', status: 'completed', createdAt: 2, deletedAt: null }, { id: 'gone', title: '삭제한 회의', status: 'completed', createdAt: 1, deletedAt: 5 }];
  const { window, document, state, render } = createList(meetings, [], { selectMeeting: (id, intent) => opened.push([id, intent]) });
  state.librarySearchHits = [{ meetingId: 'live', kind: 'notes', text: '메모' }];
  state.librarySearchIds = ['live'];
  document.getElementById('search').value = '메모';
  render();
  document.querySelector('[data-open-meeting="live"]').click();
  assert.deepEqual(opened, [['live', { query: '메모', hit: state.librarySearchHits[0] }]]);
  state.libraryFilter = 'trash'; state.librarySearchIds = null; document.getElementById('search').value = '';
  render();
  document.querySelector('[data-open-meeting="gone"]').click();
  assert.equal(opened.length, 1);
  assert.equal(state.librarySelection.has('gone'), true);
  window.happyDOM.abort();
});

test('a changed meeting replaces only its own list row and other changes still rebuild the list', () => {
  const meetings = [{ id: 'one', title: '첫 회의', status: 'completed', createdAt: at(20, 9, 0), deletedAt: null }, { id: 'two', title: '둘째 회의', status: 'completed', createdAt: at(20, 10, 0), deletedAt: null }];
  const { window, document, state, render } = createList(meetings);
  const row = id => document.querySelector(`[data-meeting-id="${id}"]`);
  const untouched = row('one'), changed = row('two');
  state.snapshot = { ...state.snapshot, meetings: [meetings[0], { ...meetings[1], title: '바뀐 회의' }] };
  render();
  assert.equal(row('one'), untouched);
  assert.notEqual(row('two'), changed);
  assert.equal(row('two').querySelector('strong').textContent, '바뀐 회의');
  state.librarySelection.add('one');
  render();
  assert.notEqual(row('one'), untouched);
  assert.equal(row('one').querySelector('[data-library-toggle]').checked, true);
  state.snapshot = { ...state.snapshot, meetings: [meetings[0]] };
  render();
  assert.deepEqual([...document.querySelectorAll('[data-meeting-id]')].map(node => node.dataset.meetingId), ['one']);
  state.snapshot = { ...state.snapshot, meetings: [] };
  render();
  assert.match(document.getElementById('meeting-list').textContent, /첫 회의를 시작/);
  window.happyDOM.abort();
});
