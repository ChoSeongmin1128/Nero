// The tabs of the Settings dialog. Each panel registers the parts that work only while it is
// shown (`setActive`) and, when it has one, the message shown under the panel (`message`).
export function createSettingsPanels({ dialog, byId, panels = {} }) {
  const names = [...dialog.querySelectorAll('.app-settings-tabs [role="tab"][data-settings-panel]')].map(tab => tab.dataset.settingsPanel);
  const parts = name => [panels[name] || []].flat();
  return {
    names,
    has: name => names.includes(name),
    render(selected) {
      for (const name of names) {
        const shown = selected === name;
        byId(`settings-${name}-tab`).setAttribute('aria-selected', String(shown));
        byId(`settings-${name}-tab`).tabIndex = shown ? 0 : -1;
        byId(`settings-${name}-panel`).hidden = !shown;
        for (const part of parts(name)) part.setActive?.(dialog.open && shown);
      }
      return parts(selected).map(part => part.message?.() || '').find(Boolean) || '';
    },
    deactivate() { for (const name of names) for (const part of parts(name)) part.setActive?.(false); },
    // Arrow keys move between tabs in a loop; Home and End go to the first and last.
    neighbor(tabId, key) {
      const index = names.findIndex(name => `settings-${name}-tab` === tabId);
      if (index < 0 || !['ArrowDown', 'ArrowUp', 'ArrowRight', 'ArrowLeft', 'Home', 'End'].includes(key)) return null;
      if (key === 'Home') return names[0];
      if (key === 'End') return names.at(-1);
      const step = ['ArrowUp', 'ArrowLeft'].includes(key) ? -1 : 1;
      return names[(index + step + names.length) % names.length];
    },
  };
}
