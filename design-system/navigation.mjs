import { mountNavigationWidth } from './navigation-width.mjs';

export function renderMeetingMenu(meeting, { escape, icon, disabled = false }) {
  const actions = meeting.deletedAt != null ? [['restore', '복원'], ['purge', '영구 삭제']] : [['rename', '이름 변경'], ['move', '폴더로 이동'], ['trash', '휴지통으로 이동']];
  return `<details class="meeting-menu document-export" data-meeting-menu="${escape(meeting.id)}"><summary aria-label="${escape(meeting.title)} 더보기" title="${escape(meeting.title)} 더보기">${icon('ellipsis')}</summary><div class="action-menu">${actions.map(([action, label]) => `<button type="button" class="${['trash', 'purge'].includes(action) ? 'danger-text' : ''}" aria-label="${escape(label)}" data-library-action="${action}" data-library-target="${escape(meeting.id)}"${disabled ? ' disabled' : ''}>${escape(label)}</button>`).join('')}</div></details>`;
}

export function recentMeetingRow({ id, title, metadata, active = false, disabled = false, menu = '', leadingIcon = '', durationLabel = '', escape }) {
  const [folder, timestamp, duration] = metadata;
  const description = [folder, timestamp].filter(Boolean);
  return `<article class="navigation-recent-row" data-recent-row="${escape(id)}"><button type="button" class="quiet recent-meeting" data-recent-meeting="${escape(id)}" aria-label="${escape([title, ...description, duration ? `녹음 길이 ${durationLabel || duration}` : ''].filter(Boolean).join(', '))}"${active ? ' aria-current="page"' : ''}${disabled ? ' disabled' : ''}>${leadingIcon ? `<span class="recent-meeting-icon" aria-hidden="true">${leadingIcon}</span>` : ''}<span class="recent-title-line"><strong title="${escape(title)}">${escape(title)}</strong>${folder ? `<span class="recent-meeting-folder" title="${escape(folder)}">${escape(folder)}</span>` : ''}</span><span class="recent-meeting-meta"><span class="recent-meeting-timestamp" title="${escape(timestamp || '')}">${escape(timestamp || '')}</span>${duration ? `<span class="recent-meeting-duration" aria-label="녹음 길이 ${escape(durationLabel || duration)}">${escape(duration)}</span>` : ''}</span></button>${menu}</article>`;
}

// The shell decides whether the sidebar shows. `onPreview` names the rail list to show, on a
// rest, a click or the keyboard; `onActivate` follows only a click or the keyboard, when the
// person chose that place; `onStateChange` follows the sidebar opening, closing or pinning.
export function mountNavigationSidebar({ shell, aside, content, trigger, backdrop, storage, storageKey = 'nero.sidebar.pinned', icons = globalThis.NeroIcons, onPreview = () => {}, onActivate = () => {}, onStateChange = () => {} }) {
  const document = shell.ownerDocument;
  let preferenceStorage = storage;
  if (!preferenceStorage) { try { preferenceStorage = document.defaultView?.localStorage; } catch {} }
  const read = () => { try { return preferenceStorage?.getItem(storageKey); } catch { return null; } };
  let pinned = read() === 'true', temporary = false, timer, hoverTimer, renderedState;
  let previewAnchor = null, previewView = null, restoringFocus = false, keyboardInteraction = true;
  const hovered = new Set();
  const edge = shell.querySelector('.navigation-edge');
  const compact = shell.querySelector('[data-navigation-compact]');
  const previews = [...compact?.querySelectorAll('[data-navigation-preview]') || []];
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
  const width = mountNavigationWidth({ shell, handle, storage: preferenceStorage, storageKey: `${storageKey}.width`, getMode: () => pinned ? 'pinned' : 'temporary', onDragging: active => { if (active) clearTimeout(timer); else hide(); } });
  const unavailable = () => shell.dataset.onboarding === 'true';
  function render() {
    if (unavailable()) temporary = false;
    const open = !unavailable() && (pinned || temporary);
    if (open) hovered.delete(edge);
    const nextState = `${pinned}:${temporary}:${unavailable()}:${previewView}`;
    if (renderedState === nextState) return;
    renderedState = nextState;
    shell.classList.toggle('collapsed', !pinned);
    shell.classList.toggle('nav-peek', temporary && !pinned);
    shell.classList.toggle('nav-pinned', pinned);
    // The rail stays in place whether the sidebar is pinned or opens on demand.
    if (compact) {
      compact.hidden = false;
      compact.inert = unavailable();
      compact.setAttribute('aria-hidden', String(unavailable()));
    }
    aside.inert = !open;
    width.update(open);
    width.refresh();
    aside.setAttribute('aria-label', '회의 탐색');
    trigger.disabled = unavailable();
    trigger.setAttribute('aria-expanded', String(open));
    trigger.setAttribute('aria-label', pinned ? '사이드바 접기' : temporary ? '사이드바 펼쳐두기' : '사이드바 열기');
    trigger.title = trigger.getAttribute('aria-label');
    trigger.innerHTML = icons?.(pinned ? 'panel-left-close' : temporary ? 'panel-left-peek' : 'panel-left') || '';
    if (backdrop) backdrop.hidden = true;
    onStateChange({ expanded: open, pinned });
  }
  function reveal() { if (unavailable()) return; clearTimeout(hoverTimer); clearTimeout(timer); if (!pinned) temporary = true; render(); }
  // A rail button chooses the sidebar's list; an unpinned sidebar opens to show it.
  function openPreview(button, { hover = false } = {}) {
    if (unavailable()) return;
    previewAnchor = button;
    previewView = button.dataset.navigationPreview;
    onPreview(previewView);
    if (!hover) onActivate(previewView);
    if (!pinned) reveal();
  }
  function openPopups() {
    return [...document.querySelectorAll('dialog[open],[role="dialog"],[role="alertdialog"],[role="menu"],[role="listbox"],[data-slot="menu-popup"],[data-slot="popover-content"],[data-slot="select-content"],details[open]:not(.recent)')].filter(node => {
      if (node.closest('[hidden],[inert],[aria-hidden="true"],[data-state="closed"],[data-closed],[data-open="false"],details:not([open])')) return false;
      if (node.matches('details') && !node.matches('.meeting-menu,.library-folder-menu,.document-export,.speed') && !node.querySelector(':scope > .action-menu,:scope > .speed-menu,:scope > [role="menu"],:scope > [data-slot="menu-popup"]')) return false;
      const style = document.defaultView.getComputedStyle(node);
      return style.display !== 'none' && style.visibility !== 'hidden';
    });
  }
  function editingNavigation() {
    const focused = document.activeElement;
    return aside.contains(focused) && focused.matches('input,textarea,select,[contenteditable]:not([contenteditable="false"])');
  }
  function availablePreviewTarget(node) {
    if (!node || node.disabled || node.closest('[hidden],[inert],[aria-hidden="true"],details:not([open])')) return false;
    for (let parent = node; parent && aside.contains(parent); parent = parent.parentElement) {
      const style = document.defaultView.getComputedStyle(parent);
      if (style.display === 'none' || style.visibility === 'hidden') return false;
    }
    return true;
  }
  function firstPreviewTarget() {
    const targets = '[data-recent-meeting],[data-library-filter],[data-navigation-item]';
    return [...aside.querySelectorAll(targets)].find(availablePreviewTarget)
      || [...aside.querySelectorAll('input[type="search"],#search')].find(availablePreviewTarget);
  }
  function firstPreviewBoundary() {
    return [...aside.querySelectorAll('[data-navigation-preview-action]')].find(availablePreviewTarget) || firstPreviewTarget();
  }
  function hide() {
    clearTimeout(hoverTimer); clearTimeout(timer);
    timer = setTimeout(() => {
      const focused = document.activeElement;
      const interacting = editingNavigation() || (aside.contains(focused) || handle.contains(focused) || compact?.contains(focused)) && (keyboardInteraction || focused.matches(':focus-visible'));
      if (!width.isDragging() && !hovered.size && !interacting && !openPopups().length) { temporary = false; render(); }
    }, 180);
  }
  function restoreFocus(node) {
    restoringFocus = true;
    node?.focus({ preventScroll: true });
    restoringFocus = false;
  }
  function closeTemporary(restore = false) {
    clearTimeout(hoverTimer); clearTimeout(timer);
    const focused = document.activeElement;
    const hiddenFocus = aside.contains(focused) || handle.contains(focused);
    temporary = false; render();
    if (restore && hiddenFocus) restoreFocus(previewAnchor?.isConnected ? previewAnchor : trigger);
  }
  function pin(value) {
    if (unavailable()) return;
    clearTimeout(hoverTimer); clearTimeout(timer); pinned = value; temporary = false;
    try { preferenceStorage?.setItem(storageKey, String(pinned)); } catch {}
    const hiddenFocus = aside.contains(document.activeElement) || handle.contains(document.activeElement);
    hovered.clear();
    render();
    if (!pinned && hiddenFocus) restoreFocus(trigger);
  }
  const enter = node => { hovered.add(node); clearTimeout(timer); };
  const leave = node => { hovered.delete(node); hide(); };
  edge?.addEventListener('pointerenter', event => { if (event.pointerType === 'mouse' && !pinned) { enter(edge); reveal(); } });
  // Pointing at the sidebar toggle shows the sidebar; clicking it pins or unpins it.
  trigger.addEventListener('pointerenter', event => {
    if (event.pointerType !== 'mouse' || pinned || unavailable()) return;
    enter(trigger);
    clearTimeout(hoverTimer);
    hoverTimer = setTimeout(reveal, 140);
  });
  trigger.addEventListener('pointerleave', () => { clearTimeout(hoverTimer); leave(trigger); });
  edge?.addEventListener('pointerleave', () => leave(edge));
  edge?.addEventListener('focusout', hide);
  aside.addEventListener('pointerenter', event => {
    if (event.pointerType !== 'mouse') return;
    enter(aside);
    if (!pinned && !temporary) hoverTimer = setTimeout(reveal, 140);
  });
  aside.addEventListener('pointerleave', () => leave(aside));
  aside.addEventListener('focusout', hide);
  aside.addEventListener('keydown', event => {
    if (pinned || !temporary || !previewAnchor || unavailable() || event.defaultPrevented || event.isComposing || event.keyCode === 229 || event.altKey || event.metaKey || event.ctrlKey || openPopups().length) return;
    const previous = event.key === 'Tab' && event.shiftKey && document.activeElement === firstPreviewBoundary();
    const back = event.key === 'ArrowLeft' && !event.shiftKey && !editingNavigation();
    if (!previous && !back) return;
    event.preventDefault();
    restoreFocus(previewAnchor);
  });
  handle.addEventListener('pointerenter', event => { if (event.pointerType === 'mouse' && (!compact || pinned || temporary)) { enter(handle); reveal(); } });
  handle.addEventListener('pointerleave', () => leave(handle));
  handle.addEventListener('focusout', hide);
  compact?.addEventListener('pointerenter', event => { if (event.pointerType === 'mouse') enter(compact); });
  compact?.addEventListener('pointerleave', () => leave(compact));
  compact?.addEventListener('focusout', hide);
  // Resting on a rail button shows its list, pinned or not; passing over one on the way
  // elsewhere does not.
  let previewHover;
  for (const button of previews) {
    button.addEventListener('pointerenter', event => {
      clearTimeout(previewHover);
      if (event.pointerType !== 'mouse' || unavailable() || openPopups().length) return;
      previewHover = setTimeout(() => { if (!unavailable() && !openPopups().length) openPreview(button, { hover: true }); }, 140);
    });
    button.addEventListener('pointerleave', () => clearTimeout(previewHover));
    button.addEventListener('click', () => openPreview(button));
    button.addEventListener('keydown', event => {
      if (!['ArrowRight', 'Enter'].includes(event.key) || event.defaultPrevented || event.isComposing || event.keyCode === 229 || event.altKey || event.metaKey || event.ctrlKey || event.shiftKey || unavailable() || openPopups().length) return;
      event.preventDefault();
      openPreview(button);
      // An empty preview list still lets the keyboard reach the view's own actions.
      (firstPreviewTarget() || firstPreviewBoundary())?.focus({ preventScroll: true });
    });
  }
  trigger.addEventListener('click', () => {
    if (unavailable()) return;
    if (pinned) pin(false);
    else pin(true);
  });
  document.addEventListener('pointerdown', event => {
    keyboardInteraction = false;
    if (temporary && !handle.contains(event.target) && !aside.contains(event.target) && !compact?.contains(event.target) && !edge?.contains(event.target) && !trigger.contains(event.target) && !openPopups().some(node => node.contains(event.target))) closeTemporary();
  });
  document.addEventListener('keydown', event => {
    keyboardInteraction = true;
    if (unavailable() || event.defaultPrevented || event.isComposing || event.keyCode === 229 || openPopups().length) return;
    if (event.key === 'Escape' && temporary) { event.preventDefault(); closeTemporary(true); }
    if ((event.metaKey || event.ctrlKey) && event.code === 'Backslash') { event.preventDefault(); pin(!pinned); }
  });
  render();
  return {
    render,
    reveal,
    navigate() { if (!pinned && !editingNavigation() && !openPopups().length) closeTemporary(); },
    resetTemporary() { closeTemporary(); },
    attention(view, on) {
      const button = previews.find(item => item.dataset.navigationPreview === view);
      if (!button) return;
      // The button's own label is kept once, before the phrase is ever added to it.
      if (!('navigationLabel' in button.dataset)) button.dataset.navigationLabel = button.getAttribute('aria-label') || '';
      const own = button.dataset.navigationLabel;
      if (on) {
        button.dataset.attention = 'true';
        button.setAttribute('aria-label', `${own || button.textContent.trim()}, 확인할 항목 있음`);
      } else {
        delete button.dataset.attention;
        if (own) button.setAttribute('aria-label', own);
        else button.removeAttribute('aria-label');
      }
    },
  };
}
