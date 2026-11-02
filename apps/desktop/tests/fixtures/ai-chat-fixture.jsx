import { mountAiChat } from '../../src/ai-chat-ui.jsx';
import { mountMeetingDetail } from '../../src/meeting-detail-ui.js';

export function mountAiChatFixture({ document, scenario = 'ready', delay = 20 }) {
  const byId = id => document.getElementById(id);
  const calls = [], timers = new Set();
  let receiver = null, ui = null, request = null, finishLogin = null, finishNotion = null;
  const later = action => { const timer = setTimeout(() => { timers.delete(timer); action(); }, delay); timers.add(timer); };
  const accounts = { accounts: [{ id: 'example-chatgpt', provider: 'chatgpt', state: scenario === 'unavailable' ? 'unavailable' : scenario === 'oauth' ? 'signed_out' : 'ready', configured: scenario !== 'unavailable', label: '예시 계정' }, { id: 'example-claude', provider: 'claude', state: 'unavailable', configured: false }] };
  const accountSnapshot = () => ({ accounts: accounts.accounts.map(account => ({ ...account })) });
  const bridge = {
    accounts: async () => accountSnapshot(),
    connect: async provider => { calls.push(['connect', provider]); return { attemptId: 'example-attempt' }; },
    finishConnect: async attemptId => { calls.push(['finishConnect', attemptId]); return new Promise(resolve => { finishLogin = resolve; }); },
    cancelConnect: async attemptId => { calls.push(['cancelConnect', attemptId]); return accountSnapshot(); },
    notionConnect: async () => ({ attemptId: 'notion-attempt' }),
    notionFinish: async () => new Promise(resolve => { finishNotion = resolve; }),
    notionCancel: async () => ({ connected: false }),
    models: async () => ({ models: [{ id: 'example-model', label: '예시 모델' }], defaultModelId: 'example-model' }),
    read: async meetingId => ({ id: null, meetingId, messages: [] }),
    subscribe: async callback => { receiver = callback; return () => { receiver = null; }; },
    send: async value => {
      calls.push(['send', value]); request = value;
      const event = body => receiver?.({ requestId: value.requestId, conversationId: 'example-conversation', meetingId: value.meetingId, ...body });
      if (scenario === 'failed') later(() => event({ type: 'failed', error: '예시 연결 오류입니다.' }));
      else later(() => { if (request !== value) return; event({ type: 'delta', text: '## 결정 사항\n- 다음 회의에서 일정을 확인합니다. [1]\n' }); later(() => { if (request !== value) return; event({ type: 'completed', messageId: `answer-${value.requestId}`, citations: [{ reference: 1, kind: 'meeting', label: '일정 확인', meetingId: 'example-meeting', segmentId: 'example-segment' }] }); }); });
      return { conversationId: 'example-conversation', requestId: value.requestId };
    },
    cancel: async requestId => { calls.push(['cancel', requestId]); const previous = request; request = null; later(() => receiver?.({ requestId, conversationId: 'example-conversation', meetingId: previous.meetingId, type: 'cancelled' })); },
    saveSummary: async value => { calls.push(['summary', value]); },
    appendNote: async value => { calls.push(['note', value]); },
    notionSnapshot: async () => ({ connected: scenario !== 'oauth', workspaceName: '예시 공간' }),
    fetchSources: async value => { calls.push(['source', value]); return { sources: [{ sourceId: 'notion:example:revision', pageId: 'example', title: '예시 자료', url: value.links[0], body: '합성 자료입니다.' }] }; },
    cancelFetch: async value => { calls.push(['cancelSource', value]); },
    disconnect: async () => accountSnapshot(),
  };
  const detail = mountMeetingDetail({ byId, finishEditing: async action => action(), onInspectorChange: (view, open) => ui?.setInspector(view, open) });
  ui = mountAiChat({ node: byId('ai-chat-inspector'), tabsNode: byId('inspector-tabs'), settingsNode: byId('ai-account-settings'), bridge,
    onSelect: view => detail.select(view), onSettings: () => { for (const panel of byId('settings-dialog').querySelectorAll('[role=tabpanel]')) panel.hidden = panel.id !== 'settings-agent-panel'; byId('settings-dialog').showModal(); ui.setSettingsActive(true); }, getTemplate: async () => 'example-template',
    onCitation: async citation => { calls.push(['citation', citation]); },
  });
  byId('meeting-detail').hidden = false;
  for (const id of ['welcome', 'library-screen', 'setup-screen']) { const node = byId(id); if (node) node.hidden = true; }
  byId('settings-close').addEventListener('click', () => byId('settings-dialog').close());
  byId('summary-content').innerHTML = globalThis.NeroUI?.summaryDocument('## 예시 요약\n다음 회의에서 일정을 확인합니다.') || '';
  byId('transcript-rows').innerHTML = globalThis.NeroUI?.transcriptRow({ index: 0, time: '00:12', start: 12, text: '다음 회의에서 일정을 확인합시다.', originalText: '', editing: false, speaker: '참석자', speakerVisible: true, searchQuery: '', disabled: true }) || '';
  byId('meeting-title').value = 'UI 검증용 예시 회의';
  byId('meeting-notes').value = '보존할 메모 초안';
  const meeting = { id: 'example-meeting', title: '예시 회의' };
  ui.setMeeting(meeting);
  detail.update({ id: meeting.id, showSummary: true, hasTranscript: true, summaryLoaded: true, transcriptLoaded: true });
  void detail.select('ai');
  return { ui, detail, calls, completeLogin() { accounts.accounts[0] = { ...accounts.accounts[0], state: 'ready' }; finishLogin?.(accountSnapshot()); }, completeNotion() { finishNotion?.({ connected: true, workspaceName: '예시 공간' }); }, dispose() { timers.forEach(clearTimeout); ui.dispose(); } };
}
