export function mountSpeechModelPicker({ node, onSelect, onPrepare, onSpeakerChange }) {
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
  const speaker = element('label', 'settings-row speech-speaker-setting');
  const speakerCopy = element('span', 'speech-speaker-copy');
  speakerCopy.append(element('strong', '', '화자 구분'));
  const speakerStatus = element('span', 'muted type-caption');
  speakerCopy.append(speakerStatus);
  const speakerInput = element('input');
  speakerInput.type = 'checkbox'; speakerInput.id = `${node.id}-speaker-enabled`;
  speakerInput.setAttribute('role', 'switch');
  speakerInput.setAttribute('aria-label', '화자 구분');
  speaker.append(speakerCopy, speakerInput);
  let speakerEnabled = true;
  speakerInput.addEventListener('change', () => {
    if (speakerInput.disabled || group.disabled || !onSpeakerChange) return;
    const requested = speakerInput.checked;
    speakerInput.checked = speakerEnabled;
    if (requested !== speakerEnabled) onSpeakerChange(requested);
  });
  group.append(choices, speaker, status, error, prepare);
  node.replaceChildren(group);
  let current = '', order = '';
  const rows = new Map();

  return { update({ progress = {}, disabled = true, message = '', error: failure = '' } = {}) {
    const entries = Array.isArray(progress.speechChoices) ? progress.speechChoices.filter(choice => choice.id !== 'appleSpeech' || choice.supported === true || choice.id === progress.speechEngine) : [];
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
      row.title.textContent = choice.id === 'nemotron' ? 'Nemotron 3.5' : choice.id === 'whisperTurbo' ? 'Whisper large-v3-turbo' : choice.id === 'appleSpeech' ? 'macOS 기본 전사' : choice.label;
      if (choice.id === 'whisperLargeV3') row.title.textContent = 'Whisper large-v3';
      if (choice.recommended) row.title.textContent += ' (추천)';
      row.description.textContent = choice.recommended && choice.recommendationReason ? choice.recommendationReason : choice.id === 'nemotron' ? '기본' : choice.id === 'whisperTurbo' ? '속도와 정확도 균형' : choice.id === 'whisperLargeV3' ? '정확도 우선, 처리 시간 증가' : choice.id === 'appleSpeech' ? 'macOS의 음성 인식 사용' : '';
      row.description.hidden = !row.description.textContent;
      row.metadata.textContent = choice.supported !== true ? '사용할 수 없음' : choice.ready ? '' : choice.id === 'appleSpeech' ? '언어 데이터 준비 필요' : '다운로드 필요';
      row.metadata.hidden = !row.metadata.textContent;
    }
    speaker.hidden = !onSpeakerChange;
    speakerEnabled = progress.speakerEnabled !== false;
    speakerInput.checked = speakerEnabled;
    speakerInput.disabled = disabled;
    speakerStatus.textContent = speakerEnabled && progress.speakerSupported === false ? '이 기기에서는 사용할 수 없음'
      : speakerEnabled && progress.speakerReady === false ? '준비 필요' : '';
    speakerStatus.hidden = !speakerStatus.textContent;
    status.textContent = message; status.hidden = !message;
    error.textContent = failure; error.hidden = !failure;
    const selected = entries.find(choice => choice.id === current);
    const speakerNeedsPreparation = speakerEnabled && progress.speakerSupported === true && progress.speakerReady === false;
    prepare.textContent = selected?.id === 'appleSpeech' ? '준비' : '모델 다운로드';
    prepare.hidden = !onPrepare || (progress.modelReady === true && !speakerNeedsPreparation) || selected?.supported !== true;
  } };
}
