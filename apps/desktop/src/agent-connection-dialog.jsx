import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { Copy, Check } from 'lucide-react';
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from '../../../design-system/components/ui/dialog.jsx';
import { bindConnectionDialogKeys, createAgentConnectionFlow } from './agent-connection-flow.js';

function ConnectionPrompt({ flow, container, finalFocus }) {
  const state = React.useSyncExternalStore(flow.subscribe, flow.getSnapshot);
  const copy = React.useRef(null);
  return <Dialog open={state.open} onOpenChange={open => { if (!open) flow.close(); }}>
    <DialogContent container={container} className="agent-prompt-dialog" data-nero-ui-owned="" initialFocus={copy} finalFocus={finalFocus}>
      <DialogHeader>
        <DialogTitle>연결 프롬프트</DialogTitle>
        <DialogDescription>사용하는 에이전트에 아래 프롬프트를 전달해 Nero를 연결합니다.</DialogDescription>
      </DialogHeader>
      <div className="agent-prompt-body">
        <div className="agent-prompt-code">
          <div className="agent-prompt-code-header">
            <span id="agent-prompt-label">연결 프롬프트</span>
            <button ref={copy} className="small with-icon" type="button" onClick={() => { void flow.copy(); }} disabled={state.copying}>
              {state.copied ? <Check size={15} aria-hidden="true" /> : <Copy size={15} aria-hidden="true" />}
              {state.copying ? '복사 중' : state.copied ? '복사됨' : '프롬프트 복사'}
            </button>
          </div>
          <pre aria-labelledby="agent-prompt-label" tabIndex={0}><code>{state.prompt}</code></pre>
        </div>
        <p className="agent-prompt-instruction">복사한 내용을 연결할 에이전트의 대화에 붙여넣으세요.</p>
        <p className="agent-prompt-feedback" role={state.error ? 'alert' : 'status'} hidden={!state.error && !state.copied}>{state.error || '프롬프트를 복사했습니다.'}</p>
      </div>
    </DialogContent>
  </Dialog>;
}

export function mountAgentConnectionDialog({ container, fallbackTrigger }) {
  const mount = container.ownerDocument.createElement('div');
  mount.dataset.agentPromptRoot = '';
  mount.dataset.neroUiOwned = '';
  container.append(mount);
  const root = createRoot(mount);
  const flow = createAgentConnectionFlow({ writeText: text => navigator.clipboard.writeText(text) });
  let origin = null, restoreFocus = true;
  const finalFocus = () => {
    if (!restoreFocus || !container.open) return false;
    const trigger = origin?.isConnected && !origin.hidden && !origin.disabled ? origin : fallbackTrigger();
    return trigger?.isConnected ? trigger : false;
  };
  root.render(<ConnectionPrompt flow={flow} container={container} finalFocus={finalFocus} />);
  const close = ({ restoreFocus: shouldRestore = true } = {}) => {
    restoreFocus = shouldRestore;
    flow.close();
  };
  bindConnectionDialogKeys(container, { isOpen: () => flow.getSnapshot().open, close });
  container.addEventListener('close', () => close({ restoreFocus: false }));
  return {
    open(info, trigger = container.ownerDocument.activeElement) {
      origin = trigger;
      restoreFocus = true;
      globalThis.NeroControls?.close?.();
      flow.open(info);
    },
    update: info => flow.update(info),
    close,
    isOpen: () => flow.getSnapshot().open,
  };
}
