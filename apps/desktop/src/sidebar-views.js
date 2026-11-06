export function mountSidebarViews({ aside, navigation, notifications, trigger, badge, onChange = () => {}, onPreview = () => {} }) {
  const document = aside.ownerDocument;
  const find = id => aside.querySelector(`#${id}`);
  const heading = find('sidebar-preview-heading'), title = find('sidebar-preview-title'), openView = find('sidebar-preview-open');
  const library = find('sidebar-library-navigation'), meetings = find('sidebar-meeting-list');
  const emptyTrash = find('sidebar-empty-trash'), newChat = find('sidebar-ai-new'), aiPreview = find('sidebar-ai-preview'), search = aside.querySelector('.search-field');
  const recentPreview = find('sidebar-recent-preview'), recentSection = find('recent-section'), recentList = find('recent-meetings');
  const compact = aside.parentElement.querySelector('[data-navigation-compact]');
  // Each sidebar view names what it shows; previews are the views with a label.
  const views = {
    navigation: { library: true, search: true },
    folders: { label: '폴더', library: true, search: true, foldersOnly: true },
    all: { label: '전체 목록', heading: true, search: true, meetings: true, open: 'all' },
    ai: { label: 'AI 대화', heading: true, ai: true, open: 'ai', newChat: true },
    trash: { label: '휴지통', heading: true, search: true, meetings: true, open: 'trash', emptyTrash: true, noCreate: true },
    recent: { label: '최근 회의', heading: true, search: true, recent: true },
    notifications: {},
  };
  let view = 'navigation', returnView = 'navigation', unread = 0, active = 0, disposed = false;
  function paint() {
    const opened = view === 'notifications', config = views[view] || {}, scoped = Boolean(config.label);
    aside.dataset.sidebarView = view;
    aside.dataset.preview = String(scoped);
    navigation.hidden = opened;
    notifications.hidden = !opened;
    if (library) library.hidden = !config.library;
    if (aiPreview) aiPreview.hidden = !config.ai;
    if (search) search.hidden = !config.search;
    if (meetings) meetings.hidden = !config.meetings;
    if (recentPreview) recentPreview.hidden = !config.recent;
    if (recentList && recentSection && recentPreview) {
      const parent = config.recent ? recentPreview : recentSection;
      if (recentList.parentElement !== parent) parent.append(recentList);
      recentSection.hidden = scoped || recentSection.dataset.hasItems === 'false';
    }
    if (heading) heading.hidden = !config.heading;
    if (title) title.textContent = config.label || '';
    if (emptyTrash) emptyTrash.hidden = !config.emptyTrash;
    if (newChat) newChat.hidden = !config.newChat;
    if (openView) {
      openView.hidden = !config.open;
      openView.dataset.sidebarFilter = config.open || 'all';
      openView.setAttribute('aria-label', `${config.label || '전체 목록'} 열기`);
      openView.title = openView.getAttribute('aria-label');
      const accessibleLabel = openView.querySelector('.sr-only');
      if (accessibleLabel) accessibleLabel.textContent = openView.getAttribute('aria-label');
    }
    for (const row of aside.querySelectorAll('#library-system-views .library-folder-item')) row.hidden = Boolean(config.foldersOnly) && Boolean(row.querySelector('[data-library-filter="all"]'));
    for (const id of ['library-ai-view', 'library-trash-view']) { const section = find(id); if (section) section.hidden = Boolean(config.foldersOnly); }
    const create = find('create-meeting'); if (create) create.hidden = opened || Boolean(config.noCreate);
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
    if (!views[name]?.label || disposed) return;
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
