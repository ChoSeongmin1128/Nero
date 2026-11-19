// The sidebar shows the list the rail chose; the content area shows what is opened from it.
export function mountSidebarViews({ aside, navigation, notifications, trigger, badge, count = () => 0, folderName = () => '', onChange = () => {}, onPreview = () => {}, storage }) {
  const document = aside.ownerDocument;
  const find = id => aside.querySelector(`#${id}`);
  const heading = find('sidebar-preview-heading'), title = find('sidebar-preview-title'), openView = find('sidebar-preview-open');
  const back = find('sidebar-preview-back'), countLabel = find('sidebar-preview-count');
  const library = find('sidebar-library-navigation'), meetings = find('sidebar-meeting-list');
  const emptyTrash = find('sidebar-empty-trash'), newChat = find('sidebar-ai-new'), aiPreview = find('sidebar-ai-preview'), search = aside.querySelector('.search-field');
  const recentPreview = find('sidebar-recent-preview'), recentSection = find('recent-section');
  const compact = aside.parentElement.querySelector('[data-navigation-compact]');
  const newFolder = document.getElementById('new-folder');
  const folderActionHome = newFolder?.parentElement;
  const folderActions = find('sidebar-folder-actions');
  const views = {
    home: { label: '최근 회의', rail: 'home', meetings: true },
    all: { label: '전체 회의', rail: 'all', meetings: true, open: 'all', count: 'all' },
    unfiled: { label: '미분류', rail: 'unfiled', meetings: true, open: 'unfiled', count: 'unfiled' },
    folders: { label: '폴더', rail: 'folders', folders: true },
    folder: { rail: 'folders', meetings: true, back: true },
    recent: { label: '최근 회의', rail: 'recent', meetings: true },
    trash: { label: '휴지통', rail: 'trash', meetings: true, open: 'trash', count: 'trash', emptyTrash: true },
    notifications: { rail: 'notifications' },
  };
  const read = () => { try { return storage?.getItem('nero.sidebar.view'); } catch { return null; } };
  const saved = read();
  let view = views[saved] && saved !== 'notifications' && saved !== 'folder' ? saved : 'home';
  let folderId = '', returnView = view, unread = 0, active = 0, disposed = false;
  function paint() {
    if (view === 'folder' && !folderName(folderId)) { view = 'folders'; folderId = ''; }
    const opened = view === 'notifications', config = views[view] || {};
    aside.dataset.sidebarView = view;
    aside.dataset.sidebarFolder = folderId;
    aside.dataset.preview = String(!opened);
    navigation.hidden = opened;
    notifications.hidden = !opened;
    if (library) library.hidden = !config.folders;
    if (aiPreview) aiPreview.hidden = true;
    if (recentPreview) recentPreview.hidden = true;
    if (recentSection) recentSection.hidden = true;
    if (search) {
      if (search.parentElement !== navigation) navigation.insertBefore(search, heading);
      search.hidden = opened;
    }
    if (newFolder && folderActions && folderActionHome) {
      const destination = config.folders ? folderActions : folderActionHome;
      if (newFolder.parentElement !== destination) destination.append(newFolder);
    }
    if (meetings) meetings.hidden = !config.meetings;
    if (heading) heading.hidden = opened;
    const scope = view === 'folder' ? folderId : config.count;
    if (title) title.textContent = view === 'folder' ? folderName(folderId) : config.label || '';
    if (countLabel) {
      const value = view === 'folders' ? count('folders') : scope ? count(scope) : null;
      countLabel.textContent = value == null ? '' : String(value);
    }
    if (back) back.hidden = !config.back;
    if (emptyTrash) emptyTrash.hidden = !config.emptyTrash;
    if (newChat) newChat.hidden = true;
    if (openView) {
      const filter = view === 'folder' ? folderId : config.open;
      openView.hidden = !filter;
      openView.dataset.sidebarFilter = filter || 'all';
    }
    for (const id of ['library-system-views', 'library-ai-view']) { const section = find(id); if (section) section.hidden = true; }
    const folderHeading = aside.querySelector('.navigation-folder-section > .library-heading');
    if (folderHeading) folderHeading.hidden = true;
    trigger.setAttribute('aria-controls', notifications.id);
    trigger.setAttribute('aria-expanded', String(opened && !aside.inert));
    trigger.setAttribute('aria-pressed', String(opened));
    trigger.dataset.unread = String(unread > 0);
    trigger.dataset.active = String(active > 0);
    trigger.setAttribute('aria-label', unread ? `알림, 확인할 알림 ${unread}개` : active ? '알림, 작업 진행 중' : '알림');
    trigger.title = unread ? '확인할 알림' : '알림';
    if (badge) { badge.hidden = !unread; badge.textContent = unread ? String(unread) : ''; badge.setAttribute('aria-hidden', 'true'); }
    for (const button of compact?.querySelectorAll('[data-navigation-preview]') || []) {
      button.setAttribute('aria-controls', aside.id);
      if (button.dataset.navigationPreview === config.rail) button.setAttribute('aria-current', 'page');
      else button.removeAttribute('aria-current');
    }
  }
  function choose(name, folder = '') {
    if (disposed || !views[name]) return;
    if (notifications.contains(document.activeElement)) document.activeElement.blur();
    view = name;
    folderId = name === 'folder' ? folder : '';
    if (name !== 'notifications') {
      returnView = name;
      try { if (name !== 'folder') storage?.setItem('nero.sidebar.view', name); } catch {}
    }
    onPreview(name, folderId);
    paint();
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
  // While the sidebar is hidden the bell shows the notifications it still holds.
  const click = () => view === 'notifications' && !aside.inert ? close() : open();
  const backClick = () => choose('folders');
  const closeFromList = event => { if (event.target.closest?.('[data-notification-action="back"]')) close(); };
  // A folder in the folder list opens its meetings here instead of the central list.
  const folderClick = event => {
    const target = event.target.closest?.('[data-library-filter]');
    if (!target || target.disabled || view !== 'folders' || ['all', 'unfiled', 'trash', 'ai'].includes(target.dataset.libraryFilter)) return;
    event.preventDefault(); event.stopPropagation();
    choose('folder', target.dataset.libraryFilter);
  };
  const keydown = event => {
    if (view !== 'notifications' || event.key !== 'Escape' || event.defaultPrevented || event.isComposing || event.keyCode === 229) return;
    if (event.target.closest?.('[role="menu"], [role="listbox"], dialog, [role="dialog"], [role="alertdialog"]')) return;
    event.preventDefault(); event.stopPropagation(); close();
  };
  const stateChanged = () => paint();
  const previewChanged = event => choose(event.detail?.view);
  trigger.addEventListener('click', click);
  back?.addEventListener('click', backClick);
  library?.addEventListener('click', folderClick, true);
  notifications.addEventListener('click', closeFromList);
  notifications.addEventListener('keydown', keydown);
  aside.addEventListener('nero:navigation-state', stateChanged);
  aside.addEventListener('nero:sidebar-preview', previewChanged);
  aside.addEventListener('nero:sidebar-content-updated', paint);
  paint();
  return {
    open, close, choose, preview: choose, isOpen: () => view === 'notifications', activeView: () => view, folder: () => folderId,
    updateCounts(counts = {}) { unread = Math.max(0, Number(counts.unread) || 0); active = Math.max(0, Number(counts.active) || 0); paint(); },
    destroy() {
      disposed = true;
      trigger.removeEventListener('click', click); back?.removeEventListener('click', backClick); library?.removeEventListener('click', folderClick, true);
      notifications.removeEventListener('click', closeFromList); notifications.removeEventListener('keydown', keydown);
      aside.removeEventListener('nero:navigation-state', stateChanged); aside.removeEventListener('nero:sidebar-preview', previewChanged); aside.removeEventListener('nero:sidebar-content-updated', paint);
    },
  };
}
