const clients = { claude: 'Claude', codex: 'ChatGPT' };

export function integrationNotices({ agents, calendar } = {}) {
  const result = [];
  if (agents?.available) {
    for (const [id, name] of Object.entries(clients)) {
      const client = agents.clients?.find(value => value.id === id);
      if (client?.state !== 'update_required') continue;
      result.push({ id: `integration:${id}:update:${client.packageVersion || ''}`, service: id,
        title: `${name} 연동`, message: '업데이트 필요' });
    }
  }
  if (calendar?.configured && calendar.accountLabel && !calendar.connected && !calendar.connecting
      && !calendar.restoring && calendar.error) {
    result.push({ id: 'integration:google-calendar:reconnect', service: 'google-calendar',
      title: 'Google Calendar', message: '연결 확인 필요' });
  }
  return result;
}

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
