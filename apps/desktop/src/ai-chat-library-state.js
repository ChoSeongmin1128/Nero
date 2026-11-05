const DAY = 24 * 60 * 60 * 1000;

export function meetingLabels(item, meetings) {
  return (item.meetingIds || []).map(id => meetings.find(meeting => meeting.id === id)).map(meeting => meeting ? (meeting.deletedAt ? `${meeting.title || '제목 없는 회의'} (휴지통)` : meeting.title || '제목 없는 회의') : '삭제된 회의');
}

// Answers still streaming stay out of every cleanup.
export function olderThan(items, days, now = Date.now()) {
  return items.filter(item => !item.activeRequestId && (item.updatedAt || item.createdAt || 0) < now - days * DAY);
}
