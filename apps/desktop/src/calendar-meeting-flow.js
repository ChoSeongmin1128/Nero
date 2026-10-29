export function calendarDay(date = new Date()) {
  return `${String(date.getFullYear()).padStart(4, '0')}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`;
}

export function parseCalendarDay(day) {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(day) || day < '1970-01-01') return null;
  const [year, month, date] = day.split('-').map(Number);
  const value = new Date(year, month - 1, date, 12);
  return Number.isFinite(value.getTime()) && calendarDay(value) === day ? value : null;
}

export function moveCalendarDay(day, offset) {
  const date = parseCalendarDay(day);
  if (!date || !Number.isInteger(offset)) return null;
  date.setDate(date.getDate() + offset);
  const next = calendarDay(date);
  return parseCalendarDay(next) ? next : null;
}

export function createCalendarMeetingFlow({ bridge, onPrepared, onConfigure = () => {}, now = () => new Date(), timeZone = () => Intl.DateTimeFormat().resolvedOptions().timeZone }) {
  let state = { open: false, loading: false, creating: false, calendar: null, occurrences: [], selectedKey: null, day: '', timeZone: '', scopeRevision: null, error: '' };
  let epoch = 0, disposed = false;
  const listeners = new Set();
  const publish = changes => { if (disposed) return; state = { ...state, ...changes }; listeners.forEach(listener => listener()); };
  const load = async day => {
    if (!state.open || state.creating || disposed || !parseCalendarDay(day)) return false;
    const current = ++epoch;
    const zone = state.timeZone;
    publish({ day, loading: true, occurrences: [], selectedKey: null, scopeRevision: null, error: '' });
    try {
      const result = await bridge.readCalendarMeetingOccurrences({ day, timeZone: zone });
      if (current !== epoch || !state.open || disposed) return false;
      if (result.day !== day || result.timeZone !== zone || !Number.isSafeInteger(result.scopeRevision) || result.scopeRevision < 0) throw Error('조회한 날짜와 캘린더 정보를 확인하지 못했습니다. 새로고침해 주세요.');
      publish({ loading: false, calendar: result.calendar, occurrences: result.occurrences, scopeRevision: result.scopeRevision });
      return true;
    } catch (error) {
      if (current === epoch && state.open) publish({ loading: false, occurrences: [], error: error?.message || '일정을 불러오지 못했습니다. 다시 시도해 주세요.' });
      return false;
    }
  };
  const flow = {
    getSnapshot: () => state,
    subscribe(listener) { listeners.add(listener); return () => listeners.delete(listener); },
    open(target = null) {
      if (disposed || state.open) return;
      publish({ target, open: true, calendar: null, occurrences: [], selectedKey: null, day: calendarDay(now()), timeZone: timeZone(), scopeRevision: null, error: '' });
      void flow.refresh();
    },
    close() {
      if (state.creating) return false;
      epoch++;
      publish({ open: false, loading: false, error: '' });
      return true;
    },
    refresh() { return !state.loading ? load(state.day) : Promise.resolve(false); },
    setDay: day => load(day),
    moveDay(offset) { const day = moveCalendarDay(state.day, offset); return day ? load(day) : Promise.resolve(false); },
    today: () => load(calendarDay(now())),
    select(key) {
      if (!state.open || state.loading || state.creating || !state.occurrences.some(event => event.key === key)) return false;
      publish({ selectedKey: key, error: '' });
      return true;
    },
    async prepare() {
      if (!state.open || state.loading || state.creating || !state.selectedKey || state.scopeRevision == null || disposed) return false;
      const current = ++epoch;
      const key = state.selectedKey;
      const scope = { day: state.day, timeZone: state.timeZone, scopeRevision: state.scopeRevision, ...(state.target ? { target: state.target } : {}) };
      publish({ creating: true, error: '' });
      try {
        const meeting = await bridge.createMeetingFromCalendar(key, scope);
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

export function calendarOccurrenceTime(event, timeZone) {
  const start = new Date(event.startAt), end = new Date(event.endAt);
  const date = new Intl.DateTimeFormat('ko-KR', { month: 'long', day: 'numeric', weekday: 'short', timeZone });
  const time = new Intl.DateTimeFormat('ko-KR', { hour: '2-digit', minute: '2-digit', timeZone });
  const sameDay = date.format(start) === date.format(end);
  return `${date.format(start)} ${time.format(start)} - ${sameDay ? '' : `${date.format(end)} `}${time.format(end)}`;
}

export function calendarJoinUrls(reference) {
  return (reference?.joinUrls || []).filter(value => {
    try { const url = new URL(value); return url.protocol === 'https:' && Boolean(url.hostname) && !url.username && !url.password; }
    catch { return false; }
  });
}
