import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { Window } from 'happy-dom';
import { renderMeetingMenu } from '../src/library-ui.js';

test('list selection preserves the checkbox and menu focus through a row refresh', () => {
  const window = new Window();
  const document = window.document;
  document.body.innerHTML = '<input id="search"><nav id="meeting-list"></nav><span id="meeting-count"></span>';
  const meetings = [{ id: 'one', title: '검토 회의', notes: '', createdAt: 1, deletedAt: null }];
  const state = { snapshot: { meetings }, librarySelection: new Set(), librarySearchHits: [], selectedId: null, connected: true, libraryBusy: false, libraryFilter: 'all', listKey: '' };
  const context = vm.createContext({ document, state, renderMeetingMenu, library: { visibleMeetings: () => meetings }, byId: id => document.getElementById(id),
    escape: value => String(value ?? ''), icon: () => '<svg aria-hidden="true"></svg>', stateFor: () => 'ready',
    meetingTimestamp: meeting => meeting.createdAt, meetingDay: () => '2026-10-28',
    listDate: { format: () => '10. 28.' }, listTime: { format: () => '10:00' }, fullDate: { format: () => '2026년 10월 28일 10:00' }, recordingStatus: () => '녹음 전', duration: () => '',
  });
  vm.runInContext(readFileSync(new URL('../../../design-system/components.js', import.meta.url), 'utf8'), context);
  const source = readFileSync(new URL('../src/main.js', import.meta.url), 'utf8');
  vm.runInContext(source.slice(source.indexOf('function renderList()'), source.indexOf('function refreshMeetingSearch()')), context);
  vm.runInContext('renderList()', context);
  const checkbox = document.querySelector('[data-library-toggle="one"]');
  assert.equal(checkbox.hidden, false);
  checkbox.focus();
  state.librarySelection.add('one');
  vm.runInContext('renderList()', context);
  assert.equal(document.activeElement.dataset.libraryToggle, 'one');
  assert.equal(document.activeElement.checked, true);
  assert.equal(document.getElementById('meeting-list').classList.contains('has-selection'), true);
  document.querySelector('summary').focus();
  state.librarySelection.clear();
  vm.runInContext('renderList()', context);
  assert.equal(document.activeElement.tagName, 'SUMMARY');
  assert.equal(document.getElementById('meeting-list').classList.contains('has-selection'), false);
  window.happyDOM.abort();
});
