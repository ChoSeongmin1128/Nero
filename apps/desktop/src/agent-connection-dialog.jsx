import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { Copy, Check } from '../../../design-system/icons-react.jsx';
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from '../../../design-system/components/ui/dialog.jsx';
import { bindConnectionDialogKeys, createAgentConnectionFlow } from './agent-connection-flow.js';

function ConnectionPrompt({ flow, container, finalFocus }) {
  const state = React.useSyncExternalStore(flow.subscribe, flow.getSnapshot);
  const copy = React.useRef(null);
  const extension = state.mode === 'claude-extension';
  return <Dialog open={state.open} onOpenChange={open => { if (!open) flow.close(); }}>
    <DialogContent container={container} className="agent-prompt-dialog" data-connection-mode={state.mode} data-nero-ui-owned="" initialFocus={copy} finalFocus={finalFocus}>
      <DialogHeader>
        <DialogTitle>{extension ? 'Claude Desktop 연결' : 'MCP 직접 연결'}</DialogTitle>
        <DialogDescription>{extension ? 'Claude Desktop에서 Nero 확장을 설치하세요.' : '아래 내용을 사용할 AI 앱의 대화에 붙여넣어 Nero를 연결하세요.'}</DialogDescription>
      </DialogHeader>
      <div className="agent-prompt-body">
        {extension && <ol className="agent-extension-steps">
          <li>Claude Desktop의 설정에서 확장 → 고급 → 확장 설치를 엽니다.</li>
          <li>아래 파일 경로를 복사해 파일 선택 창에 붙여넣고 설치를 확인합니다.</li>
        </ol>}
        <div className="agent-prompt-code">
          <div className="agent-prompt-code-header">
            <span id="agent-prompt-label">{extension ? '설치 파일' : '연결 프롬프트'}</span>
            <button ref={copy} className="small with-icon" type="button" onClick={() => { void flow.copy(); }} disabled={state.copying}>
              {state.copied ? <Check size={15} aria-hidden="true" /> : <Copy size={15} aria-hidden="true" />}
              {state.copying ? '복사 중' : state.copied ? '복사됨' : extension ? '파일 경로 복사' : '프롬프트 복사'}
            </button>
          </div>
          <pre aria-labelledby="agent-prompt-label" tabIndex={0}><code>{state.prompt}</code></pre>
        </div>
        <p className="agent-prompt-feedback" role={state.error ? 'alert' : 'status'} hidden={!state.error && !state.copied}>{state.error || (extension ? '파일 경로를 복사했습니다.' : '프롬프트를 복사했습니다.')}</p>
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
    openClaudeExtension(path, trigger = container.ownerDocument.activeElement) {
      origin = trigger;
      restoreFocus = true;
      globalThis.NeroControls?.close?.();
      flow.openClaudeExtension(path);
    },
    update: info => flow.update(info),
    close,
    isOpen: () => flow.getSnapshot().open,
  };
}
