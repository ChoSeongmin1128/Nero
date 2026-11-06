import * as React from 'react';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '../../../design-system/components/ui/select.jsx';

function AddPicker({ label, placeholder, items, describe, disabled, onPick }) {
  return <Select value={null} disabled={disabled} onValueChange={index => { const item = items[Number(index)]; if (item) onPick(item); }}>
    <SelectTrigger aria-label={label}><SelectValue>{placeholder}</SelectValue></SelectTrigger>
    <SelectContent>{items.map((item, index) => <SelectItem key={`${item.sourceId}-${index}`} value={String(index)}>{describe(item)}</SelectItem>)}</SelectContent>
  </Select>;
}

export function DomainGlossary({ meetingId, sourcesRevision, bridge }) {
  const [view, setView] = React.useState(null);
  const [entries, setEntries] = React.useState([]);
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState('');
  const [expanded, setExpanded] = React.useState(false);
  const epoch = React.useRef(0);
  React.useLayoutEffect(() => { epoch.current++; setView(null); setEntries([]); setBusy(false); setError(''); setExpanded(false); }, [meetingId]);
  async function load() {
    const token = ++epoch.current;
    setBusy(true); setError('');
    try {
      const revision = sourcesRevision;
      const result = await bridge.domainGlossaryView(meetingId);
      if (epoch.current !== token) return;
      loadedRevision.current = revision;
      setView(result); setEntries(result.snapshot.entries.map(entry => ({ ...entry })));
    } catch (e) { if (epoch.current === token) setError(e?.message || '용어집을 불러오지 못했습니다.'); }
    finally { if (epoch.current === token) setBusy(false); }
  }
  const dirty = Boolean(view) && JSON.stringify(entries) !== JSON.stringify(view.snapshot.entries);
  // Attached sources change the candidates. A change seen while editing waits until the
  // edits are saved or removed, so unsaved terms are never replaced.
  const loadedRevision = React.useRef(sourcesRevision);
  const sourcesChanged = Boolean(view) && loadedRevision.current !== sourcesRevision;
  React.useEffect(() => { if (expanded && sourcesChanged && !dirty && !busy) void load(); }, [expanded, sourcesChanged, dirty, busy]);
  function add(candidate) {
    if (entries.some(entry => entry.term.toLocaleLowerCase() === candidate.term.toLocaleLowerCase())) return;
    setEntries([...entries, { term: candidate.term, sourceId: candidate.sourceId }]);
  }
  async function save() {
    const token = epoch.current;
    setBusy(true); setError('');
    try {
      const snapshot = await bridge.domainGlossarySave({ meetingId, expectedRevision: view.snapshot.revision, entries: entries.map(entry => ({ ...entry, term: entry.term.trim() })) });
      if (epoch.current !== token) return;
      setView({ ...view, snapshot }); setEntries(snapshot.entries);
    } catch (e) { if (epoch.current === token) setError(e?.message || '용어집을 저장하지 못했습니다.'); }
    finally { if (epoch.current === token) setBusy(false); }
  }
  return <section className="domain-glossary">
    <button type="button" className="quiet small" disabled={busy} onClick={() => { if (!expanded) { setExpanded(true); if (!view) void load(); } else { epoch.current++; setExpanded(false); setBusy(false); } }} aria-expanded={expanded}>회의 용어집 {expanded ? '닫기' : '검토'}</button>
    {expanded && <div>
      <p>회의에서 사용할 전문 용어를 검토해 추가하세요. 전사 문맥은 Whisper에서 지원합니다.</p>
      {view?.unavailableSources > 0 && <p role="status">읽을 수 없는 자료 {view.unavailableSources}개는 후보에서 제외했습니다.</p>}
      {view && <>
        <p className="type-label">자료에서 찾은 후보</p>
        <AddPicker label="자료에서 찾은 후보" placeholder="후보를 선택해 추가" items={view.candidates} describe={candidate => `${candidate.term} (${candidate.sourceTitle})`} disabled={busy || !view.candidates.length} onPick={add} />
        {!view.candidates.length && <p>회의 자료에서 후보를 찾지 못했습니다.</p>}
        {view.reusable?.length > 0 && <>
          <p className="type-label">다른 회의에서 가져오기</p>
          <AddPicker label="다른 회의에서 가져오기" placeholder="같은 자료에서 승인한 용어 선택" items={view.reusable} describe={entry => `${entry.term} (${entry.meetingTitle})`} disabled={busy} onPick={add} />
        </>}
        <ul>{entries.map((entry, index) => <li key={`${entry.sourceId}-${index}`}>
          <label className="sr-only" htmlFor={`domain-term-${index}`}>용어 표기</label>
          <input id={`domain-term-${index}`} value={entry.term} maxLength={100} disabled={busy} onChange={event => setEntries(entries.map((item, i) => i === index ? { ...item, term: event.target.value } : item))} />
          <button type="button" className="quiet small" disabled={busy} onClick={() => setEntries(entries.filter((_, i) => i !== index))}>제거</button>
          <span title={entry.sourceId}>{view.candidates.find(candidate => candidate.sourceId === entry.sourceId)?.sourceTitle || view.reusable?.find(candidate => candidate.sourceId === entry.sourceId)?.meetingTitle || '이전 자료'}</span>
        </li>)}</ul>
        <button type="button" className="small" disabled={busy || entries.some(entry => !entry.term.trim())} onClick={() => { void save(); }}>용어집 저장</button>
      </>}
      {sourcesChanged && dirty && <p className="type-caption" role="status">회의 자료가 바뀌었습니다. 용어집을 저장하면 후보를 다시 불러옵니다.</p>}
      {busy && <p role="status">처리 중</p>}
      {error && <p role="alert">{error}{!view && <button type="button" className="quiet small" onClick={() => { void load(); }}>다시 시도</button>}</p>}
    </div>}
  </section>;
}
