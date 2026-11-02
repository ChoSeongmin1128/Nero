import { mountAiChat } from '../../src/ai-chat-ui.jsx';
import { mountMeetingContext } from '../../src/meeting-context-ui.jsx';
import { mountActionMenus } from '../../../../design-system/action-menu.mjs';
import { NeroControls } from '../../../../design-system/react-controls.jsx';
import { mountMeetingDetail } from '../../src/meeting-detail-ui.js';

export function mountAiChatFixture({ document, scenario = 'ready', delay = 20 }) {
  const byId = id => document.getElementById(id);
  const calls = [], timers = new Set();
  let receiver = null, ui = null, finishLogin = null, finishNotion = null, conversationCount = 0;
  const conversations = new Map(), requests = new Map();
  const later = action => { const timer = setTimeout(() => { timers.delete(timer); action(); }, delay); timers.add(timer); };
  const accounts = { accounts: [{ id: 'example-chatgpt', provider: 'chatgpt', state: scenario === 'unavailable' ? 'unavailable' : scenario === 'oauth' ? 'signed_out' : 'ready', configured: scenario !== 'unavailable', label: '예시 계정' }, { id: 'example-claude', provider: 'claude', state: 'unavailable', configured: false }] };
  if (scenario === 'unknown') accounts.accounts[0].state = 'unknown';
  if (scenario === 'plan-disabled-multi') { accounts.accounts[0].state = 'signed_out'; accounts.accounts.push({ id: 'active-plan-account', provider: 'chatgpt', state: 'plan_disabled', configured: true, label: '활성 계정', detail: '활성 계정의 구독 권한을 확인해 주세요.' }); accounts.activeAccountId = 'active-plan-account'; }
  const accountSnapshot = () => ({ checked: scenario !== 'unknown', activeAccountId: accounts.activeAccountId || null, accounts: accounts.accounts.map(account => ({ ...account })) });
  let meetingSources = scenario === 'meeting-sources' ? [{ sourceId: 'notion:example:revision', pageId: 'example', title: '제품 기획 문서', url: 'https://www.notion.so/12345678123412341234123456789012' }] : [];
  let contextRevision = 0;
  const bridge = {
    meetingSources: async () => ({ revision: contextRevision, sources: structuredClone(meetingSources) }),
    meetingAddSource: async request => { meetingSources.push({ sourceId: request.sourceId, pageId: 'example', title: '예시 자료', url: 'https://www.notion.so/12345678123412341234123456789012' }); return { revision: ++contextRevision, sources: structuredClone(meetingSources) }; },
    meetingRefreshSource: async request => { meetingSources = meetingSources.map(source => ({ ...source, sourceId: request.sourceId })); return { revision: ++contextRevision, sources: structuredClone(meetingSources) }; },
    meetingRemoveSource: async request => { meetingSources = meetingSources.filter(source => source.pageId !== request.pageId); return { revision: ++contextRevision, sources: structuredClone(meetingSources) }; },
    accounts: async () => accountSnapshot(),
    connect: async (provider, accountId) => { calls.push(['connect', provider, accountId]); return { attemptId: 'example-attempt' }; },
    finishConnect: async attemptId => { calls.push(['finishConnect', attemptId]); return new Promise(resolve => { finishLogin = resolve; }); },
    cancelConnect: async attemptId => { calls.push(['cancelConnect', attemptId]); return accountSnapshot(); },
    notionConnect: async () => ({ attemptId: 'notion-attempt' }),
    notionFinish: async () => new Promise(resolve => { finishNotion = resolve; }),
    notionCancel: async () => ({ connected: false }),
    models: async () => { calls.push(['models']); return { models: [{ id: 'example-model', label: '예시 모델' }], defaultModelId: 'example-model' }; },
    list: async () => [...conversations.values()].map(value => ({ id: value.id, title: value.title, messageCount: value.messages.length, activeRequestId: value.activeRequestId })),
    create: async () => { const id = ++conversationCount === 1 ? 'example-conversation' : `example-conversation-${conversationCount}`; const value = { id, title: '새 대화', messages: [], activeRequestId: null }; conversations.set(id, value); return structuredClone(value); },
    read: async id => { if (!conversations.has(id)) throw Error('예시 대화를 찾지 못했습니다.'); return structuredClone(conversations.get(id)); },
    remove: async id => { conversations.delete(id); for (const [requestId, request] of requests) if (request.conversationId === id) requests.delete(requestId); calls.push(['remove', id]); },
    subscribe: async callback => { receiver = callback; return () => { receiver = null; }; },
    send: async value => {
      calls.push(['send', value]); requests.set(value.requestId, value);
      const conversation = conversations.get(value.conversationId);
      const attachments = [...value.meetingIds.map(meetingId => ({ kind: 'meeting', meetingId, title: meetings.find(meeting => meeting.id === meetingId)?.title || '예시 회의' })), ...value.sourceIds.map(sourceId => ({ kind: 'notion', sourceId, title: '예시 자료' }))];
      const saveTargets = attachments.filter(item => item.kind === 'meeting').map(item => ({ meetingId: item.meetingId, title: item.title, canSaveSummary: true }));
      conversation.messages.push({ id: `question-${value.requestId}`, role: 'user', status: 'completed', text: value.message, attachments }, { id: `answer-${value.requestId}`, requestId: value.requestId, role: 'assistant', status: 'streaming', text: '', attachments, saveTargets, sequence: 0 });
      conversation.activeRequestId = value.requestId;
      conversation.title = value.message.slice(0, 36);
      const event = body => {
        const message = conversation.messages.at(-1);
        message.text = body.type === 'delta' ? message.text + (body.text || '') : body.text ?? message.text;
        message.status = body.type === 'delta' ? 'streaming' : body.type;
        message.sequence += 1;
        if (body.citations) message.citations = body.citations;
        if (body.type !== 'delta') { conversation.activeRequestId = null; requests.delete(value.requestId); }
        receiver?.({ requestId: value.requestId, conversationId: value.conversationId, sequence: message.sequence, attachments, saveTargets, ...body, text: body.type === 'delta' ? body.text : message.text });
      };
      if (scenario === 'failed') later(() => { if (requests.has(value.requestId)) event({ type: 'failed', error: '예시 연결 오류입니다.' }); });
      else later(() => { if (!requests.has(value.requestId)) return; event({ type: 'delta', text: '## 결정 사항\n- 다음 회의에서 일정을 확인합니다. [1]\n' }); later(() => { if (!requests.has(value.requestId)) return; event({ type: 'completed', messageId: `answer-${value.requestId}`, citations: [{ reference: 1, kind: 'meeting', label: '일정 확인', meetingId: 'example-meeting', segmentId: 'example-segment' }] }); }); });
      return { conversationId: value.conversationId, requestId: value.requestId };
    },
    cancel: async requestId => { calls.push(['cancel', requestId]); const previous = requests.get(requestId); requests.delete(requestId); if (!previous) return; const conversation = conversations.get(previous.conversationId); const message = conversation.messages.at(-1); message.status = 'cancelled'; message.sequence += 1; conversation.activeRequestId = null; later(() => receiver?.({ requestId, conversationId: previous.conversationId, type: 'cancelled', sequence: message.sequence, text: message.text })); },
    exportAnswer: async value => { calls.push(['exportAnswer', value]); return { text: value.format === 'md' ? '## 결정 사항\n- 예시 답변' : '결정 사항\n예시 답변', format: value.format }; },
    saveSummary: async value => { calls.push(['summary', value]); },
    appendNote: async value => { calls.push(['note', value]); },
    notionSnapshot: async () => ({ connected: scenario !== 'oauth', workspaceName: '예시 공간' }),
    fetchSources: async value => { calls.push(['source', value]); return { sources: [{ sourceId: 'notion:example:revision', pageId: 'example', title: '예시 자료', url: value.links[0], body: '합성 자료입니다.' }] }; },
    cancelFetch: async value => { calls.push(['cancelSource', value]); },
    disconnect: async () => accountSnapshot(),
  };
  const detail = mountMeetingDetail({ byId, finishEditing: async action => action() });
  const meetings = [{ id: 'example-meeting', title: '예시 회의' }, { id: 'example-meeting-2', title: '다음 회의' }];
  ui = mountAiChat({ node: byId('ai-chat-inspector'), settingsNode: byId('ai-account-settings'), trigger: byId('ai-chat-open'), layoutRoot: byId('app-body'), resizeHandle: byId('ai-chat-resize'), bridge, getMeetings: () => meetings,
    onSettings: () => { for (const panel of byId('settings-dialog').querySelectorAll('[role=tabpanel]')) panel.hidden = panel.id !== 'settings-agent-panel'; byId('settings-dialog').showModal(); ui.setSettingsActive(true); }, getTemplate: async () => 'example-template',
    writeClipboard: async text => { calls.push(['clipboard', text]); if (scenario === 'clipboard-failed') throw Error('클립보드에 복사하지 못했습니다.'); },
    onCitation: async citation => { calls.push(['citation', citation]); },
  });
  document.querySelector('.navigation-shell')?.classList.add('collapsed');
  byId('meeting-detail').hidden = false;
  for (const id of ['welcome', 'library-screen', 'setup-screen']) { const node = byId(id); if (node) node.hidden = true; }
  byId('settings-close').addEventListener('click', () => byId('settings-dialog').close());
  byId('summary-content').innerHTML = globalThis.NeroUI?.summaryDocument('## 예시 요약\n다음 회의에서 일정을 확인합니다.') || '';
  byId('transcript-rows').innerHTML = globalThis.NeroUI?.transcriptRow({ index: 0, time: '00:12', start: 12, text: '다음 회의에서 일정을 확인합시다.', originalText: '', editing: false, speaker: '참석자', speakerVisible: true, searchQuery: '', disabled: true }) || '';
  byId('meeting-title').value = 'UI 검증용 예시 회의';
  byId('meeting-day').value = '2026-11-02';
  byId('meeting-notes').value = '보존할 메모 초안';
  const meeting = meetings[0];
  const sourcesUi = mountMeetingContext({ node: byId('meeting-context-control'), bridge, getMeeting: () => meeting, onSettings: () => ui.setSettingsActive(true) });
  sourcesUi.setMeeting(meeting);
  const menus = mountActionMenus({ owner: document });
  byId('audio-region').hidden = false;
  byId('audio-duration').textContent = '58:30';
  byId('audio-toggle-icon').innerHTML = globalThis.NeroIcons?.('play') || '';
  byId('audio-mute').innerHTML = globalThis.NeroIcons?.('volume-2') || '';
  byId('audio-speed').disabled = false;
  NeroControls.enhanceAll(document);
  ui.setMeeting(meeting);
  detail.update({ id: meeting.id, showSummary: true, hasTranscript: true, summaryLoaded: true, transcriptLoaded: true });
  void detail.select('notes');
  ui.open(meeting);
  return { ui, detail, calls, completeLogin() { accounts.accounts[0] = { ...accounts.accounts[0], state: 'ready' }; finishLogin?.(accountSnapshot()); }, completeNotion() { finishNotion?.({ connected: true, workspaceName: '예시 공간' }); }, dispose() { timers.forEach(clearTimeout); sourcesUi.dispose(); menus.close(); ui.dispose(); } };
}
