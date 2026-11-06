import * as React from 'react';
import { X } from 'lucide-react';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem, SelectGroup, SelectLabel } from '../../../design-system/components/ui/select.jsx';
import { googleRows, personChoiceLabel } from './google-attendee-draft.js';
import './google-attendee-ui.css';

function rowHint(row) {
  if (row.addedTo || row.reason === 'linked') return '';
  if (row.sharedName) return '이름이 같은 참석자가 여러 명이라 자동으로 연결하지 않았습니다.';
  if (row.homonyms) return `이름이 같은 저장 인원이 ${row.homonyms}명입니다. 연결 대상을 확인해 주세요.`;
  return row.reason === 'name' ? '이름이 같은 사람으로 제안했습니다.' : '';
}

function GoogleAttendeeRow({ row, view, update, state, container }) {
  const busy = state.busy || state.googleConnecting, added = Boolean(row.addedTo);
  const selectedPeople = new Set(state.attendees.map(attendee => attendee.personId).filter(Boolean));
  const people = state.catalog.people.filter(person => !selectedPeople.has(person.id));
  const personName = id => { const person = state.catalog.people.find(item => item.id === id); return person ? personChoiceLabel(person, state.catalog, state.identityLinks) : ''; };
  const attendeeName = id => state.attendees.find(item => item.id === id)?.name || '이름 없는 참석자';
  const targetName = view.choice === 'new' ? '새 참석자' : view.choice.startsWith('attendee:') ? attendeeName(view.choice.slice(9)) : personName(view.choice.slice(7));
  const hint = rowHint(row);
  return <div className="google-attendee-row" data-source-key={row.key}>
    <input type="checkbox" className="google-attendee-check" checked={view.checked} disabled={busy || added} aria-label={`${row.label} 추가`} onChange={event => update({ checked: event.target.checked })} />
    <div className="google-attendee-identity">
      {view.choice === 'new' && !added
        ? <input className="text-input text-input--compact" aria-label={`${row.label} 이름`} value={view.name} maxLength={120} disabled={busy} onChange={event => update({ name: event.target.value })} />
        : <span className="people-name">{targetName}</span>}
      <span className="people-memberships type-caption">{[row.label !== targetName && view.choice !== 'new' ? row.label : '', row.status, row.detail, added ? '추가됨' : ''].filter(Boolean).join(' / ')}</span>
      {hint && <span className="type-caption google-attendee-hint">{hint}</span>}
    </div>
    <Select value={view.choice} onValueChange={choice => update({ choice })} disabled={busy || added}>
      <SelectTrigger aria-label={`${row.label} 연결 대상`}><SelectValue>{targetName}</SelectValue></SelectTrigger>
      <SelectContent container={container}>
        <SelectItem value="new">새 참석자</SelectItem>
        {state.attendees.length > 0 && <SelectGroup><SelectLabel>선택한 참석자</SelectLabel>{state.attendees.map(attendee => <SelectItem key={attendee.id} value={`attendee:${attendee.id}`}>{attendee.name || '이름 없는 참석자'}</SelectItem>)}</SelectGroup>}
        {people.length > 0 && <SelectGroup><SelectLabel>저장된 인원</SelectLabel>{people.map(person => <SelectItem key={person.id} value={`person:${person.id}`}>{personChoiceLabel(person, state.catalog, state.identityLinks)}</SelectItem>)}</SelectGroup>}
      </SelectContent>
    </Select>
  </div>;
}

const messages = {
  noMeetLink: '연결된 Meet 일정이 없습니다.',
  noConference: '확인할 Meet 참석자가 없습니다.',
  ambiguousConference: 'Meet 회의를 하나로 확인할 수 없습니다.',
};

export function GoogleAttendeePicker({ state, flow, container }) {
  const [entries, setEntries] = React.useState({});
  React.useEffect(() => { setEntries({}); }, [state.googleResult]);
  if (!state.googleOpen) return null;
  const result = state.googleResult;
  const rows = result && state.catalog ? googleRows(result, { attendees: state.attendees, sources: state.sources }, state.catalog, state.identityLinks) : [];
  const valid = value => value === 'new' || value?.startsWith('attendee:') && state.attendees.some(item => item.id === value.slice(9)) || value?.startsWith('person:') && state.catalog?.people.some(item => item.id === value.slice(7));
  const viewOf = row => {
    const entry = entries[row.key] || {};
    return { checked: !row.addedTo && (entry.checked ?? row.checked), choice: valid(entry.choice) ? entry.choice : row.suggestion, name: entry.name ?? row.label };
  };
  const chosen = rows.filter(row => viewOf(row).checked);
  const busy = state.busy || state.googleLoading || state.googleConnecting;
  const add = () => { if (flow.chooseGoogleRows(chosen.map(row => ({ row, ...viewOf(row) })))) setEntries({}); };
  return <section className="people-section google-attendee-picker" aria-label="Google 참석자">
    <header><h4 className="type-label">Google 참석자</h4><div className="people-actions"><button type="button" className="small quiet icon" aria-label="Google 참석자 목록 닫기" disabled={state.googleConnecting} onClick={() => flow.hideGoogle()}><X size={14} aria-hidden="true" /></button></div></header>
    {state.googleLoading && <p className="inline-feedback" role="status"><span className="activity-spinner" aria-hidden="true" />참석자 불러오는 중</p>}
    {state.googleError && <p className="notice" role="alert">{state.googleError}{!state.googleLoading && state.googleAccess === 'ready' && <> <button type="button" className="small quiet" disabled={busy} onClick={() => { void flow.loadGoogle(); }}>다시 시도</button></>}</p>}
    {result?.warning && <p className="muted type-caption">{result.warning}</p>}
    {state.googleConnecting ? <div className="people-actions" role="status"><span className="muted type-caption">{state.googlePhase === 'cancelling' ? 'Meet 연결 취소 중' : '브라우저에서 승인해 주세요.'}</span><button type="button" className="small" disabled={state.googlePhase !== 'authorizing'} onClick={() => { void flow.reopenMeetConnect(); }}>브라우저 다시 열기</button><button type="button" className="small" disabled={state.googlePhase === 'cancelling'} onClick={() => { void flow.cancelMeetConnect(); }}>Meet 연결 취소</button></div> : result?.meetStatus === 'permissionRequired' && <div className="people-actions"><span className="muted type-caption">Meet에 실제로 들어온 사람도 확인할 수 있습니다.</span><button type="button" className="small" disabled={busy} onClick={() => { void flow.connectMeet(); }}>Meet 연결</button></div>}
    {messages[result?.meetStatus] && <p className="muted type-caption" role="status">{messages[result.meetStatus]}</p>}
    {result && <div className="google-attendee-results">
      {rows.length > 0 && <div className="people-roster" aria-label="불러온 참석자">{rows.map(row => <GoogleAttendeeRow key={row.key} row={row} view={viewOf(row)} update={patch => setEntries(current => ({ ...current, [row.key]: { ...current[row.key], ...patch } }))} state={state} container={container} />)}</div>}
      {rows.length > 0 && <div className="people-actions google-attendee-actions"><button type="button" className="small primary" disabled={busy || !chosen.length} onClick={add}>{`선택한 ${chosen.length}명 추가`}</button></div>}
      {!rows.length && !state.googleLoading && <p className="people-empty">불러온 참석자가 없습니다.</p>}
    </div>}
  </section>;
}
