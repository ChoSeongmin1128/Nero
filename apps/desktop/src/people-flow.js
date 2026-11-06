import { createGoogleAttendeeDraft, removeDraftAttendee, stageGoogleAttendee, stageGoogleRow } from './google-attendee-draft.js';
const clone = value => JSON.parse(JSON.stringify(value));
export const attendeeLabel = (attendee, attendees) => attendee.name?.trim() || `참석자 ${Math.max(0, attendees.filter(item => !item.name?.trim()).findIndex(item => item.id === attendee.id)) + 1}`;

export function nameUnnamedAttendees(attendees) {
  const used = new Set(attendees.filter(item => !item.personId).map(item => item.name?.trim()).filter(Boolean));
  let number = 1;
  return attendees.map(item => {
    if (item.name?.trim()) return { ...item };
    while (used.has(`참석자 ${number}`)) number++;
    const name = `참석자 ${number++}`; used.add(name);
    return { ...item, name };
  });
}

export const splitPeopleNames = text => text.split(/[,\r\n]+/).map(name => name.trim()).filter(Boolean);
export const personGroups = (catalog, id) => catalog.groups.filter(group => group.personIds.includes(id));
export function filterPeople(catalog, query = '', groupId = '') {
  const needle = query.trim().toLocaleLowerCase();
  return catalog.people.filter(person => {
    const groups = personGroups(catalog, person.id);
    return (!groupId || (groupId === 'ungrouped' ? !groups.length : groups.some(group => group.id === groupId)))
      && (!needle || [person.name, ...groups.map(group => group.name)].some(name => name.toLocaleLowerCase().includes(needle)));
  });
}

export function addPeople(attendees, people, uuid = () => crypto.randomUUID()) {
  const selected = new Set(attendees.map(item => item.personId).filter(Boolean));
  return [...attendees, ...people.filter(person => !selected.has(person.id) && selected.add(person.id))
    .map(person => ({ id: uuid(), personId: person.id, name: person.name }))];
}

export function createAttendeeFlow({ bridge, onChanged = () => {}, uuid = () => crypto.randomUUID() }) {
  let state = { open: false, busy: false, loading: false, meeting: null, catalog: null, attendees: [], sources: [], identityLinks: [], personLinks: [], promotions: [], error: '', googleOpen: false, googleLoading: false, googleResult: null, googleError: '', googleConnecting: false, googlePhase: 'idle', googleAccess: 'unlinked' };
  let saved = '', expected = '', epoch = 0, meetingId = null, baseCatalog = null, googleEpoch = 0, attemptId = null, beginning = null, calendar = null;
  const listeners = new Set();
  const publish = changes => { state = { ...state, ...changes }; listeners.forEach(listener => listener()); };
  const draft = () => ({ attendees: state.attendees, sources: state.sources, personLinks: state.personLinks, promotions: state.promotions });
  const dirty = () => saved !== JSON.stringify(draft());
  const accept = value => {
    const next = createGoogleAttendeeDraft(value); next.attendees = nameUnnamedAttendees(next.attendees);
    saved = JSON.stringify(next); expected = value.editVersion;
    if (value.catalog) baseCatalog = clone(value.catalog);
    publish({ ...next, meeting: value.meeting, googleAccess: googleAccess(value.meeting), identityLinks: clone(value.identityLinks || []), ...(baseCatalog ? { catalog: clone(baseCatalog) } : {}) });
  };
  // Google attendees come from the event linked to this meeting, so without a linked event or a
  // connected account there is nothing to fetch and no retry that could help.
  const readCalendar = () => bridge.readCalendarSettings ? bridge.readCalendarSettings().catch(() => null) : Promise.resolve(null);
  const googleAccess = meeting => !meeting?.calendarReference ? 'unlinked' : calendar && !calendar.connected ? 'disconnected' : 'ready';
  const read = async () => {
    if (bridge.readGoogleAttendeeRoster) return bridge.readGoogleAttendeeRoster(meetingId);
    const [meeting, catalog] = await Promise.all([bridge.readMeetingAttendees(meetingId), bridge.readPeopleCatalog()]);
    return { ...meeting, catalog };
  };
  const operate = async task => {
    if (!state.open || state.busy || state.googleConnecting) return false;
    const generation = epoch;
    publish({ busy: true, error: '' });
    try { await task(); return generation === epoch; }
    catch (error) { if (generation === epoch) publish({ error: error?.message || '참석자를 저장하지 못했습니다.' }); return false; }
    finally { if (generation === epoch) publish({ busy: false }); }
  };
  const catalogFor = promotions => ({ ...baseCatalog, people: [...baseCatalog.people, ...promotions.map(promotion => promotion.person)] });
  const flow = {
    getSnapshot: () => state,
    subscribe(listener) { listeners.add(listener); return () => listeners.delete(listener); },
    isDirty: dirty,
    async open(id) {
      if (state.open && state.googleConnecting && !await flow.cancelMeetConnect()) return false;
      meetingId = id; const generation = ++epoch; googleEpoch++; baseCatalog = null; saved = JSON.stringify({ attendees: [], sources: [], personLinks: [], promotions: [] });
      publish({ open: true, busy: true, loading: true, meeting: null, catalog: null, attendees: [], sources: [], identityLinks: [], personLinks: [], promotions: [], error: '', googleOpen: false, googleLoading: false, googleResult: null, googleError: '', googleConnecting: false, googlePhase: 'idle', googleAccess: 'unlinked' });
      try { const [value, settings] = await Promise.all([read(), readCalendar()]); if (generation === epoch) { calendar = settings; accept(value); } }
      catch (error) { if (generation === epoch) publish({ error: error?.message || '참석자를 불러오지 못했습니다.' }); }
      finally { if (generation === epoch) publish({ busy: false, loading: false }); }
    },
    close() { if (state.googleConnecting || state.busy && !state.loading) return false; epoch++; googleEpoch++; publish({ open: false, busy: false, loading: false, googleLoading: false }); return true; },
    add(persons) { if (!state.busy) publish({ attendees: addPeople(state.attendees, persons, uuid) }); },
    addTemporary(name = '') { if (!state.busy) publish({ attendees: nameUnnamedAttendees([...state.attendees, { id: uuid(), ...(name.trim() ? { name: name.trim() } : {}) }]) }); },
    addInput(text) {
      if (state.busy || !state.catalog) return false;
      let attendees = state.attendees;
      for (const name of splitPeopleNames(text)) {
        if (name.length > 120) { publish({ error: '이름은 120자 이내로 입력해 주세요.' }); return false; }
        const action = attendeeInputAction(state.catalog, attendees, name);
        if (!action) { publish({ error: '이름이 겹치거나 이미 선택한 참석자입니다. 목록에서 선택해 주세요.' }); return false; }
        attendees = action.kind === 'people' ? addPeople(attendees, action.people, uuid) : [...attendees, { id: uuid(), name: action.name }];
      }
      publish({ attendees, error: '' }); return true;
    },
    rename(id, name) { if (!state.busy) publish({ attendees: state.attendees.map(item => item.id === id ? { ...item, name: name || undefined } : item) }); },
    remove(id) { if (!state.busy) { const next = removeDraftAttendee(draft(), id); publish({ ...next, catalog: catalogFor(next.promotions) }); } },
    save() {
      return operate(async () => {
        if (!dirty()) return;
        const next = clone(draft()); next.attendees = next.attendees.map(item => ({ ...item, name: item.name?.trim() || undefined }));
        let result;
        if (bridge.applyMeetingAttendeeDraft) result = await bridge.applyMeetingAttendeeDraft(state.meeting.id, expected, next);
        else {
          if (next.sources.length || next.personLinks.length || next.promotions.length) throw new Error('Nero 앱을 업데이트해 주세요.');
          result = await bridge.replaceMeetingAttendees(state.meeting.id, expected, next.attendees);
        }
        accept(result);
        await Promise.resolve(onChanged()).catch(() => {});
      });
    },
    reload() { return operate(async () => { const [value, settings] = await Promise.all([read(), readCalendar()]); calendar = settings; accept(value); publish({ googleResult: null, googleError: '' }); }); },
    async promote(id, name) {
      if (state.busy || !state.catalog) return false;
      const personName = name.trim(), attendee = state.attendees.find(item => item.id === id);
      if (!personName || personName.length > 120 || !attendee || attendee.personId) { publish({ error: '이름과 참석자를 확인해 주세요.' }); return false; }
      const person = { id: uuid(), name: personName };
      const promotions = [...state.promotions, { attendeeId: id, person }];
      const personLinks = state.sources.some(entry => entry.attendeeId === id) ? [...state.personLinks.filter(link => link.attendeeId !== id), { attendeeId: id, personId: person.id }] : state.personLinks;
      publish({ attendees: state.attendees.map(item => item.id === id ? { ...item, personId: person.id, name: personName } : item), promotions, personLinks, catalog: catalogFor(promotions), error: '' });
      return true;
    },
    async loadGoogle() {
      if (!state.open || state.busy || state.googleLoading || state.googleConnecting || state.googleAccess !== 'ready') return false;
      const generation = epoch, token = ++googleEpoch;
      publish({ googleOpen: true, googleLoading: true, googleError: '' });
      try {
        const result = await bridge.fetchGoogleAttendees(state.meeting.id, true);
        if (generation !== epoch || token !== googleEpoch) return false;
        const byKey = new Map(result.sources.map(source => [source.sourceKey, source]));
        publish({ googleResult: result, sources: state.sources.map(entry => byKey.has(entry.source.sourceKey) ? { ...entry, source: clone(byKey.get(entry.source.sourceKey)) } : entry) }); return true;
      } catch (error) {
        if (generation !== epoch || token !== googleEpoch) return false;
        // A failure that ended the connection is not retried here; the dialog points to reconnecting instead.
        const settings = await readCalendar();
        if (generation !== epoch || token !== googleEpoch) return false;
        if (settings) calendar = settings;
        const access = googleAccess(state.meeting);
        publish(access === 'ready' ? { googleError: error?.message || 'Google 참석자를 불러오지 못했습니다.' } : { googleAccess: access, googleOpen: false, googleResult: null, googleError: '' });
        return false;
      }
      finally { if (generation === epoch && token === googleEpoch) publish({ googleLoading: false }); }
    },
    chooseGoogle(source, choice) {
      if (state.busy || state.googleConnecting) return false;
      try { const next = stageGoogleAttendee(draft(), state.catalog, source, choice, uuid); publish({ ...next, catalog: catalogFor(next.promotions), googleError: '' }); return true; }
      catch (error) { publish({ googleError: error.message }); return false; }
    },
    chooseGoogleRows(entries) {
      if (state.busy || state.googleConnecting || !entries.length) return false;
      try {
        let next = draft();
        for (const { row, choice, name } of entries) next = stageGoogleRow(next, state.catalog, row, choice, name, uuid);
        publish({ ...next, catalog: catalogFor(next.promotions), googleError: '' }); return true;
      } catch (error) { publish({ googleError: error.message }); return false; }
    },
    hideGoogle() { if (!state.googleConnecting) publish({ googleOpen: false }); },
    async connectMeet() {
      if (!state.open || state.busy || state.googleConnecting) return false;
      const generation = epoch, token = ++googleEpoch;
      publish({ googleConnecting: true, googlePhase: 'opening', googleError: '' });
      try {
        beginning = bridge.beginMeetConnect(); const attempt = await beginning; beginning = null;
        if (generation !== epoch || token !== googleEpoch) return false;
        attemptId = attempt.attemptId; publish({ googlePhase: 'authorizing' });
        await bridge.finishCalendarConnect(attemptId);
        if (generation !== epoch || token !== googleEpoch) return false;
        attemptId = null; publish({ googleConnecting: false, googlePhase: 'idle' });
        return flow.loadGoogle();
      } catch (error) {
        if (generation === epoch && token === googleEpoch) { beginning = null; attemptId = null; publish({ googleConnecting: false, googlePhase: 'idle', googleError: error?.message || 'Meet 연결을 완료하지 못했습니다.' }); }
        return false;
      }
    },
    async cancelMeetConnect() {
      if (!state.googleConnecting) return true;
      if (state.googlePhase === 'cancelling') return false;
      const generation = epoch; googleEpoch++; publish({ googlePhase: 'cancelling', googleError: '' });
      try {
        let id = attemptId;
        if (!id && beginning) { try { id = (await beginning).attemptId; } catch {} }
        attemptId = id;
        if (id) await bridge.cancelCalendarConnect(id);
        attemptId = null; beginning = null;
        if (generation === epoch) publish({ googleConnecting: false, googlePhase: 'idle' }); return true;
      } catch (error) { if (generation === epoch) publish({ googlePhase: 'cancelFailed', googleError: error?.message || 'Meet 연결 취소를 다시 시도해 주세요.' }); return false; }
    },
    async reopenMeetConnect() {
      if (!attemptId || state.googlePhase !== 'authorizing') return false;
      try { await bridge.reopenCalendarConnect(attemptId); return true; }
      catch (error) { publish({ googleError: error?.message || '브라우저를 다시 열지 못했습니다.' }); return false; }
    },
    dispose() { void flow.cancelMeetConnect(); epoch++; googleEpoch++; listeners.clear(); state = { ...state, open: false }; },
  };
  return flow;
}

export function createPeopleCatalogFlow({ bridge, uuid = () => crypto.randomUUID() }) {
  let state = { catalog: null, busy: false, writing: false, draft: null, quickInput: '', error: '' }, active = false, epoch = 0;
  let originalDraft = null, refreshAfterBusy = false;
  const listeners = new Set();
  const publish = changes => { state = { ...state, ...changes }; listeners.forEach(listener => listener()); };
  const run = async (task, apply, writing = false) => {
    if (!active || state.busy) return false;
    const generation = epoch; publish({ busy: true, writing, error: '' });
    try {
      const result = await task();
      if (generation !== epoch || !active) return false;
      publish(apply(result)); return true;
    } catch (error) { if (generation === epoch) publish({ error: error?.message || '인원 목록을 저장하지 못했습니다.' }); return false; }
    finally { if (generation === epoch) {
      publish({ busy: false, writing: false });
      if (refreshAfterBusy) { refreshAfterBusy = false; if (active && !state.draft && !state.quickInput.trim()) void flow.reload(); }
    } }
  };
  const commit = (people, groups, clearQuickInput = false) => run(() => bridge.replacePeopleCatalog(state.catalog.editVersion, people, groups), catalog => ({ catalog, draft: null, ...(clearQuickInput ? { quickInput: '' } : {}) }), true);
  const flow = {
    getSnapshot: () => state,
    subscribe(listener) { listeners.add(listener); return () => listeners.delete(listener); },
    isDirty: () => Boolean(state.quickInput.trim()) || Boolean(state.draft) && JSON.stringify(state.draft) !== originalDraft,
    setActive(value) { if (active === value) return; epoch += 1; active = value; refreshAfterBusy = false; publish({ busy: false, writing: false }); if (value && !state.draft && !state.quickInput.trim()) void flow.reload(); },
    reload() { return run(() => bridge.readPeopleCatalog(), catalog => ({ catalog, draft: null, quickInput: '' })); },
    refreshIfClean() { if (!active) return; if (state.busy) { refreshAfterBusy = true; return; } if (!state.draft && !state.quickInput.trim()) return flow.reload(); },
    edit(kind, item = { id: uuid(), name: '', personIds: [] }) { if (state.catalog && !state.busy) { const draft = { kind, ...clone(item) }; originalDraft = JSON.stringify(draft); publish({ draft, error: '' }); } },
    change(changes) { if (state.draft && !state.busy) publish({ draft: { ...state.draft, ...changes } }); },
    discard() { publish({ draft: null, quickInput: '' }); },
    changeQuickInput(quickInput) { if (!state.busy) publish({ quickInput }); },
    addMany(kind, groupId = '') {
      if (!state.catalog || state.busy || state.draft) return Promise.resolve(false);
      const names = splitPeopleNames(state.quickInput);
      if (!names.length || names.some(name => name.length > 120)) { publish({ error: '이름을 1자 이상 120자 이내로 입력해 주세요.' }); return Promise.resolve(false); }
      if (groupId && !state.catalog.groups.some(group => group.id === groupId)) { publish({ error: '선택한 그룹을 다시 확인해 주세요.' }); return Promise.resolve(false); }
      if (kind === 'group') return commit(state.catalog.people, [...state.catalog.groups, ...names.map(name => ({ id: uuid(), name, personIds: [] }))], true);
      const people = names.map(name => ({ id: uuid(), name }));
      const groups = state.catalog.groups.map(group => group.id === groupId ? { ...group, personIds: [...group.personIds, ...people.map(person => person.id)] } : group);
      return commit([...state.catalog.people, ...people], groups, true);
    },
    save() {
      const item = state.draft;
      if (!item || !item.name.trim() || !state.catalog) return Promise.resolve(false);
      const key = item.kind === 'person' ? 'people' : 'groups';
      const value = key === 'people' ? { id: item.id, name: item.name.trim() } : { id: item.id, name: item.name.trim(), personIds: item.personIds };
      const next = state.catalog[key].some(entry => entry.id === item.id)
        ? state.catalog[key].map(entry => entry.id === item.id ? value : entry) : [...state.catalog[key], value];
      return commit(key === 'people' ? next : state.catalog.people, key === 'groups' ? next : state.catalog.groups);
    },
    remove(kind, id) {
      if (!state.catalog) return Promise.resolve(false);
      const people = state.catalog.people.filter(item => kind !== 'person' || item.id !== id);
      const groups = state.catalog.groups.filter(item => kind !== 'group' || item.id !== id)
        .map(group => kind === 'person' ? { ...group, personIds: group.personIds.filter(value => value !== id) } : group);
      return commit(people, groups);
    },
    dispose() { active = false; epoch += 1; listeners.clear(); },
  };
  return flow;
}

export function attendeeInputAction(catalog, attendees, query) {
  const name = query.trim();
  if (!name || !catalog) return null;
  const matches = item => item.name.trim().toLocaleLowerCase() === name.toLocaleLowerCase();
  const people = catalog.people.filter(matches), groups = catalog.groups.filter(matches);
  if (people.length + groups.length > 1) return null;
  if (people.length) return { kind: 'people', people };
  if (groups.length) return { kind: 'people', people: catalog.people.filter(person => groups[0].personIds.includes(person.id)) };
  if (attendees.some(item => item.name && matches(item))) return null;
  return { kind: 'create', name };
}
