export function snapshotSignatures(snapshot) {
  const session = snapshot.session ?? null;
  return {
    contents: JSON.stringify([snapshot.meetings, snapshot.folders || [], session ? { ...session, elapsedMs: 0, savedFrames: 0 } : null]),
    session: JSON.stringify(session),
  };
}

export function compareSnapshotSignatures(previous, next) {
  const timerOnly = previous?.contents === next.contents;
  return { changed: !timerOnly || previous.session !== next.session, timerOnly };
}
