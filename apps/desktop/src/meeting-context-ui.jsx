import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { Link2, RefreshCw, X } from 'lucide-react';
import { Popover, PopoverTrigger, PopoverContent, PopoverTitle } from '../../../design-system/components/ui/popover.jsx';
import { createMeetingContextFlow } from './meeting-context-flow.js';
import { DomainGlossary } from './domain-glossary-ui.jsx';
import { BrandIcon } from './brand-icon.jsx';

function MeetingContext({ flow, onSettings, bridge }) {
  const state = React.useSyncExternalStore(flow.subscribe, flow.getSnapshot);
  const input = React.useRef(null);
  return <Popover open={state.open} onOpenChange={open => open ? flow.open() : flow.close()}>
    <PopoverTrigger type="button" className="quiet small with-icon meeting-context-trigger" disabled={!state.meetingId} aria-label={`회의 자료${state.sources.length ? ` ${state.sources.length}개` : ''}`}><Link2 size={15} aria-hidden="true" /><span>자료</span>{state.sources.length > 0 && <span>{state.sources.length}</span>}</PopoverTrigger>
    <PopoverContent align="start" className="meeting-context-popover" data-nero-ui-owned="" initialFocus={input}>
      <PopoverTitle className="meeting-context-title">회의 자료</PopoverTitle>
      {state.loading ? <p className="meeting-context-empty" role="status">자료를 불러오는 중</p> : state.sources.length ? <ul className="meeting-context-list">{state.sources.map(source => <li key={source.sourceId}>
        <button type="button" className="quiet meeting-context-link" title={source.url} onClick={() => { void flow.openSource(source.url); }}><BrandIcon name="notion" /><span>{source.title || source.url || 'Notion 자료'}{source.unavailable ? ' (저장된 자료 없음)' : ''}</span></button>
        <button type="button" className="quiet small icon" aria-label={`${source.title || 'Notion 자료'} 새로고침`} title="새로고침" disabled={state.busy || !state.notion?.connected} onClick={() => { void flow.refresh(source); }}><RefreshCw size={14} aria-hidden="true" /></button>
        <button type="button" className="quiet small icon" aria-label={`${source.title || 'Notion 자료'} 제거`} title="제거" disabled={state.busy} onClick={() => { void flow.remove(source.pageId); }}><X size={14} aria-hidden="true" /></button>
      </li>)}</ul> : <p className="meeting-context-empty">추가한 자료가 없습니다.</p>}
      <form className="meeting-context-add" onSubmit={event => { event.preventDefault(); void flow.add(); }}>
        <label className="sr-only" htmlFor="meeting-context-link">Notion 페이지 링크</label>
        <input ref={input} id="meeting-context-link" type="url" placeholder="Notion 페이지 링크" value={state.link} onChange={event => flow.setLink(event.target.value)} disabled={state.busy} required />
        {state.phase === 'fetching' ? <button type="button" className="small" onClick={() => flow.cancel()}>취소</button> : state.busy ? <button type="button" className="small" disabled>처리 중</button> : state.notion && !state.notion.connected ? <button type="button" className="small" onClick={() => { flow.close(); onSettings(); }}>Notion 연결</button> : <button type="submit" className="small" disabled={state.loading || !state.link.trim() || !state.notion}>추가</button>}
      </form>
      {state.meetingId && <DomainGlossary meetingId={state.meetingId} sourcesRevision={state.revision} bridge={bridge} />}
      {state.error && <div className="meeting-context-error" role="alert"><span>{state.error}</span><button type="button" className="quiet small" disabled={state.busy} onClick={() => { void flow.reload(); }}>목록 다시 불러오기</button></div>}
    </PopoverContent>
  </Popover>;
}

export function mountMeetingContext({ node, bridge, getMeeting, onSettings, onChanged }) {
  const root = createRoot(node);
  const flow = createMeetingContextFlow({ bridge, getMeeting, onChanged });
  root.render(<MeetingContext flow={flow} onSettings={onSettings} bridge={bridge} />);
  return { setMeeting: meeting => flow.setMeeting(meeting), reload: () => flow.reload(), close: () => flow.close(), dispose() { flow.dispose(); root.unmount(); } };
}
