import { formatBytes } from './processing-state.js';

export function mountStorageSettings({ byId, bridge, confirm }) {
  const get = name => byId(`storage-${name}`);
  let active = false, usage = null, message = '', request = 0, working = false;
  const current = value => active && value === request;

  function render() {
    for (const name of ['meetings', 'processing', 'models']) get(name).textContent = usage ? formatBytes(usage[`${name}Bytes`]) : '';
    const reducible = Boolean(usage?.reducibleMeetings);
    get('compact-row').hidden = !reducible;
    get('compact').disabled = working;
    get('compact-hint').textContent = reducible
      ? `녹음 ${usage.reducibleMeetings}개에서 약 ${formatBytes(usage.reducibleBytes)}를 줄일 수 있습니다.${usage.heldMeetings ? ` 처리 중이거나 확인하지 않은 작업이 있는 녹음 ${usage.heldMeetings}개는 제외됩니다.` : ''}`
      : '';
    get('message').textContent = message;
    get('message').hidden = !message;
  }

  async function reload() {
    const value = ++request;
    usage = null; message = '';
    render();
    if (!active) return;
    try {
      const next = await bridge.storageUsage();
      if (current(value)) usage = next;
    } catch {
      if (current(value)) message = '저장 공간을 확인하지 못했습니다. 탭을 다시 열어 주세요.';
    } finally {
      if (current(value)) render();
    }
  }

  async function compact() {
    const value = request;
    if (!current(value) || working || !usage?.reducibleMeetings) return;
    if (!await confirm({ title: '이전 형식 녹음을 줄일까요?',
      description: '녹음을 음성 인식에 쓰는 형식(16kHz 모노)으로 바꾸고 원본 녹음 파일을 지웁니다. 음성 기록은 달라지지 않지만 원본 음질로는 되돌릴 수 없습니다.',
      action: '줄이기', destructive: true })) return;
    if (!current(value)) return;
    working = true; message = '녹음을 줄이고 있습니다. 긴 녹음은 시간이 걸릴 수 있습니다.';
    render();
    try {
      const result = await bridge.compactRecordings();
      if (!current(value)) return;
      usage = result.usage;
      message = [result.meetings ? `${formatBytes(result.freedBytes)}를 확보했습니다.` : '',
        result.failed ? `녹음 ${result.failed}개는 줄이지 못했습니다. 원래 파일은 그대로 있습니다.` : ''].filter(Boolean).join(' ');
    } catch {
      if (current(value)) message = '녹음을 줄이지 못했습니다. 원래 파일은 그대로 있습니다.';
    } finally {
      working = false;
      if (current(value)) render();
    }
  }

  get('compact').addEventListener('click', () => { void compact(); });
  return {
    compact,
    setActive(value) {
      if (value === active) return;
      active = value;
      return reload();
    },
  };
}
