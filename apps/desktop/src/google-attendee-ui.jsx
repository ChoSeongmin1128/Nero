import * as React from 'react';
import { X } from 'lucide-react';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem, SelectGroup, SelectLabel } from '../../../design-system/components/ui/select.jsx';
import { googleSourceDetail, googleSourceLabel, googleSourceStatus, googleTargetFor } from './google-attendee-draft.js';
import './google-attendee-ui.css';

function GoogleAttendeeRow({ source, state, flow, container }) {
  const [choice, setChoice] = React.useState(null);
  const draft = { attendees: state.attendees, sources: state.sources };
  const suggested = googleTargetFor(draft, state.catalog, state.identityLinks, source);
  const valid = value => value === 'new' || value?.startsWith('attendee:') && state.attendees.some(item => item.id === value.slice(9)) || value?.startsWith('person:') && state.catalog.people.some(item => item.id === value.slice(7));
  const value = valid(choice) ? choice : suggested;
  const record = state.sources.find(entry => entry.source.sourceKey === source.sourceKey);
  const alreadySelected = Boolean(record && state.attendees.some(attendee => attendee.id === record.attendeeId) && value === `attendee:${record.attendeeId}`);
  const selectedPeople = new Set(state.attendees.map(attendee => attendee.personId).filter(Boolean));
  const people = state.catalog.people.filter(person => !selectedPeople.has(person.id));
  const label = value === 'new' ? '새 참석자' : value.startsWith('attendee:') ? state.attendees.find(item => item.id === value.slice(9))?.name || '이름 없는 참석자' : state.catalog.people.find(item => item.id === value.slice(7))?.name;
  const detail = [googleSourceDetail(source), googleSourceStatus(source)].filter(Boolean).join(' / ');
  return <div className="google-attendee-row" data-source-key={source.sourceKey}>
    <div className="people-row-copy"><span className="people-name">{googleSourceLabel(source)}</span>{detail && <span className="people-memberships type-caption" title={detail}>{detail}</span>}</div>
    <div className="google-attendee-choice">
      <Select value={value} onValueChange={setChoice} disabled={state.busy || state.googleConnecting}>
        <SelectTrigger aria-label={`${googleSourceLabel(source)} 연결 대상`}><SelectValue>{label}</SelectValue></SelectTrigger>
        <SelectContent container={container}>
          <SelectItem value="new">새 참석자</SelectItem>
          {state.attendees.length > 0 && <SelectGroup><SelectLabel>선택한 참석자</SelectLabel>{state.attendees.map(attendee => <SelectItem key={attendee.id} value={`attendee:${attendee.id}`}>{attendee.name || '이름 없는 참석자'}</SelectItem>)}</SelectGroup>}
          {people.length > 0 && <SelectGroup><SelectLabel>저장된 인원</SelectLabel>{people.map(person => <SelectItem key={person.id} value={`person:${person.id}`}>{person.name}</SelectItem>)}</SelectGroup>}
        </SelectContent>
      </Select>
      <button type="button" className="small" disabled={state.busy || state.googleConnecting || alreadySelected} aria-label={`${googleSourceLabel(source)} ${alreadySelected ? '추가됨' : value === 'new' ? '새 참석자로 추가' : '연결'}`} onClick={() => { if (flow.chooseGoogle(source, value)) setChoice(null); }}>{alreadySelected ? '추가됨' : value === 'new' ? '추가' : '연결'}</button>
    </div>
  </div>;
}

const messages = {
  noMeetLink: '연결된 Meet 일정이 없습니다.',
  noConference: '확인할 Meet 참석자가 없습니다.',
  ambiguousConference: 'Meet 회의를 하나로 확인할 수 없습니다.',
};

export function GoogleAttendeePicker({ state, flow, container }) {
  if (!state.googleOpen) return null;
  const result = state.googleResult;
  return <section className="people-section google-attendee-picker" aria-label="Google 참석자">
    <header><h4 className="type-label">Google 참석자</h4><div className="people-actions"><button type="button" className="small quiet" disabled={state.busy || state.googleLoading || state.googleConnecting} onClick={() => { void flow.loadGoogle(); }}>새로고침</button><button type="button" className="small quiet icon" aria-label="Google 참석자 목록 닫기" disabled={state.googleConnecting} onClick={() => flow.hideGoogle()}><X size={14} aria-hidden="true" /></button></div></header>
    {state.googleLoading && <p className="inline-feedback" role="status"><span className="activity-spinner" aria-hidden="true" />참석자 불러오는 중</p>}
    {state.googleError && <p className="notice" role="alert">{state.googleError}</p>}
    {result?.warning && <p className="muted type-caption">{result.warning}</p>}
    {state.googleConnecting ? <div className="people-actions" role="status"><span className="muted type-caption">{state.googlePhase === 'cancelling' ? 'Meet 연결 취소 중' : '브라우저에서 승인해 주세요.'}</span><button type="button" className="small" disabled={state.googlePhase !== 'authorizing'} onClick={() => { void flow.reopenMeetConnect(); }}>브라우저 다시 열기</button><button type="button" className="small" disabled={state.googlePhase === 'cancelling'} onClick={() => { void flow.cancelMeetConnect(); }}>Meet 연결 취소</button></div> : result?.meetStatus === 'permissionRequired' && <div className="people-actions"><span className="muted type-caption">Meet 참석자도 불러올 수 있습니다.</span><button type="button" className="small" disabled={state.busy} onClick={() => { void flow.connectMeet(); }}>Meet 연결</button></div>}
    {messages[result?.meetStatus] && <p className="muted type-caption" role="status">{messages[result.meetStatus]}</p>}
    {result && <div className="google-attendee-results">{[['calendarInvitation', '일정 초대자'], ['meetParticipant', 'Meet 참석자']].map(([origin, title]) => {
      const sources = result.sources.filter(source => source.origin === origin);
      return sources.length > 0 && <section className="people-section" key={origin} aria-label={title}><h4 className="type-label">{title}</h4><div className="people-roster">{sources.map(source => <GoogleAttendeeRow key={source.sourceKey} source={source} state={state} flow={flow} container={container} />)}</div></section>;
    })}{!result.sources.length && !state.googleLoading && <p className="people-empty">불러온 참석자가 없습니다.</p>}</div>}
  </section>;
}
