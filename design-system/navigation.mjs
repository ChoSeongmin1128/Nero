import { mountNavigationWidth } from './navigation-width.mjs';

export function renderMeetingMenu(meeting, { escape, icon, disabled = false }) {
  const actions = meeting.deletedAt != null ? [['restore', '복원'], ['purge', '영구 삭제']] : [['rename', '이름 변경'], ['move', '폴더로 이동'], ['trash', '휴지통으로 이동']];
  return `<details class="meeting-menu document-export" data-meeting-menu="${escape(meeting.id)}"><summary aria-label="${escape(meeting.title)} 더보기" title="${escape(meeting.title)} 더보기">${icon('ellipsis')}</summary><div class="action-menu">${actions.map(([action, label]) => `<button type="button" class="${['trash', 'purge'].includes(action) ? 'danger-text' : ''}" aria-label="${escape(label)}" data-library-action="${action}" data-library-target="${escape(meeting.id)}"${disabled ? ' disabled' : ''}>${escape(label)}</button>`).join('')}</div></details>`;
}

export function recentMeetingRow({ id, title, metadata, active = false, disabled = false, menu = '', escape }) {
  const [folder, timestamp, duration] = metadata;
  const description = [folder, timestamp].filter(Boolean);
  return `<article class="navigation-recent-row" data-recent-row="${escape(id)}"><button type="button" class="quiet recent-meeting" data-recent-meeting="${escape(id)}" aria-label="${escape([title, ...description, duration ? `녹음 길이 ${duration}` : ''].filter(Boolean).join(', '))}"${active ? ' aria-current="page"' : ''}${disabled ? ' disabled' : ''}><span class="recent-title-line"><strong title="${escape(title)}">${escape(title)}</strong>${folder ? `<span class="recent-meeting-folder" title="${escape(folder)}">${escape(folder)}</span>` : ''}</span><span class="recent-meeting-meta"><span class="recent-meeting-timestamp" title="${escape(timestamp || '')}">${escape(timestamp || '')}</span>${duration ? `<span class="recent-meeting-duration" aria-label="녹음 길이 ${escape(duration)}">${escape(duration)}</span>` : ''}</span></button>${menu}</article>`;
}

export function mountNavigationSidebar({ shell, aside, content, trigger, backdrop, storage, storageKey = 'nero.sidebar.pinned', icons = globalThis.NeroIcons }) {
  const document = shell.ownerDocument;
  let preferenceStorage = storage;
  if (!preferenceStorage) { try { preferenceStorage = document.defaultView?.localStorage; } catch {} }
  const read = () => { try { return preferenceStorage?.getItem(storageKey); } catch { return null; } };
  let pinned = read() === 'true', temporary = false, timer, hoverTimer, renderedState;
  const edge = shell.querySelector('.navigation-edge');
  for (const [selector, label] of [['#create-meeting', '새 회의'], ['#new', '새 회의']]) {
    const button = aside.querySelector(selector);
    if (button) { button.setAttribute('aria-label', label); button.title = label; }
  }
  const handle = document.createElement('div');
  handle.className = 'navigation-resize';
  handle.tabIndex = 0;
  handle.setAttribute('role', 'separator');
  handle.setAttribute('aria-label', '사이드바 너비 조절');
  handle.setAttribute('aria-orientation', 'vertical');
  if (aside.id) handle.setAttribute('aria-controls', aside.id);
  shell.append(handle);
  const width = mountNavigationWidth({ shell, handle, storage: preferenceStorage, storageKey: `${storageKey}.width`, onDragging: active => { if (active) clearTimeout(timer); else hide(); } });
  const unavailable = () => shell.dataset.onboarding === 'true';
  function render() {
    if (unavailable()) temporary = false;
    const open = !unavailable() && (pinned || temporary);
    const nextState = `${pinned}:${temporary}:${unavailable()}`;
    if (renderedState === nextState) return;
    renderedState = nextState;
    shell.classList.toggle('collapsed', !pinned);
    shell.classList.toggle('nav-peek', temporary && !pinned);
    shell.classList.toggle('nav-pinned', pinned);
    aside.inert = !open;
    for (const node of aside.querySelectorAll('.recent-section, .recent')) node.inert = !open;
    width.update(open);
    aside.setAttribute('aria-label', '회의 탐색');
    trigger.setAttribute('aria-expanded', String(open));
    trigger.setAttribute('aria-label', pinned ? '사이드바 접기' : temporary ? '사이드바 펼쳐두기' : '사이드바 열기');
    trigger.title = trigger.getAttribute('aria-label');
    trigger.innerHTML = icons?.(pinned ? 'panel-left-close' : 'panel-left') || '';
    if (backdrop) backdrop.hidden = true;
    aside.dispatchEvent(new document.defaultView.CustomEvent('nero:navigation-state', { detail: { expanded: open, pinned } }));
  }
  function reveal() { if (unavailable()) return; clearTimeout(hoverTimer); clearTimeout(timer); if (!pinned) temporary = true; render(); }
  function hide() {
    clearTimeout(hoverTimer); clearTimeout(timer);
    timer = setTimeout(() => {
      const focused = document.activeElement;
      const interacting = (aside.contains(focused) || handle.contains(focused)) && (focused.matches('input,textarea') || focused.matches(':focus-visible'));
      if (!width.isDragging() && !handle.matches(':hover') && !interacting && !edge?.matches(':hover') && !trigger.matches(':hover') && !aside.matches(':hover')) { temporary = false; render(); }
    }, 180);
  }
  function pin(value) {
    if (unavailable()) return;
    clearTimeout(hoverTimer); clearTimeout(timer); pinned = value; temporary = false;
    try { preferenceStorage?.setItem(storageKey, String(pinned)); } catch {}
    render(); trigger.focus({ preventScroll: true });
  }
  const hover = event => { if (event.pointerType === 'mouse') reveal(); };
  edge?.addEventListener('pointerenter', hover);
  edge?.addEventListener('pointerleave', hide);
  edge?.addEventListener('focusout', hide);
  aside.addEventListener('pointerenter', event => {
    clearTimeout(timer);
    if (event.pointerType === 'mouse' && !pinned) hoverTimer = setTimeout(reveal, 140);
  });
  aside.addEventListener('pointerleave', hide);
  aside.addEventListener('focusout', hide);
  handle.addEventListener('pointerenter', reveal);
  handle.addEventListener('pointerleave', hide);
  handle.addEventListener('focusout', hide);
  trigger.addEventListener('click', () => {
    if (unavailable()) return;
    if (pinned) pin(false);
    else if (temporary) pin(true);
    else pin(true);
  });
  document.addEventListener('pointerdown', event => {
    if (temporary && !handle.contains(event.target) && !aside.contains(event.target) && !edge?.contains(event.target) && !trigger.contains(event.target)) { clearTimeout(hoverTimer); clearTimeout(timer); temporary = false; render(); }
  });
  document.addEventListener('keydown', event => {
    const popup = event.target.closest?.('dialog[open],[role="dialog"],[role="alertdialog"],[role="menu"],[role="listbox"]')
      || document.querySelector('dialog[open],:is([role="dialog"],[role="alertdialog"],[role="menu"],[role="listbox"])[data-open],[data-slot="popover-content"][data-open],[data-slot="select-content"][data-open],details[data-meeting-menu][open],details.library-folder-menu[open]');
    if (unavailable() || event.defaultPrevented || event.isComposing || event.keyCode === 229 || popup) return;
    if (event.key === 'Escape' && temporary) { event.preventDefault(); clearTimeout(hoverTimer); clearTimeout(timer); temporary = false; render(); trigger.focus(); }
    if ((event.metaKey || event.ctrlKey) && event.code === 'Backslash') { event.preventDefault(); pin(!pinned); }
  });
  aside.addEventListener('nero:sidebar-reveal', reveal);
  render();
  return {
    render,
    navigate() { clearTimeout(hoverTimer); if (!pinned) { temporary = false; render(); } },
    resetTemporary() { clearTimeout(hoverTimer); temporary = false; render(); },
  };
}
