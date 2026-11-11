import { mountResizablePane } from './resizable-pane.mjs';

const DEFAULT_WIDTH = 224;
const MIN_WIDTH = 200;
const MAX_WIDTH = 400;
const CONTENT_MIN = 400;
const OVERLAY_GAP = 48;

export function navigationRailWidth(root) {
  const document = root.ownerDocument;
  const value = document.defaultView.getComputedStyle(document.documentElement).getPropertyValue('--navigation-rail-width');
  return Math.max(0, Number.parseFloat(value) || 0);
}

export function navigationWidthPreference(value) {
  const number = typeof value === 'number' || typeof value === 'string' && value.trim() ? Number(value) : NaN;
  return Number.isFinite(number) && number > 0 ? Math.round(Math.max(MIN_WIDTH, Math.min(MAX_WIDTH, number))) : DEFAULT_WIDTH;
}

export function navigationWidthBounds(viewport, { overlay: temporary = false, railWidth = 0 } = {}) {
  const width = Math.max(0, viewport - (temporary ? railWidth : 0));
  const overlay = temporary || viewport < MIN_WIDTH + CONTENT_MIN;
  const max = Math.max(0, Math.min(MAX_WIDTH, width - (overlay ? OVERLAY_GAP : CONTENT_MIN)));
  return { min: Math.min(MIN_WIDTH, max), max, overlay };
}

export function mountNavigationWidth({ shell, handle, storage, storageKey, onDragging, getMode = () => 'pinned' }) {
  return mountResizablePane({ root: shell, handle, storage, storageKey,
    preference: navigationWidthPreference, defaultValue: DEFAULT_WIDTH, resizingClass: 'nav-resizing', onDragging,
    bounds: () => {
      const mode = getMode();
      const limits = navigationWidthBounds(shell.getBoundingClientRect().width || shell.ownerDocument.defaultView.innerWidth, { overlay: mode !== 'pinned', railWidth: navigationRailWidth(shell) });
      // History needs room for a title and short date. Clamping the temporary
      // view does not replace the user's stored pinned-sidebar preference.
      if (mode === 'ai-temporary') limits.min = Math.min(300, limits.max);
      return limits;
    },
    render(width, limits) { shell.style.setProperty('--navigation-width', `${width}px`); shell.classList.toggle('nav-overlay', limits.overlay); },
  });
}
