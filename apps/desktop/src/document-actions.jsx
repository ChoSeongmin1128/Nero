import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { Check, Copy, Download } from 'lucide-react';
import { Popover, PopoverTrigger, PopoverContent, PopoverTitle } from '../../../design-system/components/ui/popover.jsx';
import { createDocumentActionsFlow } from './document-actions-flow.js';

function DocumentActions({ flow, available, busy, kind }) {
  const state = React.useSyncExternalStore(flow.subscribe, flow.getSnapshot);
  const label = kind === 'summary' ? '요약' : '전체 음성 기록';
  return <span className="document-actions" data-nero-ui-owned="" hidden={!available}>
    {['copy', 'export'].map(action => <Popover key={action} open={state.open === action} onOpenChange={open => open ? flow.open(action) : flow.close()}>
      <PopoverTrigger className="quiet small icon" disabled={busy || state.pending} aria-label={`${label} ${action === 'copy' ? '복사' : '내보내기'}`} title={`${label} ${action === 'copy' ? '복사' : '내보내기'}`}>
        {action === 'copy' ? state.copied ? <Check size={17} aria-hidden="true" /> : <Copy size={17} aria-hidden="true" /> : <Download size={17} aria-hidden="true" />}
      </PopoverTrigger>
      <PopoverContent align="end" className="grove-action-popover" data-nero-ui-owned="">
        <PopoverTitle className="sr-only">{label} {action === 'copy' ? '복사' : '내보내기'}</PopoverTitle>
        <button className="quiet small" disabled={state.pending} onClick={() => { void flow.perform('txt', action === 'copy'); }}>{action === 'copy' ? '텍스트로 복사' : '텍스트 (.txt)'}</button>
        <button className="quiet small" disabled={state.pending} onClick={() => { void flow.perform('md', action === 'copy'); }}>{action === 'copy' ? '마크다운으로 복사' : '마크다운 (.md)'}</button>
        {kind === 'summary' && <label className="grove-action-option"><input type="checkbox" checked={state.includeCitations} disabled={state.pending} onChange={event => flow.include(event.target.checked)} /><span>인용 번호 포함</span></label>}
        {state.pending && <span className="processing-inline type-caption" role="status"><span className="activity-spinner" aria-hidden="true" />준비 중</span>}
        {state.error && <p className="notice type-caption" role="alert">{state.error}</p>}
      </PopoverContent>
    </Popover>)}
  </span>;
}

export function mountDocumentActions({ container, kind, onExport }) {
  const root = createRoot(container);
  const flow = createDocumentActionsFlow({ kind, onExport });
  return {
    update(context) { flow.update(context); root.render(<DocumentActions flow={flow} available={context.available} busy={context.busy} kind={kind} />); },
    close: () => flow.close(),
  };
}
