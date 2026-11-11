import { mountResizablePane } from '../../../design-system/resizable-pane.mjs';
import { navigationRailWidth } from '../../../design-system/navigation-width.mjs';

const DEFAULT = 330, MIN = 300, MAX = 640;
export function aiChatWidthPreference(value) {
  const number = typeof value === 'number' || typeof value === 'string' && value.trim() ? Number(value) : NaN;
  return Number.isFinite(number) && number > 0 ? Math.round(Math.max(MIN, Math.min(MAX, number))) : DEFAULT;
}
export function mountAiChatLayout({ root, handle, panel, trigger }) {
  if (!root || !handle || !panel) return { update() {}, destroy() {} };
  let storage;
  try { storage = root.ownerDocument.defaultView.localStorage; } catch {}
  const width = mountResizablePane({ root, handle, storage, storageKey: 'nero.ai.width', direction: -1,
    defaultValue: DEFAULT, preference: aiChatWidthPreference, resizingClass: 'ai-resizing',
    bounds() { const available = root.getBoundingClientRect().width || root.ownerDocument.defaultView.innerWidth; const max = Math.max(0, Math.min(MAX, available - (available <= 720 ? navigationRailWidth(root) : 420))); return { min: Math.min(MIN, max), max }; },
    render(value) { root.style.setProperty('--ai-width', `${value}px`); },
  });
  const update = open => { root.dataset.aiOpen = String(open); panel.inert = !open; panel.setAttribute('aria-hidden', String(!open)); trigger?.setAttribute('aria-expanded', String(open)); if(trigger){trigger.setAttribute('aria-label',open?'네로 패널 닫기':'네로에게 물어보기');trigger.title=trigger.getAttribute('aria-label');} width.update(open); };
  update(false);
  return { update, destroy: () => width.destroy() };
}
