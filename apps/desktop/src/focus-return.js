export function returnFocusTarget(origin, fallback) {
  if (!origin || origin.isConnected === false || origin.disabled || origin.closest?.('[hidden],[inert]')) return fallback;
  const view = origin.ownerDocument?.defaultView;
  for (let node = origin; view && node; node = node.parentElement) {
    const style = view.getComputedStyle(node);
    if (style.display === 'none' || style.visibility === 'hidden') return fallback;
  }
  return origin;
}
