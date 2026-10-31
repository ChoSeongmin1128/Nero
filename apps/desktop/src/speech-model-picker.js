export function mountSpeechModelPicker({ node, onSelect, onPrepare }) {
  const document = node.ownerDocument;
  const element = (tag, className, text) => {
    const result = document.createElement(tag);
    if (className) result.className = className;
    if (text) result.textContent = text;
    return result;
  };
  const group = element('fieldset', 'speech-model-picker');
  group.append(element('legend', 'settings-subtitle', '음성 처리 모델'));
  const choices = element('div', 'speech-model-choices');
  const status = element('p', 'muted type-caption');
  status.setAttribute('role', 'status');
  const error = element('p', 'notice type-caption');
  error.setAttribute('role', 'alert');
  const prepare = element('button', 'small', '모델 다운로드');
  prepare.type = 'button'; prepare.id = `${node.id}-prepare`;
  prepare.addEventListener('click', () => onPrepare?.());
  group.append(choices, status, error, prepare);
  node.replaceChildren(group);
  let current = '', order = '';
  const rows = new Map();

  return { update({ progress = {}, disabled = true, message = '', error: failure = '' } = {}) {
    const entries = Array.isArray(progress.speechChoices) ? progress.speechChoices : [];
    node.hidden = !entries.length;
    if (!entries.length) return;
    current = progress.speechEngine || '';
    const key = JSON.stringify(entries.map(choice => choice.id));
    if (key !== order) {
      rows.clear();
      for (const choice of entries) {
        const label = element('label', 'speech-model-choice');
        const input = element('input');
        input.type = 'radio'; input.name = `${node.id}-engine`; input.value = choice.id;
        input.id = `${node.id}-engine-${choice.id}`;
        const copy = element('span', 'speech-model-copy');
        const title = element('strong');
        const description = element('span', 'muted type-caption');
        const metadata = element('span', 'muted type-caption');
        copy.append(title, description, metadata); label.append(input, copy);
        input.addEventListener('change', () => {
          if (!input.checked || input.disabled || group.disabled) return;
          // Display only the native confirmed selection while a request is pending.
          for (const [id, row] of rows) row.input.checked = id === current;
          if (input.value !== current) onSelect(input.value);
        });
        rows.set(choice.id, { label, input, title, description, metadata });
      }
      choices.replaceChildren(...Array.from(rows.values(), row => row.label));
      order = key;
    }
    group.disabled = disabled;
    for (const choice of entries) {
      const row = rows.get(choice.id);
      row.input.checked = choice.id === current;
      row.input.disabled = choice.supported !== true;
      row.title.textContent = choice.id === 'nemotron' ? 'Nemo' : choice.id === 'whisperTurbo' ? 'Whisper' : choice.label;
      row.description.textContent = choice.id === 'nemotron' ? '기본' : choice.id === 'whisperTurbo' ? '다른 인식 결과가 필요할 때' : '';
      row.description.hidden = !row.description.textContent;
      row.metadata.textContent = choice.supported !== true ? '사용할 수 없음' : choice.ready ? '준비됨' : '다운로드 필요';
    }
    status.textContent = message; status.hidden = !message;
    error.textContent = failure; error.hidden = !failure;
    const selected = entries.find(choice => choice.id === current);
    prepare.hidden = !onPrepare || progress.modelReady === true || selected?.supported !== true;
  } };
}
