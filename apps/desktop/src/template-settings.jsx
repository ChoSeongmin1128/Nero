import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { ArrowLeft, Plus, Trash2 } from 'lucide-react';
import { ConfirmDialog } from '../../../design-system/components/ui/confirm-dialog.jsx';
import { bindTemplateDialogKeys, createTemplateSettingsFlow } from './template-settings-flow.js';

function TemplateSettings({ flow, confirm, beforeLeave }) {
  const state = React.useSyncExternalStore(flow.subscribe, flow.getSnapshot);
  const draft = state.draft;
  const nameInput = React.useRef(null);
  React.useEffect(() => { if (draft && !state.busy) nameInput.current?.focus(); }, [Boolean(draft)]);
  const reload = async () => {
    if (flow.isDirty() && !await confirm('최신 내용을 불러올까요?', '불러오기에 성공하면 저장하지 않은 수정 내용이 바뀝니다.', '불러오기')) return;
    void flow.reload();
  };
  return <div className="template-settings" data-nero-ui-owned="" aria-busy={state.busy}>
    {draft ? <>
      <div className="template-settings-heading">
        <button type="button" className="small with-icon" disabled={state.busy} onClick={async () => { if (await beforeLeave()) void flow.back(); }}><ArrowLeft size={14} aria-hidden="true" />목록</button>
        <h3 className="settings-panel-title">{state.document ? '템플릿 편집' : '새 템플릿'}</h3>
      </div>
      <form className="template-settings-form" onSubmit={event => { event.preventDefault(); void flow.save(); }}>
        <label>이름<input ref={nameInput} value={draft.name} disabled={state.busy} required onChange={event => flow.edit('name', event.target.value)} /></label>
        <label>용도<input value={draft.description} disabled={state.busy} onChange={event => flow.edit('description', event.target.value)} /></label>
        <label>요약 지침<textarea value={draft.prompt} disabled={state.busy} required spellCheck={false} onChange={event => flow.edit('prompt', event.target.value)} /></label>
        <div className="template-settings-actions">
          {state.document?.builtIn && state.document.customized && <button type="button" className="small" disabled={state.busy} onClick={async () => { if (await confirm('기본값으로 복원할까요?', '이 템플릿의 수정 내용이 기본값으로 바뀝니다.', '복원')) void flow.reset(); }}>기본값으로</button>}
          {state.document && !state.document.builtIn && <button type="button" className="small icon danger" aria-label="템플릿 삭제" title="템플릿 삭제" disabled={state.busy} onClick={async () => { if (await confirm('템플릿을 삭제할까요?', '이 템플릿을 삭제합니다.', '삭제', true)) void flow.remove(); }}><Trash2 size={15} aria-hidden="true" /></button>}
          <button type="submit" className="small primary" disabled={state.busy || !draft.name.trim() || !draft.prompt.trim() || (!flow.isDirty() && Boolean(state.document))}>{state.busy ? '처리 중' : '저장'}</button>
        </div>
      </form>
    </> : <>
      <div className="template-settings-heading"><h3 className="settings-panel-title">요약 템플릿</h3><button type="button" className="small with-icon" disabled={state.busy} onClick={() => flow.create()}><Plus size={14} aria-hidden="true" />추가</button></div>
      <p className="template-settings-description">앱과 에이전트가 함께 사용합니다.</p>
      <div className="settings-group template-settings-list">
        {state.items.map(item => <div className="template-settings-row" key={item.id}>
          <div className="template-settings-copy"><div className="template-settings-name"><strong>{item.name}</strong>{item.builtIn && <span>기본</span>}{item.customized && <span>수정됨</span>}</div><p title={item.description}>{item.description}</p></div>
          <button type="button" className="small" disabled={state.busy} aria-label={`${item.name} 편집`} onClick={() => { void flow.open(item.id); }}>편집</button>
        </div>)}
      </div>
      {!state.items.length && !state.busy && !state.error && <p className="template-settings-description">템플릿이 없습니다.</p>}
      <div className="template-settings-list-actions"><button type="button" className="small" disabled={state.busy} onClick={() => { void reload(); }}>새로고침</button></div>
    </>}
    {state.busy && <p className="inline-feedback" role="status"><span className="activity-spinner" aria-hidden="true" /> 처리 중</p>}
    {state.message && <p className="inline-feedback" role="status">{state.message}</p>}
    {state.error && <div className="template-settings-error"><p className="notice" role="alert">{state.error}</p><button type="button" className="small" disabled={state.busy} onClick={() => { void reload(); }}>다시 불러오기</button></div>}
  </div>;
}

export function mountTemplateSettings({ container, panel, bridge }) {
  const flow = createTemplateSettingsFlow({ bridge });
  panel.dataset.neroUiOwned = '';
  const root = createRoot(panel);
  let confirmation = null;
  let confirmPromise = null;
  let settle = null;
  let focusOrigin = null;
  const resolve = value => { const finish = settle; settle = null; confirmation = null; confirmPromise = null; render(); finish?.(value); };
  const confirm = (title, description, action, destructive = false) => {
    if (confirmPromise) return confirmPromise;
    focusOrigin = container.ownerDocument.activeElement;
    confirmation = { title, description, action, destructive, trigger: focusOrigin };
    confirmPromise = new Promise(done => { settle = done; });
    render();
    return confirmPromise;
  };
  const beforeLeave = async () => {
    if (confirmation || flow.getSnapshot().busy) return false;
    if (!flow.isDirty()) return true;
    if (!await confirm('수정 내용을 버릴까요?', '저장하지 않은 수정 내용이 사라집니다.', '버리기')) return false;
    flow.discard();
    return true;
  };
  const render = () => root.render(<><TemplateSettings flow={flow} confirm={confirm} beforeLeave={beforeLeave} /><ConfirmDialog request={confirmation} container={container} resolve={resolve} finalFocus={() => focusOrigin?.isConnected && container.open ? focusOrigin : false} /></>);
  const unbind = bindTemplateDialogKeys(container, { isOpen: () => Boolean(confirmation), cancel: () => resolve(false) });
  const close = () => { resolve(false); flow.setActive(false); };
  container.addEventListener('close', close);
  render();
  return {
    setActive: active => flow.setActive(active),
    beforeLeave,
    isConfirming: () => Boolean(confirmation),
    hide: close,
    dispose() { resolve(false); unbind(); container.removeEventListener('close', close); flow.dispose(); root.unmount(); },
  };
}
