import * as React from 'react';
import { ArrowUp, ArrowLeft, ArrowUpRight, Search, Square, Plus, X, FileText, RotateCcw, NotebookPen, Copy, Check, ChevronDown, ChevronLeft, ChevronRight, SquarePen, Trash2, MessageCircle, Pencil } from 'lucide-react';
import { conversationPath, questionVersions, sentQuestions } from './ai-chat-versions.js';
import { effectiveEffort, temporaryMessageId, usableAccount } from './ai-chat-flow.js';
import { Popover, PopoverTrigger, PopoverContent, PopoverTitle } from '../../../design-system/components/ui/popover.jsx';
import { brandIconUrl } from './brand-icons.js';
import { BrandIcon } from './brand-icon.jsx';
import { Dialog, DialogContent, DialogTitle, DialogDescription, DialogFooter, DialogClose } from '../../../design-system/components/ui/dialog.jsx';

const providers = [{ value: 'chatgpt', label: 'ChatGPT' }, { value: 'claude', label: 'Claude' }];
const effortNames = { none: 'None', minimal: 'Minimal', low: 'Low', medium: 'Medium', high: 'High', xhigh: 'Extra High', max: 'Max' };
const effortName = level => effortNames[level] || level;
const ProviderIcon = ({ provider }) => <img src={brandIconUrl(provider)} alt="" aria-hidden="true" className="brand-icon" />;
function ModelOption({ item, selected, provider, onChoose }) {
  return <button type="button" className="quiet ai-model-option" aria-pressed={selected} onClick={onChoose}>{provider && <ProviderIcon provider={provider} />}<span>{item.label}</span>{selected && <Check size={14} aria-hidden="true" />}</button>;
}
function ModelPicker({ flow, state, busy, onSettings }) {
  const [open, setOpen] = React.useState(false), [view, setView] = React.useState(null), [query, setQuery] = React.useState(''), [older, setOlder] = React.useState(false);
  const trigger = React.useRef(null), body = React.useRef(null), moved = React.useRef(false);
  const model = state.models.find(item => item.id === state.modelId);
  React.useEffect(() => { if (!state.active || busy) setOpen(false); }, [state.active, busy]);
  React.useEffect(() => { if (moved.current) { moved.current = false; body.current?.querySelector('button')?.focus(); } }, [view]);
  const groups = providers.map(provider => ({ ...provider, ...flow.modelsFor(provider.value) })).filter(group => group.available);
  const selected = (group, item) => state.accountId === group.account?.id && item.id === state.modelId;
  const choose = (group, item) => { flow.chooseModel(group.value, group.account.id, item.id); setOpen(false); };
  const go = next => { moved.current = true; setOlder(false); setView(next); };
  const needle = query.trim().toLocaleLowerCase();
  const matches = needle ? groups.flatMap(group => group.models.filter(item => item.label.toLocaleLowerCase().includes(needle) || item.id.toLocaleLowerCase().includes(needle)).map(item => ({ group, item }))) : [];
  const group = view && groups.find(item => item.value === view);
  const current = group ? group.models.filter(item => !item.previous) : [], previous = group ? group.models.filter(item => item.previous) : [];
  const showOlder = older || previous.some(item => selected(group, item));
  let content;
  if (needle) content = <div className="ai-model-list" ref={body}>{matches.map(({ group, item }) => <ModelOption key={`${group.value}:${item.id}`} item={item} provider={group.value} selected={selected(group, item)} onChoose={() => choose(group, item)} />)}{!matches.length && <p className="ai-chat-muted ai-model-note">일치하는 모델이 없습니다.</p>}</div>;
  else if (group) content = <div ref={body}>
    <button type="button" className="quiet ai-model-option ai-model-back" onClick={() => go(null)} aria-label={`${group.label} 모델 목록 닫기`}><ChevronLeft size={14} aria-hidden="true" /><ProviderIcon provider={group.value} /><span>{group.label}</span></button>
    <div className="ai-model-divider" role="separator" />
    <div className="ai-model-list">
      {current.map(item => <ModelOption key={item.id} item={item} selected={selected(group, item)} onChoose={() => choose(group, item)} />)}
      {previous.length > 0 && <button type="button" className="quiet ai-model-option ai-model-older" aria-expanded={showOlder} onClick={() => setOlder(!showOlder)}><span>이전 모델</span>{showOlder ? <ChevronDown size={14} aria-hidden="true" /> : <ChevronRight size={14} aria-hidden="true" />}</button>}
      {showOlder && previous.map(item => <ModelOption key={item.id} item={item} selected={selected(group, item)} onChoose={() => choose(group, item)} />)}
      {group.busy && <p className="ai-chat-muted ai-model-note" role="status">모델을 불러오는 중</p>}
      {group.error && <p className="ai-chat-error ai-model-note" role="alert">{group.error} <button type="button" className="quiet small" onClick={() => void flow.refreshModelGroup(group.value)}>다시 시도</button></p>}
      {!group.models.length && !group.busy && !group.error && <p className="ai-chat-muted ai-model-note">{state.connectionPhases[group.value] ? '연결을 마치면 모델 목록을 불러옵니다.' : '사용할 수 있는 모델이 없습니다.'}</p>}
    </div>
  </div>;
  else content = <div ref={body}>
    {groups.map(item => <button type="button" className="quiet ai-model-option ai-model-provider" key={item.value} data-provider={item.value} onClick={() => go(item.value)}><ProviderIcon provider={item.value} /><span>{item.label}</span><ChevronRight size={14} aria-hidden="true" /></button>)}
    {!groups.length && <p className="ai-chat-muted ai-model-note">연결된 AI 계정이 없습니다.</p>}
    <div className="ai-model-divider" role="separator" />
    <button type="button" className="quiet ai-model-option" onClick={() => { setOpen(false); onSettings(); }}><span>계정 설정</span><ArrowUpRight size={14} aria-hidden="true" /></button>
  </div>;
  return <Popover open={state.active && open} onOpenChange={next => { setOpen(next); if (next) { setView(null); setQuery(''); setOlder(false); flow.ensureModels(); } }}>
    <PopoverTrigger ref={trigger} type="button" className="quiet ai-model-trigger" disabled={busy} aria-label="AI 모델 선택">{model && <ProviderIcon provider={state.provider} />}<span>{model?.label || '모델 선택'}</span><ChevronDown size={13} aria-hidden="true" /></PopoverTrigger>
    <PopoverContent className="ai-model-popover" align="start" side="top" finalFocus={state.active && !busy ? trigger : false}>
      <PopoverTitle className="sr-only">AI 모델 선택</PopoverTitle>
      {groups.some(item => item.models.length) && <label className="ai-model-search"><Search size={14} aria-hidden="true" /><span className="sr-only">모델 검색</span><input type="search" placeholder="모델 검색" autoComplete="off" spellCheck={false} value={query} onChange={event => setQuery(event.target.value)} /></label>}
      {content}
    </PopoverContent>
  </Popover>;
}
function EffortPicker({ flow, state, busy }) {
  const [open, setOpen] = React.useState(false), trigger = React.useRef(null);
  const model = state.models.find(item => item.id === state.modelId), effort = model?.effort;
  React.useEffect(() => { if (!state.active || busy) setOpen(false); }, [state.active, busy]);
  if (!model) return null;
  if (effort?.supported === false) return <span className="ai-chat-muted">추론 설정 미지원</span>;
  if (effort?.supported !== true || !effort.levels?.length) return <span className="ai-chat-muted">추론 설정 확인 불가</span>;
  const value = effectiveEffort(state.models, state.modelId, state.effort);
  return <Popover open={state.active && open} onOpenChange={setOpen}>
    <PopoverTrigger ref={trigger} type="button" className="quiet ai-effort-trigger" disabled={busy} title="추론 수준"><span className="sr-only">추론 수준 </span><span>{value ? effortName(value) : '추론 수준 선택'}</span><ChevronDown size={13} aria-hidden="true" /></PopoverTrigger>
    <PopoverContent className="ai-model-popover ai-effort-popover" align="start" side="top" finalFocus={state.active && !busy ? trigger : false}>
      <PopoverTitle className="sr-only">추론 수준</PopoverTitle>
      {effort.levels.map(level => <button type="button" className="quiet ai-model-option" key={level} aria-pressed={level === value} onClick={() => { flow.selectEffort(level); setOpen(false); }}><span>{effortName(level)}</span>{level === value && <Check size={14} aria-hidden="true" />}</button>)}
    </PopoverContent>
  </Popover>;
}
function ConversationHistory({ flow, state }) {
  const [open, setOpen] = React.useState(false), [deleting, setDeleting] = React.useState(null);
  const [scope, setScope] = React.useState({ meetingId: state.meeting?.id || null, filter: state.meeting ? 'meeting' : 'all' });
  const meetingId = state.meeting?.id || null;
  const filter = scope.meetingId === meetingId ? scope.filter : meetingId ? 'meeting' : 'all';
  const items = (state.conversationList || []).filter(item => filter === 'all' || item.meetingIds?.includes(meetingId));
  React.useEffect(() => { setScope({ meetingId, filter: meetingId ? 'meeting' : 'all' }); }, [meetingId]);
  React.useEffect(() => { if (!state.active) { setOpen(false); setDeleting(null); } }, [state.active]);
  return <><Popover open={state.active && open} onOpenChange={next => { setOpen(next); if (next) void flow.loadList(true); }}>
    <PopoverTrigger className="quiet ai-chat-history-trigger" aria-label="대화 기록"><span>{state.conversation.title || '새 대화'}</span><ChevronDown size={14} aria-hidden="true" /></PopoverTrigger>
    <PopoverContent className="ai-history-popover" align="start" finalFocus={state.active ? undefined : false}>
      <PopoverTitle className="ai-popover-title">대화 기록</PopoverTitle>
      {meetingId && <div className="ai-source-kinds" aria-label="대화 범위">{[['meeting', '이 회의의 대화'], ['all', '전체 대화']].map(([value, label]) => <button type="button" className="quiet small" key={value} aria-pressed={filter === value} onClick={() => setScope({ meetingId, filter: value })}>{label}</button>)}</div>}
      {state.listBusy && <p className="ai-chat-muted" role="status">불러오는 중</p>}
      {state.listError && <p className="ai-chat-error" role="alert">{state.listError}</p>}
      <div className="ai-history-list">{items.map(item => <div className="ai-history-row" key={item.id}>
        <button type="button" className="quiet" aria-current={state.conversation.id === item.id ? 'true' : undefined} onClick={() => { void flow.selectConversation(item.id); setOpen(false); }}><MessageCircle size={15} aria-hidden="true" /><span>{item.title || '새 대화'}</span>{item.activeRequestId && <span className="activity-spinner" aria-label="답변 중" />}</button>
        <button type="button" className="quiet small icon" aria-label={`${item.title || '대화'} 휴지통으로 이동`} title="휴지통으로 이동" onClick={() => setDeleting(item)}><Trash2 size={15} aria-hidden="true" /></button>
      </div>)}</div>
      {!state.listBusy && !state.listError && !items.length && <p className="ai-chat-muted">{filter === 'meeting' ? '이 회의로 질문한 대화가 없습니다.' : '저장된 대화가 없습니다.'}</p>}
    </PopoverContent>
  </Popover><Dialog open={Boolean(deleting) && state.active} onOpenChange={next => { if (!next) setDeleting(null); }}>
    <DialogContent className="ai-conversation-confirm" data-nero-ui-owned="" showCloseButton={false}><DialogTitle>대화를 휴지통으로 옮길까요?</DialogTitle><DialogDescription>AI 대화 목록의 휴지통에서 복원할 수 있습니다.</DialogDescription><DialogFooter><DialogClose className="quiet small">취소</DialogClose><button type="button" className="small danger" onClick={async () => { if (await flow.removeConversation(deleting.id)) setDeleting(null); }}>휴지통으로 이동</button></DialogFooter></DialogContent>
  </Dialog></>;
}
function Question({ flow, conversation, message, busy, catalog, onMeeting }) {
  const [editing, setEditing] = React.useState(false), [text, setText] = React.useState('');
  const typed = message.text || '';
  const skill = message.skillId ? catalog?.skills?.find(item => item.id === message.skillId)?.name || '회의 요약 및 저장' : '';
  const template = message.template?.name || catalog?.templates?.find(item => item.id === message.templateId)?.name || '';
  const input = React.useRef(null);
  const { siblings, position } = questionVersions(conversation, message.id);
  const locked = busy || Boolean(conversation.activeRequestId || conversation.action) || temporaryMessageId(message.id);
  React.useEffect(() => { if (editing) { input.current?.focus(); input.current?.setSelectionRange(text.length, text.length); } }, [editing]);
  React.useEffect(() => { setEditing(false); }, [conversation.key]);
  const changed = Boolean(text.trim()) && text.trim() !== typed.trim();
  const submit = async () => { if (!locked && changed && await flow.editQuestion(message.id, text)) setEditing(false); };
  if (editing) return <form className="ai-question-edit" onSubmit={event => { event.preventDefault(); void submit(); }}>
    <textarea ref={input} className="text-input" aria-label="질문 수정" value={text} rows={3} onChange={event => setText(event.target.value)} onKeyDown={event => { if (event.nativeEvent.isComposing || event.isComposing) return; if (event.key === 'Escape') { event.preventDefault(); setEditing(false); } else if (event.key === 'Enter' && !event.shiftKey) { event.preventDefault(); void submit(); } }} />
    <div className="ai-question-edit-actions"><button type="button" className="quiet small" onClick={() => setEditing(false)}>취소</button><button type="submit" className="small primary" disabled={locked || !changed}>다시 보내기</button></div>
  </form>;
  return <>
    {(message.attachments?.length > 0 || skill || template) && <div className="ai-message-attachments" aria-label="전송한 자료">
      {(message.attachments || []).map((item, index) => item.kind === 'meeting' && item.meetingId && onMeeting ? <button type="button" className="quiet ai-message-meeting" key={item.meetingId} title={`${item.title || '회의'} 열기`} onClick={() => { void onMeeting(item.meetingId); }}><FileText size={12} aria-hidden="true" /><span>{item.title}</span></button> : <span key={item.meetingId || item.sourceId || index}>{item.kind === 'notion' ? <BrandIcon name="notion" /> : <FileText size={12} aria-hidden="true" />}{item.title}</span>)}
      {(skill || template) && <span className="ai-message-skill">{skill && <span>{skill}</span>}{template && <span className="ai-message-template">{template}</span>}</span>}
    </div>}
    {typed && <div className="ai-chat-text">{typed}</div>}
    <div className="ai-question-actions">
      {siblings.length > 1 && <span className="ai-question-versions" role="group" aria-label="질문 버전"><button type="button" className="quiet small icon" aria-label="이전 버전" disabled={locked || position < 1} onClick={() => { void flow.selectVersion(siblings[position - 1].id); }}><ChevronLeft size={14} aria-hidden="true" /></button><span aria-live="polite">{position + 1}/{siblings.length}</span><button type="button" className="quiet small icon" aria-label="다음 버전" disabled={locked || position >= siblings.length - 1} onClick={() => { void flow.selectVersion(siblings[position + 1].id); }}><ChevronRight size={14} aria-hidden="true" /></button></span>}
      <button type="button" className="quiet small icon" aria-label="질문 수정" title="질문 수정" disabled={locked} onClick={() => { setText(typed); setEditing(true); }}><Pencil size={13} aria-hidden="true" /></button>
    </div>
  </>;
}
function SourcePicker({ flow, state, onSettings, getMeetings }) {
  const [open, setOpen] = React.useState(false), [kind, setKind] = React.useState('meeting'), [query, setQuery] = React.useState(''), [link, setLink] = React.useState('');
  const input = React.useRef(null), owner = state.conversation.id;
  React.useEffect(() => { setOpen(false); }, [state.active, owner]);
  const busy = Boolean(state.conversation.activeRequestId || state.conversation.action);
  const meetings = getMeetings().filter(meeting => !meeting.deletedAt && meeting.title?.toLowerCase().includes(query.toLowerCase()));
  return <Popover open={open && state.active} onOpenChange={next => { setOpen(next); if (next) void flow.loadNotion(); }}>
    <PopoverTrigger type="button" className="quiet small icon" disabled={busy} aria-label="자료 추가" title="자료 추가"><Plus size={18} aria-hidden="true" /></PopoverTrigger>
    <PopoverContent className="ai-source-popover" align="start" side="top" initialFocus={input} finalFocus={state.active ? undefined : false}>
      <PopoverTitle className="sr-only">자료 추가</PopoverTitle>
      <div className="ai-source-kinds" role="group" aria-label="자료 종류"><button type="button" className="quiet small" aria-pressed={kind === 'meeting'} onClick={() => setKind('meeting')}>회의</button><button type="button" className="quiet small" aria-pressed={kind === 'skill'} onClick={() => { setKind('skill'); void flow.loadCatalog(); }}>스킬</button><button type="button" className="quiet small" aria-pressed={kind === 'notion'} onClick={() => setKind('notion')}><BrandIcon name="notion" />Notion 링크</button></div>
      {kind === 'skill' ? <div className="ai-attachment-list">{(state.catalog.skills || []).map(item => <button type="button" className="quiet ai-attachment-option" key={`skill:${item.id}`} onClick={() => { flow.selectSkill(item.id); setOpen(false); }}>{item.name}</button>)}{!state.catalog.skills?.length && <p className="ai-chat-muted">{state.catalogError || '불러오는 중'}</p>}</div> : kind === 'meeting' ? <><input ref={input} type="search" value={query} placeholder="회의 검색" aria-label="첨부할 회의 검색" onChange={event => setQuery(event.target.value)} /><div className="ai-attachment-list">{meetings.map(meeting => {
        const selected = state.conversation.meetings.some(item => item.id === meeting.id);
        return <button type="button" className="quiet ai-attachment-option" key={meeting.id} disabled={selected} onClick={() => { flow.attachMeeting(meeting); setOpen(false); }}><FileText size={16} aria-hidden="true" /><span>{meeting.title || '새 회의'}</span>{selected && <Check size={15} aria-hidden="true" />}</button>;
      })}{!meetings.length && <p className="ai-chat-muted">회의가 없습니다.</p>}</div></> : <form onSubmit={async event => { event.preventDefault(); if (await flow.addSource(link)) { setLink(''); setOpen(false); } }}>
        <input ref={input} id="ai-source-link" type="url" value={link} aria-label="Notion 페이지 링크" placeholder="Notion 페이지 링크" onChange={event => setLink(event.target.value)} disabled={state.fetching} required />
        {state.notion && !state.notion.connected && <button type="button" className="small" onClick={() => { setOpen(false); onSettings(); }}>Notion 연결</button>}
        {state.sourceError && <p role="alert" className="ai-chat-error">{state.sourceError}</p>}
        <div className="ai-source-actions">{state.fetching && <button type="button" className="small" onClick={() => { void flow.cancelSource(); }}>취소</button>}<button type="submit" className="small primary" disabled={state.fetching || !link.trim() || !state.notion?.connected}>{state.fetching ? '불러오는 중' : '추가'}</button></div>
      </form>}
    </PopoverContent>
  </Popover>;
}
function AnswerCopy({ flow, state, message }) {
  const [open, setOpen] = React.useState(false), owner = state.conversation.id;
  React.useEffect(() => { setOpen(false); }, [state.active, owner]);
  return <Popover open={open && state.active} onOpenChange={setOpen}>
    <PopoverTrigger type="button" className="quiet small icon" disabled={Boolean(state.conversation.action)} aria-label="AI 답변 복사" title="복사">{state.conversation.copiedMessageId === message.id ? <Check size={16} aria-hidden="true" /> : <Copy size={16} aria-hidden="true" />}</PopoverTrigger>
    <PopoverContent align="start" side="top" className="nero-action-popover" data-nero-ui-owned="" finalFocus={state.active ? undefined : false}><PopoverTitle className="sr-only">AI 답변 복사</PopoverTitle>{[['txt', '텍스트로 복사'], ['md', '마크다운으로 복사']].map(([format, label]) => <button key={format} type="button" className="quiet small" disabled={Boolean(state.conversation.action)} onClick={async () => { if (await flow.copy(message.id, format)) setOpen(false); }}>{label}</button>)}</PopoverContent>
  </Popover>;
}
function SaveAnswer({ flow, state, message, kind }) {
  const [open, setOpen] = React.useState(false), targets = (message.saveTargets || []).filter(target => kind !== 'summary' || target.canSaveSummary === true), label = kind === 'summary' ? '요약 저장' : '메모 추가';
  React.useEffect(() => { setOpen(false); }, [state.active, state.conversation.id]);
  if (!targets.length) return null;
  const icon = kind === 'summary' ? <FileText size={16} aria-hidden="true" /> : <NotebookPen size={16} aria-hidden="true" />;
  const disabled = Boolean(state.conversation.action);
  const save = async id => { if (await flow.save(kind, message.id, id)) setOpen(false); };
  if (targets.length === 1) return <button type="button" className="quiet small icon" disabled={disabled} aria-label={label} title={label} onClick={() => { void save(targets[0].meetingId); }}>{icon}</button>;
  return <Popover open={open && state.active} onOpenChange={setOpen}><PopoverTrigger type="button" className="quiet small icon" disabled={disabled} aria-label={label} title={label}>{icon}</PopoverTrigger><PopoverContent side="top" className="nero-action-popover" finalFocus={state.active ? undefined : false}><PopoverTitle className="ai-popover-title">저장할 회의</PopoverTitle>{targets.map(target => <button type="button" key={target.meetingId} className="quiet small" disabled={disabled} onClick={() => { void save(target.meetingId); }}>{target.title}</button>)}</PopoverContent></Popover>;
}
export function GlobalAiChat({ flow, onSettings = () => {}, onCitation = async () => {}, onClose = () => {}, getMeetings = () => [], onOpenMeeting = null }) {
  const state = React.useSyncExternalStore(flow.subscribe, flow.getSnapshot), conversation = state.conversation;
  const account = state.accounts.find(item => item.id === state.accountId && item.provider === state.provider);
  const busy = Boolean(conversation.activeRequestId || conversation.action);
  const transcript = React.useRef(null), pinned = React.useRef(true), composer = React.useRef(null);
  const [citationError, setCitationError] = React.useState('');
  const [chooserOpen, setChooserOpen] = React.useState(false), [chooserIndex, setChooserIndex] = React.useState(0);
  const slash = conversation.draft.match(/(?:^|\s)\/([^\s/]*)$/);
  const choices = (chooserOpen === 'template' ? (state.catalog.templates || []).map(item => ({ ...item, kind: 'template' })) : (state.catalog.skills || []).map(item => ({ ...item, kind: 'skill' }))).filter(item => [item.name, ...(item.aliases || [])].some(value => value.toLowerCase().includes((chooserOpen === 'template' ? '' : slash?.[1] || '').toLowerCase())));
  const choosing = Boolean(chooserOpen && (slash || chooserOpen === 'template') && choices.length);
  const choose = item => { if (item.kind === 'skill') flow.selectSkill(item.id); else flow.selectTemplate(item.id); if (slash && chooserOpen !== 'template') flow.edit(conversation.draft.slice(0, slash.index) + (conversation.draft[slash.index] === ' ' ? ' ' : '')); setChooserOpen(false); composer.current?.focus(); };
  React.useEffect(() => { if (conversation.skillId && !conversation.templateId) void flow.ensureSkillTemplate(); }, [conversation.skillId, conversation.templateId]);
  React.useEffect(() => { setChooserIndex(0); }, [conversation.draft]);
  React.useLayoutEffect(() => { pinned.current = true; if (transcript.current) transcript.current.scrollTop = transcript.current.scrollHeight; }, [conversation.id]);
  React.useLayoutEffect(() => { if (pinned.current && transcript.current) transcript.current.scrollTop = transcript.current.scrollHeight; }, [conversation.messages]);
  React.useLayoutEffect(() => { const node = composer.current; if (node) { node.style.height = 'auto'; node.style.height = `${Math.min(180, Math.max(64, node.scrollHeight))}px`; } }, [conversation.draft]);
  const reveal = async citation => { try { setCitationError(''); await onCitation(citation); } catch (error) { setCitationError(error?.message || '자료를 열지 못했습니다.'); } };
  const unavailable = account?.state === 'unavailable' || state.accountsLoaded && !account;
  const shown = conversationPath(conversation);
  // Up/Down walk through this conversation's sent questions, like a shell history. The
  // unsent draft comes back when walking past the newest question.
  const recall = React.useRef(null);
  React.useEffect(() => { recall.current = null; }, [conversation.key]);
  const walk = event => {
    const node = event.currentTarget, history = sentQuestions(conversation);
    if (event.key !== 'ArrowUp' && event.key !== 'ArrowDown' || event.shiftKey || event.altKey || event.metaKey || event.ctrlKey || node.selectionStart !== node.selectionEnd || !history.length) return false;
    const current = recall.current;
    if (event.key === 'ArrowUp') {
      if (current ? node.value.lastIndexOf('\n', node.selectionStart - 1) !== -1 : node.value !== '') return false;
      const index = (current?.index ?? history.length) - 1;
      if (index < 0) return false;
      recall.current = { index, draft: current ? current.draft : node.value };
      flow.edit(history[index]);
    } else {
      if (!current || node.value.indexOf('\n', node.selectionEnd) !== -1) return false;
      const index = current.index + 1;
      if (index < history.length) { recall.current = { ...current, index }; flow.edit(history[index]); }
      else { recall.current = null; flow.edit(current.draft); }
    }
    event.preventDefault();
    return true;
  };
  return <div className="ai-chat" data-nero-ui-owned="">
    <header className="ai-chat-header"><ConversationHistory flow={flow} state={state} /><button type="button" className="quiet small icon" aria-label="새 대화" title="새 대화" onClick={() => { void flow.newConversation({ attachCurrent: true }); }}><SquarePen size={17} aria-hidden="true" /></button><button type="button" className="quiet small icon" aria-label="AI 닫기" title="AI 닫기" onClick={onClose}><X size={18} aria-hidden="true" /></button></header>
    <div ref={transcript} className="ai-chat-messages" role="log" aria-label="AI 대화" aria-live="polite" aria-relevant="additions" onScroll={event => { const node = event.currentTarget; pinned.current = node.scrollHeight - node.scrollTop - node.clientHeight < 48; }}>
      {conversation.loading ? <p className="ai-chat-muted" role="status">대화를 불러오는 중</p> : !shown.length && <div className="ai-chat-empty"><MessageCircle size={28} strokeWidth={1.3} aria-hidden="true" /><h2>무엇을 도와드릴까요?</h2>{Boolean(conversation.meetings.length) && <button type="button" className="quiet small" onClick={() => { flow.summaryPrompt(); composer.current?.focus(); }}>결정 사항과 할 일 정리</button>}</div>}
      {shown.map(message => <article key={message.id} className={`ai-chat-message ai-chat-message--${message.role}`} aria-label={message.role === 'user' ? '내 질문' : 'AI 답변'}>
        {message.role === 'user' && <Question flow={flow} conversation={conversation} message={message} busy={busy} catalog={state.catalog} onMeeting={onOpenMeeting} />}
        {message.role === 'assistant' && message.text && (globalThis.NeroUI?.summaryDocument ? <div className="ai-chat-markdown" dangerouslySetInnerHTML={{ __html: globalThis.NeroUI.summaryDocument(message.text, { streaming: message.status === 'streaming', citable: (message.citations || []).map(item => item.reference) }) }} onClick={event => { const control = event.target.closest('[data-summary-citations]'); if (!control) return; const refs = control.dataset.summaryCitations.split(',').map(Number); const citation = message.citations?.find(item => refs.includes(item.reference)); if (citation) void reveal(citation); }} /> : <div className="ai-chat-text">{message.text}</div>)}
        {message.role === 'assistant' && message.status === 'completed' && message.actualModelId && <p className="ai-chat-muted">모델: {message.actualModelId}</p>}
        {message.status === 'streaming' && <span className="ai-chat-message-status" role="status"><span className="activity-spinner" aria-hidden="true" /><span className="sr-only">답변 중</span></span>}
        {['cancelled', 'interrupted'].includes(message.status) && <p className="ai-chat-message-status">{message.status === 'cancelled' ? '중지됨' : '답변이 중단되었습니다.'}</p>}
        {message.status === 'failed' && <p className="ai-chat-error">{message.error || '답변을 받지 못했습니다.'}</p>}
        {message.role === 'assistant' && message.status === 'completed' && message.text.trim() && <div className="ai-chat-message-actions"><AnswerCopy flow={flow} state={state} message={message} /><SaveAnswer flow={flow} state={state} message={message} kind="summary" /><SaveAnswer flow={flow} state={state} message={message} kind="note" /></div>}
        {message.role === 'assistant' && ['failed', 'cancelled', 'interrupted'].includes(message.status) && <button type="button" className="quiet small with-icon" disabled={busy || !usableAccount(account)} onClick={() => { void flow.retry(message.id); }}><RotateCcw size={14} aria-hidden="true" />다시 시도</button>}
      </article>)}
    </div>
    <div className="ai-chat-bottom">
      {(citationError || state.accountsError || state.modelError || conversation.error) && <p className="ai-chat-error" role="alert">{citationError || state.accountsError || state.modelError || conversation.error}</p>}
      {conversation.feedback && <p className="ai-chat-feedback" role="status">{conversation.feedback}</p>}
      {!state.accountsBusy && !usableAccount(account) && <div className="ai-chat-connect"><span>{unavailable ? `${state.provider === 'claude' ? 'Claude' : 'ChatGPT'}를 사용할 수 없습니다.` : 'AI 계정에 로그인해 주세요.'}</span><button type="button" className="small" onClick={onSettings}>계정 설정</button></div>}
      <form className="ai-chat-composer" onSubmit={event => { event.preventDefault(); if (choosing) choose(choices[chooserIndex] || choices[0]); else void flow.send(); }}>
        {(conversation.meetings.length > 0 || conversation.sources.length > 0) && <div className="ai-chat-sources" aria-label="포함된 자료">
          {conversation.meetings.map(meeting => <span className="ai-source-chip" key={meeting.id}><FileText size={14} aria-hidden="true" /><span title={meeting.title}>{meeting.title || '회의'}</span><button type="button" className="quiet small icon" aria-label={`${meeting.title || '회의'} 제외`} disabled={busy} onClick={() => flow.removeMeeting(meeting.id)}><X size={12} aria-hidden="true" /></button></span>)}
          {conversation.sources.map(source => <span className="ai-source-chip" key={source.sourceId}><BrandIcon name="notion" /><span title={source.title || source.url}>{source.title || 'Notion 자료'}</span><button type="button" className="quiet small icon" aria-label={`${source.title || 'Notion 자료'} 제외`} disabled={busy} onClick={() => flow.removeSource(source.sourceId)}><X size={12} aria-hidden="true" /></button></span>)}
        </div>}
        {conversation.skillId && <div className="ai-chat-sources">
          <span className="ai-source-chip ai-skill-chip"><span>{state.catalog.skills?.find(item => item.id === conversation.skillId)?.name || '회의 요약 및 저장'}</span>
            <button type="button" className="quiet small ai-skill-template" aria-label={`요약 템플릿: ${state.catalog.templates?.find(item => item.id === conversation.templateId)?.name || '선택 안 함'}`} aria-haspopup="listbox" aria-expanded={chooserOpen === 'template'} onClick={() => { setChooserOpen(chooserOpen === 'template' ? false : 'template'); void flow.loadCatalog(); composer.current?.focus(); }}><span>{state.catalog.templates?.find(item => item.id === conversation.templateId)?.name || '템플릿 선택'}</span><ChevronDown size={12} aria-hidden="true" /></button>
            <button type="button" className="quiet small icon" aria-label="스킬 제거" onClick={() => flow.selectSkill(null)}><X size={12} /></button></span>
          {conversation.meetings.length !== 1 && <span className="ai-chat-muted">요약할 회의 하나만 첨부해 주세요.</span>}
        </div>}
        {choosing && <div className="ai-command-list" role="listbox" aria-label="스킬 및 템플릿">{choices.map((item, index) => <button type="button" role="option" aria-selected={index === chooserIndex} key={`${item.kind}:${item.id}`} onMouseDown={event => event.preventDefault()} onClick={() => choose(item)}><span>{item.name}</span><small>{item.kind === 'skill' ? '스킬' : '템플릿'}</small></button>)}</div>}
        <textarea ref={composer} className="text-input" aria-label="AI에게 질문" placeholder="무엇이든 물어보세요" value={conversation.draft} rows={2} onChange={event => { const value = event.target.value; recall.current = null; flow.edit(value); if (/(?:^|\s)\/[^\s/]*$/.test(value)) { setChooserOpen('slash'); void flow.loadCatalog(); } else setChooserOpen(false); }} onKeyDown={event => { if (!event.nativeEvent.isComposing && !event.isComposing && chooserOpen && (slash || chooserOpen === 'template') && choices.length) { if (event.key === 'Escape') { event.preventDefault(); setChooserOpen(false); return; } if (event.key === 'ArrowDown' || event.key === 'ArrowUp') { event.preventDefault(); setChooserIndex((chooserIndex + (event.key === 'ArrowDown' ? 1 : choices.length - 1)) % choices.length); return; } if (event.key === 'Enter' || event.key === 'Tab') { event.preventDefault(); choose(choices[chooserIndex] || choices[0]); return; } } if (!event.nativeEvent.isComposing && !event.isComposing && walk(event)) return; if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing && !event.isComposing) { event.preventDefault(); if (flow.canSend()) void flow.send(); } }} />
        <div className="ai-chat-compose-actions"><SourcePicker flow={flow} state={state} onSettings={onSettings} getMeetings={getMeetings} /><span className="ai-chat-compose-spacer" />{conversation.contextBusy && <span className="activity-spinner" role="status" aria-label="자료 불러오는 중" />}{conversation.activeRequestId ? <button type="button" className="small primary icon" aria-label="답변 중지" title="답변 중지" disabled={conversation.stopping} onClick={() => { void flow.stop(); }}><Square size={14} aria-hidden="true" /></button> : <button type="submit" className="primary small icon" aria-label="질문 보내기" title="질문 보내기" disabled={busy || (!choosing && !flow.canSend())}><ArrowUp size={17} aria-hidden="true" /></button>}</div>
      </form>
      <div className="ai-chat-models"><ModelPicker flow={flow} state={state} busy={busy} onSettings={onSettings} /><EffortPicker flow={flow} state={state} busy={busy} /></div>
    </div>
  </div>;
}
