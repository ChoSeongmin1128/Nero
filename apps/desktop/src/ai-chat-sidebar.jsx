import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { AudioLines } from '../../../design-system/icons-react.jsx';
import './ai-chat-library.css';
import { meetingLabels } from './ai-chat-library-state.js';

const dateLabel = new Intl.DateTimeFormat('ko-KR', { month: 'numeric', day: 'numeric' });
const timeLabel = new Intl.DateTimeFormat('ko-KR', { hour: '2-digit', minute: '2-digit', hourCycle: 'h23' });
const fullDateLabel = new Intl.DateTimeFormat('ko-KR', { dateStyle: 'medium', timeStyle: 'short' });
const shortDate = value => {
  if (!value) return '';
  const date = new Date(value), today = new Date();
  return date.toDateString() === today.toDateString() ? timeLabel.format(date) : dateLabel.format(date);
};

function AiChatSidebar({ flow, getMeetings, onOpenConversation, onOpenMeeting }) {
  const state = React.useSyncExternalStore(flow.subscribe, flow.getSnapshot);
  const meetings = getMeetings();
  const items = state.conversationList || [];
  return <nav className="sidebar-ai-list" aria-label="대화 기록">
    {items.map(item => {
      const linked = (item.meetingIds || []).map(id => meetings.find(meeting => meeting.id === id && !meeting.deletedAt)).filter(Boolean);
      const labels = meetingLabels(item, meetings), title = item.title || '새 대화', timestamp = item.updatedAt || item.createdAt, when = shortDate(timestamp);
      const current = state.active && state.conversation.id === item.id;
      return <article className="navigation-recent-row sidebar-ai-row" key={item.id}>
        <button type="button" className="quiet recent-meeting" data-navigation-item="" aria-current={current ? 'page' : undefined} aria-label={[title, labels.join(', '), when, item.activeRequestId ? '답변 중' : ''].filter(Boolean).join(', ')} onClick={() => { void onOpenConversation(item.id); }}>
          <span className="recent-title-line"><strong title={title}>{title}</strong><time className="recent-meeting-timestamp" dateTime={timestamp ? new Date(timestamp).toISOString() : undefined} title={timestamp ? fullDateLabel.format(timestamp) : undefined}>{item.activeRequestId ? '답변 중' : when}</time></span>
        </button>
        {linked.length ? <button type="button" className="quiet sidebar-ai-meeting" title={`${linked[0].title || '제목 없는 회의'} 회의로 이동`} onClick={() => { void onOpenMeeting(linked[0].id, item.id); }}><AudioLines size={12} aria-hidden="true" /><span>{linked[0].title || '제목 없는 회의'}</span>{linked.length > 1 && <span className="sidebar-ai-more">외 {linked.length - 1}</span>}</button> : labels.length ? <span className="sidebar-ai-note">{labels.join(', ')}</span> : null}
      </article>;
    })}
    {state.listError && <p className="list-empty" role="alert">{state.listError}</p>}
    {!items.length && !state.listError && <p className="list-empty" role="status">{state.listBusy || !state.listLoaded ? '불러오는 중' : '저장된 대화가 없습니다.'}</p>}
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
