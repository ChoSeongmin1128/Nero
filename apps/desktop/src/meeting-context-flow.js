export function createMeetingContextFlow({ bridge, getMeeting, onChanged = () => {} }) {
  let state = { open: false, meetingId: null, loading: false, busy: false, phase: null, revision: 0, sources: [], notion: null, error: '', link: '' };
  let epoch = 0, request = null, inflight = Promise.resolve(), disposed = false;
  const listeners = new Set();
  const publish = patch => { if (!disposed) { state = { ...state, ...patch }; listeners.forEach(listener => listener()); } };
  const current = (meetingId, token) => !disposed && epoch === token && state.open && getMeeting()?.id === meetingId;
  function cancelRequest() {
    const pending = request; request = null;
    if (pending) void bridge.cancelFetch(pending).catch(() => {});
  }
  async function load(meetingId, token) {
    publish({ loading: true, error: '' });
    try {
      const value = await bridge.meetingSources(meetingId);
      if (current(meetingId, token)) publish({ revision: value.revision, sources: value.sources, loading: false });
    } catch (error) {
      if (current(meetingId, token)) publish({ loading: false, error: error?.message || '회의 자료를 불러오지 못했습니다.' });
    }
  }
  return {
    getSnapshot: () => state,
    subscribe(listener) { listeners.add(listener); return () => listeners.delete(listener); },
    open() {
      const meetingId = getMeeting()?.id;
      if (!meetingId) return;
      cancelRequest(); const token = ++epoch;
      publish({ open: true, meetingId, loading: true, busy: false, phase: null, revision: 0, sources: [], notion: null, error: '', link: '' });
      void load(meetingId, token);
      void bridge.notionSnapshot().then(notion => { if (current(meetingId, token)) publish({ notion }); }).catch(() => { if (current(meetingId, token)) publish({ error: 'Notion 연결 상태를 확인하지 못했습니다. 닫은 뒤 다시 열어 주세요.' }); });
    },
    close() { epoch++; cancelRequest(); publish({ open: false, busy: false }); },
    setMeeting(meeting) {
      if (meeting?.id === state.meetingId) return;
      epoch++; cancelRequest();
      publish({ open: false, meetingId: meeting?.id || null, loading: false, busy: false, phase: null, revision: 0, sources: [], notion: null, error: '', link: '' });
      if (!meeting?.id) return;
      const token = epoch;
      void bridge.meetingSources(meeting.id).then(value => {
        if (!disposed && epoch === token && getMeeting()?.id === meeting.id) publish({ revision: value.revision, sources: value.sources });
      }).catch(() => {});
    },
    setLink(link) { publish({ link }); },
    async reload() {
      const meetingId = getMeeting()?.id;
      if (!meetingId || state.meetingId !== meetingId) return;
      if (state.open) { await load(meetingId, epoch); return; }
      // Closed, the button still shows how many sources the meeting has.
      const token = epoch;
      try {
        const value = await bridge.meetingSources(meetingId);
        if (!disposed && epoch === token && getMeeting()?.id === meetingId) publish({ revision: value.revision, sources: value.sources });
      } catch {}
    },
    async add() {
      const { meetingId, revision, link } = state, token = epoch;
      if (!current(meetingId, token) || state.busy || state.loading || !link.trim()) return;
      const requestId = crypto.randomUUID(); request = requestId;
      publish({ busy: true, phase: 'fetching', error: '' });
      try {
        const fetching = bridge.fetchSources({ requestId, links: [link.trim()] });
        inflight = fetching.then(() => {}, () => {});
        const result = await fetching;
        if (!current(meetingId, token) || request !== requestId) return;
        const source = result.sources?.[0];
        if (!source) throw new Error('자료를 찾지 못했습니다. 링크를 확인해 주세요.');
        publish({ phase: 'saving' });
        const updated = await bridge.meetingAddSource({ meetingId, expectedRevision: revision, sourceId: source.sourceId });
        if (!current(meetingId, token) || request !== requestId) return;
        publish({ revision: updated.revision, sources: updated.sources, link: '' });
        onChanged(meetingId, updated);
      } catch (error) {
        if (current(meetingId, token) && request === requestId) publish({ error: error?.message || '자료를 추가하지 못했습니다.' });
      } finally {
        if (request === requestId) { request = null; if (current(meetingId, token)) publish({ busy: false, phase: null }); }
      }
    },
    async refresh(reference) {
      const { meetingId, revision } = state, token = epoch;
      if (!current(meetingId, token) || state.busy || state.loading || !state.notion?.connected) return;
      const requestId = crypto.randomUUID(); request = requestId;
      publish({ busy: true, phase: 'fetching', error: '' });
      try {
        const fetching = bridge.fetchSources({ requestId, links: [reference.url] });
        inflight = fetching.then(() => {}, () => {});
        const result = await fetching;
        if (!current(meetingId, token) || request !== requestId) return;
        const source = result.sources?.[0];
        if (!source || source.pageId !== reference.pageId) throw new Error('자료를 확인하지 못했습니다.');
        publish({ phase: 'saving' });
        const updated = await bridge.meetingRefreshSource({ meetingId, expectedRevision: revision, sourceId: source.sourceId });
        if (!current(meetingId, token) || request !== requestId) return;
        publish({ revision: updated.revision, sources: updated.sources }); onChanged(meetingId, updated);
      } catch (error) {
        if (current(meetingId, token) && request === requestId) publish({ error: error?.message || '자료를 새로고침하지 못했습니다.' });
      } finally {
        if (request === requestId) { request = null; if (current(meetingId, token)) publish({ busy: false, phase: null }); }
      }
    },
    // Stays busy until the cancelled request has let go of the app's single Notion slot.
    async cancel() {
      if (state.phase !== 'fetching') return;
      const settled = inflight, token = epoch;
      cancelRequest();
      await settled;
      if (epoch === token && !request) publish({ busy: false, phase: null });
    },
    async remove(pageId) {
      const { meetingId, revision } = state, token = epoch;
      if (!current(meetingId, token) || state.busy || state.loading) return;
      publish({ busy: true, phase: 'removing', error: '' });
      try {
        const updated = await bridge.meetingRemoveSource({ meetingId, expectedRevision: revision, pageId });
        if (!current(meetingId, token)) return;
        publish({ revision: updated.revision, sources: updated.sources }); onChanged(meetingId, updated);
      } catch (error) { if (current(meetingId, token)) publish({ error: error?.message || '자료를 제거하지 못했습니다.' }); }
      finally { if (current(meetingId, token)) publish({ busy: false, phase: null }); }
    },
    async openSource(url) { try { await bridge.openSource(url); } catch (error) { publish({ error: error?.message || '자료를 열지 못했습니다.' }); } },
    dispose() { epoch++; cancelRequest(); disposed = true; listeners.clear(); },
  };
}
