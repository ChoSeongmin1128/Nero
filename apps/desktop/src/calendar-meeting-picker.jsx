import { returnFocusTarget } from './focus-return.js';
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter } from '../../../design-system/components/ui/dialog.jsx';
import { Calendar } from '../../../design-system/components/ui/calendar.jsx';
import { Popover, PopoverTrigger, PopoverContent, PopoverClose } from '../../../design-system/components/ui/popover.jsx';
import { CalendarDays, ChevronLeft, ChevronRight, Copy, Check } from 'lucide-react';
import { createCalendarMeetingFlow, calendarOccurrenceTime, calendarJoinUrls, calendarDay, parseCalendarDay, moveCalendarDay } from './calendar-meeting-flow.js';
import './calendar-meeting-picker.css';

function CalendarLinks({ reference, onOpen }) {
  const [feedback, setFeedback] = React.useState({ text: '', error: false });
  const [copied, setCopied] = React.useState('');
  const operation = React.useRef(0);
  React.useEffect(() => {
    operation.current++; setFeedback({ text: '', error: false }); setCopied('');
    return () => { operation.current++; };
  }, [reference?.occurrenceKey]);
  const links = calendarJoinUrls(reference);
  const open = onOpen && (async (event, url) => {
    if (event.type === 'auxclick' && event.button !== 1) return;
    event.preventDefault();
    const request = ++operation.current;
    setFeedback({ text: '', error: false });
    try { await onOpen(url); }
    catch { if (request === operation.current) setFeedback({ text: '참가 링크를 열지 못했습니다. 링크를 복사해 브라우저에서 열어 주세요.', error: true }); }
  });
  return links.length > 0 && <div className="calendar-meeting-links">{links.map(url => <div key={url}>
    <a className="calendar-meeting-url" href={url} target="_blank" rel="noopener noreferrer" title={url}
      onClick={open ? event => { void open(event, url); } : undefined}
      onAuxClick={open ? event => { void open(event, url); } : undefined}>{url}</a>
    <button type="button" className="small quiet icon" aria-label="참가 링크 복사" title={copied === url ? '복사됨' : '참가 링크 복사'} onClick={async () => {
      const request = ++operation.current;
      try { await navigator.clipboard.writeText(url); if (request === operation.current) { setCopied(url); setFeedback({ text: '참가 링크를 복사했습니다.', error: false }); } }
      catch { if (request === operation.current) { setCopied(''); setFeedback({ text: '참가 링크를 복사하지 못했습니다.', error: true }); } }
    }}>{copied === url ? <Check size={16} aria-hidden="true" /> : <Copy size={16} aria-hidden="true" />}</button>
  </div>)}<span className={feedback.error ? 'type-caption' : 'sr-only'} role="status">{feedback.text}</span></div>;
}

function CalendarDateControls({ flow, state, container }) {
  const [open, setOpen] = React.useState(false);
  const [month, setMonth] = React.useState(() => new Date());
  const selected = parseCalendarDay(state.day);
  React.useEffect(() => { if (selected) setMonth(selected); }, [state.day]);
  const label = selected ? new Intl.DateTimeFormat('ko-KR', { year: 'numeric', month: 'long', day: 'numeric', weekday: 'short' }).format(selected) : '날짜 선택';
  return <div className="calendar-meeting-date" aria-label="일정 날짜">
    <button type="button" className="quiet small icon" aria-label="이전 날짜" title="이전 날짜" disabled={state.creating || !moveCalendarDay(state.day, -1)} onClick={() => { void flow.moveDay(-1); }}><ChevronLeft size={16} aria-hidden="true" /></button>
    <Popover open={open} onOpenChange={setOpen} modal="trap-focus">
      <PopoverTrigger className="nero-date-trigger" disabled={state.creating} aria-label={`일정 날짜: ${label}`} onClick={() => { if (selected) setMonth(selected); }}><span>{label}</span><CalendarDays size={16} aria-hidden="true" /></PopoverTrigger>
      <PopoverContent container={container} className="nero-calendar" aria-label="일정 날짜 선택">
        <PopoverClose className="sr-only">달력 닫기</PopoverClose>
        {open && <Calendar mode="single" autoFocus selected={selected || undefined} month={month} onMonthChange={setMonth} startMonth={new Date(1970, 0, 1)} disabled={{ before: new Date(1970, 0, 1) }} onSelect={date => {
          if (date) { setOpen(false); void flow.setDay(calendarDay(date)); }
        }} />}
      </PopoverContent>
    </Popover>
    <button type="button" className="quiet small icon" aria-label="다음 날짜" title="다음 날짜" disabled={state.creating || !moveCalendarDay(state.day, 1)} onClick={() => { void flow.moveDay(1); }}><ChevronRight size={16} aria-hidden="true" /></button>
    <button type="button" className="quiet small" disabled={state.creating} onClick={() => { void flow.today(); }}>오늘</button>
  </div>;
}

function CalendarMeetingPicker({ flow, container, finalFocus, onOpenLink }) {
  const state = React.useSyncExternalStore(flow.subscribe, flow.getSnapshot);
  const selected = state.occurrences.find(event => event.key === state.selectedKey);
  const busy = state.loading || state.creating;
  const calendar = state.calendar;
  return <Dialog open={state.open} onOpenChange={open => { if (!open) flow.close(); }}>
    <DialogContent container={container} className="calendar-meeting-picker" data-nero-ui-owned="" finalFocus={finalFocus} showCloseButton={!state.creating}>
      <DialogHeader><DialogTitle>일정 연결</DialogTitle><DialogDescription>선택한 날짜의 캘린더 일정</DialogDescription></DialogHeader>
      <CalendarDateControls flow={flow} state={state} container={container} />
      <div className="calendar-meeting-body" aria-busy={busy}>
        {state.loading && <p className="muted" role="status">일정을 불러오는 중</p>}
        {!state.loading && calendar && !calendar.connected && <div className="calendar-meeting-empty"><p>{calendar.restoring ? 'Google 연결을 확인하고 있습니다.' : 'Google Calendar를 연결해 주세요.'}</p><button type="button" onClick={() => flow.configure()}>캘린더 설정</button></div>}
        {!state.loading && calendar?.connected && !calendar.selectedCalendars.length && <div className="calendar-meeting-empty"><p>캘린더를 선택해 주세요.</p><button type="button" onClick={() => flow.configure()}>캘린더 설정</button></div>}
        {!state.loading && calendar?.connected && calendar.selectedCalendars.length > 0 && !state.occurrences.length && !state.error && <p className="muted">이 날짜에 선택할 일정이 없습니다.</p>}
        {!state.loading && state.occurrences.length > 0 && <fieldset className="calendar-meeting-options" disabled={state.creating}><legend className="sr-only">회의 일정</legend>{state.occurrences.map(event => <label className="calendar-meeting-option" key={event.key}>
          <input type="radio" name="calendar-meeting" checked={state.selectedKey === event.key} onChange={() => flow.select(event.key)} />
          <span><strong>{event.title}</strong><span>{calendarOccurrenceTime(event, state.timeZone)}</span><span>{event.calendarName}</span></span>
        </label>)}</fieldset>}
        {selected && <CalendarLinks reference={{ ...selected, occurrenceKey: selected.key }} onOpen={onOpenLink} />}
        {state.error && <p className="notice" role="alert">{state.error}</p>}
      </div>
      <DialogFooter><button type="button" disabled={busy} onClick={() => { void flow.refresh(); }}>새로고침</button><button type="button" disabled={state.creating} onClick={() => flow.close()}>취소</button><button type="button" className="primary" disabled={busy || !selected} onClick={() => { void flow.prepare(); }}>{state.creating ? '연결 중' : '일정 연결'}</button></DialogFooter>
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
  root.render(<CalendarMeetingPicker flow={flow} container={container} finalFocus={() => returnFocusTarget(origin, document.getElementById('navigation-toggle')) || false} onOpenLink={bridge.openCalendarJoinUrl} />);
  return {
    open(trigger = document.activeElement, target = null) { origin = trigger; globalThis.NeroControls?.close?.(); flow.open(target); },
    close: () => flow.close(),
    isOpen: () => flow.getSnapshot().open,
    dispose() { flow.dispose(); root.unmount(); mount.remove(); },
  };
}

export function mountCalendarMeetingReference({ container, bridge }) {
  container.dataset.neroUiOwned = '';
  const root = createRoot(container);
  return {
    render(meeting) {
      const reference = meeting?.calendarReference;
      container.hidden = !reference;
      root.render(reference ? <div className="calendar-meeting-reference" aria-label="연결된 일정"><span className="calendar-meeting-reference-label">일정</span><strong>{reference.title}</strong><span>{calendarOccurrenceTime(reference)}</span><span>{reference.calendarName}</span><CalendarLinks reference={reference} onOpen={bridge?.openCalendarJoinUrl} /></div> : null);
    },
    dispose() { root.unmount(); },
  };
}
