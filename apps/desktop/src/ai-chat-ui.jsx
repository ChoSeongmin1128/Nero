import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { createAiChatFlow, selectAiAccount } from './ai-chat-flow.js';
import { GlobalAiChat } from './ai-chat-shell.jsx';
import { mountAiChatLayout } from './ai-chat-layout.js';

const providers = [{ value: 'chatgpt', label: 'ChatGPT' }, { value: 'claude', label: 'Claude' }];
const accountLabel = { ready: '로그인됨', signed_out: '로그인 필요', unavailable: '사용할 수 없음', plan_disabled: '구독 확인 필요', connecting: '로그인 중' };
export const AiChat = GlobalAiChat;

export function AiAccounts({ flow }) {
  const state = React.useSyncExternalStore(flow.subscribe, flow.getSnapshot);
  return <div className="ai-accounts" data-nero-ui-owned="">
    <h4 className="settings-subtitle">Nero에서 AI 사용</h4>
    <div className="settings-group">{providers.map(provider => {
      const account = selectAiAccount(state.accounts, provider.value, { selectedId: state.accountId, activeAccountId: state.activeAccountId });
      const phase = state.connectionPhases[provider.value];
      return <div className="settings-client-row" key={provider.value}>
        <span className="settings-client-name">{provider.label}</span>
        <div className="settings-client-controls"><span className="settings-client-status" role="status">{phase === 'cancelling' ? '취소 중' : phase === 'starting' ? '로그인 중' : phase === 'waiting' ? '로그인 대기' : state.accountsBusy ? '확인 중' : accountLabel[account?.state] || '사용할 수 없음'}</span>
          {phase ? <>{phase === 'failed' && <button type="button" className="small" onClick={() => { void flow.retryConnect(provider.value); }}>다시 시도</button>}<button type="button" className="quiet small" disabled={phase === 'cancelling'} onClick={() => { void flow.cancelConnect(provider.value); }}>취소</button></> : account?.state === 'ready' ? <button type="button" className="small" disabled={state.accountsBusy || Boolean(state.conversation.activeRequestId)} onClick={() => { void flow.disconnect(account.id); }}>로그아웃</button> : <button type="button" className="small" disabled={state.accountsBusy || !account?.configured || !['signed_out', 'plan_disabled'].includes(account.state)} onClick={() => { void flow.connect(provider.value); }}>로그인</button>}
        </div>
        {account?.label && account.state === 'ready' && <p className="ai-account-label">{account.label}</p>}
        {account?.state === 'plan_disabled' && account.detail && <p className="ai-account-label" role="status">{account.detail}</p>}
      </div>;
    })}</div>
    {state.accountsError && <p className="ai-chat-error" role="alert">{state.accountsError}<button type="button" className="quiet small" disabled={state.accountsBusy} onClick={() => { void flow.loadAccounts(true); }}>다시 확인</button></p>}
    <h4 className="settings-subtitle">참고 자료</h4>
    <div className="settings-group"><div className="settings-client-row"><span className="settings-client-name">Notion</span><div className="settings-client-controls"><span className="settings-client-status" role="status">{state.connectionPhases.notion === 'cancelling' ? '취소 중' : state.connectionPhases.notion === 'starting' ? '연결 중' : state.connectionPhases.notion === 'waiting' ? '로그인 대기' : state.notionBusy ? '확인 중' : state.notion?.connected ? '연결됨' : '연결 필요'}</span>
      {state.connectionPhases.notion ? <>{state.connectionPhases.notion === 'failed' && <button type="button" className="small" onClick={() => { void flow.retryNotion(); }}>다시 시도</button>}<button type="button" className="quiet small" disabled={state.connectionPhases.notion === 'cancelling'} onClick={() => { void flow.cancelNotion(); }}>취소</button></> : state.notion?.connected ? <button type="button" className="small" disabled={state.notionBusy || state.fetching} onClick={() => { void flow.disconnectNotion(); }}>해제</button> : <button type="button" className="small" disabled={state.notionBusy} onClick={() => { void flow.connectNotion(); }}>연결</button>}
    </div>{state.notion?.workspaceName && <p className="ai-account-label">{state.notion.workspaceName}</p>}</div></div>
    {state.notionError && <p className="ai-chat-error" role="alert">{state.notionError}</p>}
  </div>;
}

export function mountAiChat({ node, settingsNode, trigger, layoutRoot, resizeHandle, bridge, onSettings, onCitation, beforeWrite, onSaved, getTemplate, getMeetings = () => [], writeClipboard }) {
  const flow = createAiChatFlow({ bridge, beforeWrite, onSaved, getTemplate, getMeetings, writeClipboard });
  const root = createRoot(node), settingsRoot = createRoot(settingsNode);
  const layout = mountAiChatLayout({ root: layoutRoot, handle: resizeHandle, panel: node.parentElement, trigger });
  const close = () => { flow.setActive(false); layout.update(false); trigger?.focus({ preventScroll: true }); };
  const open = meeting => { layout.update(true); void flow.open(meeting); };
  const toggle = () => flow.getSnapshot().active ? close() : open(flow.getSnapshot().meeting);
  trigger?.addEventListener('click', toggle);
  const escape = event => {
    if (event.key !== 'Escape' || event.defaultPrevented || event.isComposing || event.target.closest?.('[role="listbox"],[role="menu"],[data-base-ui-portal]')) return;
    event.preventDefault(); close();
  };
  node.addEventListener('keydown', escape);
  root.render(<GlobalAiChat flow={flow} onSettings={onSettings} onCitation={onCitation} onClose={close} getMeetings={getMeetings} />);
  settingsRoot.render(<AiAccounts flow={flow} />);
  return { flow, open, close, setMeeting: meeting => flow.setMeeting(meeting), setSettingsActive: active => flow.setSettingsActive(active), dispose() { trigger?.removeEventListener('click', toggle); node.removeEventListener('keydown', escape); layout.destroy(); flow.dispose(); root.unmount(); settingsRoot.unmount(); } };
}
