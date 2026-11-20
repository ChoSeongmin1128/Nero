// Which screen the content area shows, decided from the app state alone.
// 'none' is a list asked for before the library has connected.
export function screenOf({ connected, setup, homeOpen, libraryOpen, libraryFilter, meetingSelected, modelsReady, hasMeetings }) {
  if (setup) return 'setup';
  if (connected && homeOpen && !libraryOpen) return 'home';
  if (libraryOpen) return !connected ? 'none' : libraryFilter === 'ai' ? 'ai-library' : 'library';
  if (meetingSelected) return 'meeting';
  return connected && (modelsReady || hasMeetings) ? 'library' : 'welcome';
}

// The meeting the person has open, which the sidebar marks and the banner follows.
export const openedMeetingId = state => !state.libraryOpen && !state.homeOpen && !state.setupRequested ? state.selectedId ?? null : null;

// Where the content is, for navigation that reacts when it moves.
export const routeKey = state => `${Boolean(state.homeOpen)}:${Boolean(state.libraryOpen)}:${state.selectedId ?? ''}:${state.libraryFilter ?? ''}`;

// Every move of the content goes through these, so the screen flags never disagree.
export function showHome(state) { state.homeOpen = true; state.libraryOpen = false; state.setupRequested = false; }
export function showList(state, filter = state.libraryFilter) { state.libraryFilter = filter; state.libraryOpen = true; state.homeOpen = false; }
export function showMeeting(state, id) { state.selectedId = id; state.libraryOpen = false; state.homeOpen = false; state.setupRequested = false; }
