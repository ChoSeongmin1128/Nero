// The sidebar shows the list the rail chose; the content area shows what is opened from it.
export function mountSidebarViews({ aside, navigation, notifications, trigger, badge, count = () => 0, folderName = () => '', onChange = () => {}, onPreview = () => {}, reveal = () => {}, storage }) {
  const document = aside.ownerDocument;
  const find = id => aside.querySelector(`#${id}`);
  const heading = find('sidebar-preview-heading'), title = find('sidebar-preview-title'), openView = find('sidebar-preview-open');
  const back = find('sidebar-preview-back'), countLabel = find('sidebar-preview-count');
  const library = find('sidebar-library-navigation'), meetings = find('sidebar-meeting-list');
  const emptyTrash = find('sidebar-empty-trash');
  const compact = aside.parentElement.querySelector('[data-navigation-compact]');
  const newFolder = find('new-folder');
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
  let bellHover, hoverOpened = false;
  function paint() {
    if (view === 'folder' && !folderName(folderId)) { view = 'folders'; folderId = ''; }
    const opened = view === 'notifications', config = views[view] || {};
    aside.dataset.sidebarView = view;
    aside.dataset.sidebarFolder = folderId;
    aside.dataset.preview = String(!opened);
    navigation.hidden = opened;
    notifications.hidden = !opened;
    if (library) library.hidden = !config.folders;
    if (newFolder) newFolder.hidden = !config.folders;
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
    if (openView) {
      const filter = view === 'folder' ? folderId : config.open;
      openView.hidden = !filter;
      openView.dataset.sidebarFilter = filter || 'all';
    }
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
    hoverOpened = false;
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
    hoverOpened = false;
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
    reveal();
    onChange(true);
    if (focus) notifications.querySelector('[data-notification-heading]')?.focus({ preventScroll: true });
  }
  // While the sidebar is hidden the bell shows the notifications it still holds.
  const click = () => view === 'notifications' && !aside.inert && !hoverOpened ? close() : (hoverOpened = false, open());
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
  // Resting on the bell shows the notifications, like the rail's other lists.
  const bellEnter = event => {
    clearTimeout(bellHover);
    if (event.pointerType !== 'mouse' || view === 'notifications') return;
    bellHover = setTimeout(() => { if (view !== 'notifications') { open({ focus: false }); hoverOpened = true; } }, 140);
  };
  const bellLeave = () => clearTimeout(bellHover);
  trigger.addEventListener('click', click);
  trigger.addEventListener('pointerenter', bellEnter);
  trigger.addEventListener('pointerleave', bellLeave);
  back?.addEventListener('click', backClick);
  library?.addEventListener('click', folderClick, true);
  notifications.addEventListener('click', closeFromList);
  notifications.addEventListener('keydown', keydown);
  paint();
  return {
    open, close, choose, refresh: paint, isOpen: () => view === 'notifications', activeView: () => view, folder: () => folderId,
    updateCounts(counts = {}) { unread = Math.max(0, Number(counts.unread) || 0); active = Math.max(0, Number(counts.active) || 0); paint(); },
    destroy() {
      disposed = true;
      clearTimeout(bellHover); trigger.removeEventListener('pointerenter', bellEnter); trigger.removeEventListener('pointerleave', bellLeave);
      trigger.removeEventListener('click', click); back?.removeEventListener('click', backClick); library?.removeEventListener('click', folderClick, true);
      notifications.removeEventListener('click', closeFromList); notifications.removeEventListener('keydown', keydown);
    },
  };
}
