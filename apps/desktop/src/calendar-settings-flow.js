const initial = { revision: -1, configured: false, connected: false, connecting: false, restoring: false, calendars: [], selectedCalendars: [], accountLabel: null, lastSyncedAt: null, error: null };

export function createCalendarSettingsFlow({ bridge }) {
  let state = { active: false, snapshot: initial, busy: false, connecting: false, error: '' };
  let epoch = 0;
  let attemptId = null;
  let disposed = false;
  const listeners = new Set();
  const publish = changes => { if (disposed) return; state = { ...state, ...changes }; listeners.forEach(listener => listener()); };
  const accept = snapshot => {
    if (!snapshot || snapshot.revision < state.snapshot.revision) return;
    publish({ snapshot });
  };
  const run = async operation => {
    if (!state.active || state.busy) return false;
    const current = ++epoch;
    publish({ busy: true, error: '' });
    try {
      const snapshot = await operation();
      if (current !== epoch || !state.active || disposed) return false;
      accept(snapshot);
      publish({ busy: false });
      return true;
    } catch (error) {
      if (current === epoch && state.active && !disposed) publish({ busy: false, error: error?.message || '캘린더를 확인하지 못했습니다. 다시 시도해 주세요.' });
      return false;
    }
  };
  const cancel = async () => {
    const cancelled = attemptId;
    attemptId = null;
    epoch += 1;
    publish({ busy: false, connecting: false });
    if (cancelled) {
      try { accept(await bridge.cancelCalendarConnect(cancelled)); }
      catch (error) { publish({ error: error?.message || '연결 취소를 확인하지 못했습니다.' }); }
    }
  };
  const flow = {
    getSnapshot: () => state,
    subscribe(listener) { listeners.add(listener); return () => listeners.delete(listener); },
    accept,
    setActive(active) {
      if (state.active === active || disposed) return;
      publish({ active });
      if (active) void flow.reload();
      else { void cancel(); }
    },
    reload: () => run(() => bridge.readCalendarSettings()),
    refresh: () => run(() => bridge.syncCalendar()),
    select(id, selected) {
      const ids = state.snapshot.selectedCalendars.filter(value => value !== id);
      if (selected) ids.push(id);
      return run(() => bridge.selectCalendars(ids));
    },
    disconnect: () => run(() => bridge.disconnectCalendar()),
    async connect() {
      if (!state.active || state.busy || state.snapshot.connecting || !state.snapshot.configured) return false;
      const current = ++epoch;
      publish({ busy: true, connecting: true, error: '' });
      try {
        const attempt = await bridge.beginCalendarConnect();
        if (current !== epoch || !state.active || disposed) {
          await bridge.cancelCalendarConnect(attempt.attemptId);
          return false;
        }
        attemptId = attempt.attemptId;
        const snapshot = await bridge.finishCalendarConnect(attemptId);
        if (current !== epoch || !state.active || disposed) return false;
        attemptId = null;
        accept(snapshot);
        publish({ busy: false, connecting: false });
        return true;
      } catch (error) {
        if (current === epoch && state.active && !disposed) {
          attemptId = null;
          publish({ busy: false, connecting: false, error: error?.message || 'Google 연결을 완료하지 못했습니다.' });
        }
        return false;
      }
    },
    cancel,
    dispose() { void cancel(); disposed = true; listeners.clear(); },
  };
  return flow;
}

export function reminderCountdown(startAt, now = Date.now()) {
  const milliseconds = Number(startAt) - Number(now);
  if (!Number.isFinite(milliseconds) || milliseconds <= 0) return '시작 시간';
  if (milliseconds < 60_000) return `${Math.ceil(milliseconds / 1000)}초 후`;
  return `${Math.ceil(milliseconds / 60_000)}분 후`;
}
