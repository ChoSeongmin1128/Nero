import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { FileText } from 'lucide-react';
import './ai-chat-library.css';
import { meetingLabels } from './ai-chat-library-state.js';

const dateLabel = new Intl.DateTimeFormat('ko-KR', { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' });

function AiChatSidebar({ flow, getMeetings, onOpenConversation, onOpenMeeting }) {
  const state = React.useSyncExternalStore(flow.subscribe, flow.getSnapshot);
  const meetings = getMeetings();
  const items = state.conversationList || [];
  return <nav className="sidebar-ai-list" aria-label="AI 대화 목록">
    {items.map(item => {
      const linked = (item.meetingIds || []).map(id => meetings.find(meeting => meeting.id === id && !meeting.deletedAt)).filter(Boolean);
      const labels = meetingLabels(item, meetings), title = item.title || '새 대화', when = dateLabel.format(item.updatedAt || item.createdAt || 0);
      const current = state.active && state.conversation.id === item.id;
      return <article className="navigation-recent-row sidebar-ai-row" key={item.id}>
        <button type="button" className="quiet recent-meeting" data-navigation-item="" aria-current={current ? 'page' : undefined} aria-label={[title, labels.join(', '), when, item.activeRequestId ? '답변 중' : ''].filter(Boolean).join(', ')} onClick={() => { void onOpenConversation(item.id); }}>
          <span className="recent-title-line"><strong title={title}>{title}</strong><span className="recent-meeting-timestamp">{item.activeRequestId ? '답변 중' : when}</span></span>
        </button>
        {linked.length ? <button type="button" className="quiet sidebar-ai-meeting" title={`${linked[0].title || '제목 없는 회의'} 회의로 이동`} onClick={() => { void onOpenMeeting(linked[0].id, item.id); }}><FileText size={12} aria-hidden="true" /><span>{linked[0].title || '제목 없는 회의'}</span>{linked.length > 1 && <span className="sidebar-ai-more">외 {linked.length - 1}</span>}</button> : <span className="sidebar-ai-note">{labels[0] || '회의 없이 나눈 대화'}</span>}
      </article>;
    })}
    {state.listError && <p className="list-empty" role="alert">{state.listError}</p>}
    {!items.length && !state.listError && <p className="list-empty" role="status">{state.listBusy || !state.listLoaded ? '불러오는 중' : '저장된 AI 대화가 없습니다.'}</p>}
  </nav>;
}

export function mountAiChatSidebar({ node, flow, getMeetings, onOpenConversation, onOpenMeeting }) {
  const root = createRoot(node);
  let mounted = false;
  return {
    show() {
      if (!mounted) { mounted = true; root.render(<AiChatSidebar flow={flow} getMeetings={getMeetings} onOpenConversation={onOpenConversation} onOpenMeeting={onOpenMeeting} />); }
      void flow.loadList(true);
    },
    dispose() { root.unmount(); },
  };
}
