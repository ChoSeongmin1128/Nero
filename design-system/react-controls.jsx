import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { flushSync } from 'react-dom';
import { CalendarDays } from 'lucide-react';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from './components/ui/select.jsx';
import { Popover, PopoverClose, PopoverContent, PopoverTrigger } from './components/ui/popover.jsx';
import { Calendar } from './components/ui/calendar.jsx';
import { dateKey, dateValue, initialDate, nativeOptions, parseDate } from './control-values.mjs';

const records = new Set(), mounted = new WeakMap();
let active = null, sequence = 0;

function labelFor(input) {
  const labels = [...(input.labels || [])].map(node => {
    const copy = node.cloneNode(true);
    copy.querySelectorAll('input,select,textarea,button').forEach(child => child.remove());
    return copy.textContent.trim();
  }).filter(Boolean).join(' ');
  return input.getAttribute('aria-label') || labels || input.getAttribute('placeholder') || (input.tagName === 'SELECT' ? '선택' : '날짜 선택');
}

function snapshot(record) {
  const input = record.input;
  return {
    value: input.value,
    disabled: input.disabled || input.readOnly,
    required: input.required,
    min: input.min || '', max: input.max || '',
    label: input.getAttribute('aria-label') || record.label,
    invalid: input.getAttribute('aria-invalid') === 'true',
    options: record.kind === 'select' ? nativeOptions(input) : null,
  };
}

function portal(record) {
  return record.wrapper.closest('dialog[open],#desktop-root') || record.input.ownerDocument.body;
}

function renderRecord(record) {
  if (!record.host.isConnected) return;
  flushSync(() => record.root.render(record.kind === 'select' ? <NativeSelect record={record} /> : <NativeDate record={record} />));
}

function setOpen(record, open, restore = false) {
  if (open && record.state.disabled) return;
  if (open && active && active !== record) setOpen(active, false);
  if (open && !record.open && record.kind === 'date') {
    record.date = { day: record.input.value.slice(0, 10), time: record.input.value.slice(11, 16) || '09:00', month: initialDate(record.input.value, record.input.min, record.input.max), error: false };
  }
  record.open = open;
  if (open) active = record;
  else if (active === record) active = null;
  renderRecord(record);
  if (restore) record.host.querySelector('button')?.focus({ preventScroll: true });
}

function refreshRecord(record) {
  const next = snapshot(record), signature = JSON.stringify(next);
  if (record.signature === signature) return;
  const previous = record.state;
  record.signature = signature; record.state = next;
  if ((next.disabled || record.kind === 'date' && previous?.value !== next.value) && record.open) { record.open = false; if (active === record) active = null; }
  renderRecord(record);
}

function commit(record, value) {
  if (record.state.disabled || !record.input.isConnected) return;
  const changed = record.input.value !== value;
  record.input.value = value;
  if (record.adapterInvalid && record.input.validity.valid) { record.input.removeAttribute('aria-invalid'); record.adapterInvalid = false; }
  setOpen(record, false, true);
  refreshRecord(record);
  if (changed) {
    const Event = record.input.ownerDocument.defaultView.Event;
    record.input.dispatchEvent(new Event('input', { bubbles: true }));
    record.input.dispatchEvent(new Event('change', { bubbles: true }));
  }
}

function NativeSelect({ record }) {
  const state = record.state, options = state.options.filter(option => !option.hidden);
  const selectedLabel = state.options.find(option => option.value === state.value)?.label ?? '선택';
  return <Select items={options} value={state.value} open={record.open} disabled={state.disabled} modal={false}
    onOpenChange={open => setOpen(record, open)} onValueChange={value => { if (value !== null) commit(record, value); }}>
    <SelectTrigger aria-label={`${state.label}: ${selectedLabel}`}
      aria-invalid={state.invalid || undefined} data-field={record.input.dataset.field}>
      <SelectValue>{selectedLabel}</SelectValue>
    </SelectTrigger>
    <SelectContent container={portal(record)} aria-label={state.label}>
      {options.map((option, index) => <SelectItem key={`${option.value}-${index}`} value={option.value} disabled={option.disabled}>{option.label}</SelectItem>)}
    </SelectContent>
  </Select>;
}

function NativeDate({ record }) {
  const state = record.state, timed = record.input.type === 'datetime-local';
  const date = parseDate(state.value), day = record.date?.day || '';
  const label = date ? `${date.getFullYear()}.${String(date.getMonth() + 1).padStart(2, '0')}.${String(date.getDate()).padStart(2, '0')}${timed ? ' ' + state.value.slice(11, 16) : ''}` : '날짜 선택';
  const choose = selected => {
    if (!selected) return;
    record.date.day = dateKey(selected); record.date.error = false;
    if (timed) renderRecord(record); else save(record.date.day);
  };
  const save = selected => {
    const value = dateValue(selected, record.date.time, { timed, min: state.min, max: state.max });
    if (value === null) { record.date.error = true; renderRecord(record); return; }
    commit(record, value);
  };
  const today = dateKey(new Date()), inRange = value => (!state.min || value >= state.min.slice(0, 10)) && (!state.max || value <= state.max.slice(0, 10));
  return <Popover open={record.open} onOpenChange={open => setOpen(record, open)} modal="trap-focus">
    <PopoverTrigger className="grove-date-trigger" disabled={state.disabled} aria-label={`${state.label}: ${label}`} data-field={record.input.dataset.field}>
      <span className="grove-select-value">{label}</span><CalendarDays size={16} aria-hidden="true" />
    </PopoverTrigger>
    <PopoverContent ref={node => { record.popup = node; }} container={portal(record)} className="grove-calendar" aria-label={state.label}
      initialFocus={() => record.popup?.querySelector('.rdp-day_button[tabindex="0"]') || true}>
      <PopoverClose className="sr-only">달력 닫기</PopoverClose>
      {record.open && <>
        <Calendar mode="single" autoFocus selected={parseDate(day) || undefined} month={record.date.month}
          startMonth={parseDate(state.min) || undefined} endMonth={parseDate(state.max) || undefined}
          disabled={[...(state.min ? [{ before: parseDate(state.min) }] : []), ...(state.max ? [{ after: parseDate(state.max) }] : [])]}
          onMonthChange={month => { record.date.month = month; renderRecord(record); }} onSelect={choose} />
        {timed && <label className="grove-calendar-time">시간<input type="text" inputMode="numeric" aria-label="시간" placeholder="09:00" maxLength={5} value={record.date.time}
          onChange={event => { record.date.time = event.target.value; record.date.error = false; renderRecord(record); }} /></label>}
        {record.date.error && <p className="grove-calendar-error" role="alert">선택 가능한 날짜와 시간을 확인해 주세요.</p>}
        <div className="grove-calendar-footer">
          <button type="button" className="quiet small" onClick={() => save('')}>지우기</button>
          <div><button type="button" className="quiet small" disabled={!inRange(today)} onClick={() => { record.date.day = today; record.date.month = new Date(); record.date.error = false; if (timed) renderRecord(record); else save(today); }}>오늘</button>
            {timed && <button type="button" className="primary small" disabled={!day} onClick={() => save(day)}>적용</button>}</div>
        </div>
      </>}
    </PopoverContent>
  </Popover>;
}

function dispose(record) {
  if (active === record) active = null;
  record.observer.disconnect();
  record.input.removeEventListener('change', record.onChange);
  record.input.removeEventListener('invalid', record.onInvalid);
  record.root.unmount();
  record.input.hidden = record.original.hidden;
  record.input.tabIndex = record.original.tabIndex;
  if (record.wrapper.isConnected && record.input.parentElement === record.wrapper) {
    record.wrapper.before(record.input); record.wrapper.remove();
  }
  mounted.delete(record.input); records.delete(record);
}

function enhance(input, kind) {
  if (mounted.has(input) || input.closest('.grove-control-root,[data-nero-ui-owned]') || kind === 'select' && (input.multiple || input.size > 1)) return;
  const document = input.ownerDocument, hadFocus = document.activeElement === input;
  const wrapper = document.createElement('span'), host = document.createElement('span');
  wrapper.className = kind === 'select' ? 'grove-select' : 'grove-date'; host.className = 'grove-control-root';
  const record = { input, wrapper, host, kind, label: labelFor(input), open: false, original: { hidden: input.hidden, tabIndex: input.tabIndex }, id: ++sequence };
  input.before(wrapper); wrapper.append(host, input); input.hidden = true; input.tabIndex = -1;
  record.root = createRoot(host); record.state = snapshot(record);
  record.onChange = () => refreshRecord(record);
  record.onInvalid = event => { event.preventDefault(); record.adapterInvalid = true; input.setAttribute('aria-invalid', 'true'); refreshRecord(record); host.querySelector('button')?.focus({ preventScroll: true }); };
  input.addEventListener('change', record.onChange); input.addEventListener('invalid', record.onInvalid);
  record.observer = new MutationObserver(() => refreshRecord(record));
  record.observer.observe(input, { attributes: true, childList: true, subtree: true, characterData: true });
  mounted.set(input, record); records.add(record); refreshRecord(record);
  if (hadFocus) host.querySelector('button')?.focus({ preventScroll: true });
}

function enhanceAll(container = document) {
  for (const record of records) if (!record.input.isConnected || record.input.parentElement !== record.wrapper) dispose(record);
  container.querySelectorAll('select').forEach(input => enhance(input, 'select'));
  container.querySelectorAll('input[type="date"],input[type="datetime-local"]').forEach(input => enhance(input, 'date'));
}

function refresh() { for (const record of records) if (record.input.isConnected) refreshRecord(record); }
function close() { if (active) setOpen(active, false, true); }

export const GroveControls = { enhanceAll, refresh, close, isOpen: () => Boolean(active) };
if (typeof window !== 'undefined' && typeof document !== 'undefined') {
  window.GroveControls = GroveControls;
  document.addEventListener('keydown', event => {
    if (active && event.key === 'Escape' && !event.isComposing && event.keyCode !== 229) {
      event.preventDefault(); event.stopImmediatePropagation(); close();
    }
  }, true);
  let queued = false;
  const initialize = () => {
    enhanceAll();
    new MutationObserver(() => {
      if (queued) return;
      queued = true; queueMicrotask(() => { queued = false; enhanceAll(); });
    }).observe(document.body, { childList: true, subtree: true });
  };
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', initialize, { once: true }); else initialize();
}
