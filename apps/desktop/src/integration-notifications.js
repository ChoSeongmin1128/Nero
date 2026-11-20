const clients = { claude: 'Claude', codex: 'ChatGPT' };

export function integrationNotices({ agents, calendar } = {}) {
  const result = [];
  if (agents?.available) {
    for (const [id, name] of Object.entries(clients)) {
      const client = agents.clients?.find(value => value.id === id);
      if (client?.state !== 'update_required') continue;
      result.push({ id: `integration:${id}:update:${client.packageVersion || ''}`, service: id, attention: true,
        title: `${name} 연동`, ...(client.action === 'update'
          ? { message: '업데이트 필요', action: 'update', detail: `업데이트하기 전에는 ${name}가 Nero의 새 기능을 쓰지 못할 수 있습니다.` }
          : { message: client.detail || '업데이트 필요' }) });
    }
  }
  if (calendar?.configured && calendar.accountLabel && !calendar.connected && !calendar.connecting
      && !calendar.restoring && calendar.error) {
    result.push({ id: 'integration:google-calendar:reconnect', service: 'google-calendar', attention: true,
      title: 'Google Calendar', message: '연결 확인 필요', detail: '다시 연결하기 전에는 오늘 일정과 일정 알림이 나오지 않습니다.' });
  }
  return result;
}

// Notices for something that stops a feature until the person acts. They are marked where
// they are made, so home, the rail and the notification list agree on which they are.
export const attentionNotices = notices => (notices || []).filter(notice => notice.attention === true);

export function createIntegrationMonitor({ bridge, onChange = () => {} }) {
  let agents = null, calendar = null, disposed = false, generation = 0, pending = null, key = '[]', stop = () => {};
  const publish = () => {
    if (disposed) return;
    const notices = integrationNotices({ agents, calendar });
    const next = JSON.stringify(notices);
    if (next !== key) { key = next; onChange(notices); }
  };
  const acceptCalendar = value => {
    if (!disposed && value && (!calendar || value.revision >= calendar.revision)) { calendar = value; publish(); }
  };
  const acceptAgents = value => {
    if (disposed || !value) return;
    generation += 1; agents = value; publish();
  };
  const monitor = {
    acceptAgents, acceptCalendar,
    refresh() {
      if (disposed || pending) return pending;
      const current = generation;
      pending = Promise.allSettled([Promise.resolve().then(() => bridge.agentConnectionInfo(false)), Promise.resolve().then(() => bridge.readCalendarSettings())]).then(([agent, google]) => {
        if (disposed) return;
        if (agent.status === 'fulfilled' && current === generation) { generation += 1; agents = agent.value; }
        if (google.status === 'fulfilled' && google.value && (!calendar || google.value.revision >= calendar.revision)) calendar = google.value;
        publish();
      }).finally(() => { pending = null; });
      return pending;
    },
    start(target = globalThis.window, document = globalThis.document) {
      stop();
      if (disposed || !target?.addEventListener || !document?.addEventListener) return;
      const refresh = () => { if (document.visibilityState !== 'hidden') void monitor.refresh(); };
      target.addEventListener('focus', refresh);
      document.addEventListener('visibilitychange', refresh);
      const timer = target.setInterval(refresh, 60_000);
      const cleanup = () => monitor.dispose();
      target.addEventListener('beforeunload', cleanup, { once: true });
      stop = () => {
        target.clearInterval(timer); target.removeEventListener('focus', refresh);
        document.removeEventListener('visibilitychange', refresh); target.removeEventListener('beforeunload', cleanup);
      };
    },
    dispose() { disposed = true; generation += 1; stop(); },
  };
  return monitor;
}
