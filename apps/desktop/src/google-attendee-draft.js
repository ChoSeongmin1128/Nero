const clone = value => JSON.parse(JSON.stringify(value));
const sameIdentity = (a, b) => Boolean(a.email && a.email === b.email || a.googleUserId && a.googleUserId === b.googleUserId);
export const googleSourceLabel = source => source.displayName || source.email || ({ anonymous: '익명 참석자', phone: '전화 참석자', calendarResource: '회의실' }[source.kind]) || 'Google 참석자';
export const googleSourceDetail = source => source.email || (source.googleUserId ? `Google 계정 ${source.googleUserId.replace(/^users\//, '')}` : ({ calendarResource: '회의실', signedInPersonOrRoom: 'Google 계정', anonymous: '익명', phone: '전화' }[source.kind])) || '';
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
