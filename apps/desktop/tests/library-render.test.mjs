import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { Window } from 'happy-dom';
import { renderMeetingMenu } from '../src/library-ui.js';

function createList(meetings, folders = []) {
  const window = new Window();
  const document = window.document;
  document.body.innerHTML = '<input id="search"><nav id="meeting-list"></nav><span id="meeting-count"></span>';
  const state = { snapshot: { meetings, folders }, librarySelection: new Set(), librarySearchHits: [], selectedId: null, connected: true, libraryBusy: false, libraryFilter: 'all', listKey: '' };
  const context = vm.createContext({ document, state, renderMeetingMenu, library: { visibleMeetings: () => meetings }, byId: id => document.getElementById(id),
    NeroIcons: () => '<svg aria-hidden="true"></svg>', stateFor: meeting => meeting.status || 'draft',
    meetingTimestamp: meeting => meeting.createdAt, meetingDay: () => '2026-10-28',
    listDate: { format: () => '10. 28.' }, listTime: { format: () => '10:00' }, fullDate: { format: () => '2026년 10월 28일 10:00' },
  });
  vm.runInContext(readFileSync(new URL('../../../design-system/components.js', import.meta.url), 'utf8'), context);
  vm.runInContext(readFileSync(new URL('../src/ui.js', import.meta.url), 'utf8').replace(/^export /gm, ''), context);
  const source = readFileSync(new URL('../src/main.js', import.meta.url), 'utf8');
  vm.runInContext(source.slice(source.indexOf('function renderList()'), source.indexOf('function refreshMeetingSearch()')), context);
  const render = () => vm.runInContext('renderList()', context);
  render();
  return { window, document, state, context, render };
}

test('completed list rows keep metadata, actions and selection focus without a status placeholder', () => {
  const meetings = [{ id: 'one', title: '검토 회의', status: 'completed', folderId: 'team', audio: { frames: 75, sampleRate: 1 }, notes: '', createdAt: 1, deletedAt: null }];
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
  document.querySelector('summary').focus();
  state.librarySelection.clear();
  render();
  assert.equal(document.activeElement.tagName, 'SUMMARY');
  assert.equal(document.getElementById('meeting-list').classList.contains('has-selection'), false);
  window.happyDOM.abort();
});

test('other list states and the shared completed status remain visible', () => {
  const labels = { recording: '녹음 중', paused: '일시정지', finalizing: '저장 중', interrupted: '녹음 중단', draft: '녹음 전', idle: '녹음 전' };
  const meetings = Object.keys(labels).map(status => ({ id: status, title: status, status, createdAt: 1, deletedAt: null }));
  const { window, document, context } = createList(meetings);
  for (const [phase, label] of Object.entries(labels)) {
    const status = document.querySelector(`[data-meeting-id="${phase}"] .meeting-item-state`);
    assert.equal(status.textContent, label);
    assert.equal(status.classList.contains('recording'), ['recording', 'paused'].includes(phase));
  }
  const detail = document.createElement('div');
  detail.innerHTML = vm.runInContext("statusMarkup('completed')", context);
  assert.equal(detail.querySelector('.status-chip.finished').textContent, '녹음 완료');
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
