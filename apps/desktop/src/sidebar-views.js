export function mountSidebarViews({ aside, navigation, notifications, trigger, badge, onChange = () => {} }) {
  const document = aside.ownerDocument;
  let opened = false;
  let unread = 0;
  let active = 0;
  let disposed = false;
  function paint() {
    aside.dataset.sidebarView = opened ? 'notifications' : 'navigation';
    navigation.hidden = opened;
    notifications.hidden = !opened;
    trigger.setAttribute('aria-controls', notifications.id);
    trigger.setAttribute('aria-expanded', String(opened));
    trigger.setAttribute('aria-pressed', String(opened));
    trigger.dataset.unread = String(unread > 0);
    trigger.dataset.active = String(active > 0);
    trigger.setAttribute('aria-label', unread ? `알림, 읽지 않은 알림 ${unread}개` : active ? '알림, 작업 진행 중' : '알림');
    trigger.title = unread ? '읽지 않은 알림' : '알림';
    if (badge) { badge.hidden = !unread; badge.textContent = ''; badge.setAttribute('aria-hidden', 'true'); }
  }
  function close({ focus = true } = {}) {
    if (disposed || !opened) return;
    opened = false;
    paint();
    onChange(false);
    if (focus && trigger.isConnected && !trigger.closest('[inert]')) trigger.focus({ preventScroll: true });
  }
  function open({ focus = true } = {}) {
    if (disposed || aside.closest('[data-onboarding="true"]')) return;
    opened = true;
    paint();
    aside.dispatchEvent(new document.defaultView.CustomEvent('nero:sidebar-reveal'));
    onChange(true);
    if (focus) notifications.querySelector('[data-notification-heading]')?.focus({ preventScroll: true });
  }
  const click = () => opened ? close() : open();
  const back = event => {
    if (!event.target.closest?.('[data-notification-action="back"]')) return;
    close();
  };
  const keydown = event => {
    if (!opened || event.key !== 'Escape' || event.defaultPrevented || event.isComposing || event.keyCode === 229) return;
    if (event.target.closest?.('[role="menu"], [role="listbox"], dialog, [role="dialog"], [role="alertdialog"]')) return;
    event.preventDefault(); event.stopPropagation(); close();
  };
  const resized = event => { if (opened && event.detail?.expanded === false) close({ focus: notifications.contains(document.activeElement) }); };
  trigger.addEventListener('click', click);
  notifications.addEventListener('click', back);
  notifications.addEventListener('keydown', keydown);
  aside.addEventListener('nero:navigation-state', resized);
  paint();
  return {
    open, close, isOpen: () => opened,
    updateCounts(counts = {}) { unread = Math.max(0, Number(counts.unread) || 0); active = Math.max(0, Number(counts.active) || 0); paint(); },
    destroy() { disposed = true; trigger.removeEventListener('click', click); notifications.removeEventListener('click', back); notifications.removeEventListener('keydown', keydown); aside.removeEventListener('nero:navigation-state', resized); },
  };
}
