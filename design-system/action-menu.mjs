export function mountActionMenus({ owner, onEscape }) {
  let activeMenu = null;
  function menuContent(menu) { return menu?.querySelector(':scope > div'); }
  function closeMenu(menu = activeMenu, restore = false) {
    if (!menu) return;
    menu.open = false;
    menu.classList?.remove('is-open');
    if (activeMenu === menu) activeMenu = null;
    if (restore && menu.isConnected) menu.querySelector('summary')?.focus({ preventScroll: true });
  }
  function positionMenu(menu) {
    const content = menuContent(menu), trigger = menu.querySelector('summary');
    if (!content || !trigger) return;
    if (activeMenu && activeMenu !== menu) closeMenu(activeMenu);
    activeMenu = menu;
    menu.classList.add('is-open');
    const view = owner.defaultView;
    const width = view?.innerWidth || owner.documentElement?.clientWidth || 1024;
    const height = view?.innerHeight || owner.documentElement?.clientHeight || 768;
    content.style.width = `${Math.min(180, Math.max(0, width - 16))}px`;
    content.style.maxHeight = `${Math.max(0, height - 16)}px`;
    content.style.overflowY = 'auto';
    const anchor = trigger.getBoundingClientRect(), rect = content.getBoundingClientRect();
    const below = anchor.bottom + 6;
    const top = below + rect.height <= height - 8 ? below : anchor.top - rect.height - 6;
    content.style.left = `${Math.max(8, Math.min(anchor.right - rect.width, width - rect.width - 8))}px`;
    content.style.top = `${Math.max(8, Math.min(top, height - rect.height - 8))}px`;
  }
  owner?.addEventListener?.('toggle', event => {
    const menu = event.target;
    if (!menu.matches?.('[data-meeting-menu],.library-folder-menu')) return;
    if (menu.open) positionMenu(menu); else closeMenu(menu);
  }, true);
  owner?.addEventListener?.('pointerdown', event => { if (activeMenu && !activeMenu.contains(event.target)) closeMenu(); }, true);
  owner?.addEventListener?.('keydown', event => {
    if (event.key === 'Escape') {
      onEscape?.();
      if (!activeMenu) return;
      event.preventDefault(); event.stopPropagation(); closeMenu(activeMenu, true);
      return;
    }
    const menu = event.target.closest?.('[data-meeting-menu],.library-folder-menu');
    if (!menu || !['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
    if (menu.querySelector('summary')?.getAttribute?.('aria-disabled') === 'true') return;
    const buttons = [...menu.querySelectorAll('button:not(:disabled)')];
    if (!buttons.length) return;
    event.preventDefault();
    menu.open = true; positionMenu(menu);
    const index = buttons.indexOf(owner.activeElement);
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : event.key === 'ArrowDown' ? (index + 1) % buttons.length : (index < 0 ? buttons.length - 1 : (index - 1 + buttons.length) % buttons.length);
    buttons[next].focus({ preventScroll: true });
  }, true);
  owner?.addEventListener?.('scroll', event => {
    if (activeMenu && !menuContent(activeMenu)?.contains(event.target)) closeMenu();
  }, true);
  owner?.defaultView?.addEventListener('resize', () => closeMenu());
  return { close: closeMenu, position: positionMenu, get active() { return activeMenu; } };
}
