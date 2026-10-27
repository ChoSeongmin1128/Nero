export function createDragPreview(owner) {
  let preview = null, frame = null;
  const view = owner?.defaultView;

  function clear() {
    if (frame != null) view?.cancelAnimationFrame?.(frame);
    frame = null;
    preview?.remove();
    preview = null;
  }

  function show(event, title, count) {
    clear();
    if (typeof event.dataTransfer?.setDragImage !== 'function' || !owner?.body?.appendChild || !owner.createElement) return false;
    try {
      const node = owner.createElement('div');
      node.className = 'meeting-drag-preview';
      node.setAttribute('aria-hidden', 'true');
      const label = owner.createElement('span');
      label.className = 'meeting-drag-preview-title';
      label.textContent = title || '제목 없는 회의';
      node.appendChild(label);
      if (count > 1) {
        const badge = owner.createElement('span');
        badge.className = 'meeting-drag-preview-count';
        badge.textContent = `${count}개`;
        node.appendChild(badge);
      }
      preview = node;
      node.style.left = '0px';
      node.style.top = '0px';
      owner.body.appendChild(node);
      const bounds = node.getBoundingClientRect();
      if (!(bounds.width > 0 && bounds.height > 0)) { clear(); return false; }
      const x = Math.min(12, Math.floor(bounds.width / 2));
      const y = Math.floor(bounds.height / 2);
      const width = view?.innerWidth || owner.documentElement?.clientWidth || bounds.width;
      const height = view?.innerHeight || owner.documentElement?.clientHeight || bounds.height;
      node.style.left = `${Math.max(0, Math.min((event.clientX || 0) - x, width - bounds.width))}px`;
      node.style.top = `${Math.max(0, Math.min((event.clientY || 0) - y, height - bounds.height))}px`;
      // WebKit snapshots a rendered element during dragstart, before the next frame.
      event.dataTransfer.setDragImage(node, x, y);
      if (view?.requestAnimationFrame) frame = view.requestAnimationFrame(() => {
        if (preview === node) { node.remove(); preview = null; frame = null; }
      });
      return true;
    } catch {
      clear();
      return false;
    }
  }

  return { show, clear };
}
