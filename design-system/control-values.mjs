export function dateAt(year, month, day) {
  const date = new Date(0);
  date.setHours(12, 0, 0, 0);
  date.setFullYear(year, month, day);
  return date;
}

export const dateKey = date => `${String(date.getFullYear()).padStart(4, '0')}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`;

export function parseDate(value) {
  const match = /^(\d{4})-(\d{2})-(\d{2})(?:T|$)/.exec(value || '');
  if (!match) return null;
  const date = dateAt(+match[1], +match[2] - 1, +match[3]);
  return dateKey(date) === match[0].replace(/T$/, '') ? date : null;
}

export function initialDate(value, min, max, today = new Date()) {
  let date = parseDate(value) || today;
  if (min && dateKey(date) < min.slice(0, 10)) date = parseDate(min) || date;
  if (max && dateKey(date) > max.slice(0, 10)) date = parseDate(max) || date;
  return date;
}

export function dateValue(day, time, { timed, min = '', max = '' }) {
  if (!day) return '';
  if (!parseDate(day) || timed && !/^([01]\d|2[0-3]):[0-5]\d$/.test(time)) return null;
  const value = day + (timed ? `T${time}` : '');
  return min && value < min || max && value > max ? null : value;
}

export function nativeOptions(input) {
  return [...input.options].map(option => ({ value: option.value, label: option.textContent, disabled: option.disabled || Boolean(option.closest('optgroup')?.disabled), hidden: option.hidden }));
}
