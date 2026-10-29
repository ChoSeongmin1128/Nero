import { returnFocusTarget } from './focus-return.js';
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter } from '../../../design-system/components/ui/dialog.jsx';
import { createCalendarMeetingFlow, calendarOccurrenceTime, calendarJoinUrls } from './calendar-meeting-flow.js';
import './calendar-meeting-picker.css';

function CalendarLinks({ reference }) {
  const [feedback, setFeedback] = React.useState('');
  React.useEffect(() => setFeedback(''), [reference?.occurrenceKey]);
  const links = calendarJoinUrls(reference);
  return links.length > 0 && <div className="calendar-meeting-links">{links.map(url => <div key={url}>
    <span className="calendar-meeting-url" title={url}>{url}</span>
    <button type="button" className="small quiet" onClick={async () => {
      try { await navigator.clipboard.writeText(url); setFeedback('참가 링크를 복사했습니다.'); }
      catch { setFeedback('참가 링크를 복사하지 못했습니다.'); }
    }}>참가 링크 복사</button>
  </div>)}<span className="type-caption" role="status">{feedback}</span></div>;
}

function CalendarMeetingPicker({ flow, container, finalFocus }) {
  const state = React.useSyncExternalStore(flow.subscribe, flow.getSnapshot);
  const selected = state.occurrences.find(event => event.key === state.selectedKey);
  const busy = state.loading || state.creating;
  const calendar = state.calendar;
  return <Dialog open={state.open} onOpenChange={open => { if (!open) flow.close(); }}>
    <DialogContent container={container} className="calendar-meeting-picker" data-nero-ui-owned="" finalFocus={finalFocus} showCloseButton={!state.creating}>
      <DialogHeader><DialogTitle>일정에서 회의 준비</DialogTitle><DialogDescription>선택한 캘린더의 진행 중인 일정과 앞으로 24시간의 일정</DialogDescription></DialogHeader>
      <div className="calendar-meeting-body" aria-busy={busy}>
        {state.loading && <p className="muted" role="status">일정을 불러오는 중</p>}
        {!state.loading && calendar && !calendar.connected && <div className="calendar-meeting-empty"><p>{calendar.restoring ? 'Google 연결을 확인하고 있습니다.' : 'Google Calendar를 연결해 주세요.'}</p><button type="button" onClick={() => flow.configure()}>캘린더 설정</button></div>}
        {!state.loading && calendar?.connected && !calendar.selectedCalendars.length && <div className="calendar-meeting-empty"><p>캘린더를 선택해 주세요.</p><button type="button" onClick={() => flow.configure()}>캘린더 설정</button></div>}
        {!state.loading && calendar?.connected && calendar.selectedCalendars.length > 0 && !state.occurrences.length && !state.error && <p className="muted">선택할 일정이 없습니다.</p>}
        {!state.loading && state.occurrences.length > 0 && <fieldset className="calendar-meeting-options" disabled={state.creating}><legend className="sr-only">회의 일정</legend>{state.occurrences.map(event => <label className="calendar-meeting-option" key={event.key}>
          <input type="radio" name="calendar-meeting" checked={state.selectedKey === event.key} onChange={() => flow.select(event.key)} />
          <span><strong>{event.title}</strong><span>{calendarOccurrenceTime(event)}</span><span>{event.calendarName}</span></span>
        </label>)}</fieldset>}
        {selected && <CalendarLinks reference={{ ...selected, occurrenceKey: selected.key }} />}
        {state.error && <p className="notice" role="alert">{state.error}</p>}
      </div>
      <DialogFooter><button type="button" disabled={busy} onClick={() => { void flow.refresh(); }}>새로고침</button><button type="button" disabled={state.creating} onClick={() => flow.close()}>취소</button><button type="button" className="primary" disabled={busy || !selected} onClick={() => { void flow.prepare(); }}>{state.creating ? '회의 준비 중' : '회의 준비'}</button></DialogFooter>
    </DialogContent>
  </Dialog>;
}

export function mountCalendarMeetingPicker({ container, bridge, onPrepared, onConfigure }) {
  const document = container.ownerDocument;
  const mount = document.createElement('div');
  mount.dataset.neroUiOwned = '';
  container.append(mount);
  const root = createRoot(mount);
  const flow = createCalendarMeetingFlow({ bridge, onPrepared, onConfigure });
  let origin = null;
  root.render(<CalendarMeetingPicker flow={flow} container={container} finalFocus={() => returnFocusTarget(origin, document.getElementById('navigation-toggle')) || false} />);
  return {
    open(trigger = document.activeElement) { origin = trigger; globalThis.GroveControls?.close?.(); flow.open(); },
    close: () => flow.close(),
    isOpen: () => flow.getSnapshot().open,
    dispose() { flow.dispose(); root.unmount(); mount.remove(); },
  };
}

export function mountCalendarMeetingReference({ container }) {
  container.dataset.neroUiOwned = '';
  const root = createRoot(container);
  return {
    render(meeting) {
      const reference = meeting?.calendarReference;
      container.hidden = !reference;
      root.render(reference ? <div className="calendar-meeting-reference" aria-label="연결된 일정"><span className="calendar-meeting-reference-label">일정</span><strong>{reference.title}</strong><span>{calendarOccurrenceTime(reference)}</span><span>{reference.calendarName}</span><CalendarLinks reference={reference} /></div> : null);
    },
    dispose() { root.unmount(); },
  };
}
