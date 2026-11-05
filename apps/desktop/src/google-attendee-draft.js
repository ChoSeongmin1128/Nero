const clone = value => JSON.parse(JSON.stringify(value));
const normalName = value => String(value || '').trim().replace(/\s+/g, ' ').toLocaleLowerCase();
const sameIdentity = (a, b) => Boolean(a.email && a.email === b.email || a.googleUserId && a.googleUserId === b.googleUserId);
export const googleSourceLabel = source => source.displayName || source.email || ({ anonymous: '익명 참석자', phone: '전화 참석자', calendarResource: '회의실' }[source.kind]) || 'Google 참석자';
export const googleSourceStatus = source => ({ accepted: '수락', declined: '거절', tentative: '미정', needsAction: '응답 대기' }[source.responseStatus]) || '';

export function createGoogleAttendeeDraft(roster) {
  return { attendees: clone(roster.meeting.attendees || []), sources: clone(roster.sources || []), personLinks: [], promotions: [] };
}

export function googleTargetFor(draft, catalog, identityLinks, source) {
  const existing = draft.sources.find(entry => entry.source.sourceKey === source.sourceKey || sameIdentity(entry.source, source));
  if (existing) return draft.attendees.some(attendee => attendee.id === existing.attendeeId) ? `attendee:${existing.attendeeId}` : 'new';
  const matches = identityLinks.filter(link => source.email && link.emails.includes(source.email) || source.googleUserId && link.googleUserIds.includes(source.googleUserId));
  if (matches.length === 1 && catalog.people.some(person => person.id === matches[0].personId)) {
    const selected = draft.attendees.find(attendee => attendee.personId === matches[0].personId);
    return selected ? `attendee:${selected.id}` : `person:${matches[0].personId}`;
  }
  return 'new';
}

export function stageGoogleAttendee(draft, catalog, source, choice, uuid) {
  const next = clone(draft);
  let target;
  if (choice.startsWith('attendee:')) {
    target = next.attendees.find(attendee => attendee.id === choice.slice(9));
    if (!target) throw new Error('연결할 참석자를 다시 선택해 주세요.');
  } else if (choice.startsWith('person:')) {
    const person = catalog.people.find(person => person.id === choice.slice(7));
    if (!person) throw new Error('저장된 인원을 다시 선택해 주세요.');
    target = next.attendees.find(attendee => attendee.personId === person.id);
    if (!target) {
      const record = next.sources.find(entry => entry.source.sourceKey === source.sourceKey || sameIdentity(entry.source, source));
      const current = record && next.attendees.find(attendee => attendee.id === record.attendeeId);
      if (current && (!current.personId || next.personLinks.some(link => link.attendeeId === current.id && link.personId === current.personId))) {
        target = current; target.personId = person.id; target.name ||= person.name;
      }
    }
    if (!target) { target = { id: uuid(), personId: person.id, name: person.name }; next.attendees.push(target); }
  } else if (choice === 'new') {
    const label = googleSourceLabel(source);
    target = { id: uuid(), name: Array.from(label).length <= 120 ? label : 'Google 참석자' }; next.attendees.push(target);
  } else throw new Error('연결할 참석자를 선택해 주세요.');
  const merged = new Set(next.sources.filter(entry => entry.source.sourceKey === source.sourceKey || sameIdentity(entry.source, source)).map(entry => entry.attendeeId).filter(id => id !== target.id));
  if (next.attendees.some(attendee => merged.has(attendee.id) && attendee.personId && attendee.personId !== target.personId && !next.personLinks.some(link => link.attendeeId === attendee.id && link.personId === attendee.personId))) throw new Error('이미 다른 저장 인원에 연결된 계정입니다.');
  next.attendees = next.attendees.filter(attendee => !merged.has(attendee.id));
  next.sources.forEach(entry => { if (merged.has(entry.attendeeId)) entry.attendeeId = target.id; });
  const record = next.sources.find(entry => entry.source.sourceKey === source.sourceKey);
  if (record) { record.attendeeId = target.id; record.source = clone(source); }
  else next.sources.push({ attendeeId: target.id, source: clone(source) });
  next.personLinks = next.personLinks.filter(link => !merged.has(link.attendeeId) && link.attendeeId !== target.id);
  if (target.personId) next.personLinks.push({ attendeeId: target.id, personId: target.personId });
  next.promotions = next.promotions.filter(promotion => !merged.has(promotion.attendeeId) && next.attendees.some(attendee => attendee.id === promotion.attendeeId && attendee.personId === promotion.person.id));
  return next;
}

export function removeDraftAttendee(draft, attendeeId) {
  return { ...draft, attendees: draft.attendees.filter(attendee => attendee.id !== attendeeId), personLinks: draft.personLinks.filter(link => link.attendeeId !== attendeeId), promotions: draft.promotions.filter(promotion => promotion.attendeeId !== attendeeId) };
}

// One row per person: an invitation, the Meet participant matched to it, or a Meet
// participant nobody invited. Meet participants carry no email, so a match needs the same
// saved-person link or a name that is unique on both sides.
export function googleRows(result, draft, catalog, identityLinks = []) {
  const sources = result?.sources || [];
  const linkedPerson = source => {
    const people = new Set(identityLinks.filter(link => source.email && link.emails.includes(source.email) || source.googleUserId && link.googleUserIds.includes(source.googleUserId)).map(link => link.personId));
    return people.size === 1 ? [...people][0] : null;
  };
  const rows = sources.filter(source => source.origin === 'calendarInvitation').map(invitation => ({ key: invitation.sourceKey, invitation, meet: null }));
  const meets = sources.filter(source => source.origin === 'meetParticipant');
  // Saved-person links are matched for every participant before any name is considered,
  // so a stranger with the same name cannot take the row of the linked person.
  const pending = new Set(meets);
  for (const meet of meets) {
    const person = linkedPerson(meet);
    const byPerson = person ? rows.filter(row => !row.meet && linkedPerson(row.invitation) === person) : [];
    if (byPerson.length === 1) { byPerson[0].meet = meet; pending.delete(meet); }
  }
  for (const meet of meets.filter(item => pending.has(item))) {
    const name = normalName(meet.displayName);
    const byName = name && meets.filter(other => normalName(other.displayName) === name).length === 1 ? rows.filter(row => !row.meet && normalName(row.invitation.displayName) === name && !linkedPerson(row.invitation)) : [];
    if (byName.length === 1) { byName[0].meet = meet; pending.delete(meet); }
  }
  const extra = meets.filter(item => pending.has(item)).map(meet => ({ key: meet.sourceKey, invitation: null, meet }));
  const all = [...rows, ...extra];
  const labelOf = row => googleSourceLabel({ ...(row.invitation || row.meet), displayName: row.invitation?.displayName || row.meet?.displayName });
  const nameCount = new Map();
  for (const row of all) { const name = normalName(labelOf(row)); nameCount.set(name, (nameCount.get(name) || 0) + 1); }
  const fromGoogle = new Set(draft.sources.map(entry => entry.attendeeId));
  return all.map(row => {
    const parts = [row.invitation, row.meet].filter(Boolean), primary = parts[0], label = labelOf(row), name = normalName(label);
    const records = parts.map(source => draft.sources.find(entry => entry.source.sourceKey === source.sourceKey));
    const addedTo = records.every(Boolean) && new Set(records.map(record => record.attendeeId)).size === 1 && draft.attendees.some(attendee => attendee.id === records[0].attendeeId) ? records[0].attendeeId : null;
    let suggestion = 'new', reason = '', homonyms = 0;
    for (const source of parts) {
      const target = googleTargetFor(draft, catalog, identityLinks, source);
      if (target !== 'new') { suggestion = target; reason = 'linked'; break; }
    }
    const sharedName = nameCount.get(name) > 1;
    if (suggestion === 'new' && name && !sharedName && primary.kind !== 'calendarResource') {
      const people = catalog.people.filter(person => normalName(person.name) === name);
      const temporary = draft.attendees.filter(attendee => !attendee.personId && !fromGoogle.has(attendee.id) && normalName(attendee.name) === name);
      const selected = people.length === 1 && draft.attendees.find(attendee => attendee.personId === people[0].id);
      if (people.length === 1 && !(selected && fromGoogle.has(selected.id))) {
        suggestion = selected ? `attendee:${selected.id}` : `person:${people[0].id}`; reason = 'name';
      } else if (!people.length && temporary.length === 1) { suggestion = `attendee:${temporary[0].id}`; reason = 'name'; }
      homonyms = people.length > 1 ? people.length : 0;
    }
    if (addedTo) suggestion = `attendee:${addedTo}`;
    const status = [row.invitation && googleSourceStatus(row.invitation), row.meet && 'Meet 참석', primary.kind === 'calendarResource' && '회의실', !row.invitation && { anonymous: '익명', phone: '전화' }[row.meet.kind]].filter(Boolean).join(' / ');
    const checked = !addedTo && primary.kind !== 'calendarResource' && (Boolean(row.meet) || row.invitation?.responseStatus === 'accepted');
    return { key: row.key, sources: parts, label, status, detail: row.invitation?.email || '', suggestion, reason, homonyms, sharedName, addedTo, checked };
  });
}

// Saved people with the same name stay distinguishable in every choice list.
export function personChoiceLabel(person, catalog, identityLinks = []) {
  const same = catalog.people.filter(other => normalName(other.name) === normalName(person.name));
  if (same.length < 2) return person.name;
  const describe = item => [(catalog.groups || []).filter(group => group.personIds.includes(item.id)).map(group => group.name).join(', '), identityLinks.find(link => link.personId === item.id)?.emails?.[0]].filter(Boolean).join(' / ');
  const detail = describe(person);
  const clash = !detail || same.filter(other => describe(other) === detail).length > 1;
  return `${person.name} (${[detail, clash && `${same.indexOf(person) + 1}번째`].filter(Boolean).join(' / ')})`;
}

export function stageGoogleRow(draft, catalog, row, choice, name, uuid) {
  let next = draft, target = choice;
  for (const source of row.sources) {
    next = stageGoogleAttendee(next, catalog, source, target, uuid);
    target = `attendee:${next.sources.find(entry => entry.source.sourceKey === source.sourceKey).attendeeId}`;
  }
  const typed = Array.from(String(name ?? '').trim()).slice(0, 120).join('');
  if (choice !== 'new' || !typed) return next;
  return { ...next, attendees: next.attendees.map(attendee => attendee.id === target.slice(9) ? { ...attendee, name: typed } : attendee) };
}
