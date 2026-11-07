export function mountTerminologySettings({ byId, bridge, isProcessing, onChange }) {
  const row = byId('settings-terminology-row'), input = byId('terminology-enabled');
  let active = false, info = null, error = '', request = 0, saving = null;
  const current = value => active && value === request;

  function render() {
    row.hidden = !info?.available;
    input.checked = Boolean(info?.enabled);
    input.disabled = !info?.available || Boolean(saving) || isProcessing();
  }

  async function reload() {
    const value = ++request;
    info = null;
    error = '';
    render();
    if (!active) return;
    try {
      if (saving) await saving.catch(() => {});
      if (!current(value)) return;
      const next = await bridge.readTerminologySettings();
      if (!current(value)) return;
      info = next;
    } catch {
      if (!current(value)) return;
      error = '용어 설정을 불러오지 못했습니다. 탭을 다시 열어 주세요.';
    } finally {
      if (current(value)) { render(); onChange(); }
    }
  }

  async function change() {
    const previous = info, value = request;
    if (!current(value) || !previous?.available || saving || isProcessing()) { render(); return; }
    const enabled = input.checked;
    info = { ...previous, enabled };
    error = '';
    const pending = Promise.resolve().then(() => bridge.setTerminologyEnabled(enabled));
    saving = pending;
    render(); onChange();
    try {
      const next = await pending;
      if (current(value)) info = next;
    } catch {
      if (current(value)) {
        info = previous;
        error = '용어 설정을 저장하지 못했습니다.';
      }
    } finally {
      if (saving === pending) saving = null;
      if (current(value)) { render(); onChange(); }
    }
  }

  input.addEventListener('change', change);
  return {
    get error() { return error; },
    reload,
    change,
    setActive(value) {
      if (value === active) { render(); return; }
      active = value;
      return reload();
    },
  };
}
