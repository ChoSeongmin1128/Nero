export function mountSidebarViews({ aside, navigation, notifications, trigger, badge, onChange = () => {}, onPreview = () => {} }) {
  const document = aside.ownerDocument;
  const find = id => aside.querySelector(`#${id}`);
  const heading = find('sidebar-preview-heading'), title = find('sidebar-preview-title'), openView = find('sidebar-preview-open');
  const library = find('sidebar-library-navigation'), meetings = find('sidebar-meeting-list');
  const recentPreview = find('sidebar-recent-preview'), recentSection = find('recent-section'), recentList = find('recent-meetings');
  const compact = aside.parentElement.querySelector('[data-navigation-compact]');
  const labels = { all: '전체 목록', folders: '폴더', trash: '휴지통', recent: '최근 회의' };
  let view = 'navigation', returnView = 'navigation', unread = 0, active = 0, disposed = false;
  function paint() {
    const opened = view === 'notifications', scoped = Boolean(labels[view]);
    aside.dataset.sidebarView = view;
    aside.dataset.preview = String(scoped);
    navigation.hidden = opened;
    notifications.hidden = !opened;
    if (library) library.hidden = opened || ['all', 'trash', 'recent'].includes(view);
    if (meetings) meetings.hidden = !['all', 'trash'].includes(view);
    if (recentPreview) recentPreview.hidden = view !== 'recent';
    if (recentList && recentSection && recentPreview) {
      const parent = view === 'recent' ? recentPreview : recentSection;
      if (recentList.parentElement !== parent) parent.append(recentList);
      recentSection.hidden = scoped || recentSection.dataset.hasItems === 'false';
    }
    if (heading) heading.hidden = !['all', 'trash', 'recent'].includes(view);
    if (title) title.textContent = labels[view] || '';
    if (openView) {
      openView.hidden = view === 'recent';
      openView.dataset.sidebarFilter = view === 'trash' ? 'trash' : 'all';
      openView.setAttribute('aria-label', `${labels[view] || '전체 목록'} 열기`);
      openView.title = openView.getAttribute('aria-label');
      const accessibleLabel = openView.querySelector('.sr-only');
      if (accessibleLabel) accessibleLabel.textContent = openView.getAttribute('aria-label');
    }
    for (const row of aside.querySelectorAll('#library-system-views .library-folder-item')) row.hidden = view === 'folders' && Boolean(row.querySelector('[data-library-filter="all"]'));
    const trash = find('library-trash-view'); if (trash) trash.hidden = view === 'folders';
    const create = find('create-meeting'); if (create) create.hidden = opened || view === 'trash';
    trigger.setAttribute('aria-controls', notifications.id);
    trigger.setAttribute('aria-expanded', String(opened));
    trigger.setAttribute('aria-pressed', String(opened));
    trigger.dataset.unread = String(unread > 0);
    trigger.dataset.active = String(active > 0);
    trigger.setAttribute('aria-label', unread ? `알림, 읽지 않은 알림 ${unread}개` : active ? '알림, 작업 진행 중' : '알림');
    trigger.title = unread ? '읽지 않은 알림' : '알림';
    if (badge) { badge.hidden = !unread; badge.textContent = ''; badge.setAttribute('aria-hidden', 'true'); }
    for (const button of compact?.querySelectorAll('[data-navigation-preview]') || []) {
      button.setAttribute('aria-controls', aside.id);
      button.setAttribute('aria-expanded', String(aside.inert === false && view === button.dataset.navigationPreview));
    }
  }
  function close({ focus = true } = {}) {
    if (disposed || view !== 'notifications') return;
    view = returnView;
    paint();
    onChange(false);
    if (focus && trigger.isConnected && !trigger.closest('[inert]')) trigger.focus({ preventScroll: true });
  }
  function open({ focus = true } = {}) {
    if (disposed || aside.closest('[data-onboarding="true"]')) return;
    if (view !== 'notifications') returnView = view;
    view = 'notifications';
    paint();
    aside.dispatchEvent(new document.defaultView.CustomEvent('nero:sidebar-reveal'));
    onChange(true);
    if (focus) notifications.querySelector('[data-notification-heading]')?.focus({ preventScroll: true });
  }
  function preview(name) {
    if (!labels[name] || disposed) return;
    if (notifications.contains(document.activeElement)) document.activeElement.blur();
    view = returnView = name;
    onPreview(name);
    paint();
  }
  const click = () => view === 'notifications' ? close() : open();
  const back = event => { if (event.target.closest?.('[data-notification-action="back"]')) close(); };
  const keydown = event => {
    if (view !== 'notifications' || event.key !== 'Escape' || event.defaultPrevented || event.isComposing || event.keyCode === 229) return;
    if (event.target.closest?.('[role="menu"], [role="listbox"], dialog, [role="dialog"], [role="alertdialog"]')) return;
    event.preventDefault(); event.stopPropagation(); close();
  };
  const stateChanged = event => {
    if (event.detail?.pinned) { view = returnView = 'navigation'; paint(); }
    else if (event.detail?.expanded === false) {
      if (view === 'notifications') { view = returnView; onChange(false); }
      for (const button of compact?.querySelectorAll('[data-navigation-preview]') || []) button.setAttribute('aria-expanded', 'false');
      paint();
    } else paint();
  };
  const previewChanged = event => preview(event.detail?.view);
  trigger.addEventListener('click', click);
  notifications.addEventListener('click', back);
  notifications.addEventListener('keydown', keydown);
  aside.addEventListener('nero:navigation-state', stateChanged);
  aside.addEventListener('nero:sidebar-preview', previewChanged);
  aside.addEventListener('nero:sidebar-content-updated', paint);
  paint();
  return {
    open, close, preview, isOpen: () => view === 'notifications', activeView: () => view,
    updateCounts(counts = {}) { unread = Math.max(0, Number(counts.unread) || 0); active = Math.max(0, Number(counts.active) || 0); paint(); },
    destroy() {
      disposed = true;
      trigger.removeEventListener('click', click); notifications.removeEventListener('click', back); notifications.removeEventListener('keydown', keydown);
      aside.removeEventListener('nero:navigation-state', stateChanged); aside.removeEventListener('nero:sidebar-preview', previewChanged); aside.removeEventListener('nero:sidebar-content-updated', paint);
    },
  };
}
