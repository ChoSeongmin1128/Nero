import { mountBrandIcons } from './brand-icons.js';
import { mountSidebarViews } from './sidebar-views.js';
import { snapshotSignatures, compareSnapshotSignatures } from './snapshot-update.js';
import { createViewHistory } from './view-history.js';
import { mountWindowChrome } from './window-chrome.js';
import { isTauri } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { mountConfirmDialog } from '../../../design-system/components/ui/confirm-dialog.jsx';
import { requestAudioPlayback } from './audio-playback.js';
import { mountAppUpdateRecovery } from './app-update-recovery.jsx';
import { mountAppUpdateView } from './app-update-view.js';
import './app-update-view.css';
import { mountPeopleSettings, mountMeetingAttendees } from './people-ui.jsx';
import { bindDialogDismiss } from './dialog-dismiss.js';
import { createIntegrationMonitor } from './integration-notifications.js';
import { mountNotificationCenter } from './notification-center.js';
import './notification-center.css';
import { returnFocusTarget } from './focus-return.js';
import { mountCalendarMeetingPicker, mountCalendarMeetingReference } from './calendar-meeting-picker.jsx';
import { mountSummaryTemplatePicker } from './summary-template-picker.js';
import { mountCalendarSettings } from './calendar-settings.jsx';
import { recordingQuality } from './recording-quality.js';
import { mountDocumentActions } from './document-actions.jsx';
import '../../../design-system/react-controls.jsx';
import * as bridge from './bridge.js';
import { ui, icon, escape, button, duration, recordingStatus, statusMarkup, sessionControls, applySystemAppearance } from './ui.js';
import { processingFeedback, canRetryProcessing, canDismissProcessing } from './processing-feedback.js';
import { processingJobs, pendingJob, processingResults, latestMeetingProcessing, processingNotices, modelPreparation, memoryBlocked, speakerNeedsPreparation, speechModelKinds, summaryModelKinds } from './processing-state.js';
import { mountLibrary, fullDate, meetingDay } from './library-ui.js';
import { mountSpeakers } from './speakers-ui.js';
import { createMeetingSearch, mountDocumentSearch } from './meeting-search.js';
import { mountTranscriptActions, sameTranscriptVersion } from './transcript-actions.js';
import { liveTranscriptRows } from './live-transcript.js';
import { meetingResultPreview } from './meeting-result-preview.js';
import { mountMeetingDetail } from './meeting-detail-ui.js';
import { mountAiChat } from './ai-chat-ui.jsx';
import { mountAiChatLibrary } from './ai-chat-library.jsx';
import { mountAiChatSidebar } from './ai-chat-sidebar.jsx';
import { mountMeetingContext } from './meeting-context-ui.jsx';
import * as aiChatBridge from './ai-chat-bridge.js';
import { mountSummaryEvidence } from './summary-evidence.js';
import { mountAgentConnectionDialog } from './agent-connection-dialog.jsx';
import { mountAgentSettings } from './agent-settings.js';
import { mountTerminologySettings } from './terminology-settings.js';
import { mountTemplateSettings } from './template-settings.jsx';
import { mountSpeechModelPicker, speechTitle } from './speech-model-picker.js';
import { bindPreparationProgress } from './model-preparation-progress.js';
import { mountSpeechOfferDialog } from './speech-offer-dialog.js';
import { mountDropImport, uploadDroppedFile } from './drop-import.js';
import './speech-model-picker.css';

mountBrandIcons(document);
applySystemAppearance();
if (document.documentElement.dataset.windowChrome === 'macos-overlay' && isTauri()) {
  const { dispose } = mountWindowChrome({ root: document.documentElement, nativeWindow: getCurrentWindow() });
  window.addEventListener('pagehide', dispose, { once: true });
}
const byId = (id) => document.getElementById(id);
const summaryTemplatePicker = mountSummaryTemplatePicker({ node: byId('summary-template'), bridge, onAvailabilityChange: () => queueMicrotask(() => render()) });
void summaryTemplatePicker.reload();
const settingsPanels = [...byId('settings-dialog').querySelectorAll('.app-settings-tabs [role="tab"][data-settings-panel]')].map(tab => tab.dataset.settingsPanel);
const agentConnectionDialog = mountAgentConnectionDialog({ container: byId('settings-dialog'), fallbackTrigger: () => byId('settings-agent-manual') });
const templateSettings = mountTemplateSettings({ container: byId('settings-dialog'), panel: byId('settings-templates-panel'), bridge });
const peopleSettings = mountPeopleSettings({ container: byId('settings-dialog'), panel: byId('settings-people-panel'), bridge });
const meetingAttendees = mountMeetingAttendees({ container: document.body, bridge, onChanged: async () => { acceptSnapshot(await bridge.readSnapshot()); if (state.selectedId) await loadSummary(state.selectedId); } });
let integrationNotices = [];
let appUpdateNotices = [];
const integrationMonitor = createIntegrationMonitor({ bridge, onChange(notices) { integrationNotices = notices; render(); } });
const agentSettings = mountAgentSettings({ byId, bridge, dialog: agentConnectionDialog, integrationMonitor });
const terminologySettings = mountTerminologySettings({ byId, bridge, isProcessing: () => processingActive(), onChange: () => renderSettings() });
const calendarSettings = mountCalendarSettings({ panel: byId('calendar-settings'), bridge, onSnapshot: value => integrationMonitor.acceptCalendar(value) });
const calendarReference = mountCalendarMeetingReference({ container: byId('calendar-meeting-reference'), bridge });
const calendarPicker = mountCalendarMeetingPicker({ container: document.body, bridge,
  onPrepared: async meeting => {
    acceptSnapshot(await bridge.readSnapshot());
    if (meeting.deletedAt != null) { await library.show('trash'); throw Error('휴지통에 있는 회의입니다. 복원 후 열어 주세요.'); }
    await selectMeeting(meeting.id);
  },
  onConfigure: () => openSettings('agent', 'create-calendar-meeting'),
});
const microphonePreferenceKey = 'nero.recording.microphone.v1';
const recorderPreferenceKey = 'nero.recording.autoRecorder.v1';
function readAutoRecorder() { try { return globalThis.localStorage?.getItem(recorderPreferenceKey) !== 'false'; } catch { return true; } }
function readMicrophonePreference() {
  try {
    const value = JSON.parse(globalThis.localStorage?.getItem(microphonePreferenceKey) || 'null');
    return value && (value.id === null || typeof value.id === 'string') ? { id: value.id, name: typeof value.name === 'string' ? value.name : '' } : { id: null, name: '' };
  } catch { return { id: null, name: '' }; }
}
const microphonePreference = readMicrophonePreference();
let sidebarViews = null, viewHistory = null;
const state = { snapshot: { meetings: [], folders: [], session: null }, libraryOpen: false, libraryFilter: 'all', librarySelection: new Set(), libraryBusy: false, librarySearchIds: null, librarySearchHits: [], pendingSearchIntent: null, setupReturnId: null, librarySearchPending: false, librarySearchUnavailable: 0, selectedId: null, devices: [], devicesLoaded: false, deviceId: microphonePreference.id, deviceName: microphonePreference.name, settingsPanel: 'appearance', summaryFeatureError: '', settingsKey: '', settingsError: '', settingsReturnId: 'settings', setupFromSettings: false, autoRecorder: readAutoRecorder(), recorderSettingBusy: false, snapshotSignatures: null, processingKey: '', processingCancellingId: null, transcriptEditing: null, audioRequest: 0, audioLoading: null, dateSaving: null, dateKey: '', dateError: null, lastError: null, errorIsSave: false, connected: false, busy: false, composing: false, transcriptComposing: false, editLocks: 0, draft: null, saveTimer: null, saveQueue: Promise.resolve(), controlsKey: '', audioKey: '', processing: { status: 'idle', supported: false, modelReady: false, localSummaryEnabled: false }, transcriptDraft: null, transcriptKey: '', transcriptLoadId: 0, transcriptSaveQueue: Promise.resolve(), summary: null, summaryKey: '', summaryLoadId: 0, summaryLoaded: false, transcriptLoaded: false, summaryLoadError: false, transcriptLoadError: false, setupRequested: false };
// Low-memory model preparation needs an explicit acknowledgement; it is not persisted.
const speechMemoryAcknowledged = new Set();
function acknowledgeSpeechMemory(engine, checked) {
  if (checked) speechMemoryAcknowledged.add(engine); else speechMemoryAcknowledged.delete(engine);
  render();
}
function speechMemoryBlocked() {
  return memoryBlocked(state.processing.speechChoices?.find(choice => choice.id === state.processing.speechEngine), speechMemoryAcknowledged);
}
// A locally requested cancel shows as cancelling before the native job reports it.
function preparationView(kinds) {
  const view = modelPreparation(state.processing, kinds);
  const requested = view.jobId && (state.processingCancellingId === view.jobId || processingActions.get(view.jobId) === 'cancel');
  return requested && ['queued', 'running'].includes(view.state) ? { ...view, state: 'cancelling', stage: '취소 중', cancellable: false } : view;
}
const pickerOptions = { onSelect: selectSpeechEngine, onSpeakerChange: selectSpeakerEnabled, onAcknowledge: acknowledgeSpeechMemory,
  onCancelJob: id => { if (id) processingAction('cancel', id).catch(showError); }, onCancelSwitch: cancelSpeechSwitch };
const speechPickers = [
  mountSpeechModelPicker({ node: byId('setup-speech-picker'), ...pickerOptions }),
  mountSpeechModelPicker({ node: byId('settings-speech-picker'), onPrepare: prepareSpeechModels, ...pickerOptions }),
];
const setupProgress = bindPreparationProgress({ region: byId('setup-progress-region'), bar: byId('setup-progress'), spinner: byId('setup-spinner'),
  stage: byId('setup-progress-stage'), size: byId('setup-progress-bytes'), cancel: byId('setup-cancel') });
const summaryProgress = bindPreparationProgress({ region: byId('settings-summary-progress'), bar: byId('settings-feature-progress'),
  stage: byId('settings-feature-stage'), size: byId('settings-feature-size'), cancel: byId('settings-feature-cancel') });
let speechSelectionError = '', speechNotice = '';
const appUpdateRecovery = mountAppUpdateRecovery({ document, quit: () => appUpdateView.quitForRecovery() });
const appUpdateView = mountAppUpdateView({ container: byId('app-update'), bridge,
  onRecovery: status => appUpdateRecovery.update(status),
  saveEdits: () => prepareAppLeave(() => true),
  onChange(notices) { appUpdateNotices = notices; render(); },
  onInstalling(locked) {
    state.editLocks += locked ? 1 : -1;
    for (const id of ['meeting-title', 'meeting-notes']) byId(id).readOnly = state.editLocks > 0;
    render();
  },
});
let pendingQuitSessionId = null;
let selectionRequest = 0;
function beginNavigation(history = false) { if (!history) viewHistory?.cancel(); const request = ++selectionRequest; return () => request === selectionRequest; }
let speakerReplay = null;
let summaryEvidence = null;
let aiChat = null, aiLibrary = null, aiSidebar = null, meetingContext = null;
let lastLiveTranscript = null;
const processingActions = new Map();
const processingErrors = new Map();
let processingFocus = null;
function setHTML(id, html) { const node = byId(id); if (node.innerHTML !== html) node.innerHTML = html; }
function setText(id, text) { const node = byId(id); if (node.textContent !== text) node.textContent = text; }
const currentMeeting = () => state.snapshot.meetings.find((meeting) => meeting.id === state.selectedId);
const activeSession = () => ['recording', 'paused', 'finalizing'].includes(state.snapshot.session?.state) ? state.snapshot.session : null;
const stateFor = (meeting) => activeSession()?.meetingId === meeting.id ? activeSession().state : meeting.status;

function paintIconButton(node, name = node.dataset.uiIcon, label = node.getAttribute('aria-label')) {
  node.innerHTML = `${icon(name)}<span class="sr-only">${escape(label)}</span>`;
  node.setAttribute('aria-label', label);
  node.setAttribute('title', label);
}
document.querySelectorAll('[data-ui-icon]').forEach(node => paintIconButton(node));
document.querySelectorAll('[data-detail-icon]').forEach(node => { node.innerHTML = icon(node.dataset.detailIcon); });

byId('welcome-create').innerHTML = `${icon('plus')}<span>새 회의</span>`;
byId('library-create').innerHTML = `${icon('plus')}<span>새 회의 만들기</span>`;
document.querySelector('.welcome-icon').innerHTML = icon('notebook-pen');
byId('recording-icon').innerHTML = icon('mic');

function showError(error, save = false) {
  state.lastError = error;
  state.errorIsSave = save;
  byId('error-message').textContent = error instanceof Error ? error.message : String(error);
  byId('error').hidden = false;
  byId('retry-save').hidden = !save;
}

function clearError(saveOnly = false) { if (saveOnly && !state.errorIsSave) return; byId('error').hidden = true; state.errorIsSave = false; state.lastError = null; }

function refreshMeetingSearch() { if (byId('search').value.trim()) meetingSearch.update(byId('search').value, state.libraryFilter === 'trash'); }

function renderSearchStatus() {
  const query = byId('search').value.trim();
  const status = state.librarySearchPending ? '회의 내용 검색 중' : state.librarySearchUnavailable ? '일부 회의 내용을 검색하지 못했습니다.' : '';
  const text = [query ? `검색: ${query}` : '', status].filter(Boolean).join(' - ');
  setHTML('library-search-message', state.librarySearchPending ? '<span class="activity-spinner" aria-hidden="true"></span> ' + escape(text) : escape(text));
  byId('library-search-clear').hidden = !query;
  byId('library-search-status').hidden = !text;
}

function renderSaveState() {
  const saving = Boolean(state.draft?.saving);
  byId('save-status').textContent = saving ? '저장 중' : '';
  byId('save-spinner').hidden = !saving;
  byId('save-status').hidden = !saving;
}

function renderRecording(meeting) {
  const session = activeSession();
  const hasMicrophone = state.devicesLoaded && !microphoneIssue();
  const importing = Boolean(pendingJob(state.processing, { kind: 'import' }));
  const ownSession = session?.meetingId === meeting.id ? session : null;
  const phase = stateFor(meeting);
  setHTML('meeting-status', statusMarkup(phase, { imported: Boolean(meeting.audio) && meeting.recordedAt == null }));
  byId('recording-time').hidden = !ownSession;
  setText('recording-time', duration(ownSession?.elapsedMs));
  const key = JSON.stringify([meeting.id, Boolean(meeting.audio), session?.id, session?.state, session?.meetingId, state.devicesLoaded, hasMicrophone, importing, Boolean(modelsReady())]);
  if (key !== state.controlsKey) {
    state.controlsKey = key;
    const controls = byId('recording-controls');
    if (ownSession) controls.innerHTML = sessionControls(ownSession, false) + (['recording', 'paused'].includes(ownSession.state) ? ui.button({ html: icon('picture-in-picture-2'), action: 'show-recorder', className: 'quiet icon-button', attrs: { 'aria-label': '녹음 제어창 표시', title: '녹음 제어창 표시' } }) : '');
    else if (session) controls.innerHTML = button('녹음 중인 회의 열기', 'open-active', 'mic', '', false);
    else if (meeting.audio) controls.innerHTML = '';
    else if (!modelsReady()) controls.innerHTML = button('모델 준비', 'setup', null, 'primary', false);
    else controls.innerHTML = button('녹음 시작', 'start', 'mic', 'primary', !hasMicrophone || importing) + (state.devicesLoaded && !hasMicrophone ? button('녹음 설정', 'recording-settings', 'settings-2', 'quiet small', false) : '');
    controls.hidden = !controls.innerHTML;
    controls.querySelectorAll('button').forEach(node => { node.dataset.unavailable = String(node.disabled); });
  }
  byId('recording-controls').querySelectorAll('button').forEach(node => { node.disabled = state.busy || node.dataset.unavailable === 'true'; });
  const quality = ownSession?.captureQuality || meeting.captureQuality;
  const message = ownSession?.error || meeting.error || (!quality && (ownSession?.warning || meeting.warning)) || (phase === 'interrupted' ? '녹음이 중단되었습니다. 저장된 음성을 확인하세요.' : session && !ownSession ? '다른 회의에서 녹음하고 있습니다.' : !meeting.audio && !ownSession && modelsReady() && state.devicesLoaded ? microphoneIssue() : '');
  byId('recording-message').textContent = message;
  byId('recording-message').hidden = !message;
  const review = recordingQuality(quality, ownSession?.sampleRate || meeting.audio?.sampleRate, ownSession?.savedFrames ?? meeting.audio?.frames);
  const qualityNode = byId('recording-quality');
  qualityNode.hidden = !review.visible;
  qualityNode.dataset.severity = review.severity;
  const qualityKey = JSON.stringify([meeting.id, review, Boolean(ownSession), Boolean(meeting.audio)]);
  if (qualityNode.dataset.key !== qualityKey) {
    qualityNode.dataset.key = qualityKey;
    const opened = qualityNode.querySelector('details')?.open;
    const playable = Boolean(meeting.audio && !ownSession);
    const actions = review.locations.map(item => `<li><span>${escape(item.label)}</span>${item.seconds == null ? '' : `<button type="button" class="quiet small" data-quality-seek="${item.seconds}"${playable ? '' : ' disabled'}>${duration(item.seconds * 1000)} 근처 듣기</button>`}</li>`).join('');
    qualityNode.innerHTML = `<details${opened ? ' open' : ''}><summary>${escape(review.title)}</summary><p class="type-caption muted">${escape(review.message)}</p>${actions ? `<ul>${actions}</ul>` : `<button type="button" class="quiet small" data-quality-seek="0"${playable ? '' : ' disabled'}>녹음 듣기</button>`}</details>`;
  }

  prepareAudio(meeting, Boolean(ownSession));
}

function clearAudio() {
  state.audioKey = ''; state.audioRequest++; state.audioLoading = null;
  const player = byId('audio-player');
  player.pause(); player.removeAttribute('src'); player.load();
  byId('audio-region').hidden = true;
  byId('retry-audio').hidden = true;
  byId('audio-message').textContent = '';
}

const audioKeyFor = meeting => `${meeting.id}:${meeting.audio.relativePath}:${meeting.audio.frames}`;
async function prepareAudio(meeting, recording, force = false) {
  const key = recording || !meeting.audio ? '' : audioKeyFor(meeting);
  byId('audio-region').hidden = !key;
  if (state.audioKey === key && !force) return state.audioLoading;
  clearAudio();
  if (!key) return;
  const request = state.audioRequest;
  state.audioKey = key;
  const player = byId('audio-player');
  byId('audio-region').hidden = false;
  byId('audio-message').dataset.state = 'loading';
  byId('audio-message').textContent = '녹음 파일을 여는 중입니다.';
  const loading = (async () => {
  try {
    const url = await bridge.audioUrl(meeting.id);
    if (state.audioKey !== key || state.audioRequest !== request) return;
    player.src = url;
    byId('audio-message').dataset.state = 'ready';
    const milliseconds = meeting.audio.sampleRate ? meeting.audio.frames / meeting.audio.sampleRate * 1000 : 0;
    byId('audio-message').textContent = `녹음 길이 ${duration(milliseconds)}`;
  } catch (error) {
    if (state.audioKey === key && state.audioRequest === request) {
      byId('audio-message').dataset.state = 'error';
      byId('audio-message').textContent = '녹음 파일을 열 수 없습니다.';
      byId('retry-audio').hidden = false;
      showError(error);
    }
  }
  })();
  state.audioLoading = loading;
  try { await loading; } finally { if (state.audioLoading === loading) state.audioLoading = null; }
}

const modelsReady = () => state.processing.supported && state.processing.modelReady;
const localSummaryEnabled = () => state.processing.localSummaryEnabled !== false;
const localSummaryMessage = () => state.processing.localSummaryMessage || '앱 내 요약은 준비 중입니다.';
const localSummaryJob = job => ['summary', 'summary-model'].includes(job?.kind);
function displayProcessing() {
  if (localSummaryEnabled()) return state.processing;
  const withoutRetry = job => localSummaryJob(job) ? { ...job, retryable: false } : job;
  return { ...withoutRetry(state.processing), jobs: state.processing.jobs?.map(withoutRetry) };
}
function renderSetup() {
  const ready = modelsReady();
  const preparation = pendingJob(state.processing, { models: true });
  const job = preparation && preparation.kind !== 'speaker-model' ? { ...state.processing, ...preparation } : state.processing;
  const setupView = preparationView(['setup', 'model', 'summary-model']);
  const running = ['queued', 'running', 'cancelling'].includes(setupView.state);
  const visible = state.connected && !activeSession() && (state.setupRequested || (!state.libraryOpen && !state.selectedId && (!ready || (running && job.kind === 'setup'))));
  const screen = byId('setup-screen');
  if (screen.hidden !== !visible) screen.hidden = !visible;
  setText('setup-title', state.setupFromSettings ? '추가 기능' : ready && state.selectedId ? '회의 요약' : ready ? '모델 준비' : '초기 설정');
  const onboarding = String(visible && state.snapshot.meetings.length === 0), workspace = document.querySelector('.workspace');
  if (workspace.dataset.onboarding !== onboarding) workspace.dataset.onboarding = onboarding;
  setText('setup-speech-state', !state.connected ? '확인 중' : job.modelReady ? '준비됨' : !job.supported ? '지원되지 않음' : running && job.modelStage === 'speech' ? '받는 중' : '');
  setText('setup-summary-state', !localSummaryEnabled() ? localSummaryMessage() : job.summaryReady ? '준비됨' : !job.summarySupported && state.connected ? '지원되지 않음' : running && job.modelStage === 'summary' ? '받는 중' : '');
  byId('setup-summary-choice').hidden = !localSummaryEnabled() || ready || job.summaryReady || !job.summarySupported || !job.supported;
  byId('setup-summary-option').disabled = !localSummaryEnabled() || !state.connected || state.busy || Boolean(pendingJob(state.processing, { models: true }));
  byId('setup-download-summary').hidden = !localSummaryEnabled() || !ready || job.summaryReady || !job.summarySupported || running;
  byId('setup-download-summary').disabled = state.busy || Boolean(pendingJob(state.processing, { models: true }));
  setupProgress.update(setupView);
  const unsupported = state.connected && !job.supported;
  const failed = ['failed', 'cancelled'].includes(setupView.state);
  setText('setup-message', !state.connected ? '준비 상태 확인 중' : unsupported ? '현재 음성 인식은 지원되는 음성 인식 실행 환경에서 사용할 수 있습니다.' : running ? '' : failed ? setupView.message : speechNotice);
  if (byId('setup-message').hidden !== !byId('setup-message').textContent) byId('setup-message').hidden = !byId('setup-message').textContent;
  const messageState = failed ? setupView.state : '';
  if (byId('setup-message').dataset.state !== messageState) byId('setup-message').dataset.state = messageState;
  byId('setup-download').hidden = (ready && !speakerNeedsPreparation(job)) || unsupported || running;
  byId('setup-download').disabled = !state.connected || state.busy || Boolean(pendingJob(state.processing, { models: true })) || speechMemoryBlocked();
  setText('setup-download', failed ? '다시 시도' : job.speechEngine === 'appleSpeech' ? '준비' : '다운로드');
  if (state.busy) byId('setup-cancel').disabled = true;
  byId('setup-start').hidden = !ready || running || state.setupFromSettings || Boolean(state.selectedId);
  byId('setup-start').disabled = state.busy;
  byId('setup-back').hidden = !state.selectedId && !state.setupFromSettings;
  byId('setup-back').textContent = state.setupFromSettings ? '설정으로 돌아가기' : '회의로 돌아가기';
  return visible;
}
async function openSetup(fromSettings = false, returnId = document.activeElement?.id) {
  if (activeSession()) { showError('녹음을 마친 뒤 준비해 주세요.'); return; }
  await finishEditing(() => { state.setupRequested = true; state.setupFromSettings = fromSettings; state.setupReturnId = returnId; render(); byId('setup-title').focus?.({ preventScroll: true }); });
}

function renderSpeechModels() {
  const modelJob = pendingJob(state.processing, { models: true });
  const otherJob = processingJobs(state.processing).find(job => ['queued', 'running', 'cancelling'].includes(job.status) && job.id !== modelJob?.id);
  // Downloads stay cancellable in place; other work and recording lock the whole picker.
  const disabled = !state.connected || state.busy || Boolean(activeSession()) || Boolean(otherJob);
  const message = activeSession() ? '녹음을 마친 뒤 모델을 변경할 수 있습니다.'
    : otherJob ? '현재 처리가 끝난 뒤 모델을 변경할 수 있습니다.' : speechNotice;
  for (const picker of speechPickers) picker.update({ progress: state.processing, disabled, locked: Boolean(modelJob), message, error: speechSelectionError, acknowledged: speechMemoryAcknowledged });
}
function selectSpeechEngine(engine) {
  return run(async () => {
    speechSelectionError = ''; speechNotice = '';
    renderSpeechModels();
    try { await acceptProcessing(await bridge.requestSpeechEngine(engine)); }
    catch (error) { speechSelectionError = error.message || '음성 인식 모델을 변경하지 못했습니다.'; }
  });
}
function cancelSpeechSwitch() {
  return run(async () => {
    speechSelectionError = '';
    try { await acceptProcessing(await bridge.cancelSpeechSwitch()); }
    catch (error) { speechSelectionError = error.message || '전환을 취소하지 못했습니다.'; }
  });
}
function selectSpeakerEnabled(enabled) {
  return run(async () => {
    speechSelectionError = '';
    renderSpeechModels();
    try { await acceptProcessing(await bridge.setSpeakerEnabled(enabled)); }
    catch (error) { speechSelectionError = error.message || '화자 구분 설정을 변경하지 못했습니다.'; }
  });
}
function prepareSpeechModels() {
  return run(async () => {
    speechSelectionError = '';
    renderSpeechModels();
    if (speechMemoryBlocked()) return;
    try { await acceptProcessing(await bridge.prepareMeetingModels(false)); }
    catch (error) { speechSelectionError = error.message || '음성 인식 모델을 준비하지 못했습니다.'; }
  });
}
function render() {
  renderSpeechModels();
  byId('create-meeting').disabled = !state.connected || state.busy || !modelsReady();
  byId('create-calendar-meeting').disabled = !state.connected || state.busy;
  byId('welcome-create').disabled = !state.connected || state.busy;
  for (const id of ['library-create', 'library-record']) byId(id).disabled = !state.connected || state.busy || state.libraryBusy;
  setHTML('library-record', `${icon(activeSession() ? 'audio-lines' : 'mic')}<span>${activeSession() ? '녹음으로 돌아가기' : '녹음 시작'}</span>`);
  // Progress ticks re-render this screen; keep the reader's scroll position.
  const setupScroll = byId('setup-screen').scrollTop;
  const setupVisible = renderSetup();
  if (setupVisible && byId('setup-screen').scrollTop !== setupScroll) byId('setup-screen').scrollTop = setupScroll;
  speechOffer.update(state.processing.speechOffer, { blocked: !state.connected || Boolean(activeSession()) || setupVisible || Boolean(byId('settings-dialog').open) || Boolean(byId('quit-dialog').open) });
  notificationCenter.update({ processing: displayProcessing(), meetings: state.snapshot.meetings, selectedMeetingId: state.selectedId, connected: state.connected, actions: processingActions, integrations: integrationNotices, notices: appUpdateNotices });
  sidebarViews?.updateCounts(notificationCenter.counts());
  if (setupVisible && !state.setupWasVisible) sidebarViews?.close({ focus: false });
  state.setupWasVisible = setupVisible;
  for (const button of document.querySelectorAll('[data-navigation-preview]')) { const category = ['trash', 'ai'].includes(state.libraryFilter) ? state.libraryFilter : !['all', 'unfiled'].includes(state.libraryFilter) ? 'folders' : 'all'; button.setAttribute('aria-current', state.libraryOpen && category === button.dataset.navigationPreview ? 'page' : 'false'); }
  library.render();
  speakers.render();
  renderSearchStatus();
  renderSettings();
  renderProcessing();
  const meeting = currentMeeting();
  calendarReference.render(meeting);
  aiChat?.setMeeting(state.libraryOpen || setupVisible ? null : meeting);
  meetingContext?.setMeeting(state.libraryOpen || setupVisible ? null : meeting);
  byId('ai-chat-open').disabled = !state.connected;
  byId('meeting-attendees').hidden = !meeting;
  byId('meeting-attendees').disabled = !state.connected || state.busy;
  setHTML('meeting-attendees', `${icon('users')}<span>참석자${meeting?.attendees?.length ? ` ${meeting.attendees.length}명` : ''}</span>`);
  byId('create-calendar-meeting').hidden = !meeting || Boolean(meeting.audio || meeting.calendarReference || activeSession()?.meetingId === meeting.id);
  byId('import-audio').hidden = !meeting || Boolean(meeting.audio || activeSession()?.meetingId === meeting.id);
  const aiLibraryOpen = state.libraryOpen && state.libraryFilter === 'ai' && state.connected && !setupVisible;
  byId('ai-library-screen').hidden = !aiLibraryOpen;
  if (aiLibraryOpen) aiLibrary?.mount();
  byId('library-screen').hidden = aiLibraryOpen || setupVisible || !state.connected || (!state.libraryOpen && !modelsReady() && !state.snapshot.meetings.length) || (!state.libraryOpen && Boolean(meeting));
  byId('welcome').hidden = aiLibraryOpen || !byId('library-screen').hidden || setupVisible || Boolean(meeting);
  byId('meeting-detail').hidden = setupVisible || state.libraryOpen || !meeting;
  if (state.connected && !setupVisible) viewHistory?.visit(state.libraryOpen || !meeting ? { kind: 'library', filter: state.libraryFilter, view: state.libraryView, query: byId('search').value } : { kind: 'meeting', id: meeting.id });
  if (!meeting) { detail.update({ id: null }); summaryEvidence?.reset(); return; }
  renderMeetingDates(meeting);
  if (!state.draft?.dirty && !state.draft?.saving && document.activeElement !== byId('meeting-title') && document.activeElement !== byId('meeting-notes')) {
    byId('meeting-title').value = meeting.title;
    byId('meeting-notes').value = meeting.notes || '';
  }
  renderSaveState();
  renderRecording(meeting);
  renderTranscript(meeting);
  renderSummary(meeting);
  const showSummary = Boolean(state.summary?.document || resultPreview(meeting).visible || pendingJob(state.processing, { meetingId: meeting.id, kind: 'summary' }) || state.summaryLoadError);
  detail.update({ inlineNotes: !showSummary, showSummary, id: byId('meeting-detail').hidden ? null : meeting.id, recording: Boolean(activeSession()?.meetingId === meeting.id && ['recording', 'paused'].includes(activeSession().state)), summaryLoaded: state.summaryLoaded, transcriptLoaded: state.transcriptLoaded, hasSummary: Boolean(state.summary?.document), hasTranscript: Boolean(state.transcriptDraft?.doc?.segments?.length), hasPreview: resultPreview(meeting).rows.length > 0, hasAudio: Boolean(meeting.audio) });
  if (byId('meeting-detail').hidden) summaryEvidence?.reset();
  renderNotesSearch();
  documentSearch.refresh();
  speakers.syncPlayback?.();
  const intent = state.pendingSearchIntent;
  if (intent?.meetingId === meeting.id && state.transcriptLoaded && state.summaryLoaded) {
    state.pendingSearchIntent = null;
    const hit = intent.hit, doc = state.transcriptDraft?.doc;
    const valid = sameTranscriptVersion(hit.transcriptVersion, doc?.version);
    const index = valid && doc?.segments[hit.index]?.id === hit.segmentId ? hit.index : undefined;
    void documentSearch.find(intent.query, { view: hit.kind === 'speaker' ? 'transcript' : hit.kind, index });
  }
}

function renderNotesSearch() {
  const query = documentSearch.query();
  byId('meeting-notes').hidden = Boolean(query);
  byId('notes-search-preview').hidden = !query;
  setHTML('notes-search-preview', query ? globalThis.NeroUI.highlightText(byId('meeting-notes').value, query) : '');
}

function renderMeetingDates(meeting) {
  byId('recorded-at').hidden = !meeting.recordedAt;
  setText('recorded-at', meeting.recordedAt ? `녹음 ${fullDate.format(meeting.recordedAt)}` : '');
  const issue = state.dateError?.id === meeting.id ? state.dateError.message : '';
  setText('meeting-date-message', issue);
  byId('meeting-date-message').hidden = !issue;
  const input = byId('meeting-day');
  const key = JSON.stringify([meeting.id, meetingDay(meeting), state.dateSaving]);
  if (state.dateKey === key) return;
  state.dateKey = key;
  if (state.dateSaving !== meeting.id) input.value = meetingDay(meeting);
  input.disabled = Boolean(state.dateSaving);
  globalThis.NeroControls.enhanceAll();
  globalThis.NeroControls.refresh?.();
}

async function changeMeetingDate() {
  const meeting = currentMeeting();
  if (!meeting || state.dateSaving) return;
  const date = byId('meeting-day').value || null;
  state.dateSaving = meeting.id;
  state.dateError = null;
  renderMeetingDates(meeting);
  try {
    await finishEditing(async () => { await bridge.setMeetingDate(meeting.id, date); acceptSnapshot(await bridge.readSnapshot()); });
  } catch {
    state.dateError = { id: meeting.id, message: '회의 날짜를 저장하지 못했습니다. 다시 선택해 주세요.' };
  } finally { state.dateSaving = null; state.dateKey = ''; if (currentMeeting()) renderMeetingDates(currentMeeting()); }
}

function acceptSnapshot(snapshot) {
  const previousMeeting = currentMeeting();
  const signatures = snapshotSignatures(snapshot);
  const comparison = compareSnapshotSignatures(state.snapshotSignatures, signatures);
  const changed = !state.connected || comparison.changed;
  const timerOnly = state.connected && comparison.timerOnly;
  state.snapshot = snapshot;
  state.snapshotSignatures = signatures;
  state.connected = true;
  if (state.selectedId && !snapshot.meetings.some(meeting => meeting.id === state.selectedId && meeting.deletedAt == null)) { state.selectedId = null; state.draft = null; state.transcriptDraft = null; state.summary = null; state.libraryOpen = true; speakers.reset(); clearAudio(); }
  const saved = currentMeeting();
  const draft = state.draft;
  if (saved && draft?.id === saved.id && !draft.dirty && !draft.saving && (draft.title !== saved.title || draft.notes !== (saved.notes || ''))) {
    state.draft = { ...draft, title: saved.title, notes: saved.notes || '', savedTitle: saved.title, savedNotes: saved.notes || '', version: draft.version + 1 };
    byId('meeting-title').value = saved.title;
    byId('meeting-notes').value = saved.notes || '';
  }
  if (saved && previousMeeting?.id === saved.id) {
    const notesChanged = (previousMeeting.notes || '') !== (saved.notes || '');
    const attendeesChanged = JSON.stringify(previousMeeting.attendees || []) !== JSON.stringify(saved.attendees || []);
    const document = state.summary?.document;
    if ((!draft?.saving && notesChanged && document?.notesSha256 != null)
      || (attendeesChanged && document?.attendeesSha256 != null)
      || (!state.summaryLoaded && (notesChanged || attendeesChanged))) {
      if (state.summary) state.summary = { ...state.summary, stale: true };
      void loadSummary(saved.id);
    }
  }
  const first = snapshot.meetings.find(meeting => meeting.deletedAt == null);
  if (!state.selectedId && !state.libraryOpen && modelsReady() && first) { void selectMeeting(first.id); return; }
  if (!changed) return;
  if (timerOnly) {
    const ownSession = activeSession()?.meetingId === state.selectedId ? activeSession() : null;
    setText('recording-time', duration(ownSession?.elapsedMs));
    return;
  }
  render();
}

function saveDraft() {
  clearTimeout(state.saveTimer);
  const draft = state.draft;
  if (!draft?.dirty || state.composing) return state.saveQueue;
  const version = draft.version;
  const title = draft.title.trim() || '제목 없는 회의';
  const notes = draft.notes;
  const notesChanged = notes !== draft.savedNotes;
  draft.saving = true;
  renderSaveState();
  const task = state.saveQueue.catch(() => {}).then(async () => {
    let saved;
    try {
      saved = await bridge.updateMeeting(draft.id, { title: draft.savedTitle, notes: draft.savedNotes }, title, notes);
    } catch (error) {
      // Changed elsewhere since the edit began: keep the text here and let the next
      // save replace what is stored now.
      const stored = (await bridge.readSnapshot()).meetings.find(meeting => meeting.id === draft.id);
      if (stored && (stored.title !== draft.savedTitle || (stored.notes || '') !== draft.savedNotes)) {
        draft.savedTitle = stored.title;
        draft.savedNotes = stored.notes || '';
        throw new Error('다른 곳에서 메모나 제목이 바뀌어 저장하지 않았습니다. 지금 내용으로 다시 저장하면 그 변경을 덮어씁니다.');
      }
      throw error;
    }
    draft.savedTitle = saved.title;
    draft.savedNotes = saved.notes;
    // Text appended meanwhile (an AI answer) came back after this edit.
    if (saved.notes !== notes && draft.version === version && state.draft === draft) {
      draft.notes = saved.notes;
      byId('meeting-notes').value = saved.notes;
    }
    const refreshSummary = state.draft === draft && state.summary?.document?.notesSha256 != null;
    if (refreshSummary && notesChanged) state.summary = { ...state.summary, stale: true };
    acceptSnapshot(await bridge.readSnapshot());
    if (refreshSummary && state.draft === draft) await loadSummary(draft.id);
    if (draft.version === version) draft.dirty = false;
  });
  state.saveQueue = task.catch(() => {});
  return task.then(() => { clearError(true); refreshMeetingSearch(); }).catch((error) => { showError(error, true); throw error; }).finally(() => {
    if (draft.version === version) draft.saving = false;
    renderSaveState();
  });
}

async function selectMeeting(id, searchIntent = null, isHistoryCurrent = null) {
  const latest = beginNavigation(Boolean(isHistoryCurrent));
  const historyValid = isHistoryCurrent || (() => true);
  try {
    await finishEditing(() => {
      if (!latest() || !historyValid()) return;
      const meeting = state.snapshot.meetings.find(item => item.id === id);
      if (!meeting || meeting.deletedAt) return;
      const changed = state.selectedId !== id;
      detail.rememberScroll();
      state.libraryOpen = false;
      speakers.reset(); transcriptActions.reset(); documentSearch.reset();
      state.setupRequested = false; state.setupFromSettings = false;
      state.pendingSearchIntent = searchIntent ? { ...searchIntent, meetingId: id } : null;
      state.selectedId = id;
      if (changed) {
        state.transcriptDraft = null; state.transcriptKey = ''; state.transcriptEditing = null;
        state.summary = null; state.summaryKey = '';
        state.summaryLoaded = false; state.transcriptLoaded = false;
        state.summaryLoadError = false; state.transcriptLoadError = false;
        void loadSummary(id); void loadTranscript(id);
        state.draft = { id, title: meeting.title, notes: meeting.notes || '', savedTitle: meeting.title, savedNotes: meeting.notes || '', dirty: false, saving: false, version: 0 };
        byId('meeting-title').value = meeting.title;
        byId('meeting-notes').value = meeting.notes || '';
        byId('meeting-notes').scrollTop = 0;
      }
      render();
    });
    if (latest() && historyValid() && state.selectedId === id && !searchIntent) byId('meeting-detail').focus({ preventScroll: true });
  } catch { return; }
}

function microphoneIssue() {
  if (!state.devices.length) return '연결된 마이크가 없습니다.';
  if (state.deviceId && !state.devices.some(device => device.id === state.deviceId)) return '선택한 마이크가 연결되어 있지 않습니다.';
  if (!state.deviceId && !state.devices.some(device => device.isDefault)) return '시스템 기본 마이크를 찾지 못했습니다.';
  return '';
}

function renderSettings() {
  const dialog = byId('settings-dialog');
  const recording = state.settingsPanel === 'recording';
  agentSettings.setActive(dialog.open && state.settingsPanel === 'agent');
  terminologySettings.setActive(dialog.open && recording);
  templateSettings.setActive(dialog.open && state.settingsPanel === 'templates');
  calendarSettings.setActive(dialog.open && state.settingsPanel === 'agent');
  aiChat?.setSettingsActive(dialog.open && state.settingsPanel === 'agent');
  if (dialog.open && state.settingsPanel === 'people') peopleSettings.show(); else peopleSettings.hide();
  for (const name of settingsPanels) {
    const selected = state.settingsPanel === name;
    byId(`settings-${name}-tab`).setAttribute('aria-selected', String(selected));
    byId(`settings-${name}-tab`).tabIndex = selected ? 0 : -1;
    byId(`settings-${name}-panel`).hidden = !selected;
  }
  const select = byId('settings-microphone');
  const disabled = !state.connected || state.busy || Boolean(activeSession()) || !state.devices.length;
  const refresh = state.settingsKey !== JSON.stringify([state.devices, state.deviceId, state.deviceName]) || select.disabled !== disabled;
  const key = JSON.stringify([state.devices, state.deviceId, state.deviceName]);
  if (key !== state.settingsKey) {
    state.settingsKey = key;
    const missing = state.deviceId && !state.devices.some(device => device.id === state.deviceId);
    select.innerHTML = `<option value="">시스템 기본 마이크</option>${missing ? `<option value="${escape(state.deviceId)}" disabled>${escape(state.deviceName || '선택한 마이크')} (연결 안 됨)</option>` : ''}${state.devices.map(device => `<option value="${escape(device.id)}">${escape(device.name)}${device.isDefault ? ' (기본)' : ''}</option>`).join('')}`;
    select.value = state.deviceId || '';
  }
  select.disabled = disabled;
  byId('settings-microphone-hint').hidden = !activeSession();
  if (activeSession()) select.setAttribute('aria-describedby', 'settings-microphone-hint');
  else select.removeAttribute('aria-describedby');
  byId('auto-recorder').checked = state.autoRecorder;
  byId('auto-recorder').disabled = state.recorderSettingBusy;
  const job = state.processing;
  const featureJob = pendingJob(job, { kind: 'summary-model' });
  const failedFeature = !featureJob && !job.summaryReady && processingJobs(job).filter(item => item.kind === 'summary-model' && item.status === 'failed' && !item.acknowledged).at(-1);
  const featureError = localSummaryEnabled() ? state.summaryFeatureError || (failedFeature && processingFeedback(failedFeature).detail) || '' : '';
  byId('settings-summary-error').textContent = featureError;
  byId('settings-summary-error').hidden = !featureError;
  byId('settings-summary-state').textContent = !localSummaryEnabled() ? localSummaryMessage() : !state.connected ? '확인 중' : !job.summarySupported ? '지원되지 않음' : job.summaryReady ? '추가됨' : featureJob ? '다운로드 중' : '';
  byId('models').hidden = !localSummaryEnabled() || job.summaryReady || !job.summarySupported || Boolean(featureJob);
  byId('models').disabled = !state.connected || state.busy || Boolean(pendingJob(job, { models: true }));
  summaryProgress.update(localSummaryEnabled() ? preparationView(summaryModelKinds) : { state: 'idle' });
  const message = recording ? terminologySettings.error || state.settingsError || (!activeSession() && state.devicesLoaded ? microphoneIssue() : '') : '';
  byId('settings-message').textContent = message;
  byId('settings-message').hidden = !message;
  if (dialog.open) {
    globalThis.NeroControls.enhanceAll(dialog);
    if (refresh) globalThis.NeroControls.refresh?.();
  }
}

async function openSettings(panel = 'appearance', returnId = 'settings') {
  await finishEditing(() => {
    state.settingsPanel = panel;
    state.settingsReturnId = returnId;
    state.settingsError = '';
    const dialog = byId('settings-dialog');
    if (!dialog.open) dialog.showModal();
    renderSettings();
    globalThis.NeroControls.refresh?.();
    byId(`settings-${panel}-tab`).focus({ preventScroll: true });
  });
  try { await refreshDevices(); } catch { state.settingsError = '마이크 목록을 불러오지 못했습니다. 다시 시도해 주세요.'; renderSettings(); }
}

function changeMicrophone() {
  if (activeSession()) { state.settingsError = '녹음을 마친 뒤 마이크를 변경할 수 있습니다.'; state.settingsKey = ''; return renderSettings(); }
  const id = byId('settings-microphone').value || null;
  const device = state.devices.find(item => item.id === id);
  if (id && !device) { state.settingsKey = ''; return renderSettings(); }
  const preference = { id, name: device?.name || '' };
  try {
    if (!globalThis.localStorage) throw new Error('Storage unavailable');
    globalThis.localStorage.setItem(microphonePreferenceKey, JSON.stringify(preference));
    state.deviceId = preference.id; state.deviceName = preference.name; state.settingsError = '';
  } catch { state.settingsError = '마이크 설정을 저장하지 못했습니다. 다시 시도해 주세요.'; state.settingsKey = ''; }
  render();
}

async function refreshDevices() {
  state.devices = await bridge.listDevices();
  state.devicesLoaded = true;
  render();
}

async function run(action) {
  if (state.busy) return;
  const focusedId = document.activeElement?.id;
  const focusedAction = document.activeElement?.dataset.action;
  const focusedField = document.activeElement?.dataset.field;
  state.busy = true;
  render();
  try { await action(); } catch (error) { if (state.lastError !== error) showError(error); } finally {
    state.busy = false;
    render();
    if (document.activeElement === document.body) {
      const nextAction = focusedAction === 'pause' ? 'resume' : focusedAction === 'resume' ? 'pause' : focusedAction;
      const target = focusedField ? document.querySelector(`button[data-field="${focusedField}"]`) : focusedId ? byId(focusedId) : [...document.querySelectorAll('[data-action]')].find((element) => element.dataset.action === nextAction);
      if (target && !target.disabled && !target.hidden) target.focus({ preventScroll: true });
    }
  }
}

async function createMeeting({ record = false } = {}) {
  if (record && activeSession()) return selectMeeting(activeSession().meetingId);
  if (!modelsReady()) return openSetup();
  await finishEditing(async () => {
    if (record) {
      await refreshDevices();
      if (microphoneIssue()) return openSettings('recording', 'library-record');
      if (activeSession()) return selectMeeting(activeSession().meetingId);
    }
    state.setupRequested = false;
    const meeting = await bridge.createMeeting('새 회의', '');
    acceptSnapshot(await bridge.readSnapshot());
    if (state.snapshot.folders?.some(folder => folder.id === state.libraryFilter)) {
      try { acceptSnapshot(await bridge.moveMeetings([meeting.id], state.libraryFilter)); } catch (error) { showError(error); }
    }
    await selectMeeting(meeting.id);
    if (record) {
      await startSelectedRecording();
      byId('recording-controls').tabIndex = -1;
      byId('recording-controls').focus({ preventScroll: true });
    } else {
      byId('meeting-title').focus();
      byId('meeting-title').select();
    }
  });
}

async function startSelectedRecording() {
  if (activeSession()) return selectMeeting(activeSession().meetingId);
  if (!modelsReady()) return openSetup();
  await saveDraft();
  await refreshDevices();
  if (microphoneIssue()) return;
  if (activeSession()) return selectMeeting(activeSession().meetingId);
  acceptSnapshot(await bridge.startRecording(state.selectedId, state.deviceId));
}

function onEdit() {
  if (!state.draft) return;
  state.draft.title = byId('meeting-title').value;
  state.draft.notes = byId('meeting-notes').value;
  state.draft.dirty = true;
  state.draft.version += 1;
  renderSaveState();
  if (currentMeeting()) renderSummary(currentMeeting());
  clearTimeout(state.saveTimer);
  if (!state.composing) state.saveTimer = setTimeout(() => saveDraft().catch(() => {}), 600);
}

for (const id of ['meeting-title', 'meeting-notes']) {
  byId(id).addEventListener('input', onEdit);
  byId(id).addEventListener('compositionstart', () => { state.composing = true; clearTimeout(state.saveTimer); });
  byId(id).addEventListener('compositionend', () => { state.composing = false; onEdit(); });
}
byId('library-search-clear').addEventListener('click', () => {
  beginNavigation();
  byId('search').value = '';
  meetingSearch.update('', state.libraryFilter === 'trash');
  render();
  byId('library-title').focus({ preventScroll: true });
});
byId('search').addEventListener('input', () => {
  beginNavigation();
  state.libraryOpen = true; library.resetSelection(); render();
  meetingSearch.update(byId('search').value, state.libraryFilter === 'trash');
});
byId('library-create').addEventListener('click', () => run(createMeeting));
byId('library-record').addEventListener('click', () => run(() => createMeeting({ record: true })));
byId('create-meeting').addEventListener('click', () => run(createMeeting));
byId('welcome-create').addEventListener('click', () => run(createMeeting));
byId('setup-start').addEventListener('click', () => run(createMeeting));
byId('settings').addEventListener('click', () => openSettings().catch(showError));
byId('privacy-policy').addEventListener('click', () => bridge.openPrivacyPolicy().catch(showError));
async function leaveSettings(action) {
  if (templateSettings.isConfirming() || peopleSettings.isConfirming() || !await templateSettings.beforeLeave() || !await peopleSettings.beforeLeave()) return false;
  globalThis.NeroControls.close?.();
  await action();
  return true;
}
async function closeSettings() {
  return leaveSettings(() => byId('settings-dialog').close());
}
async function selectSettingsPanel(name, focus = false) {
  if (name === state.settingsPanel) return;
  await leaveSettings(() => {
    state.settingsPanel = name; renderSettings();
    if (focus) byId(`settings-${name}-tab`).focus({ preventScroll: true });
  });
}
byId('settings-close').addEventListener('click', () => { void closeSettings(); });
bindDialogDismiss(byId('settings-dialog'), { dismiss: closeSettings, blocked: () => templateSettings.isConfirming() || peopleSettings.isConfirming() });
bindDialogDismiss(byId('quit-dialog'));
byId('settings-dialog').addEventListener('close', () => { aiChat?.setSettingsActive(false); templateSettings.hide(); peopleSettings.hide(); void summaryTemplatePicker.reload(); terminologySettings.setActive(false); agentSettings.setActive(false); globalThis.NeroControls.close?.(); returnFocusTarget(byId(state.settingsReturnId) || byId('settings'), byId('navigation-toggle')).focus({ preventScroll: true }); });
for (const name of settingsPanels) byId(`settings-${name}-tab`).addEventListener('click', () => { void selectSettingsPanel(name, true); });
byId('settings-dialog').addEventListener('keydown', event => {
  if (event.isComposing || event.keyCode === 229 || (templateSettings.isConfirming() || peopleSettings.isConfirming())) return;
  const index = settingsPanels.findIndex(name => `settings-${name}-tab` === event.target.id);
  if (index < 0 || !['ArrowDown', 'ArrowUp', 'ArrowRight', 'ArrowLeft', 'Home', 'End'].includes(event.key)) return;
  event.preventDefault();
  const step = ['ArrowUp', 'ArrowLeft'].includes(event.key) ? -1 : 1;
  const name = settingsPanels[event.key === 'Home' ? 0 : event.key === 'End' ? settingsPanels.length - 1 : (index + step + settingsPanels.length) % settingsPanels.length];
  void selectSettingsPanel(name, true);
});
byId('settings-microphone').addEventListener('change', changeMicrophone);

byId('models').addEventListener('click', () => run(async () => {
  if (!localSummaryEnabled()) return;
  const restoreFocus = document.activeElement === byId('models');
  state.summaryFeatureError = '';
  try { await acceptProcessing(await bridge.prepareSummaryModel()); }
  catch (error) { state.summaryFeatureError = error?.message || '회의 요약 기능을 추가하지 못했습니다.'; if (!byId('settings-dialog').open) showError(error); }
  renderSettings();
  if (restoreFocus && byId('settings-dialog').open && state.settingsPanel === 'appearance') {
    (pendingJob(state.processing, { kind: 'summary-model' }) ? byId('settings-feature-cancel') : byId('models').hidden ? byId('settings-summary-control') : byId('models')).focus({ preventScroll: true });
  }
}));
byId('settings-feature-cancel').addEventListener('click', () => { const job = pendingJob(state.processing, { kind: 'summary-model' }); if (job?.id) processingAction('cancel', job.id).catch(showError); });
byId('setup-back').addEventListener('click', () => { const settings = state.setupFromSettings, returnId = state.setupReturnId; state.setupRequested = false; state.setupFromSettings = false; render(); if (settings) void openSettings('appearance'); else { const origin = byId(returnId || 'settings'); const fallback = !byId('summarize').hidden && !byId('summarize').disabled ? byId('summarize') : byId('detail-transcript-open'); (origin && !origin.hidden && !origin.disabled ? origin : fallback)?.focus({ preventScroll: true }); } });
byId('setup-download').addEventListener('click', () => run(async () => { if (speechMemoryBlocked()) return; state.setupRequested = true; await acceptProcessing(await bridge.prepareMeetingModels(localSummaryEnabled() && Boolean(byId('setup-summary-option').checked) && !state.processing.summaryReady)); }));
byId('setup-download-summary').addEventListener('click', () => run(async () => { if (!localSummaryEnabled()) return; state.setupRequested = true; await acceptProcessing(await bridge.prepareSummaryModel()); }));
byId('setup-cancel').addEventListener('click', () => { const id = byId('setup-cancel').dataset.jobId; if (id) processingAction('cancel', id).catch(showError); });
byId('retry-save').addEventListener('click', () => saveDraft().catch(() => {}));
byId('dismiss-error').addEventListener('click', () => clearError());
byId('recording-controls').addEventListener('click', (event) => {
  const action = event.target.closest('[data-action]')?.dataset.action;
  const session = activeSession();
  if (!action) return;
  run(async () => {
    if (action === 'recording-settings') return openSettings('recording', 'settings');
    if (action === 'open-active' && session) return selectMeeting(session.meetingId);
    if (action === 'show-recorder' && session) return bridge.showRecorder(session.id);
    if (action === 'setup') return openSetup();
    if (action === 'start') await startSelectedRecording();
    if (action === 'pause' && session) acceptSnapshot(await bridge.pauseRecording(session.id));
    if (action === 'resume' && session) acceptSnapshot(await bridge.resumeRecording(session.id));
    if (action === 'stop' && session) acceptSnapshot(await bridge.stopRecording(session.id));
  });
});
byId('meeting-day').addEventListener('change', () => { void changeMeetingDate(); });
byId('auto-recorder').addEventListener('change', async () => {
  const previous = state.autoRecorder;
  const next = byId('auto-recorder').checked;
  state.recorderSettingBusy = true;
  state.autoRecorder = next;
  renderSettings();
  try {
    await bridge.setAutoRecorder(next);
    try { globalThis.localStorage.setItem(recorderPreferenceKey, String(next)); }
    catch (error) { await bridge.setAutoRecorder(previous); throw error; }
    state.autoRecorder = next;
    state.settingsError = '';
  } catch { state.autoRecorder = previous; state.settingsError = '녹음 제어창 설정을 저장하지 못했습니다.'; }
  finally { state.recorderSettingBusy = false; renderSettings(); }
});
byId('create-calendar-meeting').addEventListener('click', event => { const trigger = event.currentTarget; void run(() => finishEditing(async () => { if (!modelsReady()) return openSetup(); const target = await bridge.draftAttachmentTarget(state.selectedId); calendarPicker.open(trigger, target); })); });
byId('retry-audio').addEventListener('click', () => { const meeting = currentMeeting(); if (meeting) void prepareAudio(meeting, Boolean(activeSession()?.meetingId === meeting.id), true); });
byId('audio-player').addEventListener('error', () => { if (byId('audio-player').getAttribute('src')) { byId('audio-message').dataset.state = 'error'; byId('audio-message').textContent = '음성을 재생할 수 없습니다. 녹음 파일을 다시 열어 주세요.'; byId('retry-audio').hidden = false; } });
// Links rendered from summaries and AI answers open in the browser, never in this window.
for (const type of ['click', 'auxclick', 'contextmenu', 'dragstart']) document.addEventListener(type, event => {
  const link = event.target?.closest?.('a[data-summary-link]');
  if (!link) return;
  event.preventDefault();
  if (type === 'click') void bridge.openDocumentLink(link.getAttribute('href')).catch(showError);
}, true);
document.addEventListener('keydown', (event) => {
  if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 's') { event.preventDefault(); void finishEditing(() => {}).catch(() => {}); }
});

async function finishEditing(action) {
  if (state.composing || state.transcriptComposing) {
    const editing = document.activeElement;
    const notesComposing = state.composing;
    const transcriptComposing = state.transcriptComposing;
    editing?.blur();
    await new Promise(requestAnimationFrame);
    state.composing = false; state.transcriptComposing = false;
    if (notesComposing) onEdit();
    if (transcriptComposing) onTranscriptEdit({ target: editing });
  }
  state.editLocks += 1;
  byId('transcript-rows').querySelectorAll('textarea').forEach((input) => { input.readOnly = true; });
  byId('meeting-title').readOnly = true;
  byId('meeting-notes').readOnly = true;
  if (currentMeeting()) renderTranscript(currentMeeting());
  try {
    const draft = state.draft;
    do { await saveDraft(); await saveTranscript(); } while (state.draft === draft && (draft?.dirty || state.transcriptDraft?.dirty));
    state.transcriptEditing = null;
    if (currentMeeting()) renderTranscript(currentMeeting());
    return await action();
  } finally {
    state.editLocks -= 1;
    byId('meeting-title').readOnly = state.editLocks > 0;
    byId('meeting-notes').readOnly = state.editLocks > 0;
    if (currentMeeting()) renderTranscript(currentMeeting());
    speakers.render();
  }
}

async function prepareAppLeave(action) {
  for (const editor of [templateSettings, peopleSettings, meetingAttendees, speakers]) {
    if (!await editor.beforeLeave()) return false;
  }
  globalThis.NeroControls.close?.();
  return finishEditing(action);
}

function onAgentChanged(change) {
  if (change.area === 'aiChat') aiChat?.flow.externalChange(change.conversationIds || []);
  else if (change.area === 'meetingSources' || change.area === 'terminology') void meetingContext?.reload();
  else if (change.area === 'settings') void terminologySettings.reload();
}

async function onAgentWriteRequest({ requestId, kind } = {}) {
  if (typeof requestId !== 'string' || !/^[a-f0-9-]{36}$/.test(requestId)) return;
  let ready = false;
  try {
    const prepare = kind === 'appUpdate' ? prepareAppLeave : finishEditing;
    const completed = await prepare(async () => {
      ready = true;
      await bridge.agentWriteReady(requestId);
      acceptSnapshot(await bridge.readSnapshot());
      await peopleSettings.refreshIfClean?.();
      if (currentMeeting()) await loadSummary(state.selectedId);
      refreshMeetingSearch();
    });
    if (completed === false && !ready) {
      await bridge.agentWriteReady(requestId, '현재 편집 내용을 저장하지 않았습니다.').catch(() => {});
    }
  } catch (error) {
    if (!ready) {
      await bridge.agentWriteReady(requestId, '현재 편집 내용을 저장하지 못했습니다.').catch(() => {});
      showError(error, true);
    }
  }
}

const processingActive = () => state.processing.status === 'running';

function renderProcessing() {
  rememberProcessingFocus();
  const notices = processingNotices(state.processing, !state.libraryOpen && !state.setupRequested ? currentMeeting()?.id : undefined);
  const progress = notices.primary ? { ...(state.processing.id === notices.primary.id ? state.processing : {}), ...notices.primary } : {};
  const feedback = processingFeedback(progress, { cancelling: state.processingCancellingId === progress.id || processingActions.get(progress.id) === 'cancel' });
  byId('import-audio').disabled = !state.connected || state.busy || Boolean(pendingJob(state.processing, { kind: 'import' })) || Boolean(activeSession());
  byId('processing').hidden = !feedback.visible || progress.kind === 'setup';
  byId('processing').dataset.state = feedback.state;
  const owner = state.snapshot.meetings.find(meeting => meeting.id === progress.meetingId);
  setText('processing-title', `${owner && owner.id !== state.selectedId ? owner.title + ': ' : ''}${feedback.title}`);
  const detail = processingErrors.get(progress.id) || feedback.detail;
  setText('processing-message', detail);
  byId('processing-message').hidden = !detail;
  const errorInfo = byId('processing-error-info');
  const diagnosticKey = `${progress.id}:${progress.status}`;
  if (errorInfo.dataset.job !== diagnosticKey) errorInfo.open = false;
  errorInfo.dataset.job = diagnosticKey;
  errorInfo.hidden = !feedback.diagnostics.length;
  setHTML('processing-error-fields', feedback.diagnostics.map(([label, value]) => `<div>${escape(label)}: ${escape(value)}</div>`).join(''));
  setText('processing-warning', feedback.warning);
  byId('processing-warning').hidden = !feedback.warning;
  byId('processing-spinner').hidden = !feedback.active;
  byId('processing-result-icon').hidden = feedback.active;
  setHTML('processing-result-icon', icon(feedback.icon));
  byId('cancel-processing').hidden = !feedback.cancelVisible;
  byId('cancel-processing').disabled = feedback.cancelDisabled;
  setText('cancel-processing', feedback.cancelLabel);
  byId('processing-progress').hidden = !feedback.progress;
  setText('processing-percent', feedback.progress ? `${feedback.progress.percent}%` : '');
  byId('processing-percent').hidden = !feedback.progress;
  if (feedback.progress) { byId('processing-progress').max = feedback.progress.max; byId('processing-progress').value = feedback.progress.value; }
  byId('retry-processing').hidden = (!localSummaryEnabled() && localSummaryJob(progress)) || !canRetryProcessing(progress);
  byId('retry-processing').disabled = processingActions.has(progress.id);
  byId('open-processing-meeting').hidden = progress.status !== 'needsReview' || !owner;
  byId('dismiss-processing').hidden = !canDismissProcessing(progress);
  byId('dismiss-processing').disabled = processingActions.has(progress.id);
  for (const [id, action] of [['cancel-processing', 'cancel'], ['retry-processing', 'retry'], ['open-processing-meeting', 'open'], ['dismiss-processing', 'ack']]) {
    byId(id).dataset.jobId = progress.id || '';
    byId(id).dataset.jobAction = action;
  }
  restoreProcessingFocus();
}

function rememberProcessingFocus() {
  if (processingFocus) return;
  const target = document.activeElement;
  const jobId = target?.dataset?.jobId;
  if (!jobId) return;
  const ids = [...new Set([...byId('processing').querySelectorAll('[data-job-id]')].map(node => node.dataset.jobId))];
  processingFocus = { target, id: jobId, action: target.dataset.jobAction, ids, index: ids.indexOf(jobId) };
}

function restoreProcessingFocus() {
  const saved = processingFocus;
  if (!saved) return;
  const active = document.activeElement;
  if (active && active !== document.body && active !== saved.target && active.isConnected !== false) { processingFocus = null; return; }
  if (processingActions.has(saved.id)) return;
  const usable = node => node && !node.disabled && !node.hidden && node.isConnected !== false && !node.closest?.('[hidden], [inert]');
  const banner = ['cancel-processing', 'retry-processing', 'open-processing-meeting', 'dismiss-processing'].map(byId).filter(node => !byId('processing').hidden && usable(node));
  const forJob = id => banner.filter(node => node.dataset.jobId === id);
  const same = forJob(saved.id);
  const nextIds = saved.ids.slice(saved.index + 1);
  const previousIds = saved.index < 0 ? [] : saved.ids.slice(0, saved.index).reverse();
  const next = nextIds.map(id => forJob(id)[0]).find(Boolean);
  const previous = previousIds.map(id => forJob(id)[0]).find(Boolean);
  const successor = forJob(processingJobs(state.processing).find(job => job.id === saved.id)?.retrySuccessorId)[0];
  const target = same.find(node => node.dataset.jobAction === saved.action) || same[0] || successor || next || previous
    || banner[0]
    || ['settings', 'navigation-toggle', 'library-title'].map(byId).find(usable);
  processingFocus = null;
  target?.focus({ preventScroll: true });
}

const processingCommands = { cancel: 'cancelProcessing', retry: 'retryProcessing', ack: 'acknowledgeProcessing' };
// Failures throw to the caller, which shows them where the action was asked; the banner also keeps them per job.
async function processingAction(action, jobId = state.processing.id) {
  const job = processingJobs(state.processing).find(item => item.id === jobId);
  const allowed = action === 'cancel' ? ['queued', 'running'].includes(job?.status) && state.processingCancellingId !== job.id
    : action === 'retry' ? canRetryProcessing(job) && (localSummaryEnabled() || !localSummaryJob(job))
      : action === 'ack' && canDismissProcessing(job);
  if (!allowed || processingActions.has(job.id)) return;
  if (action === 'cancel') state.processingCancellingId = job.id;
  rememberProcessingFocus();
  processingActions.set(job.id, action);
  processingErrors.delete(job.id);
  render();
  try {
    const progress = await bridge[processingCommands[action]](job.id);
    if (action !== 'cancel') await acceptProcessing(progress);
  } catch (error) {
    if (action === 'cancel') {
      if (state.processingCancellingId === job.id) state.processingCancellingId = null;
      processingActions.delete(job.id);
    }
    processingErrors.set(job.id, error instanceof Error ? error.message : String(error));
    throw error;
  } finally {
    // A requested cancel stays pending until the job reports that it ended.
    if (action !== 'cancel') processingActions.delete(job.id);
    render();
  }
}

function transcriptRenderKey() {
  return JSON.stringify([state.selectedId, state.transcriptDraft?.doc?.version, state.transcriptLoadId, state.transcriptEditing, documentSearch.query()]);
}

function resizeTranscriptEditor(input) {
  if (!input?.style || byId('transcript-section').hidden) return;
  input.style.height = 'auto';
  input.style.height = `${input.scrollHeight + 2}px`;
}

function renderTranscript(meeting) {
  const job = { ...state.processing, ...pendingJob(state.processing, { meetingId: meeting.id }) };
  const ownJob = Boolean(pendingJob(job, { meetingId: meeting.id }));
  const draft = state.transcriptDraft;
  const liveRows = !draft?.doc ? resultPreview(meeting).rows : [];
  const loaded = state.transcriptLoaded && !state.transcriptLoadError;
  const recognizable = Boolean(meeting.audio) && job.supported && job.modelReady && !ownJob && !state.busy;
  const failedHere = ['failed', 'cancelled'].includes(job.status) && job.kind === 'transcribe' && job.meetingId === meeting.id;
  byId('transcribe').hidden = Boolean(draft?.doc) || ownJob || !loaded;
  byId('transcribe').disabled = !recognizable;
  setText('transcribe', failedHere ? '다시 시도' : '텍스트로 변환');
  byId('retranscribe').hidden = !draft?.doc || ownJob || !loaded;
  byId('retranscribe').disabled = !recognizable;
  byId('transcript-history').hidden = !draft?.doc;
  byId('transcript-history').disabled = state.busy;
  const menu = byId('transcript-menu');
  menu.hidden = byId('retranscribe').hidden && byId('transcript-history').hidden;
  if (menu.hidden && menu.open) menu.open = false;
  byId('retry-transcript-load').hidden = !state.transcriptLoadError;
  byId('save-transcript').disabled = !draft?.dirty || draft.saving || ownJob;
  documentActions.transcript.update({ id: meeting.id, available: Boolean(draft?.doc?.segments?.length), busy: state.busy || ownJob });
  let message = '';
  if (!state.transcriptLoaded) message = '음성 기록 불러오는 중';
  else if (state.transcriptLoadError) message = '음성 기록을 불러오지 못했습니다.';
  else if (draft?.dirty) message = '저장되지 않은 수정이 있습니다.';
  else if (ownJob) message = job.kind === 'diarize' || job.phase === 'diarizing' ? '' : processingFeedback(job).title;
  else if (liveRows.length) message = activeSession()?.meetingId === meeting.id ? '초안' : '';
  else if (!draft?.doc && !meeting.audio) message = '음성 기록을 준비하고 있습니다.';
  else if (!draft?.doc && !job.supported) message = '이 환경에서는 음성 기록을 변환할 수 없습니다.';
  else if (!draft?.doc && !job.modelReady) message = '음성 인식 모델을 먼저 준비해 주세요.';
  else if (!draft?.doc) message = '아직 텍스트로 변환하지 않았습니다.';
  if (!state.transcriptLoaded) setHTML('transcript-message', '<span class="activity-spinner" aria-hidden="true"></span> ' + escape(message));
  else setText('transcript-message', message);

  byId('save-transcript').hidden = !draft?.dirty || state.transcriptEditing != null;
  byId('transcript-section').setAttribute('aria-busy', String(ownJob || !state.transcriptLoaded));
  const key = `${transcriptRenderKey()}:${liveRows.length ? JSON.stringify(liveRows) : ''}`;
  if (key !== state.transcriptKey) {
    state.transcriptKey = key;
    const scroll = byId('transcript-scroll').scrollTop;
    byId('transcript-rows').innerHTML = liveRows.length ? liveRows.map((row,index)=>globalThis.NeroUI.transcriptRow({ index, time:row.start == null ? '' : duration(row.start*1000),start:row.start ?? 0,text:row.text,originalText:row.text,editing:false,speaker:row.speaker ? `화자 ${row.speaker}` : '미지정',speakerVisible:row.speaker != null,searchQuery:'',disabled:true })).join('') : (draft?.doc?.segments || []).map((segment,index) => globalThis.NeroUI.transcriptRow({ index, time: duration(segment.start * 1000), start: segment.start, text: segment.text, originalText: segment.originalText, editing: state.transcriptEditing === index, speaker: speakers.label(segment), speakerVisible: true, searchQuery: documentSearch.query(), disabled: ownJob || state.editLocks > 0 })).join('');
    byId('transcript-rows').querySelectorAll('textarea').forEach(resizeTranscriptEditor);
    byId('transcript-scroll').scrollTop = scroll;
  }
  byId('transcript-rows').querySelectorAll('textarea').forEach((input) => { input.readOnly = ownJob || state.editLocks > 0; });
  byId('transcript-rows').querySelectorAll('[data-transcript-edit], [data-transcript-done], [data-transcript-cancel], [data-speaker-edit]').forEach(input => { input.disabled = liveRows.length > 0 || ownJob || state.editLocks > 0; });
  if (liveRows.length) byId('transcript-rows').querySelectorAll('[data-seek]').forEach(input => { input.disabled = true; });
  byId('transcript-rows').querySelectorAll('[data-transcript-done]').forEach(input => {
    const html = draft?.saving ? '<span class="activity-spinner" aria-hidden="true"></span>저장 중' : '완료';
    if (input.innerHTML !== html) input.innerHTML = html;
  });
}

function replaceTranscriptDraft(id, doc) {
  state.transcriptDraft = { id, doc, savedTexts: doc?.segments.map(segment => segment.text) || [], dirty: false, saving: false, version: 0 };
  state.transcriptEditing = null;
  state.transcriptKey = '';
}

async function loadTranscript(id) {
  const request = ++state.transcriptLoadId;
  const startingDraft = state.transcriptDraft;
  const startingVersion = JSON.stringify(startingDraft?.doc?.version);
  const current = () => request === state.transcriptLoadId && state.selectedId === id && state.transcriptDraft === startingDraft && JSON.stringify(state.transcriptDraft?.doc?.version) === startingVersion && !state.transcriptDraft?.dirty;
  try {
    const doc = await bridge.readTranscript(id);
    if (!current()) return;
    state.transcriptLoaded = true; state.transcriptLoadError = false;
    replaceTranscriptDraft(id, doc);
    render();
  } catch (error) { if (current()) { state.transcriptLoaded = true; state.transcriptLoadError = true; showError(error); render(); } }
}

function saveTranscript() {
  const draft = state.transcriptDraft;
  if (!draft?.dirty || !draft.doc || state.transcriptComposing) return state.transcriptSaveQueue;
  const version = draft.version;
  const texts = draft.doc.segments.map((segment) => segment.text);
  draft.saving = true;
  renderTranscript(currentMeeting());
  const task = state.transcriptSaveQueue.catch(() => {}).then(async () => {
    const saved = await bridge.saveTranscript(draft.id, draft.doc.version, texts);
    const latest = draft.doc.segments.map((segment) => segment.text);
    draft.doc = saved;
    draft.savedTexts = saved.segments.map(segment => segment.text);
    refreshMeetingSearch();
    if (draft.version === version) draft.dirty = false;
    else draft.doc.segments.forEach((segment,index) => { segment.text = latest[index]; });
  });
  state.transcriptSaveQueue = task.catch(() => {});
  return task.catch((error) => { showError(error); throw error; }).finally(() => {
    draft.saving = false;
    if (state.transcriptDraft === draft) {
      state.transcriptKey = transcriptRenderKey();
      render();
    }
  });
}

async function acceptProcessing(progress) {
  if (progress.revision != null && state.processing.revision != null && progress.revision < state.processing.revision) return;
  const previous = state.processing;
  const key = JSON.stringify(progress);
  if (state.processingKey === key) return;
  state.processingKey = key;
  if (progress.liveTranscript) lastLiveTranscript = progress.liveTranscript;
  const switched = previous.speechSwitch?.target;
  if (switched && !progress.speechSwitch && progress.speechEngine === switched) {
    speechNotice = `${speechTitle(progress.speechChoices?.find(choice => choice.id === switched))} 모델로 전환했습니다. 새 녹음과 가져오기부터 적용됩니다.`;
  }
  state.processing = progress;
  for (const [id, action] of processingActions) {
    const job = processingJobs(progress).find(item => item.id === id);
    if (action === 'cancel' && (!job || !['queued', 'running', 'cancelling'].includes(job.status))) processingActions.delete(id);
  }
  if (progress.id !== state.processingCancellingId || progress.status !== 'running') state.processingCancellingId = null;
  render();
  const results = processingResults(previous, progress);
  if (results.length) {
    acceptSnapshot(await bridge.readSnapshot());
    const imported = results.find(job => job.kind === 'import' && job.status === 'completed');
    if (imported) await selectMeeting(imported.meetingId);
    if (results.some(job => job.meetingId === state.selectedId)) { await loadTranscript(state.selectedId); await loadSummary(state.selectedId); }
    refreshMeetingSearch();
  }
}

function summaryIsStale() {
  const doc = state.summary?.document, transcript = state.transcriptDraft?.doc;
  const notesChanged = doc?.notesSha256 != null && state.draft?.id === state.selectedId && state.draft.notes !== state.draft.savedNotes;
  return Boolean(state.summary?.stale || notesChanged || state.transcriptDraft?.dirty || (doc && state.transcriptLoaded && !sameTranscriptVersion(doc.sourceVersion, transcript?.version)));
}

function resultPreview(meeting) {
  return meetingResultPreview({ meeting, session: state.snapshot.session, preview: state.processing.liveTranscript || lastLiveTranscript, transcript: state.transcriptDraft?.doc, processing: state.processing });
}

function renderSummary(meeting) {
  const progress = { ...state.processing, ...pendingJob(state.processing, { meetingId: meeting.id, kind: 'summary' }) };
  const doc = state.summary?.document;
  const result = resultPreview(meeting);
  const hasTranscript = Boolean(state.transcriptDraft?.doc?.segments?.length);
  const stale = summaryIsStale();
  const ownJob = Boolean(pendingJob(progress, { meetingId: meeting.id, kind: 'summary' }));
  byId('summary-section').setAttribute('aria-busy', String(ownJob || result.busy || !state.summaryLoaded || !state.transcriptLoaded));
  byId('summary-section').setAttribute('aria-label', '요약');
  byId('summary-content').hidden = !doc;
  byId('prepare-summary').hidden = !localSummaryEnabled() || progress.summaryReady || !progress.summarySupported || !hasTranscript;
  byId('prepare-summary').disabled = Boolean(pendingJob(progress, { models: true })) || state.busy;
  byId('summarize').hidden = !localSummaryEnabled() || !hasTranscript || Boolean(state.summaryLoadError) || !state.summaryLoaded || !progress.summaryReady || !progress.summarySupported;
  byId('summary-template-choice').hidden = byId('summarize').hidden;
  byId('summarize').disabled = !hasTranscript || !summaryTemplatePicker.isAvailable() || !progress.summaryReady || !progress.summarySupported || ownJob || result.busy || state.busy;
  const latest = latestMeetingProcessing(state.processing, meeting.id);
  const failed = latest?.kind === 'summary' && latest.status === 'failed' && canDismissProcessing(latest);
  byId('summarize').textContent = failed ? '현재 내용 요약' : doc ? '다시 요약' : '요약하기';
  byId('summarize').className = doc ? 'small quiet' : 'small primary';
  documentActions.summary.update({ id: meeting.id, available: Boolean(doc), busy: state.busy || ownJob });
  let message = '';
  if (!state.summaryLoaded) message = '불러오는 중';
  else if (state.summaryLoadError) message = '요약을 불러오지 못했습니다.';
  else if (doc && stale) message = '이전 내용으로 만든 요약';
  else if (!doc && !localSummaryEnabled()) message = localSummaryMessage();
  else if (ownJob || failed) message = '';
  else if (!doc && result.busy) message = '음성 기록을 처리하고 있습니다.';
  else if (!doc && !state.transcriptLoaded) message = '불러오는 중';
  else if (!doc && state.transcriptLoadError) message = '음성 기록을 불러오지 못했습니다.';
  else if (!doc && !hasTranscript) message = result.visible ? '인식된 음성이 없습니다.' : '';
  else if (!doc && !progress.summarySupported) message = '이 기기에서는 요약을 사용할 수 없습니다.';
  else if (!doc && !progress.summaryReady) message = '요약 기능을 준비하면 이 회의를 요약할 수 있습니다.';
  else if (!doc) message = '음성 기록이 준비되었습니다. 템플릿을 선택해 요약해 보세요.';
  byId('retry-summary-load').hidden = !state.summaryLoadError;
  byId('summary-message').hidden = !message;
  const loading = !state.summaryLoaded || (localSummaryEnabled() && !doc && (result.busy || !state.transcriptLoaded));
  if (loading && message) setHTML('summary-message', '<span class="activity-spinner" aria-hidden="true"></span> ' + escape(message));
  else setText('summary-message', message);
  const key = JSON.stringify([meeting.id, doc?.id, state.transcriptDraft?.doc?.version, Boolean(stale), documentSearch.query()]);
  if (key !== state.summaryKey) {
    state.summaryKey = key;
    byId('summary-content').innerHTML = globalThis.NeroUI.summaryDocument(doc?.text || '', {
      searchQuery: documentSearch.query(),
    });
  }
  summaryEvidence?.update();
}
async function loadSummary(id) {
  const request = ++state.summaryLoadId;
  const references = () => {
    const meeting = state.snapshot.meetings.find(item => item.id === id);
    return JSON.stringify([meeting?.notes || '', meeting?.attendees || []]);
  };
  const startingReferences = references();
  const current = () => request === state.summaryLoadId && id === state.selectedId && startingReferences === references();
  try {
    const summary = await bridge.readSummary(id);
    if (!current()) return;
    state.summary = summary; state.summaryKey = ''; state.summaryLoaded = true; state.summaryLoadError = false;
    render();
  } catch (error) { if (current()) { state.summaryLoaded = true; state.summaryLoadError = true; showError(error); render(); } }
}
byId('retry-transcript-load').addEventListener('click', () => { if (state.selectedId) void loadTranscript(state.selectedId); });
byId('retry-summary-load').addEventListener('click', () => { if (state.selectedId) void loadSummary(state.selectedId); });
byId('prepare-summary').addEventListener('click', () => run(() => { if (localSummaryEnabled()) return openSetup(false, 'prepare-summary'); }));
byId('summarize').addEventListener('click', () => run(() => { if (localSummaryEnabled()) return finishEditing(async () => acceptProcessing(await bridge.summarizeMeeting(state.selectedId, await summaryTemplatePicker.selected()))); }));
async function exportDocument(kind, format, copy = false, includeCitations = false, expectedId = state.selectedId) {
  return finishEditing(async () => {
    const meeting = currentMeeting();
    if (!meeting || meeting.id !== expectedId) throw new Error('회의가 바뀌었습니다. 내용을 다시 확인해 주세요.');
    const result = await bridge.exportDocument(meeting.id, kind, format, includeCitations);
    if (copy) {
      await bridge.writeClipboardText(result.text);
      byId('copy-feedback').textContent = `${kind === 'summary' ? '요약' : '전체 음성 기록'}을 복사했습니다.`;
      return true;
    }
    return bridge.exportText(result.name, result.text);
  });
}
const documentActions = Object.fromEntries(['summary', 'transcript'].map(kind => [kind, mountDocumentActions({ container: byId(`${kind}-document-actions`), kind, onExport: exportDocument })]));
byId('recording-quality').addEventListener('click', event => {
  const action = event.target.closest('[data-quality-seek]');
  const meeting = currentMeeting();
  if (!action || action.disabled || !meeting?.audio || activeSession()?.meetingId === meeting.id) return;
  const seconds = Number(action.dataset.qualitySeek);
  const audioKey = audioKeyFor(meeting);
  if (Number.isFinite(seconds) && seconds >= 0) void run(async () => { await prepareAudio(meeting, false); if (state.selectedId === meeting.id && state.audioKey === audioKey) seekRecording(seconds); });
});

function highlightTranscriptRows(rows) {
  byId('transcript-rows').querySelectorAll('[data-citation-active]').forEach(node => node.removeAttribute('data-citation-active'));
  rows.forEach(row => row.setAttribute('data-citation-active', 'true'));
  rows[0]?.scrollIntoView({ block: 'center', behavior: globalThis.matchMedia?.('(prefers-reduced-motion: reduce)')?.matches ? 'instant' : 'smooth' });
  rows[0]?.focus({ preventScroll: true });
}

function seekRecording(seconds) {
  const player = byId('audio-player');
  const meetingId = state.selectedId, audioKey = state.audioKey, audioRequest = state.audioRequest;
  if (player.getAttribute('src')) {
    player.currentTime = seconds;
    void requestAudioPlayback(player, { isCurrent: () => state.selectedId === meetingId && state.audioKey === audioKey && state.audioRequest === audioRequest, onError: showError });
  }
}

byId('import-audio').addEventListener('click', () => run(() => finishEditing(async () => {
  if (!modelsReady()) return openSetup();
  const target = await bridge.draftAttachmentTarget(state.selectedId);
  const progress = await bridge.importAudio(target);
  if (progress) await acceptProcessing(progress);
})));
for (const id of ['cancel-processing', 'retry-processing', 'dismiss-processing']) byId(id).addEventListener('click', () => { processingAction(byId(id).dataset.jobAction, byId(id).dataset.jobId).catch(showError); });
byId('open-processing-meeting').addEventListener('click', () => {
  const job = processingJobs(state.processing).find(item => item.id === byId('open-processing-meeting').dataset.jobId);
  if (job?.meetingId) void run(() => selectMeeting(job.meetingId));
});
function closeTranscriptMenu() {
  const menu = byId('transcript-menu');
  menu.open = false;
  menu.querySelector('summary')?.focus({ preventScroll: true });
}
byId('transcribe').addEventListener('click', () => { void transcriptActions.rebuild(); });
byId('retranscribe').addEventListener('click', () => { closeTranscriptMenu(); void transcriptActions.rebuild(); });
byId('transcript-history').addEventListener('click', () => { closeTranscriptMenu(); void transcriptActions.openHistory(); });
byId('save-transcript').addEventListener('click', () => finishEditing(() => {}).catch(() => {}));
function onTranscriptEdit(event) {
  const index = event.target?.dataset.segment;
  const draft = state.transcriptDraft;
  if (index == null || !draft?.doc) return;
  draft.doc.segments[Number(index)].text = event.target.value;
  draft.version += 1; draft.dirty = draft.doc.segments.some((segment,index) => segment.text !== draft.savedTexts[index]);
  resizeTranscriptEditor(event.target);
  renderTranscript(currentMeeting());
  renderSummary(currentMeeting());
}
globalThis.addEventListener?.('resize', () => { byId('transcript-rows').querySelectorAll('textarea').forEach(resizeTranscriptEditor); });
byId('transcript-rows').addEventListener('input', onTranscriptEdit);
byId('transcript-rows').addEventListener('compositionstart', () => { state.transcriptComposing = true; });
byId('transcript-rows').addEventListener('compositionend', (event) => { state.transcriptComposing = false; onTranscriptEdit(event); });
async function editTranscriptRow(index) {
  const meetingId = state.selectedId;
  if (state.editLocks || pendingJob(state.processing, { meetingId })) return;
  try {
    await finishEditing(() => {
      if (state.selectedId !== meetingId || !state.transcriptDraft?.doc?.segments[index]) return;
      state.transcriptEditing = index;
      renderTranscript(currentMeeting());
    });
    if (state.selectedId === meetingId && state.transcriptEditing === index) byId('transcript-rows').querySelector(`[data-segment="${index}"]`)?.focus({ preventScroll: true });
  } catch { return; }
}

async function completeTranscriptRow() {
  if (state.editLocks || state.transcriptEditing == null) return;
  const meetingId = state.selectedId;
  const index = state.transcriptEditing;
  try {
    await finishEditing(() => {
      if (state.selectedId !== meetingId || state.transcriptEditing !== index) return;
      state.transcriptEditing = null;
      renderTranscript(currentMeeting());
    });
    if (state.selectedId === meetingId) byId('transcript-rows').querySelector(`[data-transcript-edit="${index}"]`)?.focus({ preventScroll: true });
  } catch { return; }
}

async function cancelTranscriptRow() {
  if (state.editLocks) return;
  const meetingId = state.selectedId;
  if (state.transcriptComposing) {
    document.activeElement?.blur();
    await new Promise(requestAnimationFrame);
    state.transcriptComposing = false;
  }
  if (state.selectedId !== meetingId) return;
  const index = state.transcriptEditing, draft = state.transcriptDraft;
  if (index == null || !draft?.doc?.segments[index]) return;
  draft.doc.segments[index].text = draft.savedTexts[index];
  draft.version += 1;
  draft.dirty = draft.doc.segments.some((segment,index) => segment.text !== draft.savedTexts[index]);
  state.transcriptEditing = null;
  renderTranscript(currentMeeting());
  renderSummary(currentMeeting());
  byId('transcript-rows').querySelector(`[data-transcript-edit="${index}"]`)?.focus({ preventScroll: true });
}

byId('transcript-rows').addEventListener('click', (event) => {
  const speaker = event.target.closest('[data-speaker-edit]');
  if (speaker) { void speakers.open(Number(speaker.dataset.speakerEdit)); return; }
  const edit = event.target.closest('[data-transcript-edit]');
  if (edit) { void editTranscriptRow(Number(edit.dataset.transcriptEdit)); return; }
  if (event.target.closest('[data-transcript-done]')) { void completeTranscriptRow(); return; }
  if (event.target.closest('[data-transcript-cancel]')) { void cancelTranscriptRow(); return; }
  const target = event.target.closest('[data-seek]');
  if (!target) return;
  seekRecording(Number(target.dataset.seek));
});
byId('transcript-rows').addEventListener('keydown', event => {
  if (event.isComposing || state.transcriptComposing || event.target.dataset.segment == null) return;
  if (event.key === 'Escape') { event.preventDefault(); void cancelTranscriptRow(); }
  else if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') { event.preventDefault(); void completeTranscriptRow(); }
});

// macOS logout, Cmd+Q and Dock quit wait for an answer. Any path that does not end
// in quitting tells the app to stay; after a successful quit this does nothing.
const stayOpen = () => bridge.cancelQuit().catch(() => {});
async function onQuitRequest({ sessionId } = {}) {
  if (!activeSession()) {
    await run(() => prepareAppLeave(() => bridge.requestQuit(null)));
    await stayOpen();
    return;
  }
  if (sessionId !== activeSession().id || !await prepareAppLeave(() => true)) {
    await stayOpen();
    return;
  }
  pendingQuitSessionId = sessionId;
  const dialog = byId('quit-dialog');
  dialog.innerHTML = globalThis.NeroUI.dialogContent({ title: '녹음을 종료하고 나갈까요?', titleId: 'quit-title', bodyHTML: '<p>지금까지 녹음한 음성과 메모를 저장한 뒤 앱을 종료합니다.</p>', actionsHTML: button('계속 녹음', 'cancel-quit', null, '') + button('저장하고 종료', 'confirm-quit', null, 'primary') });
  if (!dialog.open) dialog.showModal();
  dialog.querySelector('[data-action="cancel-quit"]').focus();
}
byId('quit-dialog').addEventListener('click', (event) => {
  const action = event.target.closest('[data-action]')?.dataset.action;
  if (action === 'cancel-quit') byId('quit-dialog').close();
  if (action === 'confirm-quit') {
    const sessionId = pendingQuitSessionId;
    pendingQuitSessionId = null;
    byId('quit-dialog').close();
    void run(() => finishEditing(() => bridge.requestQuit(sessionId))).finally(stayOpen);
  }
});
// Closing without confirming (button, Escape) keeps the app open.
byId('quit-dialog').addEventListener('close', () => {
  if (pendingQuitSessionId !== null) void stayOpen();
  pendingQuitSessionId = null;
});
const dropConfirmation = mountConfirmDialog({ container: byId('drop-dialog') });
mountDropImport({ window: globalThis, overlay: byId('drop-overlay'), message: byId('drop-overlay-message'),
  onDrop: (files, onProgress) => run(() => finishEditing(async () => {
    if (files.length !== 1) { showError('음성 파일은 한 번에 하나씩 끌어 놓아 주세요.'); return; }
    if (activeSession()) { showError('녹음을 마친 뒤 음성을 가져와 주세요.'); return; }
    if (!modelsReady()) { await openSetup(); return; }
    const meeting = state.libraryOpen ? null : currentMeeting();
    // An empty draft takes the file; a meeting with audio keeps it and the file starts a new one.
    let target = null;
    if (meeting && !meeting.audio) target = await bridge.draftAttachmentTarget(meeting.id).catch(() => null);
    if (meeting && !target && !await dropConfirmation.confirm({ title: '새 회의로 가져올까요?',
      description: meeting.audio ? '이 회의에는 이미 음성이 있습니다. 끌어 놓은 파일로 새 회의를 만들고, 지금 회의의 음성과 기록은 그대로 둡니다.'
        : '이 회의에는 음성을 연결할 수 없습니다. 끌어 놓은 파일로 새 회의를 만듭니다.',
      action: '새 회의로 가져오기' })) return;
    await acceptProcessing(await uploadDroppedFile(bridge, files[0], target, onProgress));
  })) });
const speechOffer = mountSpeechOfferDialog({ dialog: byId('speech-offer-dialog'),
  onAccept: async () => { speechNotice = ''; await acceptProcessing(await bridge.respondSpeechOffer(true)); },
  onDecline: async () => { await acceptProcessing(await bridge.respondSpeechOffer(false)); } });

const detail = mountMeetingDetail({ byId, finishEditing, onError: showError, onReveal(view) {
  if (view !== 'transcript') summaryEvidence?.reset();
  if (view === 'transcript') byId('transcript-rows').querySelectorAll('textarea').forEach(resizeTranscriptEditor);
  for (const actions of Object.values(documentActions)) actions.close();
  globalThis.NeroControls.close?.();
} });
meetingContext = mountMeetingContext({ node: byId('meeting-context-control'), bridge: aiChatBridge, getMeeting: () => state.libraryOpen ? null : currentMeeting(), onSettings: () => { void openSettings('agent', 'meeting-context-control').catch(showError); } });
// Opening a conversation keeps the shown meeting; its meeting link moves the main view.
async function openAiConversation(id) { await aiChat.flow.selectConversation(id); aiChat.open(); }
async function openAiMeeting(meetingId) {
  if (!state.snapshot.meetings.some(meeting => meeting.id === meetingId && !meeting.deletedAt)) throw new Error('이 회의를 열 수 없습니다.');
  if (state.selectedId !== meetingId || state.libraryOpen) await selectMeeting(meetingId);
}
aiChat = mountAiChat({
  node: byId('ai-chat-inspector'), settingsNode: byId('ai-account-settings'), trigger: byId('ai-chat-open'), layoutRoot: byId('app-body'), resizeHandle: byId('ai-chat-resize'),
  bridge: { ...aiChatBridge,
    appendNote: request => finishEditing(async () => {
      const result = await aiChatBridge.appendNote(request);
      acceptSnapshot(await bridge.readSnapshot());
      return result;
    }),
    saveSummary: request => finishEditing(async () => {
      return aiChatBridge.saveSummary(request);
    }),
  },
  beforeWrite: () => finishEditing(() => true),
  writeClipboard: bridge.writeClipboardText,
  getTemplate: async () => (await summaryTemplatePicker.selected()).id,
  getMeetings: () => state.snapshot.meetings.filter(meeting => !meeting.deletedAt),
  onOpenMeeting: meetingId => openAiMeeting(meetingId),
  onSettings: () => { void openSettings('agent', 'ai-chat-open').catch(showError); },
  onSaved: async (kind, meetingId) => { if (state.selectedId === meetingId) { if (kind === 'summary') await loadSummary(meetingId); render(); } },
  async onCitation(citation) {
    if (citation.kind === 'notion') { await aiChatBridge.openSource(citation.url); return; }
    if (citation.kind !== 'meeting' || !citation.meetingId || !citation.segmentId || !state.snapshot.meetings.some(meeting => meeting.id === citation.meetingId && !meeting.deletedAt)) throw new Error('이 회의의 출처를 열 수 없습니다.');
    if (state.selectedId !== citation.meetingId || state.libraryOpen) await selectMeeting(citation.meetingId);
    await finishEditing(async () => {
      if (citation.meetingId !== state.selectedId) return;
      const index = state.transcriptDraft?.doc?.segments.findIndex(segment => segment.id === citation.segmentId) ?? -1;
      if (index < 0) throw new Error('현재 음성 기록에서 이 발언을 찾을 수 없습니다.');
      await detail.select('transcript');
      const row = byId('transcript-rows').querySelector(`[data-transcript-row="${index}"]`);
      if (!row) throw new Error('현재 음성 기록에서 이 발언을 찾을 수 없습니다.');
      highlightTranscriptRows([row]);
    });
  },
});
aiSidebar = mountAiChatSidebar({ node: byId('sidebar-ai-preview'), flow: aiChat.flow, getMeetings: () => state.snapshot.meetings, onOpenConversation: openAiConversation, async onOpenMeeting(meetingId, conversationId) { await openAiConversation(conversationId); await openAiMeeting(meetingId); } });
byId('sidebar-ai-new').addEventListener('click', async () => { await aiChat.flow.newConversation({ attachCurrent: true }); aiChat.open(); });
aiLibrary = mountAiChatLibrary({ node: byId('ai-library-screen'), flow: aiChat.flow, getMeetings: () => state.snapshot.meetings, onOpen: () => aiChat.open(null), container: document.body });
summaryEvidence = mountSummaryEvidence({
  container: byId('summary-content'), panel: byId('summary-evidence'),
  onOpen: () => detail.select('transcript'),
  onReset: () => highlightTranscriptRows([]),
  getContext: () => ({ meetingId: state.selectedId, summaryId: state.summary?.document?.id, currentVersion: state.transcriptDraft?.doc?.version, dirty: Boolean(state.transcriptDraft?.dirty), canPlay: Boolean(byId('audio-player').getAttribute('src')) }),
  loadCitations: bridge.readSummaryCitations,
  onError: error => console.warn('Summary evidence unavailable', error),
  async onNavigate(entry, response, context) {
    const valid = () => context.meetingId === state.selectedId && context.summaryId === state.summary?.document?.id && !context.dirty && !state.transcriptDraft?.dirty
      && state.transcriptDraft?.doc && sameTranscriptVersion(context.currentVersion, state.transcriptDraft.doc.version)
      && sameTranscriptVersion(response.currentVersion, state.transcriptDraft.doc.version)
      && entry.currentSegmentIds?.length && entry.currentSegmentIds.every(id => state.transcriptDraft.doc.segments.some(segment => segment.id === id));
    if (!valid()) return false;
    state.pendingSearchIntent = null;
    if (detail.active() !== 'transcript' || !valid()) return false;
    const rows = entry.currentSegmentIds.map(id => {
      const index = state.transcriptDraft.doc.segments.findIndex(segment => segment.id === id);
      return byId('transcript-rows').querySelector(`[data-transcript-row="${index}"]`);
    });
    if (rows.some(row => !row)) return false;
    highlightTranscriptRows(rows);
    return true;
  },
  onPlay(entry, response, context) {
    if (context.meetingId !== state.selectedId || context.summaryId !== state.summary?.document?.id || context.dirty !== Boolean(state.transcriptDraft?.dirty)
      || !state.transcriptDraft?.doc || !sameTranscriptVersion(context.currentVersion, state.transcriptDraft.doc.version) || !response.audioAvailable
      || response.sourceSessionId !== currentMeeting()?.audio?.sessionId || !Number.isFinite(entry.segment?.start) || entry.segment.start < 0) return;
    speakerReplay = null; seekRecording(entry.segment.start);
  },
});
for (const view of ['transcript', 'notes']) byId(`detail-${view}-open`).addEventListener('click', () => { state.pendingSearchIntent = null; summaryEvidence?.reset(); });
const documentSearch = mountDocumentSearch({ byId, prepare: () => finishEditing(() => true), onError: showError, selectView: view => detail.select(view), ready: () => state.summaryLoaded && state.transcriptLoaded, renderContent() {
  const meeting = currentMeeting(); if (meeting) { renderTranscript(meeting); renderSummary(meeting); renderNotesSearch(); }
} });
const transcriptActions = mountTranscriptActions({ byId, bridge, finishEditing, showError, escape,
  getContext: () => ({ meeting: currentMeeting(), transcript: state.transcriptDraft?.doc, processing: state.processing, recording: activeSession() }),
  onRebuild: async meetingId => acceptProcessing(await bridge.transcribeAudio(meetingId)),
  async onRestored(doc, meetingId) {
    if (state.selectedId === meetingId) {
      replaceTranscriptDraft(meetingId, doc);
      state.summaryKey = ''; render();
      await loadSummary(meetingId);
    }
    acceptSnapshot(await bridge.readSnapshot()); refreshMeetingSearch();
  },
});
const library = mountLibrary({ bridge, getState: () => state, byId, finishEditing, acceptSnapshot, selectMeeting, render, showError, escape, icon, recordingStatus, duration, beginNavigation, phaseOf: stateFor,
  isAudioReady: id => { const meeting = state.snapshot.meetings.find(item => item.id === id); return Boolean(meeting?.audio) && state.audioKey === audioKeyFor(meeting) && byId('audio-message').dataset.state === 'ready'; }, onViewChanged() { meetingSearch.update(byId('search').value, state.libraryFilter === 'trash'); if (state.libraryFilter === 'ai') { aiLibrary?.show(); aiChat?.open(null); } } });
const notificationCenter = mountNotificationCenter({ container: byId('notification-center'), onOpenMeeting: id => selectMeeting(id), onProcessingAction: processingAction, showError, icon, onNoticeAction: service => service === 'app-update' ? appUpdateView.perform() : undefined, onOpenIntegration: async service => {
  await openSettings('agent', 'notifications-open');
  const target = service === 'google-calendar' ? byId('calendar-settings').querySelector('.calendar-settings-account button:not(:disabled)') : byId(`settings-${service}-connect`);
  target?.scrollIntoView?.({ block: 'nearest' });
}, onChanged: () => render() });
sidebarViews = mountSidebarViews({ aside: byId('navigation-sidebar'), navigation: byId('sidebar-navigation-view'), notifications: byId('notification-center'), trigger: byId('notifications-open'), badge: byId('notification-count'), onPreview: view => { library.preview(view); if (view === 'ai') aiSidebar?.show(); } });
byId('sidebar-preview-open').addEventListener('click', () => { const filter = byId('sidebar-preview-open').dataset.sidebarFilter; void library.show(['trash', 'ai'].includes(filter) ? filter : 'all'); });
viewHistory = createViewHistory({
  canVisit: route => route.kind === 'meeting' ? state.snapshot.meetings.some(meeting => meeting.id === route.id && !meeting.deletedAt) : ['all', 'unfiled', 'trash'].includes(route.filter) || state.snapshot.folders.some(folder => folder.id === route.filter),
  async navigate(route, { isCurrent }) {
    sidebarViews.close({ focus: false });
    if (route.kind === 'meeting') { await selectMeeting(route.id, null, isCurrent); return isCurrent() && !state.libraryOpen && state.selectedId === route.id; }
    return await library.show(route.filter, { isCurrent, history: true, beforeCommit() { byId('search').value = route.query || ''; state.libraryView = route.view || 'folders'; } });
  },
  onChange(status) { byId('navigation-back').disabled = !status.canBack; byId('navigation-forward').disabled = !status.canForward; },
});
for (const [id, direction] of [['navigation-back', -1], ['navigation-forward', 1]]) byId(id).addEventListener('click', () => { void viewHistory.go(direction).catch(showError); });
document.addEventListener('keydown', event => {
  if (event.defaultPrevented || event.isComposing || event.target.closest?.('input,textarea,[contenteditable="true"],dialog[open],[role="dialog"],[role="alertdialog"]')) return;
  const direction = event.altKey && !event.metaKey && !event.ctrlKey ? { ArrowLeft: -1, ArrowRight: 1 }[event.key] : event.metaKey && !event.altKey && !event.ctrlKey ? { '[': -1, ']': 1 }[event.key] : null;
  if (direction) { event.preventDefault(); void viewHistory.go(direction).catch(showError); }
});
const speakerConfirmation = mountConfirmDialog({ container: byId('speaker-dialog') });
const speakers = mountSpeakers({ confirmDiscard: () => speakerConfirmation.confirm({ title: '화자 수정을 버릴까요?', description: '저장하지 않은 화자 이름과 발언 배정이 사라집니다.', action: '버리기', destructive: true }), bridge, getState: () => state, byId, finishEditing, showError,
  canPlay: () => Boolean(byId('audio-player').getAttribute('src')),
  isPlaying: () => !byId('audio-player').paused && !byId('audio-player').ended,
  onPause() { speakerReplay = null; byId('audio-player').pause(); },
  onPlay(seconds, context) {
    const segment = state.transcriptDraft?.doc?.segments[context.index];
    if (context.meetingId !== state.selectedId || segment?.id !== context.segmentId) return;
    speakerReplay = { meetingId: context.meetingId, end: context.endSeconds };
    seekRecording(seconds);
  }, onProcessing: acceptProcessing, onUpdated(doc, meetingId) {
  if (state.selectedId !== meetingId) return;
  if (state.transcriptDraft) { replaceTranscriptDraft(meetingId, doc); state.summaryKey = ''; }
  render(); refreshMeetingSearch();
} });
const meetingSearch = createMeetingSearch({ search: bridge.searchMeetings, apply(result) {
  state.librarySearchHits = result.hits; state.librarySearchIds = result.ids; state.librarySearchPending = result.pending; state.librarySearchUnavailable = result.unavailable;
  library.render(); renderSearchStatus();
} });

for (const event of ['play', 'pause', 'ended']) byId('audio-player').addEventListener(event, () => speakers.syncPlayback());
byId('audio-player').addEventListener('timeupdate', () => {
  if (speakerReplay?.meetingId === state.selectedId && byId('audio-player').currentTime >= speakerReplay.end) {
    speakerReplay = null; byId('audio-player').pause(); speakers.syncPlayback();
  }
});

try {
  await bridge.subscribeSnapshots(acceptSnapshot);
  await bridge.subscribeAgentWriteRequest(onAgentWriteRequest);
  await bridge.subscribeAgentChanged(onAgentChanged);
  await bridge.subscribeProcessingError(({ message }) => showError(message));
  await bridge.subscribeProcessing((progress) => { void acceptProcessing(progress).catch(showError); });
  await bridge.setAutoRecorder(state.autoRecorder);
  await acceptProcessing(await bridge.processingStatus());
  await bridge.subscribeOpenMeeting(async (meetingId, error) => { await selectMeeting(meetingId); if (error) showError(error); });
  await bridge.subscribeQuitRequest(onQuitRequest);
  await bridge.subscribeCloseRequest(() => run(() => prepareAppLeave(() => bridge.hideMain())));
  acceptSnapshot(await bridge.readSnapshot());
  await refreshDevices();
  await integrationMonitor.refresh();
  integrationMonitor.start();
  await appUpdateView.start();
} catch (error) {
  if (!state.connected) byId('welcome-message').textContent = '데스크톱 앱에서 열어 주세요.';
  showError(error);
}
render();

byId('meeting-attendees').addEventListener('click', () => run(() => finishEditing(() => { const meeting = currentMeeting(); if (meeting) return meetingAttendees.open(meeting.id, byId('meeting-attendees')); })));
