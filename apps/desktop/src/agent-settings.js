const statusLabels = { not_connected: '연결 안 됨', update_required: '업데이트 필요', installation_unknown: '설치 확인 필요', installed: '설치됨', disabled: '사용 안 함', connected: '연결됨', error: '확인 불가', unavailable: '사용 불가' };
const pendingLabels = { updating: '업데이트 중', checking: '확인 중', connecting: '연결 중' };

export function mountAgentSettings({ byId, bridge, dialog, integrationMonitor, schedule = setTimeout, unschedule = clearTimeout }) {
  let active = false;
  const connection = { info: null, loading: false, error: '', request: 0, refresh: 0, timer: null, pending: {}, errors: {} };

  function render() {
    const { info, error } = connection;
    dialog.update(info);
    const available = Boolean(info?.available && info.executablePath && info.config);
    const message = error || (info && !available ? info.unavailableReason || '연결 도구가 포함되지 않은 빌드입니다.' : '');
    byId('settings-agent-status').textContent = message;
    byId('settings-agent-status').hidden = !message;
    for (const client of ['claude', 'codex']) {
      const entry = info?.clients?.find(item => item.id === client);
      const pending = Boolean(connection.pending[client]);
      const pendingLabel = pendingLabels[connection.pending[client]];
      byId(`settings-${client}-spinner`).hidden = !pending && !(connection.loading && !info);
      const status = byId(`settings-${client}-status`);
      status.textContent = pending ? pendingLabel : !info ? (error ? '확인 불가' : '확인 중') : statusLabels[entry?.state] || '사용 불가';
      status.dataset.state = pending ? connection.pending[client] : !info ? (error ? 'error' : 'loading') : entry?.state || 'unavailable';
      const button = byId(`settings-${client}-connect`);
      button.hidden = !pending && entry?.state === 'connected';
      button.disabled = pending || !available || !entry || entry.state === 'unavailable' || entry.state === 'connected';
      button.textContent = pending ? pendingLabel : entry?.action === 'connect' && entry?.state === 'installation_unknown' ? '연결 화면 열기' : entry?.action === 'recheck' || ['installation_unknown', 'disabled'].includes(entry?.state) ? '다시 확인' : entry?.state === 'update_required' ? '업데이트' : entry?.state === 'error' ? '다시 확인' : '연결';
      button.setAttribute?.('aria-label', `${client === 'claude' ? 'Claude' : 'ChatGPT'} ${button.textContent}`);
      const errorNode = byId(`settings-${client}-error`);
      const failed = Boolean(connection.errors[client]) || entry?.state === 'error';
      const needsAction = entry?.state === 'disabled' || (entry?.state === 'installation_unknown' && entry?.action === 'recheck');
      errorNode.textContent = failed || needsAction ? connection.errors[client] || entry?.detail || '' : '';
      errorNode.className = failed ? 'notice' : 'settings-client-note';
      errorNode.setAttribute?.('role', failed ? 'alert' : 'status');
      errorNode.hidden = !errorNode.textContent;
    }
  }

  async function load(force = false, forceHostRefresh = false) {
    if (!active) {
      reset();
      return;
    }
    if (connection.loading || (!force && connection.info)) return;
    unschedule(connection.timer);
    connection.timer = null;
    const request = connection.request;
    const refresh = ++connection.refresh;
    connection.loading = true;
    connection.error = '';
    render();
    try {
      const info = await bridge.agentConnectionInfo(forceHostRefresh);
      if (request !== connection.request || refresh !== connection.refresh || !active) return;
      connection.info = info;
      integrationMonitor.acceptAgents(info);
    } catch {
      if (request !== connection.request || refresh !== connection.refresh || !active) return;
      connection.error = '연결 상태를 불러오지 못했습니다.';
    } finally {
      if (request === connection.request) {
        connection.loading = false;
        if (active) {
          render();
          connection.timer = schedule(() => { connection.timer = null; void load(true); }, 3000);
        }
      }
    }
  }

  async function connect(client) {
    const entry = connection.info?.clients?.find(item => item.id === client);
    if (!active || connection.pending[client] || !connection.info?.available || !entry || ['unavailable', 'connected'].includes(entry.state)) return;
    const request = connection.request;
    const checking = entry.action === 'recheck' || (!entry.action && ['error', 'installation_unknown', 'disabled'].includes(entry.state));
    connection.pending[client] = checking ? 'checking' : entry.state === 'update_required' ? 'updating' : 'connecting';
    connection.errors[client] = '';
    connection.refresh += 1;
    render();
    try {
      const info = await (checking ? bridge.agentConnectionInfo(true) : bridge.connectAgentClient(client));
      if (request !== connection.request || !active) return;
      integrationMonitor.acceptAgents(info);
      const updated = info.clients?.find(item => item.id === client);
      if (updated) connection.info = { ...connection.info, clients: connection.info.clients.map(item => item.id === client ? updated : item) };
      if (client === 'claude' && info.claudeExtensionPath) dialog.openClaudeExtension(info.claudeExtensionPath, byId('settings-claude-connect'));
    } catch (error) {
      const message = typeof error === 'string' ? error : error?.message;
      const reason = typeof message === 'string' && message.length <= 160 && !/[\r\n]/.test(message) && /[가-힣]/.test(message) ? message : checking ? '확인하지 못했습니다. 다시 시도해 주세요.' : '연결하지 못했습니다. 다시 시도해 주세요.';
      if (request === connection.request && active) connection.errors[client] = reason;
    } finally {
      delete connection.pending[client];
      connection.refresh += 1;
      if (active) {
        render();
        if (!checking) void load(true);
      }
    }
  }

  // A request still in flight keeps its pending row across reopening; its reply is ignored.
  function reset() {
    dialog.close({ restoreFocus: false });
    connection.request += 1;
    connection.refresh += 1;
    unschedule(connection.timer);
    connection.timer = null;
    connection.info = null;
    connection.loading = false;
    connection.error = '';
    connection.errors = {};
  }

  function openPrompt() {
    if (active) dialog.open(connection.info, byId('settings-agent-manual'));
  }

  byId('settings-agent-manual').addEventListener('click', openPrompt);
  for (const client of ['claude', 'codex']) byId(`settings-${client}-connect`).addEventListener('click', () => { void connect(client); });
  globalThis.addEventListener?.('focus', () => { if (active) void load(true); });
  return {
    load,
    connect,
    openPrompt,
    setActive(value) {
      if (value === active) { render(); return; }
      active = value;
      if (value) return load(false, true);
      reset();
      render();
    },
  };
}
