import { formatBytes } from './processing-state.js';
import { qwenResourceNotice, mossResourceNotice } from './speech-resource-notice.js';

// One-time offer to existing users. Only the buttons record a decision; closing with
// Escape leaves the offer for the next launch.
export function mountSpeechOfferDialog({ dialog, onAccept, onDecline }) {
  const document = dialog.ownerDocument;
  const element = (tag, className, text) => {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text) node.textContent = text;
    return node;
  };
  const title = element('h2', '', '기본 전사 모델 변경');
  title.id = 'speech-offer-title';
  const intro = element('p');
  const facts = element('ul', 'speech-offer-facts');
  const memoryText = element('p', 'notice type-caption');
  const memoryChoice = element('label', 'setup-choice');
  const memoryInput = element('input');
  memoryInput.type = 'checkbox'; memoryInput.id = 'speech-offer-memory';
  memoryChoice.append(memoryInput, element('span', '', '그래도 이 모델을 준비합니다'));
  const memory = element('div', 'speech-memory-warning');
  memory.append(memoryText, memoryChoice);
  const error = element('p', 'notice type-caption');
  error.setAttribute('role', 'alert');
  const keep = element('button', 'quiet', '지금은 유지');
  keep.type = 'button'; keep.dataset.action = 'decline-speech-offer';
  const accept = element('button', 'primary', '다운로드 후 전환');
  accept.type = 'button'; accept.dataset.action = 'accept-speech-offer';
  const footer = element('div', 'modal-footer');
  const row = element('div', 'row');
  row.append(keep, accept);
  footer.append(element('span'), row);
  dialog.replaceChildren(title, intro, facts, memory, error, footer);

  let current = null, shown = false, busy = false;
  const sync = () => {
    const warning = current?.memoryWarning || '';
    memory.hidden = !warning;
    accept.disabled = busy || (Boolean(warning) && !memoryInput.checked);
    keep.disabled = busy;
  };
  const respond = async (handler) => {
    busy = true; error.hidden = true; sync();
    try {
      await handler();
      dialog.close();
    } catch (failure) {
      error.textContent = failure?.message || '전환을 시작하지 못했습니다. 설정에서 다시 선택할 수 있습니다.';
      error.hidden = false;
    } finally {
      busy = false; sync();
    }
  };
  memoryInput.addEventListener('change', sync);
  accept.addEventListener('click', () => { if (!accept.disabled) void respond(onAccept); });
  keep.addEventListener('click', () => { if (!keep.disabled) void respond(onDecline); });

  return {
    update(offer, { blocked = false } = {}) {
      current = offer || null;
      if (!current) {
        if (dialog.open && !busy) dialog.close();
        return;
      }
      accept.textContent = current.downloadBytes > 0 ? '다운로드 후 전환' : '이 모델로 전환';
      intro.textContent = `${current.title.split(' · ')[0]} 모델로 전환합니다. 녹음 종료 후 전사하며 녹음 중 미리보기는 제공하지 않습니다.`;
      const items = [
        current.downloadBytes > 0 ? `받을 크기: 약 ${formatBytes(current.downloadBytes)}. 받는 동안에는 지금 모델을 계속 사용합니다.` : '필요한 모델이 이미 준비되어 있습니다.',
        current.engine === 'qwenKo' ? qwenResourceNotice : mossResourceNotice,
        '기존 회의의 음성 기록은 바뀌지 않습니다.',
        '설정 > 음성 인식 > 전사 모델에서 언제든 바꿀 수 있습니다.',
      ];
      facts.replaceChildren(...items.map(text => element('li', '', text)));
      memoryText.textContent = current.memoryWarning || '';
      sync();
      if (!shown && !blocked && !dialog.open) {
        shown = true;
        dialog.showModal();
        accept.disabled ? keep.focus() : accept.focus();
      }
    },
  };
}
