(function (root) {
  function create(M, options = {}) {
    const scenes = ["document", "call", "app"],
      exampleTitles = {
        "long-title": "제품 운영 회의: 다음 분기 출시 일정과 고객 문의 대응, 신규 요청 우선순위, 담당자별 후속 작업 및 최종 승인 기준 확인",
        "unbroken-title": "고객문의자동분류및서비스운영개선요청우선순위담당자배정후속작업마감일정최종승인기준통합검토회의",
      },
      scenarios = ["upcoming", "recording", "paused", "permission", "mic-disconnected", "other-recording", "finished", ...Object.keys(exampleTitles)],
      state = {
        scene: "document",
        notification: "upcoming",
        control: "hidden",
        overlay: { placement: "notch", expanded: false, position: null },
        menuOpen: false,
        confirmStop: false,
        error: null,
        exampleCase: null,
      };
    const initialNow = typeof options.now === "function" ? options.now() : options.now;
    let store, targetId, finishedId, stopOwnerId, previousOwnerId,
      elapsed = Object.create(null),
      virtualTime = Number.isFinite(Number(initialNow)) ? Number(initialNow) : 0,
      reminderAt = null, scheduledAt = 0;
    const find = (id) => [store, ...(store.archives || [])].find((meeting) => meeting.meetingId === id);
    const active = () => M.activeRecording(store);
    const label = (seconds) => {
      const whole = Math.floor(seconds || 0);
      return String(Math.floor(whole / 60)).padStart(2, "0") + ":" + String(whole % 60).padStart(2, "0");
    };
    const canControl = () => !!active() || !!state.error;
    function setExpanded(expanded) {
      state.overlay.expanded = expanded;
      state.control = expanded ? "expanded" : "compact";
    }
    function resetOverlay() {
      state.overlay = { placement: "notch", expanded: false, position: null };
    }
    function position(value) {
      if (!value || typeof value.x !== "number" || typeof value.y !== "number" || !Number.isFinite(value.x) || !Number.isFinite(value.y)) return null;
      return { x: Math.max(0, Math.min(1, value.x)), y: Math.max(0, Math.min(1, value.y)) };
    }
    function reset(scenario) {
      const original = M.initial();
      M.finishMeeting(original);
      store = M.prepareMeeting(original, exampleTitles[scenario] || "주간 회의", ["a", "b", "c"], ["guide"]);
      targetId = store.meetingId;
      finishedId = stopOwnerId = previousOwnerId = null;
      reminderAt = null;
      scheduledAt = virtualTime + 300000;
      elapsed = Object.create(null);
      Object.assign(state, {
        scene: "document", notification: "upcoming", control: "hidden",
        menuOpen: false, confirmStop: false, error: null,
        exampleCase: Object.hasOwn(exampleTitles, scenario) ? scenario : null,
      });
      resetOverlay();
      if (scenario === "other-recording") {
        store = M.startMeeting(store, "다른 회의", ["a", "b"], ["guide"]);
        store = M.switchMeeting(store, targetId);
      } else if (["recording", "paused", "mic-disconnected", "finished"].includes(scenario)) {
        M.beginRecording(store);
        if (scenario === "paused") M.toggleRecording(store);
        if (["mic-disconnected", "finished"].includes(scenario)) {
          finishedId = M.finishRecording(store);
          state.notification = scenario === "finished" ? "finished" : null;
          if (scenario === "mic-disconnected") {
            state.error = "mic-disconnected";
            setExpanded(false);
          }
        } else state.notification = null;
      } else if (scenario === "permission") { state.error = "permission"; setExpanded(false); }
      const owner = active();
      if (owner) {
        previousOwnerId = owner.meetingId;
        elapsed[owner.meetingId] = 0;
        setExpanded(false);
      }
    }
    function snapshot() {
      const meeting = find(targetId) || store,
        owner = active(),
        capture = owner || find(finishedId) || meeting;
      return {
        open: true,
        scene: state.scene,
        meeting: {
          id: meeting.meetingId,
          title: meeting.title,
          scheduleLabel: new Date(scheduledAt).toLocaleTimeString("ko-KR", { hour: "2-digit", minute: "2-digit", hour12: false }),
          countdownLabel: scheduledAt > virtualTime ? Math.ceil((scheduledAt - virtualTime) / 60000) + "분 뒤 시작" : "회의 시작 시간입니다",
          attendeeLabel: (meeting.attendees || []).map((id) => M.personName(id, store)).join(", ") || "참석자 미지정",
        },
        capture: {
          phase: owner ? owner.phase : capture.phase,
          id: capture.meetingId,
          title: capture.title,
          elapsedLabel: label(elapsed[capture.meetingId]),
          microphoneLabel: state.error === "mic-disconnected" ? "마이크 연결 끊김" : state.error === "permission" ? "마이크 권한 필요" : "MacBook 마이크",
        },
        notification: state.notification,
        control: state.control,
        overlay: { ...state.overlay, position: state.overlay.position ? { ...state.overlay.position } : null },
        menuOpen: state.menuOpen,
        confirmStop: state.confirmStop,
        error: state.error,
        exampleCase: state.exampleCase,
        otherRecording: !!owner && owner.meetingId !== targetId,
        clockLabel: new Date(virtualTime).toLocaleTimeString("ko-KR", { hour: "2-digit", minute: "2-digit", hour12: false }),
      };
    }
    function sync(nextStore) {
      if (nextStore) store = nextStore;
      const owner = active(), ownerId = owner?.meetingId || null;
      if (ownerId && ownerId !== previousOwnerId) {
        if (elapsed[ownerId] === undefined) elapsed[ownerId] = 0;
        resetOverlay();
        setExpanded(false);
        state.error = null;
        if (ownerId === targetId) {
          state.notification = null;
          reminderAt = null;
        }
      } else if (!ownerId && previousOwnerId) {
        finishedId = previousOwnerId;
        state.notification = state.error === "mic-disconnected" ? null : "finished";
        if (state.error === "mic-disconnected") setExpanded(false);
        else state.control = "hidden";
        state.menuOpen = false;
      }
      if (state.confirmStop && stopOwnerId !== ownerId) {
        state.confirmStop = false;
        stopOwnerId = null;
      }
      previousOwnerId = ownerId;
      return snapshot();
    }
    function openMeeting(id) {
      if (id && id !== store.meetingId) store = M.switchMeeting(store, id) || store;
      store.page = "meeting";
      store.tab = store.phase === "finished" ? "summary" : "transcript";
      state.scene = "app";
      state.menuOpen = false;
      state.notification = null;
    }
    function advance(seconds) {
      const amount = Number(seconds);
      if (!Number.isFinite(amount) || amount <= 0) return snapshot();
      virtualTime += amount * 1000;
      const owner = active();
      if (owner?.phase === "recording") elapsed[owner.meetingId] = (elapsed[owner.meetingId] || 0) + amount;
      if (state.notification === "snoozed" && reminderAt !== null && virtualTime >= reminderAt) {
        state.notification = find(targetId)?.phase === "ready" ? "upcoming" : null;
        reminderAt = null;
      }
      return snapshot();
    }
    function apply(action, value) {
      const owner = active();
      if (action === "scenario" && scenarios.includes(value)) reset(value);
      else if (action === "scene" && scenes.includes(value)) {
        state.scene = value;
        state.menuOpen = false;
      } else if (action === "dismiss-reminder") {
        state.notification = "dismissed";
        reminderAt = null;
      } else if (action === "snooze-reminder" && find(targetId)?.phase === "ready") {
        state.notification = "snoozed";
        reminderAt = virtualTime + 300000;
      } else if (action === "start" && !state.error && !owner && find(targetId)?.phase === "ready") {
        openMeeting(targetId);
      } else if (action === "pause" && owner?.phase === "recording") M.toggleRecording(store);
      else if (action === "resume" && owner?.phase === "paused") M.toggleRecording(store);
      else if (action === "stop" && owner) {
        state.confirmStop = true;
        stopOwnerId = owner.meetingId;
        state.menuOpen = false;
      } else if (action === "cancel-stop") {
        state.confirmStop = false;
        stopOwnerId = null;
      } else if (action === "confirm-stop") {
        if (state.confirmStop && owner && owner.meetingId === stopOwnerId) finishedId = M.finishRecording(store);
        state.confirmStop = false;
        stopOwnerId = null;
      } else if (action === "open-app") {
        openMeeting(owner?.meetingId || finishedId || targetId);
      } else if (action === "toggle-menu") state.menuOpen = !state.menuOpen;
      else if (["expand", "collapse", "toggle-expanded", "toggle-control"].includes(action) && canControl()) {
        setExpanded(action === "expand" || (action !== "collapse" && state.control !== "expanded"));
      } else if (action === "detach" && canControl()) {
        const nextPosition = position(value);
        if (nextPosition) {
          state.overlay.placement = "floating";
          state.overlay.position = nextPosition;
          if (state.control === "hidden") setExpanded(state.overlay.expanded);
        }
      } else if (action === "move" && canControl() && state.overlay.placement === "floating") {
        const nextPosition = position(value);
        if (nextPosition) state.overlay.position = nextPosition;
      } else if (action === "dock" && canControl()) {
        resetOverlay();
        setExpanded(false);
      } else if (action === "hide-control") {
        state.control = "hidden";
        state.menuOpen = false;
      } else if (action === "restore-control" && canControl()) {
        setExpanded(true);
        state.menuOpen = false;
      } else if (action === "error" && value === "permission" && !owner && find(targetId)?.phase === "ready") {
        state.error = "permission";
        setExpanded(false);
      } else if (action === "error" && value === "mic-disconnected" && owner) {
        state.error = "mic-disconnected";
        finishedId = M.finishRecording(store);
      } else if (action === "retry" && state.error) {
        state.error = null;
        state.control = "hidden";
        openMeeting(finishedId || targetId);
      } else if (action === "advance") return advance(value);
      return sync();
    }
    reset("upcoming");
    return { get store() { return store; }, state, apply, advance, sync, snapshot };
  }
  const api = { create };
  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else root.NeroDesktopModel = api;
})(typeof globalThis === "object" ? globalThis : this);
