import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { createCalendarSettingsFlow, calendarConnectionView } from './calendar-settings-flow.js';
import './calendar-settings.css';

function CalendarList({ calendars, selected, busy, connecting, onSelect }) {
  const list = React.useRef(null);
  const pointer = React.useRef({ inside: false, down: false });
  const movement = React.useRef(null);
  const animations = React.useRef([]);
  const [, wake] = React.useReducer(value => value + 1, 0);
  const selectedIds = new Set(selected);
  const sorted = [...calendars].sort((a, b) => Number(selectedIds.has(b.id)) - Number(selectedIds.has(a.id))
    || a.name.localeCompare(b.name, 'ko') || a.id.localeCompare(b.id));
  const [order, setOrder] = React.useState(() => sorted.map(item => item.id));
  const lookup = new Map(calendars.map(item => [item.id, item]));
  const ids = [...order.filter(id => lookup.has(id)), ...sorted.filter(item => !order.includes(item.id)).map(item => item.id)];
  const stopMotion = () => { animations.current.forEach(animation => animation.cancel()); animations.current = []; };
  React.useEffect(() => {
    const node = list.current;
    if (!node || busy || pointer.current.inside || pointer.current.down || node.contains(node.ownerDocument.activeElement)) return;
    const next = sorted.map(item => item.id);
    if (JSON.stringify(order) === JSON.stringify(next)) return;
    const scroll = [];
    for (let ancestor = node; ancestor; ancestor = ancestor.parentElement) scroll.push([ancestor, ancestor.scrollTop]);
    movement.current = { positions: new Map([...node.children].map(row => [row.dataset.calendarId, row.getBoundingClientRect().top])), scroll };
    setOrder(next);
  });
  React.useLayoutEffect(() => {
    const previous = movement.current;
    if (!previous || !list.current) return;
    movement.current = null;
    stopMotion();
    for (const [node, top] of previous.scroll) node.scrollTop = top;
    const reduced = list.current.ownerDocument.defaultView.matchMedia('(prefers-reduced-motion: reduce)').matches;
    if (reduced) return;
    for (const row of list.current.children) {
      const from = previous.positions.get(row.dataset.calendarId);
      const offset = from == null ? 0 : from - row.getBoundingClientRect().top;
      if (offset && row.animate) animations.current.push(row.animate([
        { transform: `translateY(${offset}px)`, opacity: 0.75 },
        { transform: 'translateY(0)', opacity: 1 },
      ], { duration: 180, easing: 'ease-out' }));
    }
  }, [order]);
  React.useEffect(() => {
    const media = list.current.ownerDocument.defaultView.matchMedia('(prefers-reduced-motion: reduce)');
    const change = () => { if (media.matches) stopMotion(); };
    media.addEventListener('change', change);
    return () => { media.removeEventListener('change', change); stopMotion(); };
  }, []);
  React.useEffect(() => {
    const document = list.current.ownerDocument;
    const release = () => { pointer.current.down = false; wake(); };
    const blur = () => { pointer.current = { inside: false, down: false }; stopMotion(); wake(); };
    document.addEventListener('pointerup', release, true);
    document.addEventListener('pointercancel', release, true);
    document.defaultView.addEventListener('blur', blur);
    return () => {
      document.removeEventListener('pointerup', release, true);
      document.removeEventListener('pointercancel', release, true);
      document.defaultView.removeEventListener('blur', blur);
    };
  }, []);
  return <div ref={list} className="settings-group calendar-settings-list" aria-busy={busy}
    onPointerEnter={() => { pointer.current.inside = true; stopMotion(); }}
    onPointerLeave={() => { pointer.current.inside = false; wake(); }}
    onPointerDown={() => { pointer.current.down = true; }}
    onBlurCapture={() => queueMicrotask(wake)}>
    {ids.map(id => { const calendar = lookup.get(id); return <label className="calendar-settings-calendar" data-calendar-id={id} key={id}>
      <input type="checkbox" checked={selectedIds.has(id)} disabled={connecting} aria-disabled={busy || connecting}
        onClick={event => { if (busy) event.preventDefault(); }}
        onChange={event => { if (!busy) void onSelect(id, event.target.checked); }} />
      <span title={calendar.name}>{calendar.name}</span>
    </label>; })}
  </div>;
}

function CalendarSettings({ flow }) {
  const state = React.useSyncExternalStore(flow.subscribe, flow.getSnapshot);
  const snapshot = state.snapshot;
  const view = calendarConnectionView(state);
  const { connecting } = view;
  return <section className="calendar-settings" data-nero-ui-owned="" aria-label="Google Calendar" aria-busy={state.busy}>
    <h4 className="settings-subtitle">Google Calendar</h4>
    <p className="calendar-settings-description">시작 5분 전에 알림</p>
    <div className="settings-group"><div className="calendar-settings-account">
      <div><strong>{view.label}</strong>{snapshot.connected && <p>읽기 전용</p>}</div>
      <div className="calendar-settings-actions">
        {connecting && <><span className="activity-spinner" aria-hidden="true" /><button type="button" className="small" disabled={state.phase !== 'authorizing' || state.reopening} onClick={() => { void flow.reopen(); }}>브라우저 다시 열기</button></>}
        {connecting ? <button type="button" className="small" onClick={() => { void flow.cancel(); }}>취소</button> : view.action && <button type="button" className="small" disabled={state.busy || !snapshot.configured || snapshot.restoring} onClick={() => { void (view.retryable ? flow.retry() : flow.connect()); }}>{view.action}</button>}
        {(snapshot.accountLabel || snapshot.connected) && !connecting && <button type="button" className="small" disabled={state.busy || snapshot.restoring} onClick={() => { void flow.disconnect(); }}>연결 해제</button>}
      </div>
    </div></div>
    {snapshot.connected && <>
      <h4 className="calendar-settings-subtitle">알림을 받을 캘린더</h4>
      <CalendarList calendars={snapshot.calendars} selected={snapshot.selectedCalendars} busy={state.busy} connecting={snapshot.connecting} onSelect={(id, checked) => flow.select(id, checked)} />
      {!snapshot.selectedCalendars.length && <p className="calendar-settings-description">캘린더를 선택하면 일정 알림을 받습니다.</p>}
      <div className="calendar-settings-actions"><button type="button" className="small" disabled={state.busy || snapshot.connecting} onClick={() => { void flow.refresh(); }}>새로고침</button>{snapshot.lastSyncedAt && <span className="calendar-settings-synced">{new Date(snapshot.lastSyncedAt).toLocaleTimeString('ko-KR', { hour: '2-digit', minute: '2-digit' })} 확인</span>}</div>
    </>}
    {!snapshot.configured && !snapshot.error && state.snapshot.revision >= 0 && <p className="notice" role="status">이 버전에는 Google Calendar 연결 정보가 없습니다.</p>}
    {view.error && <p className="notice" role="alert">{view.error}</p>}
  </section>;
}

export function mountCalendarSettings({ panel, bridge, onSnapshot = () => {} }) {
  const flow = createCalendarSettingsFlow({ bridge });
  const unsubscribeSnapshot = flow.subscribe(() => onSnapshot(flow.getSnapshot().snapshot));
  panel.dataset.neroUiOwned = '';
  const root = createRoot(panel);
  let disposed = false;
  let unlisten = null;
  Promise.resolve(bridge.subscribeCalendarSettings(value => flow.accept(value))).then(stop => { if (disposed) stop?.(); else unlisten = stop; }).catch(() => {});
  root.render(<CalendarSettings flow={flow} />);
  return {
    setActive: active => flow.setActive(active),
    hide: () => flow.setActive(false),
    beforeLeave: async () => true,
    isConfirming: () => false,
    dispose() { disposed = true; unlisten?.(); unsubscribeSnapshot(); flow.dispose(); root.unmount(); },
  };
}
