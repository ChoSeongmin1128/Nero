const initial = { revision: -1, configured: false, connected: false, connecting: false, restoring: false, calendars: [], selectedCalendars: [], accountLabel: null, lastSyncedAt: null, error: null };

export function createCalendarSettingsFlow({ bridge }) {
  let state = { active: false, snapshot: initial, busy: false, connecting: false, phase: 'idle', reopening: false, error: '' };
  let epoch = 0;
  let attemptId = null;
  let disposed = false;
  let beginning = null;
  let cancellation = null;
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
  const cancel = () => {
    if (cancellation) return cancellation;
    const current = ++epoch;
    const pendingBegin = beginning;
    publish({ busy: true, connecting: false, reopening: false, phase: 'cancelling' });
    cancellation = Promise.resolve().then(async () => {
      try {
        const started = pendingBegin ? await pendingBegin.catch(() => null) : null;
        if (beginning === pendingBegin) beginning = null;
        const cancelled = attemptId || started?.attemptId;
        if (cancelled) { attemptId = cancelled; accept(await bridge.cancelCalendarConnect(cancelled)); }
        attemptId = null;
        if (current === epoch) publish({ busy: false, phase: 'idle', error: '' });
        return true;
      } catch (error) {
        if (current === epoch) publish({ busy: false, phase: 'cancelFailed', error: error?.message || '연결 취소를 확인하지 못했습니다. 다시 시도해 주세요.' });
        return false;
      } finally { cancellation = null; }
    });
    return cancellation;
  };
  const flow = {
    getSnapshot: () => state,
    subscribe(listener) { listeners.add(listener); return () => listeners.delete(listener); },
    accept,
    setActive(active) {
      if (state.active === active || disposed) return;
      publish({ active });
      if (active) void (cancellation || Promise.resolve()).then(() => flow.reload());
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
    async reopen() {
      if (!attemptId || state.phase !== 'authorizing' || state.reopening) return false;
      const current = epoch;
      publish({ reopening: true });
      try { await bridge.reopenCalendarConnect(attemptId); return true; }
      catch (error) { if (current === epoch) publish({ error: error?.message || '브라우저를 열지 못했습니다. 다시 시도해 주세요.' }); return false; }
      finally { if (current === epoch) publish({ reopening: false }); }
    },
    async retry() {
      if (!(await cancel()) || !state.active || disposed) return false;
      return flow.connect();
    },
    async connect() {
      if (!state.active || state.busy || state.snapshot.connecting || !state.snapshot.configured || cancellation || state.phase === 'cancelFailed') return false;
      const current = ++epoch;
      publish({ busy: true, connecting: true, phase: 'opening', error: '' });
      try {
        beginning = bridge.beginCalendarConnect();
        const attempt = await beginning;
        beginning = null;
        if (current !== epoch || !state.active || disposed) {
          if (cancellation) await cancellation;
          else await bridge.cancelCalendarConnect(attempt.attemptId);
          return false;
        }
        attemptId = attempt.attemptId;
        publish({ phase: 'authorizing' });
        const snapshot = await bridge.finishCalendarConnect(attemptId);
        if (current !== epoch || !state.active || disposed) return false;
        attemptId = null;
        accept(snapshot);
        publish({ busy: false, connecting: false, reopening: false, phase: 'idle', error: '' });
        return true;
      } catch (error) {
        if (current === epoch && state.active && !disposed) {
          beginning = null;
          attemptId = null;
          publish({ busy: false, connecting: false, phase: 'failed', error: error?.message || 'Google 연결을 완료하지 못했습니다.' });
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

export function calendarConnectionView(state) {
  const { snapshot, phase } = state;
  const connecting = state.connecting || (phase === 'idle' && snapshot.connecting);
  const cancelling = phase === 'cancelling';
  const retryable = ['failed', 'cancelFailed'].includes(phase);
  return {
    connecting, cancelling, retryable,
    label: cancelling ? '연결 취소 중' : connecting ? '브라우저에서 승인 중' : snapshot.restoring ? '연결 확인 중' : snapshot.connected ? snapshot.accountLabel : '연결 안 됨',
    action: retryable ? '다시 시도' : snapshot.connected ? null : 'Google 연결',
    error: state.error || (!connecting && snapshot.error) || '',
  };
}
