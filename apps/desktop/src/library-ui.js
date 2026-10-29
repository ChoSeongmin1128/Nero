import { mountNavigationSidebar } from '../../../design-system/navigation.mjs';
import { pendingJob } from './processing-state.js';
import { createDragPreview } from './drag-preview.js';
export function filterLibraryMeetings(meetings, filter = 'all', query = '') {
  const term = query.trim().toLocaleLowerCase();
  return meetings.filter(meeting => {
    const deleted = meeting.deletedAt != null;
    const inView = filter === 'trash' ? deleted : !deleted && (filter === 'all' || (filter === 'unfiled' ? !meeting.folderId : meeting.folderId === filter));
    return inView && (!term || `${meeting.title}\n${meeting.notes || ''}`.toLocaleLowerCase().includes(term));
  });
}

export function recentLibraryMeetings(meetings) {
  return meetings.filter(meeting => meeting.deletedAt == null)
    .sort((a, b) => (b.recordedAt ?? b.createdAt ?? 0) - (a.recordedAt ?? a.createdAt ?? 0) || a.id.localeCompare(b.id))
    .slice(0, 5);
}
const recentDate = new Intl.DateTimeFormat('ko-KR', { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit', hourCycle: 'h23' });

export function mountLibrary({ bridge, getState, byId, finishEditing, acceptSnapshot, selectMeeting, render: renderApp, showError, escape, icon, duration, onViewChanged }) {
  const dialog = byId('library-dialog');
  const dialogBody = byId('library-dialog-body');
  const owner = dialog.ownerDocument;
  let storage;
  try { storage = owner.defaultView?.localStorage; } catch {}
  const readPreference = key => { try { return storage?.getItem(key); } catch { return null; } };
  const savePreference = (key, value) => { try { storage?.setItem(key, value); } catch {} };
  const shell = owner.querySelector?.('.workspace');
  const sidebar = shell ? mountNavigationSidebar({ shell, aside: byId('navigation-sidebar'), content: owner.querySelector('.main-content'), trigger: byId('navigation-toggle'), storage, icons: icon }) : null;
  const savedView = getState().libraryView || readPreference('nero.library.view');
  getState().libraryView = ['folders', 'all'].includes(savedView) ? savedView : 'folders';
  const recents = byId('recent-section');
  recents.open = readPreference('nero.library.recents') !== 'false';
  recents.addEventListener('toggle', () => savePreference('nero.library.recents', String(recents.open)));
  let navigationKey = '', cardsKey = '', breadcrumbKey = '';
  const folderOverview = () => state().libraryFilter === 'all' && state().libraryView === 'folders' && !query();
  const dragPreview = createDragPreview(owner);
  let folderKey = '', recentKey = '', pending = null, returnFocus = null, dragSources = [];
  let activeMenu = null, dropTarget = null, selectionAnchor = null;
  const state = () => getState();
  const selection = () => state().librarySelection ||= new Set();
  const selectedIds = () => [...selection()];
  const query = () => byId('search')?.value?.trim() || '';
  const current = id => state().snapshot.meetings.find(meeting => meeting.id === id);
  const sourcesForMove = ids => ids.map(id => current(id)).every(meeting => meeting && meeting.deletedAt == null)
    ? ids.map(id => ({ id, folderId: current(id).folderId || null })) : [];
  const sourcesUnchanged = sources => Boolean(state().connected && sources.length) && sources.every(source => {
    const meeting = current(source.id);
    return meeting && meeting.deletedAt == null && (meeting.folderId || null) === source.folderId;
  });
  function moveDestination(filter) {
    if (filter === 'unfiled') return { folderId: null, name: '미분류' };
    const folder = state().snapshot.folders?.find(item => item.id === filter);
    return folder ? { folderId: folder.id, name: folder.name } : null;
  }
  async function moveChecked(sources, destination) {
    acceptSnapshot(await bridge.readSnapshot());
    if (!sourcesUnchanged(sources)) throw new Error('회의의 위치가 변경되었습니다. 다시 선택해 주세요.');
    const latest = moveDestination(destination.folderId || 'unfiled');
    if (!latest || latest.name !== destination.name) throw new Error('대상 폴더가 변경되었습니다. 다시 선택해 주세요.');
    return bridge.moveMeetings(sources.map(source => source.id), destination.folderId);
  }
  const runningRecording = ids => {
    const session = state().snapshot.session;
    return session && ['recording', 'paused'].includes(session.state) && ids.includes(session.meetingId);
  };
  function visibleMeetings() {
    const meetings = filterLibraryMeetings(state().snapshot.meetings, state().libraryFilter);
    const found = state().librarySearchIds;
    const ids = found != null ? new Set(found) : null;
    const visible = query() && ids ? meetings.filter(meeting => ids.has(meeting.id)) : filterLibraryMeetings(meetings, state().libraryFilter, query());
    return visible.filter(meeting => !folderOverview() || !meeting.folderId).sort((a, b) => (b.recordedAt ?? b.createdAt ?? 0) - (a.recordedAt ?? a.createdAt ?? 0) || (b.createdAt ?? 0) - (a.createdAt ?? 0));
  }
  function resetSelection() {
    selection().clear();
    selectionAnchor = null;
  }
  async function show(filter = 'all') {
    try {
      await finishEditing(() => {});
      const changed = state().libraryFilter !== filter;
      state().libraryFilter = filter;
      state().libraryOpen = true;
      sidebar?.navigate();
      resetSelection();
      renderApp();
      byId('library-title')?.focus({ preventScroll: true });
      if (changed || query()) await onViewChanged?.(filter);
    } catch (error) { showError(error); }
  }
  function toggleSelection(id, event = {}) {
    if (state().libraryBusy || !state().connected) return;
    const ids = visibleMeetings().map(meeting => meeting.id);
    const target = ids.indexOf(id);
    if (target < 0) return;
    if (event.shiftKey) {
      let anchor = ids.indexOf(selectionAnchor);
      if (anchor < 0) { anchor = target; selectionAnchor = id; }
      if (!event.metaKey && !event.ctrlKey) selection().clear();
      ids.slice(Math.min(anchor, target), Math.max(anchor, target) + 1).forEach(value => selection().add(value));
    } else {
      if (selection().has(id)) selection().delete(id); else selection().add(id);
      selectionAnchor = selection().size ? id : null;
    }
    renderApp();
  }
  function handleRowSelection(id, event = {}) {
    if (!event.shiftKey && !event.metaKey && !event.ctrlKey) return false;
    event.preventDefault?.();
    toggleSelection(id, event);
    return true;
  }
  function selectAll() {
    if (state().libraryBusy || !state().connected) return;
    const visible = visibleMeetings();
    visible.forEach(meeting => selection().add(meeting.id));
    if (visible.length && !selectionAnchor) selectionAnchor = visible[0].id;
    renderApp();
  }
  function setDialogBusy(busy) {
    if (!dialog.open) return;
    dialog.setAttribute('aria-busy', String(busy));
    dialogBody.querySelectorAll('button,input').forEach(node => { node.disabled = busy; });
    const submit = dialogBody.querySelector('[data-library-confirm]');
    if (submit) {
      submit.innerHTML = busy ? '<span class="activity-spinner" aria-hidden="true"></span><span>처리 중</span>' : escape(pending?.confirm || '저장');
    }
  }
  function closeDialog() {
    if (state().libraryBusy) return;
    dialog.close();
    pending = null;
    restoreFocus();
  }
  function restoreFocus(onlyIfIdle = false) {
    const active = dialog.ownerDocument?.activeElement;
    if (onlyIfIdle && active && active !== dialog.ownerDocument.body && !dialog.contains?.(active)) return;
    const target = returnFocus?.isConnected && !returnFocus.disabled && !returnFocus.hidden ? returnFocus : byId(state().libraryOpen ? 'library-title' : 'new-folder');
    target?.focus({ preventScroll: true });
  }
  function openDialog({ title, content, confirm = '저장', destructive = false, action, detach = false }) {
    if (state().libraryBusy) return;
    returnFocus = dialog.ownerDocument?.activeElement;
    pending = { confirm, action, detach };
    dialogBody.innerHTML = `<form class="library-form"><header class="dialog-header"><h2 id="library-dialog-title">${escape(title)}</h2><button type="button" class="quiet icon-button" data-library-close aria-label="닫기">${icon('x')}</button></header><div class="library-dialog-content">${content}<p id="library-dialog-error" class="field-error" role="alert" hidden></p></div><footer class="dialog-actions"><button type="button" data-library-close>취소</button><button type="submit" class="${destructive ? 'danger' : 'primary'}" data-library-confirm>${escape(confirm)}</button></footer></form>`;
    dialog.setAttribute('aria-labelledby', 'library-dialog-title');
    dialog.setAttribute('role', destructive ? 'alertdialog' : 'dialog');
    if (!dialog.open) dialog.showModal();
    const input = dialogBody.querySelector('[autofocus]');
    if (input) { input.focus(); input.select?.(); }
    else if (destructive) dialogBody.querySelector('footer [data-library-close]')?.focus();
  }
  function report(error) {
    const field = dialog.open && byId('library-dialog-error');
    if (field) { field.textContent = error instanceof Error ? error.message : String(error); field.hidden = false; }
    else showError(error);
  }
  async function mutate(action, done = () => {}, { detach = false } = {}) {
    if (state().libraryBusy) return false;
    state().libraryBusy = true;
    renderApp();
    setDialogBusy(true);
    try {
      // Complete saves before releasing edit locks for the potentially long native job.
      await finishEditing(() => {});
      const operation = action();
      // Confirmation is complete. The native cancellation may take time, so let
      // the user navigate and edit other meetings while only library actions wait.
      if (detach && dialog.open) dialog.close();
      const snapshot = await operation;
      if (snapshot?.meetings) acceptSnapshot(snapshot);
      else acceptSnapshot(await bridge.readSnapshot());
      done();
      await onViewChanged?.(state().libraryFilter);
      if (dialog.open) dialog.close();
      pending = null;
      return true;
    } catch (error) {
      // Purging may commit its tombstone before physical cleanup reports a failure.
      try { acceptSnapshot(await bridge.readSnapshot()); } catch {}
      report(error);
      return false;
    } finally {
      state().libraryBusy = false;
      renderApp();
      setDialogBusy(false);
      if (dialog.open) dialogBody.querySelector('[autofocus]')?.focus();
      else restoreFocus(detach);
    }
  }
  const description = ids => ids.length === 1 ? `"${current(ids[0])?.title || '제목 없는 회의'}"` : `${ids.length}개 회의`;
  function folderDialog(id = null) {
    const folder = state().snapshot.folders?.find(value => value.id === id);
    if (id && !folder) return;
    openDialog({ title: folder ? '폴더 이름 변경' : '새 폴더', content: `<label for="library-folder-name">폴더 이름</label><input id="library-folder-name" class="text-input" type="text" value="${escape(folder?.name || '')}" maxlength="120" required autofocus autocomplete="off">`, confirm: folder ? '저장' : '만들기', action: () => {
      const name = byId('library-folder-name').value.trim();
      if (!name) throw new Error('폴더 이름을 입력해 주세요.');
      return folder ? bridge.renameFolder(id, name) : bridge.createFolder(name);
    } });
  }
  function deleteFolder(id) {
    const folder = state().snapshot.folders?.find(value => value.id === id);
    if (!folder) return;
    openDialog({ title: '폴더를 삭제할까요?', content: `<p>${escape(folder.name)}</p><p class="muted">회의는 삭제하지 않고 미분류로 옮깁니다.</p>`, confirm: '폴더 삭제', destructive: true, action: () => bridge.deleteFolder(id) });
  }
  function renameMeeting(id) {
    const meeting = current(id);
    if (!meeting || meeting.deletedAt != null) return;
    openDialog({ title: '회의 이름 변경', content: `<label for="library-meeting-name">회의 이름</label><input id="library-meeting-name" class="text-input" type="text" value="${escape(meeting.title)}" required autofocus autocomplete="off">`, action: () => {
      const title = byId('library-meeting-name').value.trim();
      if (!title) throw new Error('회의 이름을 입력해 주세요.');
      const latest = current(id);
      if (!latest || latest.deletedAt != null) throw new Error('회의를 찾을 수 없습니다.');
      return bridge.updateMeeting(id, title, latest.notes || '');
    } });
  }
  function moveDialog(ids) {
    if (!ids.length) return;
    const sources = sourcesForMove(ids);
    if (!sources.length) return;
    let folderId = ids.every(id => current(id)?.folderId === current(ids[0])?.folderId) ? current(ids[0])?.folderId || '' : null;
    const folders = [{ id: '', name: '미분류' }, ...(state().snapshot.folders || [])];
    openDialog({ title: '폴더로 이동', content: `<p class="muted">${escape(description(ids))}</p><div class="library-folder-choices" role="radiogroup" aria-label="이동할 폴더">${folders.map(folder => `<label class="library-folder-choice"><input type="radio" name="library-destination" value="${escape(folder.id)}"${folderId === folder.id ? ' checked' : ''}><span>${icon(folder.id ? 'folder' : 'inbox')}${escape(folder.name)}</span></label>`).join('')}</div>`, confirm: '이동', action: () => {
      const chosen = dialogBody.querySelector('input[name="library-destination"]:checked');
      if (!chosen) throw new Error('이동할 폴더를 선택해 주세요.');
      const destination = moveDestination(chosen.value || 'unfiled');
      if (!destination) throw new Error('대상 폴더를 찾을 수 없습니다.');
      return moveChecked(sources, destination);
    } });
  }
  function trashDialog(ids) {
    if (!ids.length) return;
    if (runningRecording(ids)) { showError('녹음을 종료한 뒤 삭제해 주세요.'); return; }
    const cancels = ids.some(meetingId => pendingJob(state().processing, { meetingId }));
    openDialog({ title: '휴지통으로 이동할까요?', content: `<p>${escape(description(ids))}</p><p class="muted">${cancels ? '진행 중인 처리를 취소한 뒤 이동합니다. ' : ''}휴지통에서 복원할 수 있습니다.</p>`, confirm: cancels ? '처리 취소 후 휴지통' : '휴지통으로 이동', destructive: true, detach: true, action: () => bridge.trashMeetings(ids) });
  }
  function purgeDialog(ids) {
    if (!ids.length) return;
    openDialog({ title: '회의를 영구 삭제할까요?', content: `<p>${escape(description(ids))}</p><p class="muted">원음, 음성 기록, 메모와 회의록을 삭제합니다. 복원할 수 없습니다.</p>`, confirm: '영구 삭제', destructive: true, detach: true, action: () => bridge.purgeMeetings(ids) });
  }
  function emptyTrashDialog() {
    if (state().libraryBusy || !state().connected || state().libraryFilter !== 'trash' || dialog.open) return;
    const targets = filterLibraryMeetings(state().snapshot.meetings, 'trash').map(({ id, deletedAt }) => ({ id, deletedAt }));
    if (!targets.length) return;
    const ids = targets.map(meeting => meeting.id);
    if (runningRecording(ids)) { showError('녹음을 종료한 뒤 삭제해 주세요.'); return; }
    openDialog({ title: '휴지통을 비울까요?', content: `<p>휴지통의 회의 ${targets.length}개를 영구 삭제합니다.</p><p class="muted">녹음, 음성 기록, 메모와 회의록은 복원할 수 없습니다.</p>`, confirm: '영구 삭제', destructive: true, action: async () => {
      const latest = await bridge.readSnapshot();
      acceptSnapshot(latest);
      if (targets.some(target => !latest.meetings.some(meeting => meeting.id === target.id && meeting.deletedAt != null && meeting.deletedAt === target.deletedAt))) {
        throw new Error('휴지통의 회의가 변경되었습니다. 취소한 뒤 삭제 대상을 다시 확인해 주세요.');
      }
      if (runningRecording(ids)) throw new Error('녹음을 종료한 뒤 삭제해 주세요.');
      return bridge.purgeMeetings(ids);
    } });
  }
  function afterRemoval() {
    state().libraryOpen = true;
    resetSelection();
  }
  function render() {
    const snapshot = state().snapshot;
    const folders = snapshot.folders || [];
    if (!['all', 'unfiled', 'trash'].includes(state().libraryFilter) && !folders.some(folder => folder.id === state().libraryFilter)) state().libraryFilter = 'unfiled';
    const visible = visibleMeetings();
    const visibleIds = new Set(visible.map(meeting => meeting.id));
    for (const id of selection()) if (!visibleIds.has(id)) selection().delete(id);
    if (!selection().size || !visibleIds.has(selectionAnchor)) selectionAnchor = null;
    const filter = state().libraryFilter;
    const disabled = Boolean(state().libraryBusy || !state().connected);
    const navigation = [{ id: 'all', name: '회의 목록', icon: 'notebook-pen' }, { id: 'unfiled', name: '미분류', icon: 'inbox' }, ...folders.map(folder => ({ ...folder, icon: 'folder' })), { id: 'trash', name: '휴지통', icon: 'trash-2' }];
    const key = JSON.stringify([filter, state().libraryOpen, folders, disabled, snapshot.meetings.map(meeting => [meeting.id, meeting.folderId, meeting.deletedAt])]);
    if (key !== folderKey) {
      clearDropTarget();
      folderKey = key;
      const focused = dialog.ownerDocument?.activeElement?.dataset?.libraryFilter;
      byId('folder-list').innerHTML = navigation.map(item => `<div class="library-folder-item"><button type="button" class="quiet library-folder-link" data-library-filter="${escape(item.id)}"${state().libraryOpen && filter === item.id ? ' aria-current="page"' : ''}${disabled ? ' disabled' : ''}>${icon(item.icon)}<span class="library-folder-name" title="${escape(item.name)}">${escape(item.name)}</span><span class="library-folder-count">${filterLibraryMeetings(snapshot.meetings, item.id).length}</span></button>${folders.some(folder => folder.id === item.id) ? `<details class="library-folder-menu"><summary aria-label="${escape(item.name)} 폴더 메뉴">${icon('ellipsis')}</summary><div class="action-menu"><button type="button" data-folder-rename="${escape(item.id)}"${disabled ? ' disabled' : ''}>이름 변경</button><button type="button" class="danger" data-folder-delete="${escape(item.id)}"${disabled ? ' disabled' : ''}>폴더 삭제</button></div></details>` : ''}</div>`).join('');
      if (focused) [...byId('folder-list').querySelectorAll('[data-library-filter]')].find(node => node.dataset.libraryFilter === focused)?.focus({ preventScroll: true });
    }
    const recent = recentLibraryMeetings(snapshot.meetings);
    const activeId = state().libraryOpen ? null : state().selectedId;
    const nextRecentKey = JSON.stringify([disabled, activeId, recent.map(meeting => [meeting.id, meeting.title, meeting.recordedAt, meeting.createdAt, meeting.audio?.frames, meeting.audio?.sampleRate])]);
    byId('recent-section').hidden = !recent.length;
    if (recentKey !== nextRecentKey) {
      recentKey = nextRecentKey;
      const focused = owner.activeElement?.dataset?.recentMeeting;
      byId('recent-meetings').innerHTML = recent.map(meeting => `<button type="button" class="quiet recent-meeting" data-recent-meeting="${escape(meeting.id)}"${activeId === meeting.id ? ' aria-current="page"' : ''}${disabled ? ' disabled' : ''}><strong title="${escape(meeting.title)}">${escape(meeting.title)}</strong><time>${escape(recentDate.format(meeting.recordedAt ?? meeting.createdAt ?? 0))}${meeting.audio?.sampleRate && duration ? ` / ${escape(duration(meeting.audio.frames / meeting.audio.sampleRate * 1000))}` : ''}</time></button>`).join('');
      if (focused) [...byId('recent-meetings').querySelectorAll('[data-recent-meeting]')].find(node => node.dataset.recentMeeting === focused)?.focus({ preventScroll: true });
    }
    const overview = folderOverview();
    byId('library-view').hidden = filter !== 'all';
    byId('library-view').querySelectorAll('[data-library-view]').forEach(button => {
      button.setAttribute('aria-pressed', String(button.dataset.libraryView === state().libraryView));
      button.disabled = disabled;
    });
    byId('library-folders').hidden = !overview || !folders.length;
    byId('library-unfiled-heading').hidden = !overview || !folders.length;
    const nextCardsKey = JSON.stringify([folders, disabled, snapshot.meetings.map(meeting => [meeting.id, meeting.folderId, meeting.deletedAt])]);
    if (cardsKey !== nextCardsKey) {
      cardsKey = nextCardsKey;
      byId('library-folders').innerHTML = folders.map(folder => `<button type="button" class="library-folder-card" data-library-filter="${escape(folder.id)}"${disabled ? ' disabled' : ''}>${icon('folder')}<span><strong title="${escape(folder.name)}">${escape(folder.name)}</strong><small>${filterLibraryMeetings(snapshot.meetings, folder.id).length}개 회의</small></span>${icon('chevron-right')}</button>`).join('');
    }
    const selectedMeeting = state().libraryOpen ? null : current(state().selectedId);
    const parent = selectedMeeting ? selectedMeeting.folderId || 'unfiled' : filter;
    const parentName = navigation.find(item => item.id === parent)?.name || '미분류';
    const nextBreadcrumbKey = JSON.stringify([filter, state().libraryOpen, selectedMeeting?.id, selectedMeeting?.title, parent, parentName, disabled]);
    if (breadcrumbKey !== nextBreadcrumbKey) {
      breadcrumbKey = nextBreadcrumbKey;
      byId('navigation-breadcrumbs').innerHTML = `<button type="button" class="quiet small" data-library-filter="all"${disabled ? ' disabled' : ''}${!selectedMeeting && filter === 'all' ? ' aria-current="page"' : ''}>회의 목록</button>${parent !== 'all' ? `<span aria-hidden="true">/</span><button type="button" class="quiet small" data-library-filter="${escape(parent)}"${disabled ? ' disabled' : ''}${!selectedMeeting ? ' aria-current="page"' : ''}>${escape(parentName)}</button>` : ''}${selectedMeeting ? `<span aria-hidden="true">/</span><span class="breadcrumb-current" aria-current="page" title="${escape(selectedMeeting.title)}">${escape(selectedMeeting.title)}</span>` : ''}`;
    }
    const nextNavigationKey = `${state().libraryOpen}:${state().selectedId}:${filter}`;
    if (navigationKey !== nextNavigationKey) { navigationKey = nextNavigationKey; sidebar?.navigate(); }
    byId('library-title').textContent = navigation.find(item => item.id === filter)?.name || '회의';
    byId('library-count').innerHTML = state().libraryBusy ? '<span class="activity-spinner" aria-hidden="true"></span><span role="status">처리 중</span>' : `${overview ? filterLibraryMeetings(snapshot.meetings, 'all').length : visible.length}개`;
    const count = selection().size;
    byId('library-batch').hidden = !count;
    byId('library-selected-count').textContent = `${count}개 선택`;
    for (const id of ['library-move', 'library-trash', 'library-restore', 'library-purge']) byId(id).disabled = disabled || !count;
    byId('library-move').hidden = filter === 'trash';
    byId('library-trash').hidden = filter === 'trash';
    byId('library-restore').hidden = filter !== 'trash';
    byId('library-purge').hidden = filter !== 'trash';
    byId('library-empty-trash').hidden = filter !== 'trash' || count > 0;
    byId('library-empty-trash').disabled = disabled || !filterLibraryMeetings(snapshot.meetings, 'trash').length;
    byId('library-select-all').disabled = disabled || !visible.length;
    byId('library-selection-clear').disabled = disabled || !count;
    byId('new-folder').disabled = disabled;
    const meeting = current(state().selectedId);
    const location = byId('meeting-folder-context');
    if (location) {
      const name = folders.find(folder => folder.id === meeting?.folderId)?.name || '미분류';
      location.textContent = name;
      location.title = name;
      location.hidden = !meeting;
    }
    byId('meeting-move').disabled = disabled || !meeting || meeting.deletedAt != null;
    byId('meeting-trash').disabled = disabled || !meeting || meeting.deletedAt != null || Boolean(meeting && runningRecording([meeting.id]));
  }
  function actionFor(action, ids) {
    if (!ids.length || state().libraryBusy) return;
    if (action === 'rename') renameMeeting(ids[0]);
    if (action === 'move') moveDialog(ids);
    if (action === 'trash') trashDialog(ids);
    if (action === 'purge') purgeDialog(ids);
    if (action === 'restore') void mutate(() => bridge.restoreMeetings(ids), afterRemoval);
  }
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
  function clearDropTarget() {
    dropTarget?.classList.remove('is-drop-target');
    dropTarget = null;
  }
  function clearDrag() {
    dragSources = [];
    clearDropTarget();
    dragPreview.clear();
  }
  owner?.addEventListener?.('toggle', event => {
    const menu = event.target;
    if (!menu.matches?.('[data-meeting-menu],.library-folder-menu')) return;
    if (menu.open) positionMenu(menu); else closeMenu(menu);
  }, true);
  owner?.addEventListener?.('pointerdown', event => { if (activeMenu && !activeMenu.contains(event.target)) closeMenu(); }, true);
  owner?.addEventListener?.('keydown', event => {
    if (event.key !== 'Escape') return;
    clearDrag();
    if (!activeMenu) return;
    event.preventDefault(); event.stopPropagation(); closeMenu(activeMenu, true);
  }, true);
  owner?.addEventListener?.('scroll', event => {
    if (activeMenu && !menuContent(activeMenu)?.contains(event.target)) closeMenu();
  }, true);
  owner?.defaultView?.addEventListener('resize', () => closeMenu());
  owner?.addEventListener?.('drop', () => { clearDropTarget(); dragPreview.clear(); }, true);
  owner?.addEventListener?.('dragend', clearDrag);
  dialog.addEventListener('submit', event => {
    event.preventDefault();
    if (!pending || state().libraryBusy) return;
    const intent = pending;
    void mutate(intent.action, () => { resetSelection(); if (state().snapshot.meetings.every(meeting => meeting.id !== state().selectedId || meeting.deletedAt != null)) state().libraryOpen = true; }, { detach: intent.detach });
  });
  dialog.addEventListener('click', event => { if (event.target.closest('[data-library-close]')) closeDialog(); });
  dialog.addEventListener('cancel', event => { event.preventDefault(); closeDialog(); });
  byId('new-folder').addEventListener('click', () => folderDialog());
  byId('library-empty-trash').addEventListener('click', emptyTrashDialog);
  byId('folder-list').addEventListener('click', event => {
    const filter = event.target.closest('[data-library-filter]');
    const rename = event.target.closest('[data-folder-rename]');
    const remove = event.target.closest('[data-folder-delete]');
    if (filter) void show(filter.dataset.libraryFilter);
    if (rename) folderDialog(rename.dataset.folderRename);
    if (remove) deleteFolder(remove.dataset.folderDelete);
    if (rename || remove) event.target.closest('details')?.removeAttribute('open');
  });
  for (const id of ['library-folders', 'navigation-breadcrumbs']) byId(id).addEventListener('click', event => {
    const button = event.target.closest('[data-library-filter]');
    if (button && !button.disabled) void show(button.dataset.libraryFilter);
  });
  byId('library-view').addEventListener('click', event => {
    const button = event.target.closest('[data-library-view]');
    if (!button || button.disabled || state().libraryBusy || !state().connected) return;
    const mode = button.dataset.libraryView;
    if (!['folders', 'all'].includes(mode) || mode === state().libraryView) return;
    state().libraryView = mode;
    savePreference('nero.library.view', mode);
    resetSelection(); renderApp();
  });
  byId('recent-meetings').addEventListener('click', event => {
    const target = event.target.closest('[data-recent-meeting]');
    const meeting = target && current(target.dataset.recentMeeting);
    if (!meeting || meeting.deletedAt != null || state().libraryBusy || !state().connected) return;
    void selectMeeting(meeting.id);
  });
  byId('library-back').addEventListener('click', () => void show(state().libraryFilter));
  byId('library-select-all').addEventListener('click', selectAll);
  byId('library-selection-clear').addEventListener('click', () => { if (state().libraryBusy || !state().connected) return; resetSelection(); renderApp(); });
  for (const action of ['move', 'trash', 'restore', 'purge']) byId(`library-${action}`).addEventListener('click', () => actionFor(action, selectedIds()));
  for (const action of ['move', 'trash']) byId(`meeting-${action}`).addEventListener('click', () => { closeMenu(activeMenu, true); actionFor(action, [state().selectedId].filter(Boolean)); });
  byId('meeting-list').addEventListener('click', event => {
    const toggle = event.target.closest('[data-library-toggle]');
    const action = event.target.closest('[data-library-action]');
    if (toggle) { event.preventDefault(); event.stopPropagation(); toggleSelection(toggle.dataset.libraryToggle, event); }
    if (action) { event.preventDefault(); event.stopPropagation(); actionFor(action.dataset.libraryAction, [action.dataset.libraryTarget]); action.closest('details')?.removeAttribute('open'); }
  });
  function handleSelectionKeydown(event) {
    if (event.defaultPrevented || event.isComposing || event.keyCode === 229 || dialog.open || activeMenu || !state().libraryOpen || state().libraryBusy || !state().connected) return;
    if (event.target.closest?.('textarea,select,[contenteditable]:not([contenteditable="false"]),input:not([type="checkbox"]):not([type="radio"]):not([type="button"])')) return;
    if ((event.metaKey || event.ctrlKey) && !event.altKey && event.key.toLowerCase() === 'a') {
      event.preventDefault(); selectAll();
    } else if (event.key === 'Escape' && selection().size) {
      event.preventDefault(); event.stopPropagation(); resetSelection(); renderApp();
    }
  }
  for (const id of ['meeting-list', 'library-batch']) byId(id).addEventListener('keydown', handleSelectionKeydown);
  byId('meeting-list').addEventListener('contextmenu', event => {
    const row = event.target.closest('[data-meeting-id]');
    const menu = row?.querySelector('[data-meeting-menu]');
    if (!menu) return;
    event.preventDefault();
    menu.open = true;
    positionMenu(menu);
    menu.querySelector('button:not(:disabled)')?.focus({ preventScroll: true });
  });
  byId('meeting-list').addEventListener('dragstart', event => {
    clearDrag();
    const row = event.target.closest('[data-meeting-id]');
    const ids = row && selection().has(row.dataset.meetingId) ? selectedIds() : row ? [row.dataset.meetingId] : [];
    if (!row || !state().connected || state().libraryBusy || !event.dataTransfer || event.target.closest('input,summary,[data-library-action]')) { event.preventDefault(); return; }
    dragSources = sourcesForMove(ids);
    if (!dragSources.length) { event.preventDefault(); return; }
    try {
      closeMenu();
      event.dataTransfer.effectAllowed = 'move';
      event.dataTransfer.setData('application/x-nero-meetings', JSON.stringify(dragSources.map(source => source.id)));
      dragPreview.show(event, current(row.dataset.meetingId)?.title, dragSources.length);
    } catch {
      event.preventDefault();
      clearDrag();
    }
  });
  byId('meeting-list').addEventListener('dragend', clearDrag);
  function dropDestination(event) {
    const target = event.target.closest('[data-library-filter]');
    if (!target || target.disabled || target.isConnected === false || state().libraryBusy || !sourcesUnchanged(dragSources)
      || !Array.from(event.dataTransfer?.types || []).includes('application/x-nero-meetings')) return null;
    const filter = target.dataset.libraryFilter;
    if (filter === 'trash') return { target, filter };
    const destination = moveDestination(filter);
    return destination && dragSources.some(source => source.folderId !== destination.folderId) ? { target, filter, destination } : null;
  }
  for (const id of ['folder-list', 'library-folders']) {
    byId(id).addEventListener('dragover', event => {
      const destination = dropDestination(event);
      if (!destination) { clearDropTarget(); if (event.dataTransfer) event.dataTransfer.dropEffect = 'none'; return; }
      const { target } = destination;
      event.preventDefault();
      event.dataTransfer.dropEffect = 'move';
      if (dropTarget !== target) { clearDropTarget(); dropTarget = target; target.classList.add('is-drop-target'); }
    });
    byId(id).addEventListener('dragleave', event => { if (dropTarget && !dropTarget.contains(event.relatedTarget)) clearDropTarget(); });
    byId(id).addEventListener('drop', event => {
      const destination = dropDestination(event);
      clearDropTarget();
      dragPreview.clear();
      if (!destination) { dragSources = []; return; }
      event.preventDefault();
      const sources = dragSources; dragSources = [];
      if (destination.filter === 'trash') trashDialog(sources.map(source => source.id));
      else void mutate(() => moveChecked(sources, destination.destination), resetSelection);
    });
  }
  return { render, show, resetSelection, toggleSelection, handleRowSelection, selectedIds, visibleMeetings };
}
