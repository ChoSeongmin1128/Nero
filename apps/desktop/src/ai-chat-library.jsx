import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { MessageCircle, Trash2, RotateCcw } from 'lucide-react';
import { Dialog, DialogContent, DialogTitle, DialogDescription, DialogFooter, DialogClose } from '../../../design-system/components/ui/dialog.jsx';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '../../../design-system/components/ui/select.jsx';
import './ai-chat-library.css';
import { meetingLabels, olderThan } from './ai-chat-library-state.js';

const periods = [{ value: '10', label: '10일 넘은 대화', days: 10 }, { value: '30', label: '30일 넘은 대화', days: 30 }, { value: '90', label: '90일 넘은 대화', days: 90 }];
const dateLabel = new Intl.DateTimeFormat('ko-KR', { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' });

function Row({ item, meetings, selected, current, onToggle, onOpen, trash }) {
  const labels = meetingLabels(item, meetings);
  return <div className="ai-library-row" aria-current={current ? 'true' : undefined}>
    <input type="checkbox" checked={selected} aria-label={`${item.title || '새 대화'} 선택`} disabled={Boolean(item.activeRequestId)} onChange={event => onToggle(item.id, event.target.checked)} />
    <button type="button" className="quiet ai-library-open" disabled={trash} onClick={() => onOpen(item.id)}>
      <MessageCircle size={15} aria-hidden="true" />
      <span className="ai-library-copy"><span className="ai-library-title">{item.title || '새 대화'}</span><span className="type-caption muted">{labels.length ? labels.join(', ') : '회의 없이 나눈 대화'}</span></span>
      <span className="type-caption muted ai-library-meta">{dateLabel.format(item.updatedAt || item.createdAt || 0)}{item.activeRequestId ? ' / 답변 중' : ''}</span>
    </button>
  </div>;
}

function AiChatLibrary({ flow, getMeetings, onOpen, container }) {
  const state = React.useSyncExternalStore(flow.subscribe, flow.getSnapshot);
  const [view, setView] = React.useState('active'), [selected, setSelected] = React.useState(() => new Set()), [period, setPeriod] = React.useState('10');
  const [confirm, setConfirm] = React.useState(null), [busy, setBusy] = React.useState(false);
  const trash = view === 'trash';
  React.useEffect(() => { void flow.loadList(true); }, []);
  const switchView = value => { if (value === view) return; setView(value); setSelected(new Set()); if (value === 'trash') void flow.loadTrash(); };
  const items = trash ? state.trashList : state.conversationList;
  const meetings = getMeetings();
  const visible = new Set(items.map(item => item.id));
  const chosen = [...selected].filter(id => visible.has(id));
  const toggle = (id, on) => setSelected(current => { const next = new Set(current); if (on) next.add(id); else next.delete(id); return next; });
  const ask = (action, ids, title, description, label) => { if (ids.length) setConfirm({ action, ids, title, description, label }); };
  const run = async () => {
    if (!confirm || busy) return;
    setBusy(true);
    const done = confirm.action === 'trash' ? await flow.trashConversations(confirm.ids) : confirm.action === 'restore' ? await flow.restoreConversations(confirm.ids) : await flow.purgeConversations(confirm.ids);
    setBusy(false);
    if (done) { setConfirm(null); setSelected(new Set()); }
  };
  const days = periods.find(item => item.value === period).days, stale = olderThan(state.conversationList, days);
  const idle = items.filter(item => !item.activeRequestId).map(item => item.id);
  const error = trash ? state.trashError : state.listError;
  return <>
    <header className="library-screen-header"><div><h1 id="ai-library-title" tabIndex={-1}>AI 대화</h1><span className="type-caption muted">{items.length}개</span></div>
      <div className="library-view" role="group" aria-label="대화 보기">{[['active', '대화'], ['trash', '휴지통']].map(([value, label]) => <button type="button" key={value} className="quiet small" aria-pressed={view === value} onClick={() => switchView(value)}>{label}</button>)}</div>
    </header>
    <div className="library-list-actions ai-library-actions">
      {trash ? <>
        <button type="button" className="quiet small with-icon" disabled={!chosen.length} onClick={() => ask('restore', chosen, `선택한 대화 ${chosen.length}개를 복원할까요?`, '대화 목록으로 돌아갑니다.', '복원')}><RotateCcw size={14} aria-hidden="true" />선택 복원</button>
        <button type="button" className="quiet small with-icon danger-text" disabled={!chosen.length} onClick={() => ask('purge', chosen, `선택한 대화 ${chosen.length}개를 완전히 삭제할까요?`, '완전히 삭제한 대화는 되돌릴 수 없습니다.', '완전히 삭제')}><Trash2 size={14} aria-hidden="true" />선택 완전히 삭제</button>
        <button type="button" className="quiet small danger-text" disabled={!items.length} onClick={() => ask('purge', items.map(item => item.id), '휴지통을 비울까요?', `휴지통의 대화 ${items.length}개를 완전히 삭제합니다. 되돌릴 수 없습니다.`, '휴지통 비우기')}>휴지통 비우기</button>
      </> : <>
        <button type="button" className="quiet small with-icon danger-text" disabled={!chosen.length} onClick={() => ask('trash', chosen, `선택한 대화 ${chosen.length}개를 휴지통으로 옮길까요?`, '휴지통에서 복원할 수 있습니다.', '휴지통으로 이동')}><Trash2 size={14} aria-hidden="true" />선택 삭제</button>
        <span className="ai-library-period"><Select value={period} onValueChange={setPeriod}><SelectTrigger aria-label="정리할 기간"><SelectValue>{periods.find(item => item.value === period).label}</SelectValue></SelectTrigger><SelectContent container={container}>{periods.map(item => <SelectItem key={item.value} value={item.value}>{item.label}</SelectItem>)}</SelectContent></Select>
          <button type="button" className="quiet small" disabled={!stale.length} onClick={() => ask('trash', stale.map(item => item.id), `${days}일 넘은 대화 ${stale.length}개를 휴지통으로 옮길까요?`, '마지막으로 대화한 날을 기준으로 고릅니다. 휴지통에서 복원할 수 있습니다.', '휴지통으로 이동')}>{stale.length ? `${stale.length}개 정리` : '정리할 대화 없음'}</button></span>
        <button type="button" className="quiet small danger-text" disabled={!idle.length} onClick={() => ask('trash', idle, '대화를 모두 휴지통으로 옮길까요?', `대화 ${idle.length}개를 휴지통으로 옮깁니다. 답변 중인 대화는 남깁니다.`, '모두 휴지통으로')}>전체 삭제</button>
      </>}
    </div>
    {error && <p className="notice" role="alert">{error}</p>}
    <div className="ai-library-list" aria-label={trash ? '휴지통의 대화' : 'AI 대화 목록'}>
      {items.map(item => <Row key={item.id} item={item} meetings={meetings} trash={trash} selected={selected.has(item.id)} current={!trash && state.conversation.id === item.id} onToggle={toggle} onOpen={id => { void flow.selectConversation(id); onOpen(); }} />)}
      {!items.length && <p className="people-empty">{trash ? '휴지통이 비어 있습니다.' : '저장된 AI 대화가 없습니다.'}</p>}
    </div>
    <Dialog open={Boolean(confirm)} onOpenChange={next => { if (!next && !busy) setConfirm(null); }}>
      <DialogContent container={container} className="ai-conversation-confirm" data-nero-ui-owned="" showCloseButton={false}>
        <DialogTitle>{confirm?.title}</DialogTitle><DialogDescription>{confirm?.description}</DialogDescription>
        <DialogFooter><DialogClose className="quiet small" disabled={busy}>취소</DialogClose><button type="button" className={`small ${confirm?.action === 'restore' ? 'primary' : 'danger'}`} disabled={busy} onClick={() => { void run(); }}>{confirm?.label}</button></DialogFooter>
      </DialogContent>
    </Dialog>
  </>;
}

export function mountAiChatLibrary({ node, flow, getMeetings, onOpen, container }) {
  const root = createRoot(node);
  let mounted = false;
  const mount = () => { if (mounted) return false; mounted = true; root.render(<AiChatLibrary flow={flow} getMeetings={getMeetings} onOpen={onOpen} container={container} />); return true; };
  return {
    mount,
    show() { if (!mount()) void flow.loadList(true); },
    dispose() { root.unmount(); },
  };
}
