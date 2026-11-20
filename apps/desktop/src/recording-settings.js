const microphoneKey = 'nero.recording.microphone.v1';
const recorderKey = 'nero.recording.autoRecorder.v1';

export function readMicrophonePreference(storage = globalThis.localStorage) {
  try {
    const value = JSON.parse(storage?.getItem(microphoneKey) || 'null');
    return value && (value.id === null || typeof value.id === 'string') ? { id: value.id, name: typeof value.name === 'string' ? value.name : '' } : { id: null, name: '' };
  } catch { return { id: null, name: '' }; }
}

export function microphoneIssue({ devices, deviceId }) {
  if (!devices.length) return '연결된 마이크가 없습니다.';
  if (deviceId && !devices.some(device => device.id === deviceId)) return '선택한 마이크가 연결되어 있지 않습니다.';
  if (!deviceId && !devices.some(device => device.isDefault)) return '시스템 기본 마이크를 찾지 못했습니다.';
  return '';
}

// The recording panel of Settings: microphone, new-meeting folder, the recorder window and
// recording-time transcription. The device list and chosen microphone stay in the app state
// because recording reads them; this panel saves the choice and reports it with `onMicrophone`.
export function mountRecordingSettings({ byId, bridge, escape, getState, activeSession, folder, acceptProcessing, onMicrophone, onChange, storage = globalThis.localStorage }) {
  let error = '', deviceKey = '', recorderBusy = false, transcriptionBusy = false;
  let autoRecorder = (() => { try { return storage?.getItem(recorderKey) !== 'false'; } catch { return true; } })();
  const fail = message => { error = message; onChange(); };

  function render() {
    const state = getState();
    const select = byId('settings-microphone');
    const disabled = !state.connected || state.busy || Boolean(activeSession()) || !state.devices.length;
    const key = JSON.stringify([state.devices, state.deviceId, state.deviceName]);
    const changed = key !== deviceKey || select.disabled !== disabled;
    if (key !== deviceKey) {
      deviceKey = key;
      const missing = state.deviceId && !state.devices.some(device => device.id === state.deviceId);
      select.innerHTML = `<option value="">시스템 기본 마이크</option>${missing ? `<option value="${escape(state.deviceId)}" disabled>${escape(state.deviceName || '선택한 마이크')} (연결 안 됨)</option>` : ''}${state.devices.map(device => `<option value="${escape(device.id)}">${escape(device.name)}${device.isDefault ? ' (기본)' : ''}</option>`).join('')}`;
      select.value = state.deviceId || '';
    }
    select.disabled = disabled;
    byId('settings-microphone-hint').hidden = !activeSession();
    if (activeSession()) select.setAttribute('aria-describedby', 'settings-microphone-hint');
    else select.removeAttribute('aria-describedby');
    const folderSelect = byId('settings-new-meeting-folder');
    const folders = state.snapshot.folders || [];
    const folderKey = JSON.stringify([folders.map(item => [item.id, item.name]), folder.value()]);
    if (folderSelect.dataset.key !== folderKey) {
      folderSelect.dataset.key = folderKey;
      folderSelect.innerHTML = `<option value="">미분류</option>${folders.map(item => `<option value="${escape(item.id)}">${escape(item.name)}</option>`).join('')}`;
      folderSelect.value = folders.some(item => item.id === folder.value()) ? folder.value() : '';
    }
    folderSelect.disabled = !state.connected || folder.busy();
    byId('auto-recorder').checked = autoRecorder;
    byId('auto-recorder').disabled = recorderBusy;
    // Core says whether this Mac and the chosen engine can recognize chunks while recording.
    byId('recording-transcription-row').hidden = state.processing.recordingTranscriptionAvailable !== true;
    if (!transcriptionBusy) byId('recording-transcription').checked = state.processing.recordingTranscription === true;
    byId('recording-transcription').disabled = transcriptionBusy;
    return changed;
  }

  function changeMicrophone() {
    const state = getState();
    if (activeSession()) { deviceKey = ''; return fail('녹음을 마친 뒤 마이크를 변경할 수 있습니다.'); }
    const id = byId('settings-microphone').value || null;
    const device = state.devices.find(item => item.id === id);
    if (id && !device) { deviceKey = ''; return onChange(); }
    const preference = { id, name: device?.name || '' };
    try {
      if (!storage) throw new Error('Storage unavailable');
      storage.setItem(microphoneKey, JSON.stringify(preference));
      error = '';
      onMicrophone(preference);
    } catch { deviceKey = ''; fail('마이크 설정을 저장하지 못했습니다. 다시 시도해 주세요.'); }
  }
  async function changeRecorder() {
    const previous = autoRecorder, next = byId('auto-recorder').checked;
    recorderBusy = true; autoRecorder = next; onChange();
    try {
      await bridge.setAutoRecorder(next);
      try { storage.setItem(recorderKey, String(next)); }
      catch (failure) { await bridge.setAutoRecorder(previous); throw failure; }
      error = '';
    } catch { autoRecorder = previous; error = '녹음 제어창 설정을 저장하지 못했습니다.'; }
    finally { recorderBusy = false; onChange(); }
  }
  async function changeFolder() {
    const select = byId('settings-new-meeting-folder');
    try { await folder.set(select.value || null); error = ''; }
    catch { error = '새 회의 폴더를 저장하지 못했습니다.'; }
    finally { select.dataset.key = ''; onChange(); }
  }
  async function changeTranscription() {
    const next = byId('recording-transcription').checked;
    transcriptionBusy = true; onChange();
    try { await acceptProcessing(await bridge.setRecordingTranscription(next)); error = ''; }
    catch { error = '녹음 중 음성 기록 설정을 저장하지 못했습니다.'; }
    finally { transcriptionBusy = false; onChange(); }
  }
  byId('settings-microphone').addEventListener('change', changeMicrophone);
  byId('auto-recorder').addEventListener('change', () => changeRecorder());
  byId('settings-new-meeting-folder').addEventListener('change', () => changeFolder());
  byId('recording-transcription').addEventListener('change', () => changeTranscription());

  return {
    render,
    autoRecorder: () => autoRecorder,
    // Opening Settings starts without an old error; a failed device read reports here.
    reset() { error = ''; },
    fail,
    message() {
      const state = getState();
      return error || (!activeSession() && state.devicesLoaded ? microphoneIssue(state) : '');
    },
  };
}
