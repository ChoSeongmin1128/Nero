import { calendarDay } from './calendar-meeting-flow.js';

const dayTitle = new Intl.DateTimeFormat('ko-KR', { month: 'long', day: 'numeric', weekday: 'long' });

// Today's events in start order, each marked as running now or already over.
export function homeEvents(occurrences = [], now = Date.now()) {
  return [...occurrences]
    .filter(event => Number.isFinite(event.startAt) && Number.isFinite(event.endAt))
    .sort((a, b) => a.startAt - b.startAt || a.key.localeCompare(b.key))
    .map(event => ({ ...event, current: event.startAt <= now && now < event.endAt, over: event.endAt <= now }));
}

// The screen a person opens Nero to: record the meeting happening now, or start one.
export function mountHome({ container, bridge, icon, escape, onRecord, onCreate, onImport, onRecordEvent, onConnectCalendar, onFolder, device = () => '', folders = () => [], folder = () => null, disabled = () => false, now = () => new Date(), timeZone = () => Intl.DateTimeFormat().resolvedOptions().timeZone }) {
  let schedule = { loading: false, connected: false, events: [], scopeRevision: null, day: '', timeZone: '', error: '' };
  let request = 0, renderKey = '', retry = null, restoringTries = 0;
  const clock = zone => new Intl.DateTimeFormat('ko-KR', { hour: '2-digit', minute: '2-digit', hourCycle: 'h23', timeZone: zone });

  async function refresh() {
    const current = ++request;
    const day = calendarDay(now()), zone = timeZone();
    schedule = { ...schedule, loading: true, day, timeZone: zone, error: '' };
    render();
    try {
      const result = await bridge.readCalendarMeetingOccurrences({ day, timeZone: zone });
      if (current !== request) return;
      const restoring = Boolean(result?.calendar?.restoring);
      schedule = { loading: false, restoring, connected: Boolean(result?.calendar?.connected), events: result?.occurrences || [], scopeRevision: result?.scopeRevision ?? null, day, timeZone: zone, error: '' };
      // Right after launch the saved Google connection is still being checked. When that
      // outlasts the retries the note says so instead of spinning; a later calendar change reloads.
      clearTimeout(retry);
      if (restoring && restoringTries < 20) { restoringTries += 1; retry = setTimeout(() => { void refresh(); }, 1500); }
      else if (restoring) schedule = { ...schedule, restoring: false, error: 'Google Calendar 연결을 아직 확인하지 못했습니다. 연결되면 오늘 일정이 표시됩니다.' };
      else restoringTries = 0;
    } catch (error) {
      if (current !== request) return;
      schedule = { ...schedule, loading: false, events: [], error: error?.message || '오늘 일정을 불러오지 못했습니다.' };
    }
    render();
  }

  function render() {
    if (container.hidden) return;
    const at = now().getTime();
    const events = homeEvents(schedule.events, at);
    const running = events.some(event => event.current);
    const off = disabled();
    const choices = folders(), chosen = choices.some(item => item.id === folder()) ? folder() : '';
    const key = JSON.stringify([dayTitle.format(now()), Math.floor(at / 60000), schedule, off, device(), choices.map(item => [item.id, item.name]), chosen]);
    if (key === renderKey) return;
    renderKey = key;
    const format = clock(schedule.timeZone || timeZone());
    const button = (attrs, iconName, label, primary = false) => `<button type="button" class="small with-icon${primary ? ' primary' : ''}" ${attrs}${off ? ' disabled' : ''}>${icon(iconName)}<span>${escape(label)}</span></button>`;
    const rows = events.map(event => `<li class="home-event"${event.current ? ' data-current="true"' : ''}${event.over ? ' data-over="true"' : ''}><time>${escape(format.format(event.startAt))} - ${escape(format.format(event.endAt))}</time><strong title="${escape(event.title)}">${escape(event.title || '제목 없는 일정')}</strong>${event.current ? '<span class="status-chip">진행 중</span>' : ''}${button(`data-home-event="${escape(event.key)}"`, 'mic', '녹음', event.current)}</li>`).join('');
    // A saved connection still being checked at launch reads as loading, not as disconnected.
    const scheduleBlock = schedule.connected || schedule.restoring
      ? `<section class="home-schedule" aria-labelledby="home-schedule-title"><h2 id="home-schedule-title">오늘 일정</h2>${rows ? `<ul>${rows}</ul>` : schedule.loading || schedule.restoring ? '<p class="home-note"><span class="processing-inline" role="status"><span class="activity-spinner" aria-hidden="true"></span>일정을 불러오는 중입니다.</span></p>' : '<p class="home-note">오늘 일정이 없습니다.</p>'}</section>`
      : schedule.error ? `<p class="home-note">${escape(schedule.error)}</p>` : '';
    const hint = !schedule.connected && !schedule.restoring && !schedule.loading && !schedule.error ? `<p class="home-note"><button type="button" class="quiet small" data-home-action="calendar">Google Calendar 연결</button><span>하면 오늘 일정에서 바로 녹음할 수 있습니다.</span></p>` : '';
    const options = `<option value="">미분류</option>${choices.map(item => `<option value="${escape(item.id)}"${item.id === chosen ? ' selected' : ''}>${escape(item.name)}</option>`).join('')}`;
    const where = `<div class="home-context"><label class="home-folder">${icon('folder')}<span>저장 위치</span><select data-home-folder aria-label="새 회의 저장 위치"${off ? ' disabled' : ''}>${options}</select></label>${device() ? `<span class="home-device">${icon('mic')}<span>${escape(device())}</span></span>` : ''}</div>`;
    container.innerHTML = `<div class="home-content"><h1 class="home-date" tabindex="-1">${escape(dayTitle.format(now()))}</h1>${scheduleBlock}<div class="home-actions">${rows ? '<span class="home-actions-label">일정 없이</span>' : ''}${button('data-home-action="record"', 'mic', '녹음 시작', !running)}${button('data-home-action="create"', 'square-pen', '빈 회의')}${button('data-home-action="import"', 'upload', '파일 가져오기')}</div>${where}${hint}</div>`;
    const select = container.querySelector('[data-home-folder]');
    if (select) select.value = chosen;
    globalThis.NeroControls?.enhanceAll?.(container);
  }

  container.addEventListener('change', event => {
    const select = event.target.closest?.('[data-home-folder]');
    if (select && !select.disabled) void onFolder?.(select.value || null);
  });
  container.addEventListener('click', event => {
    const target = event.target.closest?.('button');
    if (!target || target.disabled || !container.contains(target)) return;
    if (target.dataset.homeEvent) {
      const key = target.dataset.homeEvent;
      void onRecordEvent?.(key, { day: schedule.day, timeZone: schedule.timeZone, scopeRevision: schedule.scopeRevision });
      return;
    }
    const action = target.dataset.homeAction;
    if (action === 'record') void onRecord?.();
    if (action === 'create') void onCreate?.();
    if (action === 'import') void onImport?.();
    if (action === 'calendar') void onConnectCalendar?.();
  });
  // Today's schedule is read when home first shows on a given day.
  function ensure() { if (schedule.day !== calendarDay(now()) && !schedule.loading) void refresh(); }
  return { refresh, ensure, render, focus: () => container.querySelector('.home-date')?.focus({ preventScroll: true }) };
}
