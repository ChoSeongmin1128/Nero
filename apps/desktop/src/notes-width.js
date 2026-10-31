import { mountResizablePane } from '../../../design-system/resizable-pane.mjs';

const DEFAULT = 320, MIN = 240, MAX = 560, BODY_MIN = 420;
export function notesWidthPreference(value) {
  const number = typeof value === 'number' || typeof value === 'string' && value.trim() ? Number(value) : NaN;
  return Number.isFinite(number) && number > 0 ? Math.round(Math.max(MIN, Math.min(MAX, number))) : DEFAULT;
}
export function notesWidthBounds(width) {
  const max = Math.max(0, Math.min(MAX, width - BODY_MIN));
  return { min: Math.min(MIN, max), max };
}
export function mountNotesWidth({ root, handle }) {
  let storage;
  try { storage = root.ownerDocument.defaultView.localStorage; } catch {}
  return mountResizablePane({ root, handle, storage, storageKey: 'nero.notes.width', direction: -1,
    defaultValue: DEFAULT, preference: notesWidthPreference, resizingClass: 'notes-resizing',
    bounds: () => notesWidthBounds(root.getBoundingClientRect().width || root.ownerDocument.defaultView.innerWidth),
    render(width) { root.style.setProperty('--notes-width', `${width}px`); },
  });
}
