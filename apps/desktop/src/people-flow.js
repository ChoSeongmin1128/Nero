const clone = value => JSON.parse(JSON.stringify(value));
export const attendeeLabel = (attendee, index) => attendee.name?.trim() || `참석자 ${index + 1}`;

export function addPeople(attendees, people, uuid = () => crypto.randomUUID()) {
  const selected = new Set(attendees.map(item => item.personId).filter(Boolean));
  return [...attendees, ...people.filter(person => !selected.has(person.id) && selected.add(person.id))
    .map(person => ({ id: uuid(), personId: person.id, name: person.name }))];
}

export function createAttendeeFlow({ bridge, onChanged = () => {}, uuid = () => crypto.randomUUID() }) {
  let state = { open: false, busy: false, meeting: null, catalog: null, attendees: [], error: '' };
  let saved = [], expected = '', epoch = 0;
  const listeners = new Set();
  const publish = changes => { state = { ...state, ...changes }; listeners.forEach(listener => listener()); };
  const dirty = () => JSON.stringify(saved) !== JSON.stringify(state.attendees);
  const accept = value => {
    saved = clone(value.meeting.attendees || []); expected = value.editVersion;
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
      const generation = ++epoch;
      publish({ open: true, busy: true, meeting: null, catalog: null, attendees: [], error: '' }); saved = [];
      try {
        const [meeting, catalog] = await Promise.all([bridge.readMeetingAttendees(id), bridge.readPeopleCatalog()]);
        if (generation !== epoch) return;
        accept(meeting); publish({ catalog });
      } catch (error) { if (generation === epoch) publish({ error: error?.message || '참석자를 불러오지 못했습니다.' }); }
      finally { if (generation === epoch) publish({ busy: false }); }
    },
    close() { if (state.busy) return false; epoch += 1; publish({ open: false }); return true; },
    add(persons) { if (!state.busy) publish({ attendees: addPeople(state.attendees, persons, uuid) }); },
    addTemporary() { if (!state.busy) publish({ attendees: [...state.attendees, { id: uuid() }] }); },
    rename(id, name) { if (!state.busy) publish({ attendees: state.attendees.map(item => item.id === id ? { ...item, name: name || undefined } : item) }); },
    remove(id) { if (!state.busy) publish({ attendees: state.attendees.filter(item => item.id !== id) }); },
    save() { return operate(persist); },
    reload() { return operate(async () => { const [meeting, catalog] = await Promise.all([bridge.readMeetingAttendees(state.meeting.id), bridge.readPeopleCatalog()]); accept(meeting); publish({ catalog }); }); },
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
  let state = { catalog: null, busy: false, draft: null, error: '' }, active = false, epoch = 0;
  let originalDraft = null;
  const listeners = new Set();
  const publish = changes => { state = { ...state, ...changes }; listeners.forEach(listener => listener()); };
  const run = async (task, apply) => {
    if (!active || state.busy) return false;
    const generation = epoch; publish({ busy: true, error: '' });
    try {
      const result = await task();
      if (generation !== epoch || !active) return false;
      publish(apply(result)); return true;
    } catch (error) { if (generation === epoch) publish({ error: error?.message || '인원 목록을 저장하지 못했습니다.' }); return false; }
    finally { if (generation === epoch) publish({ busy: false }); }
  };
  const commit = (people, groups) => run(() => bridge.replacePeopleCatalog(state.catalog.editVersion, people, groups), catalog => ({ catalog, draft: null }));
  const flow = {
    getSnapshot: () => state,
    subscribe(listener) { listeners.add(listener); return () => listeners.delete(listener); },
    isDirty: () => Boolean(state.draft) && JSON.stringify(state.draft) !== originalDraft,
    setActive(value) { if (active === value) return; epoch += 1; active = value; publish({ busy: false }); if (value && !state.draft) void flow.reload(); },
    reload() { return run(() => bridge.readPeopleCatalog(), catalog => ({ catalog, draft: null })); },
    refreshIfClean() { if (active && !state.busy && !state.draft) return flow.reload(); },
    edit(kind, item = { id: uuid(), name: '', personIds: [] }) { if (state.catalog && !state.busy) { const draft = { kind, ...clone(item) }; originalDraft = JSON.stringify(draft); publish({ draft, error: '' }); } },
    change(changes) { if (state.draft && !state.busy) publish({ draft: { ...state.draft, ...changes } }); },
    discard() { publish({ draft: null }); },
    save() {
      const item = state.draft;
      if (!item || !item.name.trim() || !state.catalog) return Promise.resolve(false);
      const key = item.kind === 'person' ? 'people' : 'groups';
      const value = key === 'people' ? { id: item.id, name: item.name.trim() } : { id: item.id, name: item.name.trim(), personIds: item.personIds };
      const next = state.catalog[key].filter(entry => entry.id !== item.id).concat(value);
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
