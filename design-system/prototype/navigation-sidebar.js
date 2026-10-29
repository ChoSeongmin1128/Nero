export function mountNavigationSidebar({ shell, aside, content, trigger, backdrop, storage = localStorage }) {
  let pinned = storage.getItem('nero-proposal-sidebar-pinned') === 'true';
  let temporary = false;
  const brand = aside.querySelector('.brand');
  const toolbar = trigger.parentElement;
  const edge = document.createElement('div');
  edge.className = 'nav-hover-edge'; edge.setAttribute('aria-hidden', 'true'); shell.append(edge);
  let timer;
  function render() {
    const open = pinned || temporary;
    shell.classList.toggle('collapsed', !pinned);
    shell.classList.toggle('nav-peek', temporary && !pinned);
    shell.classList.toggle('nav-pinned', pinned);
    content.inert = false;
    aside.inert = !open;
    aside.removeAttribute('role');
    aside.removeAttribute('aria-modal');
    aside.setAttribute('aria-label', '회의 탐색');
    trigger.setAttribute('aria-expanded', String(open));
    if (open && trigger.parentElement !== brand) brand.append(trigger);
    else if (!open && trigger.parentElement !== toolbar) toolbar.prepend(trigger);
    trigger.setAttribute('aria-label', pinned ? '사이드바 접기' : temporary ? '사이드바 고정' : '사이드바 열기');
    trigger.title = trigger.getAttribute('aria-label');
    trigger.innerHTML = GroveIcons(temporary && !pinned ? 'chevrons-right' : 'panel-left-close');
    edge.hidden = open;
    backdrop.hidden = true;
  }
  function reveal() { clearTimeout(timer); if (!pinned) temporary = true; render(); }
  function hide() {
    clearTimeout(timer);
    timer = setTimeout(() => {
      const focused = document.activeElement;
      const interacting = aside.contains(focused) && (focused.matches('input,textarea') || focused.matches(':focus-visible'));
      if (!interacting && !trigger.matches(':hover') && !aside.matches(':hover')) { temporary = false; render(); }
    }, 180);
  }
  function pin(value) {
    clearTimeout(timer); pinned = value; temporary = false;
    storage.setItem('nero-proposal-sidebar-pinned', String(pinned)); render(); trigger.focus();
  }
  function hover(event) { if (event.pointerType === 'mouse') reveal(); }
  trigger.addEventListener('pointerenter', hover);
  edge.addEventListener('pointerenter', hover);
  edge.addEventListener('pointerleave', hide);
  trigger.addEventListener('pointerleave', hide);
  aside.addEventListener('pointerenter', () => clearTimeout(timer));
  aside.addEventListener('pointerleave', hide);
  aside.addEventListener('focusout', hide);
  trigger.onclick = () => {
    if (pinned) pin(false);
    else if (temporary) pin(true);
    else { temporary = true; render(); trigger.focus(); }
  };

  const outside = event => {
    if (temporary && !aside.contains(event.target) && !trigger.contains(event.target)) { temporary = false; render(); }
  };
  document.addEventListener('pointerdown', outside);
  function escape(event) {
    if (event.key === 'Escape' && temporary) { temporary = false; render(); trigger.focus(); }
    if ((event.metaKey || event.ctrlKey) && event.code === 'Backslash') { event.preventDefault(); pin(!pinned); }
  }
  document.addEventListener('keydown', escape);
  render();
  return {
    render,
    navigate() { if (!pinned) { temporary = false; render(); } },
    resetTemporary() { temporary = false; render(); },
  };
}
