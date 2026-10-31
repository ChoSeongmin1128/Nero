export function createViewHistory({ navigate, canVisit = () => true, onChange = () => {}, limit = 100 }) {
  let entries = [], index = -1, pending = null, epoch = 0;
  const key = route => JSON.stringify([route.kind, route.id || null, route.filter || null, route.view || null]);
  const status = () => ({ canBack: !pending && index > 0, canForward: !pending && index < entries.length - 1, busy: Boolean(pending) });
  const publish = () => onChange(status());
  function visit(route) {
    if (!route || !canVisit(route)) return;
    const value = { ...route };
    if (pending) {
      if ([pending.from, pending.target].includes(key(value))) return;
      pending = null; epoch++;
    }
    if (index >= 0 && key(entries[index]) === key(value)) { entries[index] = value; return; }
    entries = entries.slice(0, index + 1); entries.push(value);
    if (entries.length > limit) entries.shift();
    index = entries.length - 1; publish();
  }
  async function go(direction) {
    if (pending || ![-1, 1].includes(direction)) return false;
    let target = index + direction;
    while (target >= 0 && target < entries.length && !canVisit(entries[target])) {
      entries.splice(target, 1); if (target < index) index--;
      target = index + direction;
    }
    if (target < 0 || target >= entries.length) { publish(); return false; }
    const request = ++epoch;
    pending = { from: key(entries[index]), target: key(entries[target]) }; publish();
    try {
      if (!await navigate({ ...entries[target] }, { isCurrent: () => request === epoch }) || request !== epoch) return false;
      index = target; return true;
    } finally {
      if (request === epoch) { pending = null; publish(); }
    }
  }
  return { visit, go, status, cancel() { if (!pending) return; pending = null; epoch++; publish(); } };
}
