import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { createCalendarSettingsFlow, calendarConnectionView } from './calendar-settings-flow.js';
import './calendar-settings.css';

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
        {connecting ? <button type="button" className="small" onClick={() => { void flow.cancel(); }}>취소</button> : <button type="button" className="small" disabled={state.busy || !snapshot.configured || snapshot.restoring} onClick={() => { void (view.retryable ? flow.retry() : flow.connect()); }}>{view.action}</button>}
        {(snapshot.accountLabel || snapshot.connected) && !connecting && <button type="button" className="small" disabled={state.busy || snapshot.restoring} onClick={() => { void flow.disconnect(); }}>연결 해제</button>}
      </div>
    </div></div>
    {snapshot.connected && <>
      <h4 className="calendar-settings-subtitle">알림을 받을 캘린더</h4>
      <div className="settings-group calendar-settings-list">{snapshot.calendars.map(calendar => <label className="calendar-settings-calendar" key={calendar.id}>
        <input type="checkbox" checked={snapshot.selectedCalendars.includes(calendar.id)} disabled={state.busy || snapshot.connecting} onChange={event => { void flow.select(calendar.id, event.target.checked); }} />
        <span title={calendar.name}>{calendar.name}</span>
      </label>)}</div>
      {!snapshot.selectedCalendars.length && <p className="calendar-settings-description">캘린더를 선택하면 일정 알림을 받습니다.</p>}
      <div className="calendar-settings-actions"><button type="button" className="small" disabled={state.busy || snapshot.connecting} onClick={() => { void flow.refresh(); }}>새로고침</button>{snapshot.lastSyncedAt && <span className="calendar-settings-synced">{new Date(snapshot.lastSyncedAt).toLocaleTimeString('ko-KR', { hour: '2-digit', minute: '2-digit' })} 확인</span>}</div>
    </>}
    {!snapshot.configured && !snapshot.error && state.snapshot.revision >= 0 && <p className="notice" role="status">이 버전에는 Google Calendar 연결 정보가 없습니다.</p>}
    {view.error && <p className="notice" role="alert">{view.error}</p>}
  </section>;
}

export function mountCalendarSettings({ panel, bridge }) {
  const flow = createCalendarSettingsFlow({ bridge });
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
    dispose() { disposed = true; unlisten?.(); flow.dispose(); root.unmount(); },
  };
}
