import { formatBytes, memoryBlocked, memoryWarningFor, modelPreparation, speakerNeedsPreparation, speechModelKinds } from './processing-state.js';
import { bindPreparationProgress } from './model-preparation-progress.js';
import { qwenResourceNotice, mossResourceNotice } from './speech-resource-notice.js';

const titles = {
  moss: 'MOSS',
  nemotron: 'Nemotron 3.5',
  qwenKo: 'Qwen3-ASR 한국어 (실험적)',
  whisperTurbo: 'Whisper large-v3-turbo',
  whisperLargeV3: 'Whisper large-v3',
  appleSpeech: 'macOS 기본 전사',
};
const summaries = {
  moss: '녹음 후 구간별 전사, 실패 구간만 다시 시도',
  nemotron: '녹음과 함께 음성 기록 준비',
  qwenKo: '직접 선택 시 준비, CPU/GPU와 메모리 사용량이 많을 수 있음',
  whisperTurbo: '속도와 정확도 균형',
  whisperLargeV3: '정확도 우선, 처리 시간 증가',
  appleSpeech: '별도 전사 모델 없음, 시스템 한국어 언어 데이터가 필요할 수 있음',
};
export const speechTitle = choice => titles[choice?.id] || choice?.label || '';

function write(node, key, value) {
  if (node[key] !== value) node[key] = value;
}

// Shared by initial setup and Settings. Selecting a model that is not downloaded while
// another one works asks first, then downloads while the current model stays in use.
export function mountSpeechModelPicker({ node, speakerNode = null, legend = '음성 처리 모델', onSelect, onPrepare, onSpeakerChange, onAcknowledge, onCancelJob, onCancelSwitch, onRetryCleanup }) {
  const document = node.ownerDocument;
  const element = (tag, className, text) => {
    const result = document.createElement(tag);
    if (className) result.className = className;
    if (text) result.textContent = text;
    return result;
  };
  const smallButton = (text, handler) => {
    const result = element('button', 'small', text);
    result.type = 'button';
    result.addEventListener('click', () => { if (!result.disabled && !group.disabled) handler(); });
    return result;
  };
  const group = element('fieldset', 'speech-model-picker');
  if (legend) group.append(element('legend', 'settings-subtitle', legend));
  else group.setAttribute('aria-label', '음성 처리 모델');
  const choices = element('div', 'speech-model-choices');
  const status = element('p', 'muted type-caption');
  status.setAttribute('role', 'status');
  const error = element('p', 'notice type-caption');
  error.setAttribute('role', 'alert');

  const stagedPanel = element('div', 'speech-model-confirm');
  const stagedText = element('p', 'type-caption');
  const memoryText = element('p', 'notice type-caption');
  const memoryChoice = element('label', 'setup-choice');
  const memoryInput = element('input');
  memoryInput.type = 'checkbox'; memoryInput.id = `${node.id}-memory-acknowledged`;
  memoryChoice.append(memoryInput, element('span', '', '그래도 이 모델을 준비합니다'));
  const memory = element('div', 'speech-memory-warning');
  memory.append(memoryText, memoryChoice);
  // The request is native's now; show only the confirmed selection until it answers.
  const confirm = smallButton('다운로드 후 전환', () => { const target = staged; staged = ''; render(); if (target) onSelect(target); });
  confirm.id = `${node.id}-confirm`;
  const dismiss = smallButton('취소', () => { staged = ''; render(); });
  const stagedActions = element('div', 'speech-model-actions');
  stagedActions.append(confirm, dismiss);
  stagedPanel.append(stagedText, stagedActions);

  const progressRegion = element('div', 'setup-progress-region speech-model-progress');
  const bar = element('progress');
  bar.setAttribute('aria-label', '음성 처리 모델 다운로드 진행률');
  const meta = element('div', 'setup-progress-meta');
  const live = element('span', 'processing-inline');
  live.setAttribute('role', 'status');
  const spinner = element('span', 'activity-spinner');
  spinner.setAttribute('aria-hidden', 'true');
  const stage = element('span');
  live.append(spinner, stage);
  const size = element('span', 'muted type-caption');
  const cancel = smallButton('취소', () => { switching ? onCancelSwitch?.() : onCancelJob?.(cancel.dataset.jobId); });
  cancel.id = `${node.id}-cancel`;
  meta.append(live, size, cancel);
  progressRegion.append(bar, meta);
  const progress = bindPreparationProgress({ region: progressRegion, bar, spinner, stage, size, cancel });

  const failure = element('div', 'speech-model-failure');
  const failureText = element('p', 'notice type-caption');
  failureText.setAttribute('role', 'alert');
  const retry = smallButton('다시 시도', () => { switching ? onSelect(switching) : onPrepare?.(); });
  const forget = smallButton('전환 취소', () => onCancelSwitch?.());
  const failureActions = element('div', 'speech-model-actions');
  failureActions.append(retry, forget);
  failure.append(failureText, failureActions);

  const cleanup = element('div', 'speech-model-cleanup');
  const cleanupText = element('p', 'type-caption');
  cleanupText.setAttribute('role', 'status');
  const cleanupRetry = smallButton('다시 정리', () => onRetryCleanup?.());
  cleanupRetry.id = `${node.id}-cleanup-retry`;
  cleanup.append(cleanupText, cleanupRetry);

  const prepare = smallButton('모델 다운로드', () => onPrepare?.());
  prepare.id = `${node.id}-prepare`;
  const speakerGroup = element('div', 'settings-group speech-speaker-options');
  const speaker = element('label', 'settings-row speech-speaker-setting');
  const speakerCopy = element('span', 'speech-speaker-copy');
  speakerCopy.append(element('span', '', '화자 구분'));
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
  memoryInput.addEventListener('change', () => {
    if (memoryInput.disabled || group.disabled || !warnedEngine) return;
    onAcknowledge?.(warnedEngine, memoryInput.checked);
  });
  speakerGroup.append(speaker);
  group.append(choices, ...(speakerNode ? [] : [speakerGroup]), stagedPanel, memory, progressRegion, failure, cleanup, status, error, prepare);
  node.replaceChildren(group);
  if (speakerNode) speakerNode.replaceChildren(speakerGroup);

  let order = '', staged = '', switching = '', warnedEngine = '', last = {};
  const rows = new Map();

  function render() { update(last); }

  function update(options = {}) {
    last = options;
    const { progress: state = {}, disabled = true, locked = false, message = '', error: failureMessage = '', acknowledged = new Set() } = options;
    const entries = Array.isArray(state.speechChoices) ? state.speechChoices.filter(choice => choice.id !== 'appleSpeech' || choice.supported === true || choice.id === state.speechEngine) : [];
    write(node, 'hidden', !entries.length);
    write(speakerGroup, 'hidden', !entries.length || !onSpeakerChange);
    if (!entries.length) return;
    const active = state.speechEngine || '';
    switching = state.speechSwitch?.target || '';
    const selected = switching || active;
    if (staged && (staged === active || staged === switching || !entries.some(choice => choice.id === staged))) staged = '';
    const key = JSON.stringify(entries.map(choice => choice.id));
    if (key !== order) {
      rows.clear();
      for (const choice of entries) {
        const label = element('label', 'speech-model-choice');
        const input = element('input');
        input.type = 'radio'; input.name = `${node.id}-engine`; input.value = choice.id;
        input.id = `${node.id}-engine-${choice.id}`;
        const copy = element('span', 'speech-model-copy');
        const title = element('strong', 'speech-model-title');
        const description = element('span', 'muted type-caption speech-model-description');
        const reason = element('span', 'muted type-caption speech-model-reason');
        const metadata = element('span', 'muted type-caption speech-model-metadata');
        const resourceNotice = ['qwenKo', 'moss'].includes(choice.id) ? element('span', 'muted type-caption speech-resource-notice', choice.id === 'moss' ? mossResourceNotice : qwenResourceNotice) : null;
        copy.append(title, description);
        label.append(input, copy, metadata, reason);
        if (resourceNotice) label.append(resourceNotice);
        input.addEventListener('change', () => {
          if (!input.checked || input.disabled || group.disabled) return;
          const current = last.progress || {};
          const choice = current.speechChoices?.find(item => item.id === input.value);
          const now = current.speechSwitch?.target || current.speechEngine;
          // Display only the native confirmed selection while a request is pending.
          for (const [id, row] of rows) row.input.checked = id === now;
          if (!choice || input.value === now) { staged = ''; render(); return; }
          if (current.modelReady !== true) { staged = ''; onSelect(input.value); return; }
          staged = input.value;
          render();
        });
        rows.set(choice.id, { label, input, title, description, reason, metadata, resourceNotice });
      }
      choices.replaceChildren(...Array.from(rows.values(), row => row.label));
      order = key;
    }
    write(group, 'disabled', disabled);
    const preparation = modelPreparation(state, speechModelKinds);
    const busy = ['queued', 'running', 'cancelling'].includes(preparation.state);
    for (const choice of entries) {
      const row = rows.get(choice.id);
      write(row.input, 'checked', choice.id === (staged || selected));
      write(row.input, 'disabled', choice.supported !== true || locked);
      write(row.title, 'textContent', `${speechTitle(choice)}${choice.recommended ? ' (추천)' : ''}`);
      write(row.description, 'textContent', summaries[choice.id] || choice.description || '');
      write(row.description, 'hidden', !row.description.textContent);
      const contextual = [active, staged, switching].includes(choice.id);
      const hardwareReason = choice.recommended && contextual && choice.id === 'whisperLargeV3' ? choice.recommendationReason || '' : '';
      write(row.reason, 'textContent', hardwareReason);
      write(row.reason, 'hidden', !row.reason.textContent);
      if (row.resourceNotice) write(row.resourceNotice, 'hidden', !contextual);
      const metadata = choice.supported !== true ? '사용할 수 없음'
        : choice.id === switching ? (busy ? '준비 중' : '전환 대기')
          : choice.id === active && choice.ready ? '사용 중'
            : choice.ready ? '준비됨' : choice.id === 'appleSpeech' ? '언어 데이터 준비 필요' : '다운로드 필요';
      write(row.metadata, 'textContent', metadata);
      write(row.metadata, 'hidden', !metadata);
    }
    speaker.hidden = !onSpeakerChange;
    speakerEnabled = state.speakerEnabled !== false;
    write(speakerInput, 'checked', speakerEnabled);
    write(speakerInput, 'disabled', disabled || locked);
    const speakerText = speakerEnabled && state.speakerSupported === false ? '이 기기에서는 사용할 수 없음'
      : speakerEnabled && state.speakerReady === false ? '준비 필요' : '';
    write(speakerStatus, 'textContent', speakerText);
    write(speakerStatus, 'hidden', !speakerText);

    const stagedChoice = entries.find(choice => choice.id === staged);
    write(stagedPanel, 'hidden', !stagedChoice);
    if (stagedChoice) {
      const speechSize = stagedChoice.modelBytes ? ` 전사 모델 전체 크기: ${formatBytes(stagedChoice.modelBytes)}. 이미 준비된 파일은 다시 받지 않습니다.` : '';
      const speakerSize = state.speakerEnabled !== false && state.speakerSupported !== false && state.speakerReady === false && state.speakerBytes > 0
        ? ` 화자 구분 모델 ${formatBytes(state.speakerBytes)}는 별도로 준비합니다.` : '';
      write(confirm, 'textContent', stagedChoice.ready ? '이 모델로 전환' : '다운로드 후 전환');
      write(stagedText, 'textContent', stagedChoice.ready
        ? `${speechTitle(stagedChoice)} 모델로 전환합니다. 이후 새 녹음과 변환에 적용하며, 기존 회의는 다시 처리하지 않습니다. 전환이 끝나면 사용하지 않는 이전 전사 모델 파일을 정리합니다.`
        : `${speechTitle(stagedChoice)} 모델을 준비한 뒤 전환합니다. 준비하는 동안에는 지금 모델을 계속 사용합니다.${speechSize}${speakerSize} 전환이 끝나면 사용하지 않는 이전 전사 모델 파일을 정리합니다.`);
    }
    const warningChoice = stagedChoice || entries.find(choice => choice.id === active);
    const warning = memoryWarningFor(warningChoice);
    warnedEngine = warning ? warningChoice.id : '';
    write(memory, 'hidden', !warning);
    write(memoryText, 'textContent', warning);
    write(memoryInput, 'checked', Boolean(warning) && acknowledged.has(warningChoice.id));
    write(memoryInput, 'disabled', disabled);
    write(confirm, 'disabled', Boolean(stagedChoice) && memoryBlocked(stagedChoice, acknowledged));

    progress.update(preparation);
    write(cancel, 'textContent', preparation.state === 'cancelling' ? '취소 중' : switching ? '전환 취소' : '취소');
    const switchError = state.speechSwitch?.error || '';
    const failed = Boolean(switchError) || ['failed', 'cancelled'].includes(preparation.state);
    write(failure, 'hidden', !failed);
    if (failed) write(failureText, 'textContent', switchError ? `전환하지 못했습니다. ${switchError}` : preparation.message);
    write(forget, 'hidden', !switching);
    write(retry, 'hidden', !switching && !onPrepare);

    const cleanupState = state.speechCleanup;
    write(cleanup, 'hidden', !cleanupState?.pending);
    const cleanupError = cleanupState?.error || '';
    write(cleanupText, 'className', cleanupError ? 'notice type-caption' : 'muted type-caption');
    cleanupText.setAttribute('role', cleanupError ? 'alert' : 'status');
    write(cleanupText, 'textContent', !cleanupState?.pending ? '' : cleanupError
      ? `이전 전사 모델 파일을 정리하지 못했습니다. ${cleanupError}`
      : '이전 전사 모델 파일 정리 대기 중. 다른 전사 작업에서 사용하는 파일은 정리를 보류합니다.');
    write(cleanupRetry, 'hidden', !cleanupError || !onRetryCleanup);
    write(cleanupRetry, 'disabled', disabled || locked);

    const choiceStatus = message || (active && !entries.some(choice => choice.id === active) ? `현재 ${titles[active] || active} 사용 중` : '');
    write(status, 'textContent', choiceStatus);
    write(status, 'hidden', !choiceStatus);
    write(error, 'textContent', failureMessage);
    write(error, 'hidden', !failureMessage);
    const current = entries.find(choice => choice.id === active);
    write(prepare, 'textContent', current?.id === 'appleSpeech' ? '준비' : '모델 다운로드');
    write(prepare, 'hidden', !onPrepare || busy || failed || Boolean(switching) || Boolean(stagedChoice)
      || (state.modelReady === true && !speakerNeedsPreparation(state)) || current?.supported !== true);
    write(prepare, 'disabled', memoryBlocked(current, acknowledged));
  }
  return { update };
}

