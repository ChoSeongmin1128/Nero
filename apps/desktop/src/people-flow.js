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
  let state = { open: false, busy: false, loading: false, meeting: null, catalog: null, attendees: [], error: '' };
  let saved = [], expected = '', epoch = 0, meetingId = null;
  const listeners = new Set();
  const publish = changes => { state = { ...state, ...changes }; listeners.forEach(listener => listener()); };
  const dirty = () => JSON.stringify(saved) !== JSON.stringify(state.attendees);
  const accept = value => {
    saved = nameUnnamedAttendees(clone(value.meeting.attendees || [])); expected = value.editVersion;
    publish({ meeting: value.meeting, attendees: clone(saved) });
  };
  const operate = async task => {
    if (!state.open || state.busy) return false;
    const generation = epoch;
    publish({ busy: true, error: '' });
    try { await task(); return generation === epoch; }
    catch (error) { if (generation === epoch) publish({ error: error?.message || '참석자를 저장하지 못했습니다.' }); return false; }
    finally { if (generation === epoch) publish({ busy: false }); }
  };
  const persist = async () => {
    if (!dirty()) return;
    accept(await bridge.replaceMeetingAttendees(state.meeting.id, expected, state.attendees.map(item => ({ ...item, name: item.name?.trim() || undefined }))));
    await onChanged();
  };
  return {
    getSnapshot: () => state,
    subscribe(listener) { listeners.add(listener); return () => listeners.delete(listener); },
    isDirty: dirty,
    async open(id) {
      meetingId = id;
      const generation = ++epoch;
      publish({ open: true, busy: true, loading: true, meeting: null, catalog: null, attendees: [], error: '' }); saved = [];
      try {
        const [meeting, catalog] = await Promise.all([bridge.readMeetingAttendees(id), bridge.readPeopleCatalog()]);
        if (generation !== epoch) return;
        accept(meeting); publish({ catalog });
      } catch (error) { if (generation === epoch) publish({ error: error?.message || '참석자를 불러오지 못했습니다.' }); }
      finally { if (generation === epoch) publish({ busy: false, loading: false }); }
    },
    close() { if (state.busy && !state.loading) return false; epoch += 1; publish({ open: false, busy: false, loading: false }); return true; },
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
    remove(id) { if (!state.busy) publish({ attendees: state.attendees.filter(item => item.id !== id) }); },
    save() { return operate(persist); },
    reload() { return operate(async () => { const [meeting, catalog] = await Promise.all([bridge.readMeetingAttendees(meetingId), bridge.readPeopleCatalog()]); accept(meeting); publish({ catalog }); }); },
    promote(id, name) {
      return operate(async () => {
        const personName = name.trim();
        if (!personName) throw new Error('이름을 입력해 주세요.');
        await persist();
        const result = await bridge.promoteMeetingAttendee(state.meeting.id, expected, state.catalog.editVersion, id, personName);
        accept(result); publish({ catalog: result.catalog }); await onChanged();
      });
    },
    dispose() { epoch += 1; listeners.clear(); state = { ...state, open: false }; },
  };
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
