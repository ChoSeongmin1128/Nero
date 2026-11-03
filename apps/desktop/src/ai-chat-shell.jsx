import * as React from 'react';
import { ArrowUp, ArrowLeft, Bot, Square, Plus, X, FileText, RotateCcw, NotebookPen, Copy, Check, ChevronDown, ChevronRight, SquarePen, Trash2, MessageCircle, Link2 } from 'lucide-react';
import { Popover, PopoverTrigger, PopoverContent, PopoverTitle } from '../../../design-system/components/ui/popover.jsx';
import { brandIconUrl } from './brand-icons.js';
import { Dialog, DialogContent, DialogTitle, DialogFooter, DialogClose } from '../../../design-system/components/ui/dialog.jsx';

const providers = [{ value: 'chatgpt', label: 'ChatGPT' }, { value: 'claude', label: 'Claude' }];
function ModelPicker({ flow, state, busy, onSettings }) {
  const [open, setOpen] = React.useState(false), trigger = React.useRef(null);
  const model = state.models.find(item => item.id === state.modelId);
  React.useEffect(() => { if (!state.active || busy) setOpen(false); }, [state.active, busy]);
  return <Popover open={state.active && open} onOpenChange={setOpen}>
    <PopoverTrigger ref={trigger} type="button" className="quiet ai-model-trigger" disabled={busy} aria-label="AI 모델 선택"><Bot size={15} aria-hidden="true" /><span>{model?.label || '모델 선택'}</span><ChevronDown size={13} aria-hidden="true" /></PopoverTrigger>
    <PopoverContent className="ai-model-popover" align="start" side="top" finalFocus={state.active && !busy ? trigger : false}>
      <PopoverTitle className="sr-only">AI 모델 선택</PopoverTitle>
      {providers.map(provider => { const group = flow.modelsFor(provider.value); return <section key={provider.value}>
        <div className="ai-model-heading-row"><img src={brandIconUrl(provider.value)} alt="" className="brand-icon" /><span>{provider.label}</span><button type="button" className="quiet small" disabled={!group.account || !['ready', 'unknown'].includes(group.account.state) || group.busy} onClick={() => void flow.refreshModelGroup(provider.value)}>새로고침</button></div>
        {group.models.map(item => <button type="button" className="quiet ai-model-option" key={item.id} onClick={() => { flow.chooseModel(provider.value, group.account.id, item.id); setOpen(false); }}><span>{item.label}</span>{state.accountId === group.account.id && item.id === state.modelId && <Check size={14} aria-label="선택됨" />}</button>)}
        {group.busy && <p className="ai-chat-muted" role="status">모델을 불러오는 중</p>}{group.error && <p className="ai-chat-error" role="alert">{group.error}</p>}
        {!group.models.length && !group.busy && <p className="ai-chat-muted">{group.account && ['ready', 'unknown'].includes(group.account.state) ? '새로고침하여 모델을 불러오세요.' : '계정 연결이 필요합니다.'}</p>}
      </section>; })}
      <button type="button" className="quiet ai-model-option" onClick={() => { setOpen(false); onSettings(); }}>계정 설정</button>
    </PopoverContent>
  </Popover>;
}
function ConversationHistory({ flow, state }) {
  const [open, setOpen] = React.useState(false), [deleting, setDeleting] = React.useState(null);
  React.useEffect(() => { if (!state.active) { setOpen(false); setDeleting(null); } }, [state.active]);
  return <><Popover open={state.active && open} onOpenChange={next => { setOpen(next); if (next) void flow.loadConversations(); }}>
    <PopoverTrigger className="quiet ai-chat-history-trigger" aria-label="대화 기록"><span>{state.conversation.title || '새 대화'}</span><ChevronDown size={14} aria-hidden="true" /></PopoverTrigger>
    <PopoverContent className="ai-history-popover" align="start" finalFocus={state.active ? undefined : false}>
      <PopoverTitle className="ai-popover-title">대화 기록</PopoverTitle>
      {state.listBusy && <p className="ai-chat-muted" role="status">불러오는 중</p>}
      {state.listError && <p className="ai-chat-error" role="alert">{state.listError}</p>}
      <div className="ai-history-list">{(state.conversationList || []).map(item => <div className="ai-history-row" key={item.id}>
        <button type="button" className="quiet" aria-current={state.conversation.id === item.id ? 'true' : undefined} onClick={() => { void flow.selectConversation(item.id); setOpen(false); }}><MessageCircle size={15} aria-hidden="true" /><span>{item.title || '새 대화'}</span>{item.activeRequestId && <span className="activity-spinner" aria-label="답변 중" />}</button>
        <button type="button" className="quiet small icon" aria-label={`${item.title || '대화'} 삭제`} onClick={() => setDeleting(item)}><Trash2 size={15} aria-hidden="true" /></button>
      </div>)}</div>
      {!state.listBusy && !state.conversationList?.length && <p className="ai-chat-muted">저장된 대화가 없습니다.</p>}
    </PopoverContent>
  </Popover><Dialog open={Boolean(deleting) && state.active} onOpenChange={next => { if (!next) setDeleting(null); }}>
    <DialogContent className="ai-conversation-confirm" data-nero-ui-owned="" showCloseButton={false}><DialogTitle>대화를 삭제할까요?</DialogTitle><DialogFooter><DialogClose className="quiet small">취소</DialogClose><button type="button" className="small danger" onClick={async () => { if (await flow.removeConversation(deleting.id)) setDeleting(null); }}>삭제</button></DialogFooter></DialogContent>
  </Dialog></>;
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
      <div className="ai-source-kinds" role="group" aria-label="자료 종류"><button type="button" className="quiet small" aria-pressed={kind === 'meeting'} onClick={() => setKind('meeting')}>회의</button><button type="button" className="quiet small" aria-pressed={kind === 'template'} onClick={() => { setKind('template'); void flow.loadCatalog(); }}>템플릿</button><button type="button" className="quiet small" aria-pressed={kind === 'notion'} onClick={() => setKind('notion')}>Notion 링크</button></div>
      {kind === 'template' ? <div className="ai-attachment-list">{(state.catalog.skills || []).map(item => <button type="button" className="quiet ai-attachment-option" key={`skill:${item.id}`} onClick={() => { flow.selectSkill(item.id); setOpen(false); }}>{item.name}</button>)}{(state.catalog.templates || []).map(item => <button type="button" className="quiet ai-attachment-option" key={`template:${item.id}`} onClick={() => { flow.selectTemplate(item.id); setOpen(false); }}>{item.name}</button>)}</div> : kind === 'meeting' ? <><input ref={input} type="search" value={query} placeholder="회의 검색" aria-label="첨부할 회의 검색" onChange={event => setQuery(event.target.value)} /><div className="ai-attachment-list">{meetings.map(meeting => {
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
export function GlobalAiChat({ flow, onSettings = () => {}, onCitation = async () => {}, onClose = () => {}, getMeetings = () => [] }) {
  const state = React.useSyncExternalStore(flow.subscribe, flow.getSnapshot), conversation = state.conversation;
  const account = state.accounts.find(item => item.id === state.accountId && item.provider === state.provider);
  const busy = Boolean(conversation.activeRequestId || conversation.action);
  const transcript = React.useRef(null), pinned = React.useRef(true), composer = React.useRef(null);
  const [citationError, setCitationError] = React.useState('');
  const [chooserOpen, setChooserOpen] = React.useState(false), [chooserIndex, setChooserIndex] = React.useState(0);
  const slash = conversation.draft.match(/(?:^|\s)\/([^\s/]*)$/);
  const choices = [...(chooserOpen === 'template' ? [] : (state.catalog.skills || []).map(item => ({ ...item, kind: 'skill' }))), ...(state.catalog.templates || []).map(item => ({ ...item, kind: 'template' }))].filter(item => [item.name, ...(item.aliases || [])].some(value => value.toLowerCase().includes((chooserOpen === 'template' ? '' : slash?.[1] || '').toLowerCase())));
  const choose = item => { if (item.kind === 'skill') flow.selectSkill(item.id); else flow.selectTemplate(item.id); if (slash && chooserOpen !== 'template') flow.edit(conversation.draft.slice(0, slash.index) + (conversation.draft[slash.index] === ' ' ? ' ' : '')); setChooserOpen(item.kind === 'skill' && !conversation.templateId ? 'template' : false); composer.current?.focus(); };
  React.useEffect(() => { if (conversation.skillId && !conversation.templateId) { setChooserOpen('template'); void flow.loadCatalog(); } }, [conversation.skillId, conversation.templateId]);
  React.useEffect(() => { setChooserIndex(0); }, [conversation.draft]);
  React.useLayoutEffect(() => { pinned.current = true; if (transcript.current) transcript.current.scrollTop = transcript.current.scrollHeight; }, [conversation.id]);
  React.useLayoutEffect(() => { if (pinned.current && transcript.current) transcript.current.scrollTop = transcript.current.scrollHeight; }, [conversation.messages]);
  React.useLayoutEffect(() => { const node = composer.current; if (node) { node.style.height = 'auto'; node.style.height = `${Math.min(180, Math.max(64, node.scrollHeight))}px`; } }, [conversation.draft]);
  const reveal = async citation => { try { setCitationError(''); await onCitation(citation); } catch (error) { setCitationError(error?.message || '자료를 열지 못했습니다.'); } };
  const unavailable = account?.state === 'unavailable' || state.accountsLoaded && !account;
  return <div className="ai-chat" data-nero-ui-owned="">
    <header className="ai-chat-header"><ConversationHistory flow={flow} state={state} /><button type="button" className="quiet small icon" aria-label="새 대화" title="새 대화" onClick={() => { void flow.newConversation({ attachCurrent: true }); }}><SquarePen size={17} aria-hidden="true" /></button><button type="button" className="quiet small icon" aria-label="AI 닫기" title="AI 닫기" onClick={onClose}><X size={18} aria-hidden="true" /></button></header>
    <div ref={transcript} className="ai-chat-messages" role="log" aria-label="AI 대화" aria-live="polite" aria-relevant="additions text" onScroll={event => { const node = event.currentTarget; pinned.current = node.scrollHeight - node.scrollTop - node.clientHeight < 48; }}>
      {conversation.loading ? <p className="ai-chat-muted" role="status">대화를 불러오는 중</p> : !conversation.messages.length && <div className="ai-chat-empty"><MessageCircle size={28} strokeWidth={1.3} aria-hidden="true" /><h2>무엇을 도와드릴까요?</h2>{Boolean(conversation.meetings.length) && <button type="button" className="quiet small" onClick={() => { flow.summaryPrompt(); composer.current?.focus(); }}>결정 사항과 할 일 정리</button>}</div>}
      {conversation.messages.map(message => <article key={message.id} className={`ai-chat-message ai-chat-message--${message.role}`} aria-label={message.role === 'user' ? '내 질문' : 'AI 답변'}>
        {message.role === 'user' && message.attachments?.length > 0 && <div className="ai-message-attachments" aria-label="전송한 자료">{message.attachments.map((item, index) => <span key={item.meetingId || item.sourceId || index}><FileText size={12} aria-hidden="true" />{item.title}</span>)}</div>}
        {message.text && (message.role === 'assistant' && globalThis.NeroUI?.summaryDocument ? <div className="ai-chat-markdown" dangerouslySetInnerHTML={{ __html: globalThis.NeroUI.summaryDocument(message.text) }} onClick={event => { const control = event.target.closest('[data-summary-citations]'); if (!control) return; const refs = control.dataset.summaryCitations.split(',').map(Number); const citation = message.citations?.find(item => refs.includes(item.reference)); if (citation) void reveal(citation); }} /> : <div className="ai-chat-text">{message.text}</div>)}
        {message.role === 'assistant' && message.status === 'completed' && message.actualModelId && <p className="ai-chat-muted">모델: {message.actualModelId}</p>}
        {message.role === 'assistant' && message.template && <p className="ai-chat-muted">템플릿: {message.template.name} ({message.template.revision})</p>}
        {message.status === 'streaming' && <span className="ai-chat-message-status" role="status"><span className="activity-spinner" aria-hidden="true" /><span className="sr-only">답변 중</span></span>}
        {['cancelled', 'interrupted'].includes(message.status) && <p className="ai-chat-message-status">{message.status === 'cancelled' ? '중지됨' : '답변이 중단되었습니다.'}</p>}
        {message.status === 'failed' && <p className="ai-chat-error">{message.error || '답변을 받지 못했습니다.'}</p>}
        {message.role === 'assistant' && message.status === 'completed' && message.text.trim() && <div className="ai-chat-message-actions"><AnswerCopy flow={flow} state={state} message={message} /><SaveAnswer flow={flow} state={state} message={message} kind="summary" /><SaveAnswer flow={flow} state={state} message={message} kind="note" /></div>}
        {message.role === 'assistant' && ['failed', 'cancelled', 'interrupted'].includes(message.status) && <button type="button" className="quiet small with-icon" disabled={busy || !['ready', 'unknown'].includes(account?.state)} onClick={() => { void flow.retry(message.id); }}><RotateCcw size={14} aria-hidden="true" />다시 시도</button>}
      </article>)}
    </div>
    <div className="ai-chat-bottom">
      {(citationError || state.accountsError || state.modelError || conversation.error) && <p className="ai-chat-error" role="alert">{citationError || state.accountsError || state.modelError || conversation.error}</p>}
      {conversation.feedback && <p className="ai-chat-feedback" role="status">{conversation.feedback}</p>}
      {!state.accountsBusy && !['ready', 'unknown'].includes(account?.state) && <div className="ai-chat-connect"><span>{unavailable ? `${state.provider === 'claude' ? 'Claude' : 'ChatGPT'}를 사용할 수 없습니다.` : 'AI 계정에 로그인해 주세요.'}</span><button type="button" className="small" onClick={onSettings}>계정 설정</button></div>}
      <form className="ai-chat-composer" onSubmit={event => { event.preventDefault(); void flow.send(); }}>
        {(conversation.meetings.length > 0 || conversation.sources.length > 0) && <div className="ai-chat-sources" aria-label="포함된 자료">
          {conversation.meetings.map(meeting => <span className="ai-source-chip" key={meeting.id}><FileText size={14} aria-hidden="true" /><span title={meeting.title}>{meeting.title || '회의'}</span><button type="button" className="quiet small icon" aria-label={`${meeting.title || '회의'} 제외`} disabled={busy} onClick={() => flow.removeMeeting(meeting.id)}><X size={12} aria-hidden="true" /></button></span>)}
          {conversation.sources.map(source => <span className="ai-source-chip" key={source.sourceId}><Link2 size={14} aria-hidden="true" /><span title={source.title || source.url}>{source.title || 'Notion 자료'}</span><button type="button" className="quiet small icon" aria-label={`${source.title || 'Notion 자료'} 제외`} disabled={busy} onClick={() => flow.removeSource(source.sourceId)}><X size={12} aria-hidden="true" /></button></span>)}
        </div>}
        {(conversation.templateId || conversation.skillId) && <div className="ai-chat-sources">
          {conversation.skillId && <span className="ai-source-chip">{state.catalog.skills?.find(item => item.id === conversation.skillId)?.name || '회의 요약'}<button type="button" className="quiet small icon" aria-label="스킬 제거" onClick={() => flow.selectSkill(null)}><X size={12} /></button></span>}
          {conversation.skillId && !conversation.templateId && <span className="ai-source-chip"><button type="button" className="quiet small" onClick={() => { setChooserOpen('template'); void flow.loadCatalog(); composer.current?.focus(); }}>템플릿 선택</button></span>}
          {conversation.skillId && !conversation.templateId && <span className="ai-chat-muted">요약할 템플릿을 선택해 주세요.</span>}
          {conversation.templateId && <span className="ai-source-chip"><button type="button" className="quiet small" onClick={() => { setChooserOpen('template'); void flow.loadCatalog(); composer.current?.focus(); }}>{state.catalog.templates?.find(item => item.id === conversation.templateId)?.name || '템플릿'}</button><button type="button" className="quiet small icon" aria-label="템플릿 제거" onClick={() => flow.selectTemplate(null)}><X size={12} /></button></span>}
          {conversation.skillId && conversation.meetings.length > 1 && <span className="ai-chat-muted">요약할 회의 하나만 첨부해 주세요.</span>}
          {conversation.skillId && <select aria-label="요약할 회의" value={conversation.summaryTargetId || ''} onChange={event => flow.selectSummaryTarget(event.target.value)}><option value="">요약할 회의 선택</option>{conversation.meetings.map(item => <option key={item.id} value={item.id}>{item.title}</option>)}</select>}
        </div>}
        {chooserOpen && (slash || chooserOpen === 'template') && choices.length > 0 && <div className="ai-command-list" role="listbox" aria-label="스킬 및 템플릿">{choices.map((item, index) => <button type="button" role="option" aria-selected={index === chooserIndex} key={`${item.kind}:${item.id}`} onMouseDown={event => event.preventDefault()} onClick={() => choose(item)}><span>{item.name}</span><small>{item.kind === 'skill' ? '스킬' : '템플릿'}</small></button>)}</div>}
        <textarea ref={composer} className="text-input" aria-label="AI에게 질문" placeholder="무엇이든 물어보세요" value={conversation.draft} rows={2} onChange={event => { const value = event.target.value; flow.edit(value); if (/(?:^|\s)\/[^\s/]*$/.test(value)) { setChooserOpen('slash'); void flow.loadCatalog(); } else setChooserOpen(false); }} onKeyDown={event => { if (!event.nativeEvent.isComposing && !event.isComposing && chooserOpen && (slash || chooserOpen === 'template') && choices.length) { if (event.key === 'Escape') { event.preventDefault(); setChooserOpen(false); return; } if (event.key === 'ArrowDown' || event.key === 'ArrowUp') { event.preventDefault(); setChooserIndex((chooserIndex + (event.key === 'ArrowDown' ? 1 : choices.length - 1)) % choices.length); return; } if (event.key === 'Enter' || event.key === 'Tab') { event.preventDefault(); choose(choices[chooserIndex] || choices[0]); return; } } if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing && !event.isComposing) { event.preventDefault(); if (flow.canSend()) void flow.send(); } }} />
        <div className="ai-chat-compose-actions"><SourcePicker flow={flow} state={state} onSettings={onSettings} getMeetings={getMeetings} /><span className="ai-chat-compose-spacer" />{conversation.contextBusy && <span className="activity-spinner" role="status" aria-label="자료 불러오는 중" />}{conversation.activeRequestId ? <button type="button" className="small primary icon" aria-label="답변 중지" title="답변 중지" disabled={conversation.stopping} onClick={() => { void flow.stop(); }}><Square size={14} aria-hidden="true" /></button> : <button type="submit" className="primary small icon" aria-label="질문 보내기" title="질문 보내기" disabled={!flow.canSend()}><ArrowUp size={17} aria-hidden="true" /></button>}</div>
      </form>
      <div className="ai-chat-models"><ModelPicker flow={flow} state={state} busy={busy} onSettings={onSettings} /></div>
    </div>
  </div>;
}
