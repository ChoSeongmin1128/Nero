import assert from 'node:assert/strict';
import test from 'node:test';
import { openedMeetingId, routeKey, screenOf, showHome, showList, showMeeting } from '../src/screen-state.js';

const base = { connected: true, setup: false, homeOpen: false, libraryOpen: false, libraryFilter: 'all', meetingSelected: false, modelsReady: true, hasMeetings: true };

test('exactly one screen follows from the state', () => {
  assert.equal(screenOf({ ...base, setup: true, homeOpen: true }), 'setup');
  assert.equal(screenOf({ ...base, homeOpen: true, meetingSelected: true }), 'home');
  assert.equal(screenOf({ ...base, homeOpen: true, libraryOpen: true }), 'library', 'an opened list wins over home');
  assert.equal(screenOf({ ...base, libraryOpen: true, libraryFilter: 'ai' }), 'ai-library');
  assert.equal(screenOf({ ...base, libraryOpen: true, connected: false }), 'none');
  assert.equal(screenOf({ ...base, meetingSelected: true }), 'meeting');
  assert.equal(screenOf({ ...base, meetingSelected: true, connected: false }), 'meeting', 'an open meeting stays while reconnecting');
  assert.equal(screenOf(base), 'library');
  assert.equal(screenOf({ ...base, modelsReady: false, hasMeetings: false }), 'welcome');
  assert.equal(screenOf({ ...base, connected: false, homeOpen: true }), 'welcome', 'home waits for the library');
});

test('the opened meeting is the selection only while a meeting is in front', () => {
  assert.equal(openedMeetingId({ selectedId: 'a' }), 'a');
  assert.equal(openedMeetingId({ selectedId: 'a', homeOpen: true }), null);
  assert.equal(openedMeetingId({ selectedId: 'a', libraryOpen: true }), null);
  assert.equal(openedMeetingId({ selectedId: 'a', setupRequested: true }), null);
  assert.equal(openedMeetingId({}), null);
  assert.notEqual(routeKey({ selectedId: 'a' }), routeKey({ selectedId: 'b' }));
  assert.notEqual(routeKey({ libraryOpen: true, libraryFilter: 'all' }), routeKey({ libraryOpen: true, libraryFilter: 'trash' }));
});

test('moving the content sets the screen flags together', () => {
  const state = { homeOpen: true, libraryOpen: false, setupRequested: true, libraryFilter: 'all', selectedId: null };
  showList(state, 'trash');
  assert.deepEqual([state.homeOpen, state.libraryOpen, state.libraryFilter], [false, true, 'trash']);
  showMeeting(state, 'a');
  assert.deepEqual([state.homeOpen, state.libraryOpen, state.setupRequested, state.selectedId], [false, false, false, 'a']);
  assert.equal(openedMeetingId(state), 'a');
  showHome(state);
  assert.deepEqual([state.homeOpen, state.libraryOpen, state.selectedId], [true, false, 'a'], 'home keeps the last meeting to return to');
  assert.equal(openedMeetingId(state), null);
});
