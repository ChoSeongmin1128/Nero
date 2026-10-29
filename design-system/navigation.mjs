export function renderMeetingMenu(meeting, { escape, icon, disabled = false }) {
  const actions = meeting.deletedAt != null ? [['restore', '복원'], ['purge', '영구 삭제']] : [['rename', '이름 변경'], ['move', '폴더로 이동'], ['trash', '휴지통으로 이동']];
  return `<details class="meeting-menu document-export" data-meeting-menu="${escape(meeting.id)}"><summary aria-label="${escape(meeting.title)} 더보기" title="${escape(meeting.title)} 더보기">${icon('ellipsis')}</summary><div class="action-menu">${actions.map(([action, label]) => `<button type="button" class="${['trash', 'purge'].includes(action) ? 'danger-text' : ''}" aria-label="${escape(label)}" data-library-action="${action}" data-library-target="${escape(meeting.id)}"${disabled ? ' disabled' : ''}>${escape(label)}</button>`).join('')}</div></details>`;
}

export function recentMeetingRow({ id, title, metadata, active = false, disabled = false, menu = '', escape }) {
  const [folder, timestamp, duration] = metadata;
  const description = [folder, timestamp].filter(Boolean);
  return `<article class="navigation-recent-row" data-recent-row="${escape(id)}"><button type="button" class="quiet recent-meeting" data-recent-meeting="${escape(id)}"${active ? ' aria-current="page"' : ''}${disabled ? ' disabled' : ''}><span class="recent-title-line"><strong title="${escape(title)}">${escape(title)}</strong>${duration ? `<span class="recent-meeting-duration">${escape(duration)}</span>` : ''}</span><span class="recent-meeting-meta" title="${escape(description.join(', '))}">${description.map(value => `<span>${escape(value)}</span>`).join('')}</span></button>${menu}</article>`;
}

export function mountNavigationSidebar({ shell, aside, content, trigger, backdrop, storage, storageKey = 'nero.sidebar.pinned', icons = globalThis.GroveIcons }) {
  const document = shell.ownerDocument;
  const read = () => { try { return (storage ?? globalThis.localStorage)?.getItem(storageKey); } catch { return null; } };
  let pinned = read() === 'true', temporary = false, timer, renderedState;
  const brand = aside.querySelector('.navigation-brand') || aside.querySelector('.brand');
  const rail = shell.querySelector('.navigation-rail');
  const triggerHome = trigger.parentElement;
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
    aside.setAttribute('aria-label', '회의 탐색');
    trigger.setAttribute('aria-expanded', String(open));
    const destination = open ? brand : triggerHome;
    if (trigger.parentElement !== destination) destination.append(trigger);
    if (rail) rail.inert = unavailable() || open;
    trigger.setAttribute('aria-label', pinned ? '사이드바 접기' : temporary ? '사이드바 고정' : '사이드바 열기');
    trigger.title = trigger.getAttribute('aria-label');
    trigger.innerHTML = icons?.(temporary && !pinned ? 'pin' : pinned ? 'panel-left-close' : 'panel-left') || '';
    if (backdrop) backdrop.hidden = true;
  }
  function reveal() { if (unavailable()) return; clearTimeout(timer); if (!pinned) temporary = true; render(); }
  function hide() {
    clearTimeout(timer);
    timer = setTimeout(() => {
      const focused = document.activeElement;
      const interacting = aside.contains(focused) && (focused.matches('input,textarea') || focused.matches(':focus-visible'));
      if (!interacting && !rail?.matches(':hover') && !trigger.matches(':hover') && !aside.matches(':hover')) { temporary = false; render(); }
    }, 180);
  }
  function pin(value) {
    if (unavailable()) return;
    clearTimeout(timer); pinned = value; temporary = false;
    try { (storage ?? globalThis.localStorage)?.setItem(storageKey, String(pinned)); } catch {}
    render(); trigger.focus({ preventScroll: true });
  }
  const hover = event => { if (event.pointerType === 'mouse') reveal(); };
  rail?.addEventListener('pointerenter', hover);
  rail?.addEventListener('pointerleave', hide);
  rail?.addEventListener('focusout', hide);
  trigger.addEventListener('pointerenter', hover);
  trigger.addEventListener('pointerleave', hide);
  aside.addEventListener('pointerenter', () => clearTimeout(timer));
  aside.addEventListener('pointerleave', hide);
  aside.addEventListener('focusout', hide);
  trigger.addEventListener('click', () => {
    if (unavailable()) return;
    if (pinned) pin(false);
    else if (temporary) pin(true);
    else { temporary = true; render(); trigger.focus(); }
  });
  document.addEventListener('pointerdown', event => {
    if (temporary && !aside.contains(event.target) && !rail?.contains(event.target) && !trigger.contains(event.target)) { temporary = false; render(); }
  });
  document.addEventListener('keydown', event => {
    const popup = event.target.closest?.('dialog[open],[role="dialog"],[role="alertdialog"],[role="menu"],[role="listbox"]')
      || document.querySelector('dialog[open],:is([role="dialog"],[role="alertdialog"],[role="menu"],[role="listbox"])[data-open],[data-slot="popover-content"][data-open],[data-slot="select-content"][data-open],details[data-meeting-menu][open],details.library-folder-menu[open]');
    if (unavailable() || event.defaultPrevented || popup) return;
    if (event.key === 'Escape' && temporary) { temporary = false; render(); trigger.focus(); }
    if ((event.metaKey || event.ctrlKey) && event.code === 'Backslash') { event.preventDefault(); pin(!pinned); }
  });
  render();
  return {
    render,
    navigate() { if (!pinned) { temporary = false; render(); } },
    resetTemporary() { temporary = false; render(); },
  };
}
