export function createCalendarMeetingFlow({ bridge, onPrepared, onConfigure = () => {} }) {
  let state = { open: false, loading: false, creating: false, calendar: null, occurrences: [], selectedKey: null, error: '' };
  let epoch = 0, disposed = false;
  const listeners = new Set();
  const publish = changes => { if (disposed) return; state = { ...state, ...changes }; listeners.forEach(listener => listener()); };
  const flow = {
    getSnapshot: () => state,
    subscribe(listener) { listeners.add(listener); return () => listeners.delete(listener); },
    open() {
      if (disposed || state.open) return;
      publish({ open: true, calendar: null, occurrences: [], selectedKey: null, error: '' });
      void flow.refresh();
    },
    close() {
      if (state.creating) return false;
      epoch++;
      publish({ open: false, loading: false, error: '' });
      return true;
    },
    async refresh() {
      if (!state.open || state.loading || state.creating || disposed) return false;
      const current = ++epoch;
      publish({ loading: true, selectedKey: null, error: '' });
      try {
        const result = await bridge.readCalendarMeetingOccurrences();
        if (current !== epoch || !state.open || disposed) return false;
        publish({ loading: false, calendar: result.calendar, occurrences: result.occurrences });
        return true;
      } catch (error) {
        if (current === epoch && state.open) publish({ loading: false, occurrences: [], error: error?.message || '일정을 불러오지 못했습니다. 다시 시도해 주세요.' });
        return false;
      }
    },
    select(key) {
      if (!state.open || state.loading || state.creating || !state.occurrences.some(event => event.key === key)) return false;
      publish({ selectedKey: key, error: '' });
      return true;
    },
    async prepare() {
      if (!state.open || state.loading || state.creating || !state.selectedKey || disposed) return false;
      const current = ++epoch;
      const key = state.selectedKey;
      publish({ creating: true, error: '' });
      try {
        const meeting = await bridge.createMeetingFromCalendar(key);
        if (current !== epoch || disposed) return false;
        await onPrepared(meeting);
        publish({ creating: false, open: false });
        return true;
      } catch (error) {
        if (current === epoch) publish({ creating: false, error: error?.message || '회의를 준비하지 못했습니다. 다시 시도해 주세요.' });
        return false;
      }
    },
    configure() { if (flow.close()) onConfigure(); },
    dispose() { epoch++; disposed = true; listeners.clear(); },
  };
  return flow;
}

export function calendarOccurrenceTime(event) {
  const start = new Date(event.startAt), end = new Date(event.endAt);
  const date = new Intl.DateTimeFormat('ko-KR', { month: 'long', day: 'numeric', weekday: 'short' });
  const time = new Intl.DateTimeFormat('ko-KR', { hour: '2-digit', minute: '2-digit' });
  const sameDay = start.toDateString() === end.toDateString();
  return `${date.format(start)} ${time.format(start)} - ${sameDay ? '' : `${date.format(end)} `}${time.format(end)}`;
}

export function calendarJoinUrls(reference) {
  return (reference?.joinUrls || []).filter(value => {
    try { const url = new URL(value); return url.protocol === 'https:' && Boolean(url.hostname) && !url.username && !url.password; }
    catch { return false; }
  });
}
