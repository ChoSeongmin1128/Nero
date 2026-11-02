import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { Tabs } from '@base-ui/react/tabs';
import { ArrowUp, Square, Plus, X, FileText, RotateCcw, NotebookPen } from 'lucide-react';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '../../../design-system/components/ui/select.jsx';
import { Popover, PopoverTrigger, PopoverContent, PopoverTitle } from '../../../design-system/components/ui/popover.jsx';
import { createAiChatFlow } from './ai-chat-flow.js';

const providers = [{ value: 'chatgpt', label: 'ChatGPT' }, { value: 'claude', label: 'Claude' }];
const accountLabel = { ready: '연결됨', signed_out: '연결 필요', unavailable: '사용할 수 없음', connecting: '연결 중' };

function SourcePicker({ flow, state, onSettings }) {
  const [open, setOpen] = React.useState(false), [link, setLink] = React.useState('');
  const input = React.useRef(null);
  React.useEffect(() => { if (!state.active) setOpen(false); }, [state.active]);
  const busy = Boolean(state.conversation.activeRequestId || state.conversation.action);
  return <Popover open={open && state.active} onOpenChange={setOpen}>
    <PopoverTrigger className="quiet small with-icon" disabled={busy} onClick={() => { void flow.loadNotion(); }}><Plus size={14} aria-hidden="true" />자료 추가</PopoverTrigger>
    <PopoverContent className="ai-source-popover" initialFocus={input} finalFocus={state.active ? undefined : false}>
      <PopoverTitle className="ai-source-title">Notion 자료</PopoverTitle>
      <form onSubmit={async event => { event.preventDefault(); if (await flow.addSource(link)) { setLink(''); setOpen(false); } }}>
        <label htmlFor="ai-source-link">페이지 링크</label>
        <input ref={input} id="ai-source-link" type="url" value={link} placeholder="https://www.notion.so/..." onChange={event => setLink(event.target.value)} disabled={state.fetching} required />
        {state.notion && !state.notion.connected && <button type="button" className="small" onClick={() => { setOpen(false); onSettings(); }}>Notion 연결</button>}
        {state.sourceError && <p role="alert" className="ai-chat-error">{state.sourceError}</p>}
        <div className="ai-source-actions">{state.fetching ? <button type="button" className="small" onClick={() => { void flow.cancelSource(); }}>취소</button> : null}<button type="submit" className="small primary" disabled={state.fetching || !link.trim() || !state.notion?.connected}>{state.fetching ? '불러오는 중' : '추가'}</button></div>
      </form>
    </PopoverContent>
  </Popover>;
}

export function AiChat({ flow, onSettings = () => {}, onCitation = async () => {} }) {
  const state = React.useSyncExternalStore(flow.subscribe, flow.getSnapshot);
  const conversation = state.conversation;
  const account = state.accounts.find(item => item.id === state.accountId && item.provider === state.provider);
  const busy = Boolean(conversation.activeRequestId || conversation.action);
  const transcript = React.useRef(null), pinned = React.useRef(true), composer = React.useRef(null);
  const meetingId = state.meeting?.id;
  const [providerOpen, setProviderOpen] = React.useState(false), [modelOpen, setModelOpen] = React.useState(false);
  React.useEffect(() => { if (!state.active) { setProviderOpen(false); setModelOpen(false); } }, [state.active]);
  const [citationError, setCitationError] = React.useState('');
  const revealCitation = async citation => { try { setCitationError(''); await onCitation(citation); } catch (error) { setCitationError(error?.message || '자료를 열지 못했습니다.'); } };
  React.useLayoutEffect(() => { pinned.current = true; if (transcript.current) transcript.current.scrollTop = transcript.current.scrollHeight; }, [meetingId]);
  React.useLayoutEffect(() => { if (pinned.current && transcript.current) transcript.current.scrollTop = transcript.current.scrollHeight; }, [conversation.messages]);
  const unavailable = account?.state === 'unavailable' || (state.accountsLoaded && !account);
  return <div className="ai-chat" data-nero-ui-owned="">
    <div className="ai-chat-toolbar">
      <Select open={state.active && providerOpen} onOpenChange={setProviderOpen} value={state.provider} onValueChange={value => flow.selectProvider(value)} items={providers} disabled={busy}>
        <SelectTrigger className="ai-provider-select" aria-label="AI 선택"><SelectValue /></SelectTrigger>
        <SelectContent>{providers.map(provider => <SelectItem key={provider.value} value={provider.value}>{provider.label}</SelectItem>)}</SelectContent>
      </Select>
      {state.models.length > 0 && <Select open={state.active && modelOpen} onOpenChange={setModelOpen} value={state.modelId} onValueChange={value => flow.selectModel(value)} items={state.models.map(model => ({ value: model.id, label: model.label }))} disabled={busy || state.modelsBusy}>
        <SelectTrigger className="ai-model-select" aria-label="모델 선택"><SelectValue /></SelectTrigger>
        <SelectContent>{state.models.map(model => <SelectItem key={model.id} value={model.id}>{model.label}</SelectItem>)}</SelectContent>
      </Select>}
    </div>
    <div ref={transcript} className="ai-chat-messages" role="log" aria-label="AI 대화" aria-live="polite" aria-relevant="additions text" onScroll={event => { const node = event.currentTarget; pinned.current = node.scrollHeight - node.scrollTop - node.clientHeight < 48; }}>
      {conversation.loading ? <p className="ai-chat-empty" role="status">대화를 불러오는 중</p> : !conversation.messages.length && <div className="ai-chat-empty"><p>회의에 대해 물어보세요.</p><button type="button" className="small" disabled={busy} onClick={() => { flow.summaryPrompt(); composer.current?.focus(); }}>결정 사항과 할 일 정리</button></div>}
      {conversation.messages.map(message => <article key={message.id} className={`ai-chat-message ai-chat-message--${message.role}`} aria-label={message.role === 'user' ? '내 질문' : 'AI 답변'}>
        <span className="ai-chat-role">{message.role === 'user' ? '나' : 'AI'}</span>
        {message.text && (message.role === 'assistant' && globalThis.NeroUI?.summaryDocument ? <div className="ai-chat-markdown" dangerouslySetInnerHTML={{ __html: globalThis.NeroUI.summaryDocument(message.text) }} onClick={event => { const control = event.target.closest('[data-summary-citations]'); if (!control) return; const refs = control.dataset.summaryCitations.split(',').map(Number); const citation = message.citations?.find(item => refs.includes(item.reference)); if (citation) void revealCitation(citation); else setCitationError('답변이 완료되면 출처를 확인할 수 있습니다.'); }} /> : <div className="ai-chat-text">{message.text}</div>)}
        {message.status === 'streaming' && <p className="ai-chat-message-status" role="status">{message.text ? '답변 중' : '생각 중'}</p>}
        {['cancelled', 'interrupted'].includes(message.status) && <p className="ai-chat-message-status">{message.status === 'cancelled' ? '중지됨' : '답변이 중단되었습니다.'}</p>}
        {message.status === 'failed' && <p className="ai-chat-error">{message.error || '답변을 받지 못했습니다.'}</p>}
        {message.citations?.length > 0 && <ul className="ai-chat-citations" aria-label="참고 자료">{message.citations.map((citation, index) => <li key={`${citation.reference}:${index}`}><button type="button" className="quiet small" onClick={() => { void revealCitation(citation); }}>[{citation.reference}] {citation.label || `자료 ${index + 1}`}</button></li>)}</ul>}
        {message.role === 'assistant' && message.status === 'completed' && message.text.trim() && <div className="ai-chat-message-actions">
          <button type="button" className="quiet small with-icon" disabled={busy} onClick={() => { void flow.save('summary', message.id); }}><FileText size={14} aria-hidden="true" />요약 저장</button>
          <button type="button" className="quiet small with-icon" disabled={busy} onClick={() => { void flow.save('note', message.id); }}><NotebookPen size={14} aria-hidden="true" />메모 추가</button>
        </div>}
        {message.role === 'assistant' && ['failed', 'cancelled', 'interrupted'].includes(message.status) && <button type="button" className="quiet small with-icon" disabled={busy || !account || account.state !== 'ready'} onClick={() => { void flow.retry(message.id); }}><RotateCcw size={14} aria-hidden="true" />다시 시도</button>}
      </article>)}
    </div>
    <div className="ai-chat-bottom">
      {citationError && <p className="ai-chat-error" role="alert">{citationError}</p>}
      {(state.accountsError || state.modelError || conversation.error) && <p className="ai-chat-error" role="alert">{state.accountsError || state.modelError || conversation.error}</p>}
      {!conversation.loaded && !conversation.loading && conversation.error && <button type="button" className="small" onClick={() => { void flow.loadConversation(true); }}>대화 다시 불러오기</button>}
      {conversation.feedback && <p className="ai-chat-feedback" role="status">{conversation.feedback}</p>}
      {!state.accountsBusy && account?.state !== 'ready' && <div className="ai-chat-connect"><span>{unavailable ? `${state.provider === 'claude' ? 'Claude' : 'ChatGPT'}를 사용할 수 없습니다.` : 'AI 계정을 연결해 주세요.'}</span><button type="button" className="small" onClick={onSettings}>계정 설정</button></div>}
      <div className="ai-chat-sources" aria-label="포함된 자료">
        {conversation.includeMeeting && state.meeting && <span className="ai-source-chip"><FileText size={13} aria-hidden="true" /><span title={state.meeting.title}>{state.meeting.title || '현재 회의'}</span><button type="button" className="quiet small icon" aria-label="현재 회의 제외" disabled={busy} onClick={() => flow.includeMeeting(false)}><X size={12} aria-hidden="true" /></button></span>}
        {!conversation.includeMeeting && <button type="button" className="quiet small" disabled={busy} onClick={() => flow.includeMeeting(true)}>현재 회의 포함</button>}
        {conversation.sources.map(source => <span className="ai-source-chip" key={source.sourceId}><FileText size={13} aria-hidden="true" /><span title={source.title || source.url}>{source.title || 'Notion 자료'}</span><button type="button" className="quiet small icon" aria-label={`${source.title || 'Notion 자료'} 제외`} disabled={busy} onClick={() => flow.removeSource(source.sourceId)}><X size={12} aria-hidden="true" /></button></span>)}
      </div>
      <form className="ai-chat-composer" onSubmit={event => { event.preventDefault(); void flow.send(); }}>
        <textarea ref={composer} className="text-input" aria-label="AI에게 질문" placeholder="질문을 입력하세요" value={conversation.draft} disabled={!state.meeting} rows={3} onChange={event => flow.edit(event.target.value)} onKeyDown={event => { if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing && !event.isComposing) { event.preventDefault(); if (flow.canSend()) void flow.send(); } }} />
        <div className="ai-chat-compose-actions"><SourcePicker flow={flow} state={state} onSettings={onSettings} />{conversation.activeRequestId ? <button type="button" className="small icon" aria-label="답변 중지" title="답변 중지" disabled={conversation.stopping} onClick={() => { void flow.stop(); }}><Square size={14} aria-hidden="true" /></button> : <button type="submit" className="primary small icon" aria-label="질문 보내기" title="질문 보내기" disabled={!flow.canSend()}><ArrowUp size={16} aria-hidden="true" /></button>}</div>
      </form>
    </div>
  </div>;
}

export function AiAccounts({ flow }) {
  const state = React.useSyncExternalStore(flow.subscribe, flow.getSnapshot);
  return <div className="ai-accounts" data-nero-ui-owned="">
    <h4 className="settings-subtitle">AI 계정</h4>
    <div className="settings-group">{providers.map(provider => {
      const account = state.accounts.find(item => item.provider === provider.value && item.state === 'ready') || state.accounts.find(item => item.provider === provider.value);
      const attempt = state.attempts[provider.value];
      return <div className="settings-client-row" key={provider.value}>
        <span className="settings-client-name">{provider.label}</span>
        <div className="settings-client-controls"><span className="settings-client-status" role="status">{state.accountsBusy ? '확인 중' : attempt ? '로그인 대기' : accountLabel[account?.state] || '사용할 수 없음'}</span>
          {attempt ? <><button type="button" className="small" disabled={state.accountsBusy} onClick={() => { void flow.finishConnect(provider.value); }}>연결 확인</button><button type="button" className="quiet small" disabled={state.accountsBusy} onClick={() => { void flow.cancelConnect(provider.value); }}>취소</button></> : account?.state === 'ready' ? <button type="button" className="small" disabled={state.accountsBusy || Boolean(state.conversation.activeRequestId)} onClick={() => { void flow.disconnect(account.id); }}>해제</button> : <button type="button" className="small" disabled={state.accountsBusy || !account?.configured || account.state !== 'signed_out'} onClick={() => { void flow.connect(provider.value); }}>연결</button>}
        </div>
        {account?.label && account.state === 'ready' && <p className="ai-account-label">{account.label}</p>}
      </div>;
    })}</div>
    {state.accountsError && <p className="ai-chat-error" role="alert">{state.accountsError}<button type="button" className="quiet small" disabled={state.accountsBusy} onClick={() => { void flow.loadAccounts(true); }}>다시 확인</button></p>}
    <h4 className="settings-subtitle">참고 자료</h4>
    <div className="settings-group"><div className="settings-client-row"><span className="settings-client-name">Notion</span><div className="settings-client-controls"><span className="settings-client-status" role="status">{state.notionBusy ? '확인 중' : state.notionAttempt ? '로그인 대기' : state.notion?.connected ? '연결됨' : '연결 필요'}</span>
      {state.notionAttempt ? <><button type="button" className="small" disabled={state.notionBusy} onClick={() => { void flow.finishNotion(); }}>연결 확인</button><button type="button" className="quiet small" disabled={state.notionBusy} onClick={() => { void flow.cancelNotion(); }}>취소</button></> : state.notion?.connected ? <button type="button" className="small" disabled={state.notionBusy || state.fetching} onClick={() => { void flow.disconnectNotion(); }}>해제</button> : <button type="button" className="small" disabled={state.notionBusy} onClick={() => { void flow.connectNotion(); }}>연결</button>}
    </div>{state.notion?.workspaceName && <p className="ai-account-label">{state.notion.workspaceName}</p>}</div></div>
    {state.notionError && <p className="ai-chat-error" role="alert">{state.notionError}</p>}
  </div>;
}

export function mountAiChat({ node, settingsNode, tabsNode, bridge, onSelect, onSettings, onCitation, beforeWrite, onSaved, getTemplate }) {
  const flow = createAiChatFlow({ bridge, beforeWrite, onSaved, getTemplate });
  const root = createRoot(node), settingsRoot = createRoot(settingsNode), tabsRoot = createRoot(tabsNode);
  root.render(<AiChat flow={flow} onSettings={onSettings} onCitation={onCitation} />);
  settingsRoot.render(<AiAccounts flow={flow} />);
  let selected = null;
  const setInspector = (view, open) => {
    flow.setActive(open && view === 'ai');
    if (selected === view) return;
    selected = view;
    tabsRoot.render(<Tabs.Root value={view} onValueChange={value => { void onSelect(value); }} data-nero-ui-owned=""><Tabs.List className="ai-inspector-tabs" aria-label="보조 패널"><Tabs.Tab id="inspector-notes-tab" value="notes" aria-controls="notes-section">메모</Tabs.Tab><Tabs.Tab id="inspector-ai-tab" value="ai" aria-controls="ai-chat-inspector">AI</Tabs.Tab></Tabs.List></Tabs.Root>);
  };
  setInspector('notes', false);
  return { flow, setInspector, setMeeting: meeting => flow.setMeeting(meeting), setSettingsActive: active => flow.setSettingsActive(active), dispose() { flow.dispose(); root.unmount(); settingsRoot.unmount(); tabsRoot.unmount(); } };
}
