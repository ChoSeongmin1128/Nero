import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { Plus, Pencil, Trash2, UserPlus, X, Search, Users, UserRound } from 'lucide-react';
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter } from '../../../design-system/components/ui/dialog.jsx';
import { Tabs } from '@base-ui/react/tabs';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '../../../design-system/components/ui/select.jsx';
import { attendeeInputAction, attendeeLabel, createAttendeeFlow, createPeopleCatalogFlow, filterPeople, personGroups, splitPeopleNames } from './people-flow.js';
import { ConfirmDialog } from '../../../design-system/components/ui/confirm-dialog.jsx';
import './people-ui.css';

function useConfirmation() {
  const [request, setRequest] = React.useState(null);
  const pending = React.useRef(null);
  const ask = React.useCallback((title, description, label = '삭제', destructive = true) => {
    if (pending.current) return Promise.resolve(false);
    return new Promise(resolve => { pending.current = resolve; setRequest({ title, description, label, destructive }); });
  }, []);
  const resolve = value => { const finish = pending.current; pending.current = null; setRequest(null); finish?.(value); };
  React.useEffect(() => () => pending.current?.(false), []);
  return { request, ask, resolve };
}

function Feedback({ state }) {
  return <>{state.busy && <p className="inline-feedback" role="status"><span className="activity-spinner" aria-hidden="true" /> 처리 중</p>}{state.error && <p className="notice" role="alert">{state.error}</p>}</>;
}

const composing = event => event.nativeEvent?.isComposing || event.isComposing || event.keyCode === 229;

function QuickAdd({ kind, group, state, flow }) {
  const hint = React.useId();
  const submit = value => {
    if (value !== undefined) flow.changeQuickInput(value);
    void flow.addMany(kind, group?.id);
  };
  const change = event => {
    const value = event.currentTarget.value;
    flow.changeQuickInput(value);
    if (!composing(event) && /[,\r\n]/.test(value) && splitPeopleNames(value).length) submit();
  };
  return <form className="people-quick-add" onSubmit={event => { event.preventDefault(); submit(); }}>
    <label className="type-label" htmlFor={`${hint}-input`}>{kind === 'group' ? '그룹 추가' : group ? `${group.name}에 사람 추가` : '사람 추가'}</label>
      <div className="people-quick-controls"><input id={`${hint}-input`} aria-label={kind === 'group' ? '그룹 이름 연속 추가' : '사람 이름 연속 추가'} aria-describedby={hint}
        value={state.quickInput} placeholder="이름 입력 후 Enter" readOnly={state.busy} autoComplete="off" spellCheck={false}
        onChange={change} onCompositionEnd={change} onKeyDown={event => { if (event.key === 'Enter' && composing(event)) event.preventDefault(); }}
        onPaste={event => {
          const pasted = event.clipboardData.getData('text');
          if (state.busy || !/[,\r\n]/.test(pasted)) return;
          event.preventDefault(); const input = event.currentTarget;
          submit(input.value.slice(0, input.selectionStart) + pasted + input.value.slice(input.selectionEnd));
        }} />
        <button type="submit" className="small with-icon" disabled={state.busy || !splitPeopleNames(state.quickInput).length}><Plus size={14} aria-hidden="true" />추가</button>
      </div>
    <p id={hint} className="people-input-hint">Enter 또는 쉼표로 연속 추가합니다. 여러 이름은 줄바꿈해서 붙여넣을 수 있습니다.</p>
  </form>;
}

function PeopleSettings({ flow, container, controls }) {
  const state = React.useSyncExternalStore(flow.subscribe, flow.getSnapshot);
  const confirmation = useConfirmation();
  const [tab, setTab] = React.useState('person'), [query, setQuery] = React.useState(''), [groupId, setGroupId] = React.useState(''), [memberQuery, setMemberQuery] = React.useState('');
  const beforeLeave = async () => {
    const current = flow.getSnapshot();
    if (current.writing || confirmation.request) return false;
    if (!current.draft && !current.quickInput.trim()) return true;
    if (!flow.isDirty()) { flow.discard(); return true; }
    if (!await confirmation.ask('수정 내용을 버릴까요?', '저장하지 않은 이름과 그룹 변경은 반영되지 않습니다.', '버리기')) return false;
    flow.discard(); return true;
  };
  controls.current = { beforeLeave, isConfirming: () => Boolean(confirmation.request) };
  const catalog = state.catalog, draft = state.draft;
  React.useEffect(() => setMemberQuery(''), [draft?.id]);
  const filter = groupId === 'ungrouped' || catalog?.groups.some(group => group.id === groupId) ? groupId : '';
  const selectedGroup = catalog?.groups.find(group => group.id === filter);
  const switchTab = async value => {
    if (value !== tab && await beforeLeave()) { setTab(value); setQuery(''); setGroupId(''); }
  };
  const edit = async (kind, item) => { if (await beforeLeave()) flow.edit(kind, item); };
  const remove = async (kind, item) => {
    const description = kind === 'person' ? '인원 목록에서 삭제합니다. 이전 회의의 참석자 정보는 유지됩니다.' : '그룹을 삭제합니다. 인원과 이전 회의는 유지됩니다.';
    if (await confirmation.ask(`${item.name} 삭제`, description)) await flow.remove(kind, item.id);
  };
  return <div className="people-settings" data-nero-ui-owned="" aria-busy={state.busy}>
    <h3 className="settings-panel-title">참석자</h3>
    <Tabs.Root value={tab} onValueChange={value => { void switchTab(value); }} className="people-tabs">
      <Tabs.List className="content-tabs" aria-label="참석자 관리"><Tabs.Tab value="person" disabled={state.writing}>사람 <span className="muted">{catalog?.people.length || 0}</span></Tabs.Tab><Tabs.Tab value="group" disabled={state.writing}>그룹 <span className="muted">{catalog?.groups.length || 0}</span></Tabs.Tab></Tabs.List>
      {['person', 'group'].map(kind => <Tabs.Panel key={kind} value={kind} className="people-tab-panel">
        {draft ? <form className="people-editor" onSubmit={event => { event.preventDefault(); void flow.save(); }}>
      <h4 className="type-label">{draft.kind === 'person' ? '사람' : '그룹'} 편집</h4>
      <label>이름<input autoFocus value={draft.name} maxLength={120} required disabled={state.busy} onChange={event => flow.change({ name: event.target.value })} /></label>
      {draft.kind === 'group' && <fieldset disabled={state.busy}><legend>그룹에 포함할 사람 <span className="muted">{draft.personIds.length}명 선택</span></legend>
        <label className="people-search"><Search size={15} aria-hidden="true" /><input type="search" aria-label="그룹에 포함할 사람 검색" placeholder="사람 검색" value={memberQuery} onChange={event => setMemberQuery(event.target.value)} /></label>
        <div className="people-member-list">{filterPeople(catalog, memberQuery).map(person => <label className="people-check" key={person.id}><input type="checkbox" checked={draft.personIds.includes(person.id)} onChange={event => flow.change({ personIds: event.target.checked ? [...draft.personIds, person.id] : draft.personIds.filter(id => id !== person.id) })} /><span>{person.name}</span></label>)}</div>
        {!catalog.people.length && <p className="muted">사람을 먼저 추가해 주세요.</p>}
        {catalog.people.length > 0 && !filterPeople(catalog, memberQuery).length && <p className="people-empty">검색 결과가 없습니다.</p>}
      </fieldset>}
      <div className="people-actions"><button type="button" disabled={state.busy} onClick={async () => { if (await beforeLeave()) flow.discard(); }}>취소</button><button type="submit" className="primary" disabled={state.busy || !draft.name.trim()}>저장</button></div>
    </form> : catalog && (() => {
      const items = kind === 'person' ? filterPeople(catalog, query, filter) : catalog.groups.filter(group => group.name.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase()));
      return <>
        <div className="people-list-tools"><label className="people-search"><Search size={15} aria-hidden="true" /><input type="search" aria-label={kind === 'person' ? '사람 검색' : '그룹 검색'} placeholder={kind === 'person' ? '이름 또는 소속 그룹 검색' : '그룹 검색'} value={query} disabled={state.busy} onChange={event => setQuery(event.target.value)} /></label>
          {kind === 'person' && <Select value={filter} onValueChange={setGroupId} disabled={state.busy}><SelectTrigger aria-label="소속 그룹 필터"><SelectValue>{filter === 'ungrouped' ? '그룹 없음' : selectedGroup?.name || '전체 그룹'}</SelectValue></SelectTrigger><SelectContent container={container}><SelectItem value="">전체 그룹</SelectItem><SelectItem value="ungrouped">그룹 없음</SelectItem>{catalog.groups.map(group => <SelectItem key={group.id} value={group.id}>{group.name}</SelectItem>)}</SelectContent></Select>}
        </div>
        <QuickAdd kind={kind} group={kind === 'person' ? selectedGroup : null} state={state} flow={flow} />
        <div className="people-list-count muted type-caption" role="status">{items.length}{kind === 'person' ? '명' : '개 그룹'}{query || filter ? ' 검색됨' : ''}</div>
        <div className="settings-group people-catalog-list">{items.length ? items.map(item => {
          const details = kind === 'person' ? personGroups(catalog, item.id).map(group => group.name).join(', ') || '그룹 없음' : catalog.people.filter(person => item.personIds.includes(person.id)).map(person => person.name).join(', ') || '등록된 사람이 없습니다';
          return <div className="people-row" key={item.id}><div className="people-row-copy"><span className="people-name">{item.name}{kind === 'group' && <span className="muted type-caption">{item.personIds.length}명</span>}</span><span className="people-memberships type-caption" title={details}>{details}</span></div><button type="button" className="small quiet icon" disabled={state.busy} aria-label={`${item.name} 편집`} title="편집" onClick={() => { void edit(kind, item); }}><Pencil size={15} aria-hidden="true" /></button><button type="button" className="small quiet icon danger" disabled={state.busy} aria-label={`${item.name} 삭제`} title="삭제" onClick={() => { void remove(kind, item); }}><Trash2 size={15} aria-hidden="true" /></button></div>;
        }) : <p className="people-empty">{query || filter ? '검색 결과가 없습니다.' : `등록된 ${kind === 'person' ? '사람이' : '그룹이'} 없습니다.`}</p>}</div>
      </>;
    })()}
      </Tabs.Panel>)}
    </Tabs.Root>
    <Feedback state={state} />
    {(!catalog || state.error) && !state.busy && <button type="button" onClick={async () => { if (await beforeLeave()) void flow.reload(); }}>다시 불러오기</button>}
    <ConfirmDialog request={confirmation.request && { ...confirmation.request, action: confirmation.request.label }} container={container} resolve={confirmation.resolve} />
  </div>;
}

export function mountPeopleSettings({ panel, container, bridge }) {
  const flow = createPeopleCatalogFlow({ bridge }), root = createRoot(panel), controls = { current: null };
  panel.dataset.neroUiOwned = '';
  root.render(<PeopleSettings flow={flow} container={container} controls={controls} />);
  return { refreshIfClean: () => flow.refreshIfClean(), show: () => flow.setActive(true), hide: () => flow.setActive(false), beforeLeave: () => controls.current?.beforeLeave() ?? true, isConfirming: () => controls.current?.isConfirming() ?? false, dispose() { flow.dispose(); root.unmount(); } };
}

function AttendeeChip({ item, label, busy, flow, onPromote }) {
  const [editing, setEditing] = React.useState(false);
  return <div className="attendee-chip">
    {editing ? <input autoFocus aria-label={`${label} 이름`} value={item.name || ''} placeholder={label} maxLength={120} disabled={busy}
      onChange={event => flow.rename(item.id, event.target.value)} onBlur={() => setEditing(false)}
      onKeyDown={event => { if (event.key === 'Enter' && !event.nativeEvent?.isComposing && !event.isComposing && event.keyCode !== 229) { event.preventDefault(); event.currentTarget.blur(); } }} />
      : <button type="button" className="quiet attendee-chip-name" title="이름 수정" disabled={busy} onClick={() => setEditing(true)}>{label}</button>}
    {!item.personId && <button type="button" className="quiet small icon" aria-label={`${label} 인원 목록에 저장`} title="인원 목록에 저장" disabled={busy} onClick={onPromote}><UserPlus size={14} aria-hidden="true" /></button>}
    <button type="button" className="quiet small icon" aria-label={`${label} 회의에서 제외`} title="회의에서 제외" disabled={busy} onClick={() => flow.remove(item.id)}><X size={14} aria-hidden="true" /></button>
  </div>;
}

function AttendeeDialog({ flow, container, origin, controls }) {
  const state = React.useSyncExternalStore(flow.subscribe, flow.getSnapshot);
  const confirmation = useConfirmation();
  const [query, setQuery] = React.useState(''), [promoting, setPromoting] = React.useState(null), [name, setName] = React.useState('');
  React.useEffect(() => { if (state.open) { setQuery(''); setPromoting(null); setName(''); } }, [state.open]);
  const close = async () => {
    if (!state.open) return true;
    if ((state.busy && !state.loading) || confirmation.request || promoting) return false;
    if (flow.isDirty() && !await confirmation.ask('참석자 변경을 버릴까요?', '저장하지 않은 참석자 변경은 반영되지 않습니다.', '버리기')) return false;
    return flow.close();
  };
  controls.current = { beforeLeave: close };
  const searchInput = React.useRef(null);
  const selected = new Set(state.attendees.map(item => item.personId).filter(Boolean));
  const matches = item => item.name.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase());
  const matchingGroups = state.catalog?.groups.filter(group => matches(group) && group.personIds.some(id => !selected.has(id))) || [];
  const matchingPeople = state.catalog?.people.filter(person => matches(person) && !selected.has(person.id)) || [];
  const add = people => { flow.add(people); setQuery(''); searchInput.current?.focus(); };
  const addTyped = value => { if (flow.addInput(value)) { setQuery(''); searchInput.current?.focus(); } };
  const addNew = () => { if (query.trim()) addTyped(query); else { flow.addTemporary(); setQuery(''); searchInput.current?.focus(); } };
  const typedName = query.trim();
  const inputAction = attendeeInputAction(state.catalog, state.attendees, query);
  const canAddName = inputAction?.kind === 'create';
  return <>
    <Dialog open={state.open} onOpenChange={open => { if (!open) void close(); }}>
      <DialogContent container={container} className="people-dialog" data-nero-ui-owned="" finalFocus={() => origin.current?.isConnected ? origin.current : false}>
        <DialogHeader><DialogTitle>참석자</DialogTitle><DialogDescription className="sr-only">회의에 참석한 사람을 선택하거나 임시 참석자를 추가합니다.</DialogDescription></DialogHeader>
        <div className="people-dialog-body" aria-busy={state.busy}>
          <Feedback state={state} />
          {state.error && !state.busy && <button type="button" className="small" onClick={async () => { if (!flow.isDirty() || await confirmation.ask('최신 내용을 불러올까요?', '불러오기에 성공하면 저장하지 않은 참석자 변경이 바뀝니다.', '불러오기', false)) await flow.reload(); }}>다시 불러오기</button>}
          {state.meeting && <>
            <section className="attendee-add" aria-label="참석자 추가">
              <label className="attendee-search"><Search size={16} aria-hidden="true" /><span className="sr-only">사람, 그룹 검색 또는 새 이름 입력</span><input ref={searchInput} type="search" value={query} placeholder="사람, 그룹 검색 또는 새 이름 입력"
                onChange={event => { const value = event.currentTarget.value; setQuery(value); if (!composing(event) && /[,\r\n]/.test(value)) addTyped(value); }}
                onCompositionEnd={event => { if (/[,\r\n]/.test(event.currentTarget.value)) addTyped(event.currentTarget.value); }}
                onPaste={event => { const value = event.clipboardData.getData('text'); if (/[,\r\n]/.test(value)) { event.preventDefault(); const input = event.currentTarget; addTyped(input.value.slice(0, input.selectionStart) + value + input.value.slice(input.selectionEnd)); } }}
                onKeyDown={event => { if (event.key === 'Enter' && !composing(event) && query.trim()) { event.preventDefault(); addTyped(query); } }} disabled={state.busy} /></label>
              <div className="attendee-options">
                {matchingGroups.length > 0 && <><h4 className="attendee-option-heading">그룹</h4>{matchingGroups.map(group => <button type="button" className="people-option" key={group.id} disabled={state.busy} onClick={() => add(state.catalog.people.filter(person => group.personIds.includes(person.id)))}><Users size={16} aria-hidden="true" /><span>{group.name}</span><span className="muted type-caption">{group.personIds.filter(id => !selected.has(id)).length}명 추가</span></button>)}</>}
                {matchingPeople.length > 0 && <><h4 className="attendee-option-heading">사람</h4>{matchingPeople.map(person => <button type="button" className="people-option" key={person.id} disabled={state.busy} aria-label={`${person.name} 추가`} onClick={() => add([person])}><UserRound size={16} aria-hidden="true" /><span className="people-option-copy">{person.name}<span className="people-memberships type-caption">{personGroups(state.catalog, person.id).map(group => group.name).join(', ')}</span></span><Plus size={15} aria-hidden="true" /></button>)}</>}
                {canAddName && <button type="button" className="people-option attendee-create" disabled={state.busy} onClick={addNew}><Plus size={16} aria-hidden="true" /><span>{typedName} 추가</span></button>}
                {!typedName && <button type="button" className="people-option attendee-create" disabled={state.busy} onClick={addNew}><Plus size={16} aria-hidden="true" /><span>이름 없이 추가</span></button>}
                {typedName && !matchingPeople.length && !matchingGroups.length && !canAddName && <p className="people-empty" role="status">이미 선택한 참석자입니다.</p>}
              </div>
            </section>
            <section className="people-section attendee-selected" aria-label="선택한 참석자"><h4 className="type-label">선택한 참석자 <span className="muted">{state.attendees.length}명</span></h4>
              <div className="attendee-chips">{state.attendees.map(item => <AttendeeChip key={item.id} item={item} label={attendeeLabel(item, state.attendees)} busy={state.busy} flow={flow} onPromote={() => { setName(item.name || ''); setPromoting(item.id); }} />)}{!state.attendees.length && <p className="people-empty">아직 선택한 참석자가 없습니다.</p>}</div>
            </section>
          </>}
        </div>
        <DialogFooter><button type="button" disabled={state.busy && !state.loading} onClick={() => { void close(); }}>취소</button><button type="button" className="primary" disabled={state.busy || !state.meeting} onClick={async () => { if (await flow.save()) flow.close(); }}>적용</button></DialogFooter>
      </DialogContent>
    </Dialog>
    <Dialog open={Boolean(promoting)} onOpenChange={open => { if (!open && !state.busy) setPromoting(null); }}>
      <DialogContent container={container} className="people-confirm" showCloseButton={false} data-nero-ui-owned="">
        <DialogHeader><DialogTitle>인원 목록에 추가</DialogTitle><DialogDescription>다른 회의에서도 선택할 수 있습니다.</DialogDescription></DialogHeader>
        <form className="people-editor" onSubmit={async event => { event.preventDefault(); if (await flow.promote(promoting, name)) setPromoting(null); }}><label>이름<input autoFocus value={name} required maxLength={120} disabled={state.busy} onChange={event => setName(event.target.value)} /></label><Feedback state={state} /><DialogFooter><button type="button" disabled={state.busy} onClick={() => setPromoting(null)}>취소</button><button type="submit" className="primary" disabled={state.busy || !name.trim()}>추가</button></DialogFooter></form>
      </DialogContent>
    </Dialog>
    <ConfirmDialog request={confirmation.request && { ...confirmation.request, action: confirmation.request.label }} container={container} resolve={confirmation.resolve} />
  </>;
}

export function mountMeetingAttendees({ container, bridge, onChanged }) {
  const mount = container.ownerDocument.createElement('div'); mount.dataset.neroUiOwned = ''; container.append(mount);
  const root = createRoot(mount), flow = createAttendeeFlow({ bridge, onChanged }), origin = { current: null }, controls = { current: null };
  root.render(<AttendeeDialog flow={flow} container={container} origin={origin} controls={controls} />);
  return { beforeLeave: () => controls.current?.beforeLeave() ?? true, open(id, trigger) { origin.current = trigger; globalThis.NeroControls?.close?.(); return flow.open(id); }, dispose() { flow.dispose(); root.unmount(); mount.remove(); } };
}
