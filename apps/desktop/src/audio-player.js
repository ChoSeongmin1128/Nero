const playbackRates = [0.75, 1, 1.25, 1.5, 2];

export function formatAudioTime(seconds) {
  const total = Math.floor(Number.isFinite(seconds) ? Math.max(0, seconds) : 0);
  const hours = Math.floor(total / 3600), minutes = Math.floor(total / 60) % 60;
  return `${hours ? `${hours}:` : ''}${hours ? String(minutes).padStart(2, '0') : minutes}:${String(total % 60).padStart(2, '0')}`;
}

function spokenTime(seconds) {
  const total = Math.floor(seconds);
  return `${total >= 3600 ? `${Math.floor(total / 3600)}시간 ` : ''}${total >= 60 ? `${Math.floor(total / 60) % 60}분 ` : ''}${total % 60}초`;
}

export function connectAudioPlayer(document, icons = globalThis.GroveIcons, controls = globalThis.GroveControls) {
  const get = id => document.getElementById(id);
  const audio = get('audio-player');
  if (!audio) return;
  const toggle = get('audio-toggle'), seek = get('audio-seek'), mute = get('audio-mute'), speed = get('audio-speed');
  const status = get('audio-player-status'), inspectorToggle = get('inspector-audio-toggle'), inspectorStatus = get('inspector-audio-status');
  let metadataReady = audio.readyState >= 1, generation = 0, lastIcon;
  function clearStatus() { status.textContent = ''; status.hidden = true; if (inspectorStatus) { inspectorStatus.textContent = ''; inspectorStatus.hidden = true; } }
  function render() {
    const ready = metadataReady && !!audio.getAttribute('src') && !audio.error;
    const duration = ready && Number.isFinite(audio.duration) ? Math.max(0, audio.duration) : 0;
    const current = ready && Number.isFinite(audio.currentTime) ? Math.min(duration || Infinity, Math.max(0, audio.currentTime)) : 0;
    const playing = ready && !audio.paused && !audio.ended;
    toggle.disabled = mute.disabled = speed.disabled = !ready;
    seek.disabled = !ready || duration <= 0;
    seek.max = String(duration); seek.value = String(current);
    seek.setAttribute('aria-valuetext', `${spokenTime(current)} / ${spokenTime(duration)}`);
    get('audio-current').textContent = formatAudioTime(current);
    get('audio-duration').textContent = formatAudioTime(duration);
    const label = playing ? '일시정지' : audio.ended && ready ? '다시 재생' : '재생';
    get('audio-toggle-label').textContent = label;
    toggle.setAttribute('aria-label', label);
    toggle.setAttribute('title', label);
    if (inspectorToggle) { inspectorToggle.disabled = !ready; inspectorToggle.textContent = label; inspectorToggle.setAttribute('aria-label', `녹음 ${label}`); }
    if (inspectorStatus) { inspectorStatus.textContent = status.textContent; inspectorStatus.hidden = status.hidden; }
    const icon = playing ? 'pause' : 'play';
    if (lastIcon !== icon) { get('audio-toggle-icon').innerHTML = icons?.(icon) || ''; lastIcon = icon; }
    mute.setAttribute('aria-pressed', String(audio.muted));
    mute.setAttribute('aria-label', audio.muted ? '음소거 해제' : '음소거');
    const muteLabel = audio.muted ? '음소거 해제' : '음소거';
    mute.innerHTML = `${icons?.(audio.muted ? 'volume-x' : 'volume-2') || ''}<span class="sr-only">${muteLabel}</span>`;
    mute.setAttribute('title', muteLabel);
    if (playbackRates.includes(audio.playbackRate)) speed.value = String(audio.playbackRate);
  }
  function renderControls() { render(); (controls || globalThis.GroveControls)?.refresh(); }
  function reset() { generation += 1; metadataReady = false; clearStatus(); renderControls(); }
  for (const name of ['emptied', 'loadstart']) audio.addEventListener(name, reset);
  for (const name of ['loadedmetadata', 'durationchange']) audio.addEventListener(name, () => {
    metadataReady = audio.readyState >= 1; renderControls();
  });
  for (const name of ['timeupdate', 'seeking', 'seeked', 'play', 'pause', 'ended', 'volumechange']) audio.addEventListener(name, render);
  audio.addEventListener('ratechange', renderControls);
  audio.addEventListener('error', () => { generation += 1; metadataReady = false; clearStatus(); renderControls(); });
  const togglePlayback = async () => {
    if (toggle.disabled) return;
    clearStatus();
    if (!audio.paused && !audio.ended) { audio.pause(); return; }
    const request = generation;
    try {
      if (audio.ended) audio.currentTime = 0;
      await audio.play();
    } catch (error) {
      if (request !== generation || error?.name === 'AbortError') return;
      status.textContent = '재생을 시작하지 못했습니다. 다시 재생해 주세요.';
      status.hidden = false; render();
    }
  };
  toggle.addEventListener('click', togglePlayback);
  inspectorToggle?.addEventListener('click', togglePlayback);
  seek.addEventListener('input', () => {
    if (seek.disabled) return;
    audio.currentTime = Math.min(audio.duration, Math.max(0, Number(seek.value)));
    clearStatus(); render();
  });
  mute.addEventListener('click', () => { if (!mute.disabled) { audio.muted = !audio.muted; render(); } });
  speed.addEventListener('change', () => {
    const rate = Number(speed.value);
    if (!speed.disabled && playbackRates.includes(rate)) audio.playbackRate = rate;
  });
  renderControls();
}

if (typeof document !== 'undefined') connectAudioPlayer(document);
