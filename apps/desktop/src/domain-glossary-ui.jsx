import * as React from 'react';

export function DomainGlossary({ meetingId, bridge }) {
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
      const result = await bridge.domainGlossaryView(meetingId);
      if (epoch.current !== token) return;
      setView(result); setEntries(result.snapshot.entries.map(entry => ({ ...entry })));
    } catch (e) { if (epoch.current === token) setError(e?.message || '용어집을 불러오지 못했습니다.'); }
    finally { if (epoch.current === token) setBusy(false); }
  }
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
        <label htmlFor="domain-glossary-candidates">자료에서 찾은 후보</label>
        <select id="domain-glossary-candidates" value="" disabled={busy || !view.candidates.length} onChange={event => { if (event.target.value === '') return; const candidate = view.candidates[Number(event.target.value)]; if (candidate) add(candidate); }}>
          <option value="">후보를 선택해 추가</option>
          {view.candidates.map((candidate, index) => <option key={`${candidate.sourceId}-${index}`} value={index}>{candidate.term} ({candidate.sourceTitle})</option>)}
        </select>
        {!view.candidates.length && <p>회의 자료에서 후보를 찾지 못했습니다.</p>}
        {view.reusable?.length > 0 && <>
          <label htmlFor="domain-glossary-reuse">다른 회의에서 가져오기</label>
          <select id="domain-glossary-reuse" value="" disabled={busy} onChange={event => { if (event.target.value === '') return; const entry = view.reusable[Number(event.target.value)]; if (entry) add(entry); }}>
            <option value="">같은 자료에서 승인한 용어 선택</option>
            {view.reusable.map((entry, index) => <option key={`${entry.sourceId}-${index}`} value={index}>{entry.term} ({entry.meetingTitle})</option>)}
          </select>
        </>}
        <ul>{entries.map((entry, index) => <li key={`${entry.sourceId}-${index}`}>
          <label className="sr-only" htmlFor={`domain-term-${index}`}>용어 표기</label>
          <input id={`domain-term-${index}`} value={entry.term} maxLength={100} disabled={busy} onChange={event => setEntries(entries.map((item, i) => i === index ? { ...item, term: event.target.value } : item))} />
          <button type="button" className="quiet small" disabled={busy} onClick={() => setEntries(entries.filter((_, i) => i !== index))}>제거</button>
          <span title={entry.sourceId}>{view.candidates.find(candidate => candidate.sourceId === entry.sourceId)?.sourceTitle || view.reusable?.find(candidate => candidate.sourceId === entry.sourceId)?.meetingTitle || '이전 자료'}</span>
        </li>)}</ul>
        <button type="button" className="small" disabled={busy || entries.some(entry => !entry.term.trim())} onClick={() => { void save(); }}>용어집 저장</button>
        <button type="button" className="quiet small" disabled={busy} onClick={() => { void load(); }}>다시 불러오기</button>
      </>}
      {busy && <p role="status">처리 중</p>}
      {error && <p role="alert">{error}</p>}
    </div>}
  </section>;
}
