const M = NeroModel,
  UI = NeroUI,
  $ = (q) => document.querySelector(q),
  windowsDesign = new URLSearchParams(location.search).get("platform") === "windows" && new URLSearchParams(location.search).get("session") === "windows-design",
  fixture = new URLSearchParams(location.search).get("meetings"),
  manyMeetings = fixture === "many",
  variedMeetings = fixture === "varied",
  KEY = windowsDesign ? "nero-design-system-v1-windows-design" : variedMeetings
    ? "nero-design-system-v1-varied-demo"
    : manyMeetings
      ? "nero-design-system-v1-library-demo"
      : "nero-design-system-v1";
const initialState = () =>
  manyMeetings || variedMeetings
    ? NeroMeetingFixture(M, variedMeetings ? "varied" : "many")
    : { ...M.initial(), phase: "finished" };
let s = initialState();
try {
  const saved = JSON.parse(localStorage.getItem(KEY));
  if (saved && saved.version === 3) {
    s = { ...s, ...saved.state };
    if (
      !Object.prototype.hasOwnProperty.call(saved.state, "startedAt") &&
      saved.state.meetingId !== "demo"
    )
      s.startedAt = null;
  }
} catch {}
let desktopSession = null;
let ui = {
  modal: null,
  companion: "notes",
  source: "all",
  attendees: [],
  peopleSearch: "",
  group: null,
  groupName: "",
  groupMembers: [],
  raw: false,
  edit: null,
  search: "",
  sourceFilter: "all",
  selectedGroup: "all",
  evidence: null,
  librarySearch: "",
  libraryStatus: "all",
  libraryMonth: "all",
  librarySort: "newest",
  libraryPage: 0,
};
const navigationKey = KEY + "-navigation";
let navigation = { order: "opened", limit: 3, opened: [] };
try {
  const saved = JSON.parse(localStorage.getItem(navigationKey));
  if (saved) navigation = {
    order: ["opened", "date"].includes(saved.order) ? saved.order : "opened",
    limit: [3, 5, 8].includes(saved.limit) ? saved.limit : 3,
    opened: Array.isArray(saved.opened) ? saved.opened.filter((id) => typeof id === "string").slice(0, 50) : [],
  };
} catch {}
function rememberMeeting() {
  navigation.opened = [s.meetingId, ...navigation.opened.filter((id) => id !== s.meetingId)].slice(0, 50);
  saveNavigation();
}
function saveNavigation() {
  if (desktopSession) return;
  try { localStorage.setItem(navigationKey, JSON.stringify(navigation)); }
  catch { toast("최근 회의 설정을 보관하지 못했습니다."); }
}
if (s.page === "meeting") rememberMeeting();
const composingInputs = new Set();
let pointerActive = false, settlingComposition = false, pendingRender = false;
function flushRender() {
  if (pendingRender && !pointerActive && !composingInputs.size && !settlingComposition) render();
}
const personName = (id) => M.personName(id, s);
const personRole = (id) => M.peopleFor(s).find((p) => p.id === id)?.role || "";
const personLabel = (id) => M.personLabel(id, s);
const personKey = (value) => String(value || "").trim().normalize("NFC").toLocaleLowerCase();
const attendeeSummary = (m) => {
  const ids = m.attendees || [];
  return ids.length > 2
    ? `${ids.slice(0, 2).map(personLabel).join(", ")} 외 ${ids.length - 2}명`
    : ids.map(personLabel).join(", ") || "참석자 미지정";
};
const esc = UI.escape;
const icon = (name) => NeroIcons(name);
const btn = (text, action, cls = "", disabled = false) => UI.button({
  html: text, action, className: cls, withIcon: text.includes("<svg"), disabled,
  attrs: action === "close" && cls.split(/\s+/).includes("icon") ? { "aria-label": "닫기", title: "닫기" } : {},
});
const badge = UI.badge;
const persist = () => {
  if (desktopSession) return;
  try {
    localStorage.setItem(KEY, JSON.stringify({ version: 3, state: s }));
  } catch {
    toast("변경을 보관하지 못했습니다.");
  }
};
function toast(text) {
  $("#toast").textContent = text;
  $("#toast").classList.add("show");
  clearTimeout(toast.timer);
  toast.timer = setTimeout(() => $("#toast").classList.remove("show"), 3000);
}
let viewState = NeroViewState(document);
let previousLibraryQuery = null,
  previousSourceQuery = null;
const viewKey = (name, ...parts) => name + "|" + JSON.stringify(parts);
function viewBindings() {
  const main =
    s.page === "meeting" ? viewKey("meeting", s.meetingId, s.tab) : s.page;
  const library = viewKey(
    "library",
    ui.librarySearch,
    ui.libraryStatus,
    ui.libraryMonth,
    ui.librarySort,
    ui.libraryPage,
  );
  const sourceList = viewKey("source-list", ui.sourceFilter, ui.search);
  const companion = viewKey("companion", s.meetingId, ui.companion);
  const modal = ui.modal
    ? viewKey(
        "modal",
        ui.modal,
        s.meetingId,
        ["speaker", "compare"].includes(ui.modal) ? ui.turn : null,
        ui.modal === "group" ? ui.group : null,
      )
    : null;
  return {
    modal,
    regions: [
      ["$document", "document|" + main],
      [".sidebar", "sidebar"],
      [".meeting-navigation", "recent"],
      [".workspace", "workspace|" + main],
      [".library-page", library],
      [".library-controls", "library-controls"],
      ['[data-field="source-search"]', "source-search-control"],
      [
        ".meeting-body",
        (el) =>
          viewKey(
            "body",
            s.meetingId,
            s.tab,
            el.querySelector("[data-document-id]")?.dataset.documentId || null,
          ),
      ],
      [".preparation", viewKey("preparation", s.meetingId)],
      [
        ".reading-pane",
        (el) =>
          viewKey(
            "reading",
            s.meetingId,
            el.dataset.documentId || "transcript",
          ),
      ],
      [".companion-pane", companion],
      [".question-pane", companion + "|question"],
      [".minutes-layout", viewKey("minutes", s.meetingId)],
      [".minutes-editor", viewKey("minutes-editor", s.meetingId)],
      [
        ".minutes-reference",
        viewKey("minutes-reference", s.meetingId, ui.minutesRef || "notes"),
      ],
      [
        ".source-layout",
        (el) =>
          viewKey(
            "source-layout",
            ui.sourceFilter,
            el.querySelector(".doc-detail")?.dataset.documentId,
          ),
      ],
      [".source-list", sourceList],
      [".doc-detail", (el) => viewKey("document", el.dataset.documentId)],
      [".people-page", viewKey("people", ui.selectedGroup)],
      [".group-list", "group-navigation"],
      [".selector-nav", "selector-navigation|" + modal],
      [
        ".participant-list",
        viewKey("participants", modal, ui.source, ui.peopleSearch),
      ],
      [".modal", modal],
    ],
  };
}
function render(options = {}) {
  if (composingInputs.size || pointerActive || settlingComposition) {
    pendingRender = true;
    return;
  }
  pendingRender = false;
  if (options.capture !== false) viewState.capture();
  const libraryQuery = JSON.stringify([
    ui.librarySearch,
    ui.libraryStatus,
    ui.libraryMonth,
    ui.librarySort,
    ui.libraryPage,
  ]);
  const sourceQuery = JSON.stringify([ui.sourceFilter, ui.search]);
  if (libraryQuery !== previousLibraryQuery) viewState.forget("library|");
  if (sourceQuery !== previousSourceQuery) viewState.forget("source-list|");
  $("#app").innerHTML = shell();
  renderModal();
  globalThis.NeroControls?.enhanceAll();
  persist();
  const next = viewBindings();
  viewState.restore(next.regions, next.modal);
  previousLibraryQuery = libraryQuery;
  previousSourceQuery = sourceQuery;
  globalThis.NeroDesktopController?.sync();
}
function beginDesktopPreview(store) {
  if (desktopSession) return;
  persist();
  viewState.capture();
  desktopSession = { s, ui, navigation, viewState, previousLibraryQuery, previousSourceQuery };
  s = store;
  ui = { ...M.clone(ui), modal: null, edit: null, raw: false, companion: "notes", search: "", peopleSearch: "", selectedGroup: "all", sourceFilter: "all", librarySearch: "", libraryStatus: "all", libraryMonth: "all", librarySort: "newest", libraryPage: 0 };
  navigation = { order: "opened", limit: 3, opened: [s.meetingId] };
  viewState = NeroViewState(document);
  previousLibraryQuery = previousSourceQuery = null;
}
function endDesktopPreview() {
  if (!desktopSession) return;
  ({ s, ui, navigation, viewState, previousLibraryQuery, previousSourceQuery } = desktopSession);
  desktopSession = null;
  composingInputs.clear();
  pointerActive = settlingComposition = pendingRender = false;
  render({ capture: false });
}
function recordingElapsed(recording) {
  return desktopSession ? globalThis.NeroDesktopController?.elapsed(recording.meetingId) || "00:00" : recording.emptyMeeting ? "00:00" : "18:42";
}
function meetingDate(m) {
  if (!m.startedAt || Number.isNaN(Date.parse(m.startedAt))) return "";
  return new Intl.DateTimeFormat("ko-KR", {
    year: "numeric",
    month: "long",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  }).format(new Date(m.startedAt));
}
function compactMeetingDate(m) {
  if (!m.startedAt || Number.isNaN(Date.parse(m.startedAt))) return "날짜 미정";
  const parts = Object.fromEntries(
    new Intl.DateTimeFormat("ko-KR", {
      year: "numeric",
      month: "numeric",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
      hourCycle: "h23",
    })
      .formatToParts(new Date(m.startedAt))
      .map(({ type, value }) => [type, value]),
  );
  return `${parts.year}.${parts.month}.${parts.day} ${parts.hour}:${parts.minute}`;
}
function orderedMeetings() {
  return [s, ...(s.archives || [])].sort(
    (a, b) =>
      (Date.parse(b.startedAt) || 0) -
        (Date.parse(a.startedAt) || 0) ||
      a.meetingId.localeCompare(b.meetingId),
  );
}
function meetingState(m) {
  return m.phase === "ready"
    ? "준비 중"
    : m.phase === "recording"
      ? "녹음 중"
      : m.phase === "paused"
        ? "일시정지"
        : "녹음 종료";
}
function referenceCount(m) {
  const selected = M.documents.filter((d) => (m.scope || []).includes(d.id));
  const unavailable = selected.filter((d) => !M.available(s, d.id)).length;
  return `참고 자료 ${selected.length}개${unavailable ? ` <span class="unavailable">(사용 불가 ${unavailable})</span>` : ""}`;
}
function meetingItem(m, current = false, currentAction = "page:meeting") {
  return `<button class="meeting-link ${current && s.page === "meeting" ? "selected" : ""}" data-action="${current ? currentAction : "open-meeting:" + m.meetingId}" ${current && s.page === "meeting" ? 'aria-current="true"' : ""}><span class="meeting-symbol ${m.phase}">${icon(m.phase === "ready" ? "calendar" : m.phase === "finished" ? "file-text" : "mic")}</span><span class="meeting-item-copy"><strong title="${esc(m.title)}">${esc(m.title)}</strong><span class="meeting-item-meta"><span class="meeting-item-date" title="${esc(meetingDate(m) || "날짜 미정")}" aria-label="${esc(meetingDate(m) || "날짜 미정")}">${esc(compactMeetingDate(m))}</span><span class="meeting-item-status">${meetingState(m)}</span></span></span></button>`;
}
function meetingMonth(m) {
  if (!m.startedAt) return "undated";
  const d = new Date(m.startedAt);
  return Number.isNaN(d.getTime())
    ? "undated"
    : `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`;
}
function monthLabel(value) {
  if (value === "undated") return "날짜 미정";
  const [year, month] = value.split("-");
  return `${year}년 ${Number(month)}월`;
}
function libraryResults() {
  const query = ui.librarySearch.trim().toLocaleLowerCase();
  return orderedMeetings()
    .filter((m) => {
      const text = [
        m.title,
        m.note,
        m.draft,
        ...(m.attendees || []).map(personName),
      ]
        .join(" ")
        .toLocaleLowerCase();
      const status =
        ui.libraryStatus === "all" ||
        (ui.libraryStatus === "active"
          ? ["recording", "paused"].includes(m.phase)
          : m.phase === ui.libraryStatus);
      return (
        text.includes(query) &&
        status &&
        (ui.libraryMonth === "all" || meetingMonth(m) === ui.libraryMonth)
      );
    })
    .sort((a, b) =>
      ui.librarySort === "title"
        ? a.title.localeCompare(b.title, "ko")
        : !Date.parse(a.startedAt) && Date.parse(b.startedAt) ? 1
          : Date.parse(a.startedAt) && !Date.parse(b.startedAt) ? -1
          : (ui.librarySort === "oldest" ? 1 : -1) *
            ((Date.parse(a.startedAt) || 0) - (Date.parse(b.startedAt) || 0)),
    );
}
const MEETING_PAGE_SIZE = 16;
function meetingLibrary() {
  const all = orderedMeetings(),
    results = libraryResults(),
    pageSize = MEETING_PAGE_SIZE;
  const pageCount = Math.max(1, Math.ceil(results.length / pageSize));
  ui.libraryPage = Math.max(0, Math.min(ui.libraryPage, pageCount - 1));
  const months = [...new Set(all.map(meetingMonth))].sort().reverse();
  const rows = results.slice(
    ui.libraryPage * pageSize,
    (ui.libraryPage + 1) * pageSize,
  );
  const filtered =
    ui.librarySearch || ui.libraryStatus !== "all" || ui.libraryMonth !== "all";
  const filterMenu = (kind, label, glyph, options, selected) => `<button type="button" class="library-filter-trigger" popovertarget="library-${kind}-menu" data-field="library-${kind}-trigger" aria-haspopup="menu" aria-label="${label}: ${esc(options.find(([value]) => value === selected)?.[1] || label)}">${icon(glyph)}<span>${esc(options.find(([value]) => value === selected)?.[1] || label)}</span>${icon("chevron-down")}</button><div id="library-${kind}-menu" class="library-filter-menu" popover="auto" data-filter-menu data-trigger-field="library-${kind}-trigger" role="menu" aria-label="${label}">${options.map(([value, text]) => `<button type="button" class="filter-menu-option" role="menuitemradio" aria-checked="${selected === value}" data-action="library-${kind}:${value}"><span>${esc(text)}</span><span class="menu-option-check" aria-hidden="true">${icon("check")}</span></button>`).join("")}</div>`;
  return `<section class="library-page"><header class="library-heading"><div><h1>모든 회의 <span>${all.length}</span></h1></div>${btn(icon("plus") + "새 회의", "new")}</header><div class="library-controls"><label class="library-search">${icon("search")}<input data-field="library-search" aria-label="회의 검색" placeholder="회의 이름, 참석자, 메모 검색" value="${esc(ui.librarySearch)}"></label><div class="library-filters">${filterMenu("month", "회의 날짜", "calendar", [["all", "모든 날짜"], ...months.map((value) => [value, monthLabel(value)])], ui.libraryMonth)}${filterMenu("sort", "회의 정렬", "arrow-up-down", [["newest", "최신순"], ["oldest", "오래된순"], ["title", "이름순"]], ui.librarySort)}</div></div><div class="library-status"><div class="library-status-options" role="group" aria-label="회의 상태">${[
    ["all", "전체"],
    ["ready", "준비 중"],
    ["active", "녹음 중"],
    ["finished", "녹음 종료"],
  ]
    .map(
      ([v, label]) =>
        `<button data-action="library-status:${v}" aria-pressed="${ui.libraryStatus === v}" class="${ui.libraryStatus === v ? "active" : ""}"><span class="filter-check" aria-hidden="true">${icon("check")}</span>${label}</button>`,
    )
    .join(
      "",
    )}</div><span class="library-count" role="status">${results.length}개${filtered ? " 검색됨" : ""}</span>${filtered ? btn("필터 초기화", "library-clear", "quiet small") : ""}</div><div class="library-column-head" aria-hidden="true"><span></span><span>회의 이름</span><span>참석자</span><span>날짜</span><span>상태</span><span></span></div><div class="library-results">${rows.length ? rows.map((m) => UI.meetingRow({
    title: m.title, phase: m.phase, count: (m.attendees || []).length,
    attendeeLabel: (m.attendees || []).map(personName).join(", ") || "참석자 미지정",
    date: compactMeetingDate(m), label: meetingState(m),
    action: m.meetingId === s.meetingId ? "page:meeting" : "open-meeting:" + m.meetingId,
  })).join("") : `<div class="library-empty">${icon("search")}<h2>일치하는 회의가 없습니다</h2>${btn("검색과 필터 초기화", "library-clear")}</div>`}</div><footer class="library-pagination"><span>${results.length ? `${ui.libraryPage * pageSize + 1}-${Math.min((ui.libraryPage + 1) * pageSize, results.length)} / ${results.length}개` : "0개"}</span><div class="row">${btn(icon("chevron-left") + "이전", "library-page:prev", "small", ui.libraryPage === 0)}<label><select data-field="library-page" aria-label="회의 목록 페이지">${Array.from({ length: pageCount }, (_, i) => `<option value="${i}" ${i === ui.libraryPage ? "selected" : ""}>${i + 1} / ${pageCount} 페이지</option>`).join("")}</select></label>${btn("다음" + icon("chevron-right"), "library-page:next", "small", ui.libraryPage === pageCount - 1)}</div></footer></section>`;
}
function shell() {
  const recent = navigation.order === "date" ? orderedMeetings().slice(0, navigation.limit) : navigation.opened.map((id) => orderedMeetings().find((m) => m.meetingId === id)).filter(Boolean).slice(0, navigation.limit);
  return `<div class="app-shell"><aside class="sidebar"><div class="wordmark">Nero</div>${btn(icon("plus") + "<span>새 회의</span>", "new", "new-button").replace("<button ", '<button aria-label="새 회의" title="새 회의" ')}<nav class="nav" aria-label="주 탐색">${[
    ["library", "모든 회의", "meeting"],
    ["sources", "자료함", "sources"],
    ["people", "사람과 그룹", "people"],
  ]
    .map(
      ([id, label, symbol]) =>
        `<button type="button" data-action="page:${id}" class="${s.page === id ? "active" : ""}" aria-label="${label}" title="${label}" ${s.page === id ? 'aria-current="page"' : ""}>${icon(symbol)}<span>${label}</span>${id === "library" ? `<small>${1 + (s.archives || []).length}</small>` : ""}</button>`,
    )
    .join(
      "",
    )}</nav><section class="meeting-navigation" aria-label="${navigation.order === "opened" ? "최근 열어본 회의" : "회의 날짜순"}"><div class="side-label">${navigation.order === "opened" ? "최근 열어본 회의" : "회의 날짜순"}</div>${recent.map((m) => meetingItem(m, m.meetingId === s.meetingId)).join("")}${btn(icon("search") + "회의 검색", "page:library", "quiet side-find")}</section><div class="sidebar-footer"><button type="button" class="settings-trigger quiet" data-open-settings aria-haspopup="dialog" aria-controls="settings-dialog" aria-label="설정" title="설정">${icon("settings")}<span>설정</span></button></div></aside><main class="workspace">${s.page === "library" ? meetingLibrary() : s.page === "meeting" ? meeting() : s.page === "sources" ? sources() : people()}${recordingBar()}</main></div>`;
}
function recordingBar() {
  const recording = M.activeRecording(s);
  if (!recording) return "";
  return `<footer class="recordingbar"><div class="row"><span class="record-indicator ${recording.phase === "paused" ? "paused" : ""}"></span>${btn(esc(recording.title), "recording-return", "quiet recording-title")}<span class="time">${recordingElapsed(recording)}</span><span class="tiny muted">${recording.phase === "paused" ? "일시정지" : "녹음 중"}</span></div><div class="row">${btn(icon(recording.phase === "paused" ? "play" : "pause") + (recording.phase === "paused" ? "녹음 계속" : "일시정지"), "pause", "small")}${btn(icon("square") + "녹음 종료", "finish", "small")}</div></footer>`;
}
function meeting() {
  if (s.tab === "notes") {
    s.tab = "transcript";
    ui.companion = "notes";
  }
  const ready = s.phase === "ready";
  return `<div class="topline">${btn(icon("chevron-left") + "모든 회의", "page:library", "quiet small")}<span class="status-chip ${s.phase}">${meetingState(s)}</span>${btn("참석자 " + s.attendees.length + "명", "attendees", "quiet small")}</div><header class="heading"><div class="row between"><h1>${esc(s.title)}</h1>${ready ? btn(icon("mic") + "녹음 시작", "record-start", "primary", !!M.activeRecording(s) || !s.title.trim()) : ""}</div><div class="metadata">${meetingDate(s) ? `<span>${esc(meetingDate(s))}</span>` : ""}<span class="attendee-summary">${esc(attendeeSummary(s))}</span></div><nav class="tabbar" aria-label="회의 보기">${btn(icon(ready ? "calendar" : "meeting") + (ready ? "회의 준비" : "대화와 메모"), "tab:transcript", s.tab === "transcript" ? "active" : "")}${btn(icon("book-open") + referenceCount(s), "meeting-sources", s.tab === "sources" ? "active" : "")}${!ready ? btn(icon("file-text") + "회의록", "tab:summary", s.tab === "summary" ? "active" : "") : ""}</nav></header>${s.tab === "sources" ? sources(true) : s.tab === "summary" && !ready ? summary() : ready ? preparation() : `<div class="meeting-body"><section class="reading-pane" aria-label="전사"><div class="reading-tools"><div class="segment">${btn("읽기", "transcript:read", !ui.raw ? "active" : "")}${btn("원문", "transcript:raw", ui.raw ? "active" : "")}</div><span class="tiny muted">${s.emptyMeeting ? "" : s.phase === "finished" ? "최종 전사" : "18:42까지의 대화"}</span>${s.phase === "finished" && !s.emptyMeeting ? btn("최종 전사 비교", "final-compare", "quiet small") : ""}</div><div class="transcript">${s.emptyMeeting ? `<div class="q-empty"><h2>${s.phase === "finished" ? "기록된 대화가 없습니다" : "아직 기록된 대화가 없습니다"}</h2></div>` : M.turns(s).map(turn).join("")}</div></section>${companionPane()}</div>`}`;
}
function preparation() {
  return `<div class="preparation"><section class="prepare-main"><div class="section-heading"><h2>회의 준비</h2></div><label class="field">회의 이름<input data-field="meeting-title" value="${esc(s.title)}"></label><div class="prepare-row"><div><h3>참석자</h3><p>${esc(attendeeSummary(s))}</p></div>${btn("참석자 선택", "attendees", "small")}</div><div class="prepare-row"><div><h3>참고 자료</h3><p>${
    s.scope.length
      ? M.documents
          .filter((d) => s.scope.includes(d.id))
          .map((d) => esc(d.title))
          .join(", ")
      : "선택한 자료 없음"
  }</p></div>${btn("자료 선택", "scope", "small")}</div><div class="prepare-device"><span class="tiny muted">녹음 입력</span><strong>${windowsDesign ? "기본 마이크" : "MacBook 마이크"}</strong></div>${M.activeRecording(s) ? '<div class="notice">다른 회의를 녹음 중입니다. 녹음을 종료한 뒤 시작할 수 있습니다.</div>' : ""}</section><aside class="prepare-notes">${memoEditor("메모 입력")}</aside></div>`;
}
function memoEditor(placeholder = "메모 입력") {
  return `<div class="section-heading"><h3>회의 메모</h3></div><textarea class="meeting-memo" data-field="note" aria-label="회의 메모" rows="12" placeholder="${placeholder}">${esc(s.note)}</textarea>${btn("자료 노트 가져오기", "knowledge-notes", "quiet small")}`;
}
function companionPane() {
  return `<aside class="companion-pane"><nav class="companion-tabs" aria-label="회의 도구">${btn(icon("doc") + "메모", "companion:notes", ui.companion === "notes" ? "active" : "")}${btn(icon("ask") + "질문", "companion:question", ui.companion === "question" ? "active" : "")}</nav>${ui.companion === "question" ? questionPane() : `<div class="memo-pane">${memoEditor()}</div>`}</aside>`;
}
function turn(t) {
  const personId = ui.raw ? t.person : M.speakerAt(s, t.id);
  const personIndex = M.peopleFor(s).findIndex((p) => p.id === personId);
  const editing = ui.edit === t.id;
  return UI.utterance({
    speaker: personLabel(personId), tone: personIndex, unknown: personIndex < 0,
    time: t.time, timeAction: "time:" + t.id,
    current: t.id === 5 && s.phase === "recording", editing,
    text: ui.raw ? t.text : M.textAt(s, t.id),
    corrected: !!s.corrections[t.id] && !ui.raw,
    contentHTML: editing ? `<label class="field">전사 수정<textarea id="edit-text" data-field="edit-text" rows="4">${esc(s.editDrafts?.[t.id] ?? M.textAt(s, t.id))}</textarea></label><div class="row">${btn("적용", "edit-save:" + t.id, "primary small")}${btn("취소", "edit-cancel", "small")}</div>` : "",
    actionsHTML: btn("수정", "edit:" + t.id, "small quiet", ui.raw) + btn("이름 지정", "speaker:" + t.id, "small quiet", ui.raw) + (s.corrections[t.id] ? btn("원문과 비교", "compare:" + t.id, "small quiet") : ""),
  });
}
function questionPane() {
  const a = s.answer,
    changed = a && a.context !== M.answerContext(s);
  const usable = M.hasTranscript(s) || s.scope.some((id) => M.available(s, id));
  return `<section class="question-pane" aria-label="회의에 질문"><details class="scope"><summary>참고 범위: ${M.hasTranscript(s) ? "대화 + " : ""}자료 ${s.scope.length}개</summary><p class="tiny muted">${M.hasTranscript(s) ? (s.phase === "finished" ? "최종 전사" : "지금까지의 대화") : "기록된 대화 없음"}</p>${M.documents
    .filter((d) => s.scope.includes(d.id))
    .map(
      (d) =>
        `<button class="scope-document ${!M.available(s, d.id) ? "unavailable" : ""}" data-action="source-open:${d.id}">${icon("doc")}<span>${esc(d.title)}${!M.available(s, d.id) ? "<small>사용 불가 / 답변에서 제외</small>" : ""}</span></button>`,
    )
    .join(
      "",
    )}${btn("참고 자료 변경", "scope", "quiet small")}</details>${a ? `<div><div class="question-bubble">${esc(a.question)}</div><div class="answer-meta">${a.transcript ? a.time + "까지의 대화와 선택한 자료" : "선택한 자료로 답변"}</div>${changed ? '<div class="notice">참고 내용이 바뀌었습니다. 다시 질문해 주세요.</div>' : ""}<div class="answer"><p>${esc(a.body)} ${a.transcript ? btn("대화 근거", "evidence:transcript", "citation") : ""}</p>${M.answerMaterialText(a) ? `<p>${esc(M.answerMaterialText(a))}</p>` : ""}${a.sources.length ? btn("자료 근거 " + a.sources.length, "evidence:source", "small quiet") : ""}</div>${btn("회의 메모에 추가", "answer-note", "small", changed)}</div>` : `<div class="q-empty"><h3>${usable ? "질문 예시" : "참고할 내용이 없습니다"}</h3>${usable ? btn(M.hasTranscript(s) ? "장소 확인은 누가 맡았나요?" : "모임 전에 무엇을 준비해야 하나요?", "suggest", "suggestion") : btn("참고 자료 선택", "scope", "small")}</div>`}<form class="composer" id="question-form"><label class="tiny muted" for="question">질문</label><textarea id="question" data-field="question" placeholder="질문 입력" rows="3">${esc(s.question)}</textarea><div class="row between"><span></span><button class="primary small" type="submit" ${!s.question.trim() || !usable ? "disabled" : ""}>질문 보내기</button></div></form></section>`;
}
function summary() {
  const savedChanged = M.saveDirty(s);
  const editing =
    ui.minutesMode === "edit" || (!s.draft.trim() && !s.task.trim());
  return `<div class="minutes-layout"><aside class="minutes-reference"><h3>참고 내용</h3><nav class="companion-tabs">${btn("회의 메모", "minutes-ref:notes", ui.minutesRef !== "transcript" ? "active" : "")}${btn("전사", "minutes-ref:transcript", ui.minutesRef === "transcript" ? "active" : "")}</nav>${
    ui.minutesRef === "transcript"
      ? `<div class="minutes-transcript">${
          s.emptyMeeting
            ? '<p class="muted">전사가 없습니다.</p>'
            : M.turns(s)
                .map(
                  (t) =>
                    `<article><span class="tiny muted">${esc(personName(M.speakerAt(s, t.id)))} / ${t.time}</span><p>${esc(M.textAt(s, t.id))}</p></article>`,
                )
                .join("")
        }</div>`
      : `<div class="minutes-note">${s.note ? esc(s.note) : '<span class="muted">회의 메모가 없습니다.</span>'}</div>${btn("회의록에 메모 추가", "minutes-add-note", "small", !s.note.trim())}`
  }${btn("대화와 메모로 돌아가기", "return-transcript", "quiet small")}</aside><section class="minutes-editor"><div class="minutes-heading"><h2>회의록</h2><div class="row"><div class="segment" aria-label="회의록 보기">${btn("읽기", "minutes-mode:read", !editing ? "active" : "", !s.draft.trim() && !s.task.trim())}${btn("편집", "minutes-mode:edit", editing ? "active" : "")}</div></div></div>${!s.draft.trim() ? `<div class="minutes-empty">${s.phase !== "finished" ? "<p>녹음 종료 후 초안을 만들 수 있습니다.</p>" : ""}${s.phase === "finished" ? btn("초안 만들기", "minutes-prepare", "primary", !M.hasTranscript(s) && !s.note.trim() && !s.scope.some((id) => M.available(s, id))) : ""}</div>` : ""}${editing ? `<div class="editor"><label for="summary">회의 내용</label><textarea id="summary" data-field="draft" rows="6" placeholder="결정 사항과 논의 내용">${esc(s.draft)}</textarea><div class="divider"></div><label for="task">할 일</label><textarea id="task" class="task" data-field="task" rows="3" placeholder="담당자, 할 일, 기한">${esc(s.task)}</textarea></div>` : UI.minutes([{ title: "회의 내용", text: s.draft }, { title: "할 일", text: s.task, emptyText: "작성된 할 일이 없습니다." }])}<div class="minutes-save"><span class="tiny muted">${s.saveState === "uncertain" ? "저장 결과 확인 필요" : s.saveState === "saved" ? (savedChanged ? "Notion 저장 후 수정됨" : "Notion에 저장됨") : ""}</span><div class="row">${btn(icon("copy") + "회의록 복사", "minutes-copy", "small", !s.draft.trim() && !s.task.trim())}${btn(s.saveState === "uncertain" ? "저장 결과 확인" : s.saveState === "saved" ? "저장된 내용 보기" : "Notion에 저장", "save-open", "primary", s.saveState === "idle" && !s.draft.trim() && !s.task.trim())}</div></div></section></div>`;
}
function sources(inMeeting = false) {
  const selected = M.documents.filter((d) => s.scope.includes(d.id));
  const filtered = (inMeeting ? selected : M.documents).filter(
    (d) =>
      (inMeeting || ui.sourceFilter === "all" || d.type === ui.sourceFilter) &&
      (d.title.includes(ui.search) || d.collection.includes(ui.search)),
  );
  const doc = filtered.find((d) => d.id === s.selectedDoc) || filtered[0];
  if (inMeeting)
    return `<div class="meeting-source-tools"><div class="row"><label class="tiny muted" for="meeting-doc">참고 자료</label><select id="meeting-doc" data-field="meeting-doc" ${!selected.length ? "disabled" : ""}>${selected.length ? selected.map((d) => `<option value="${d.id}" ${d.id === doc?.id ? "selected" : ""}>${esc(d.title)}${M.available(s, d.id) ? "" : " (사용 불가)"}</option>`).join("") : "<option>선택한 자료 없음</option>"}</select></div>${btn("자료 선택 변경", "scope", "small")}</div><div class="meeting-body"><article class="reading-pane meeting-document" data-document-id="${esc(doc?.id || "empty")}">${doc ? `<div class="row between"><span class="tiny muted">${doc.type} / ${esc(doc.collection)}</span>${btn("다시 가져오기", "refresh:" + doc.id, "quiet small")}</div><h2>${esc(doc.title)}</h2><div class="doc-body">${M.available(s, doc.id) ? esc(doc.body) : !M.providerConnected(s, doc.type) ? '<div class="notice">' + esc(doc.type) + " 연결이 필요합니다. " + btn("연결 확인", "connections", "small") + "</div>" : '<div class="notice">자료를 가져오지 못했습니다. 다시 가져오거나 참고 자료에서 제외해 주세요.</div>'}</div>${btn("회의에서 제외", "source-remove:" + doc.id, "quiet small")}` : `<div class="source-empty"><h2>선택한 자료가 없습니다</h2>${btn("자료 선택", "scope", "primary")}</div>`}</article>${companionPane()}</div>`;
  const header = `<div class="topline"><span></span>${btn("자료 연결", "connections", "quiet small")}</div><header class="heading source-heading"><div class="row between"><h1>자료함</h1>${btn("자료 노트 " + s.knowledgeNotes.length, "knowledge-notes", "small")}</div><nav class="tabbar" aria-label="자료 출처">${["all", "Notion", "Slack"].map((type) => btn(type === "all" ? "전체 자료 " + M.documents.length : type, "source-filter:" + type, ui.sourceFilter === type ? "active" : "")).join("")}</nav></header>`;
  return `${header}<div class="source-layout "><aside class="source-list" aria-label="전체 자료 목록"><input class="search" type="search" data-field="source-search" aria-label="자료 검색" placeholder="제목 또는 출처 검색" value="${esc(ui.search)}">${filtered.map((d) => `<button class="doc-row ${doc?.id === d.id ? "active" : ""}" data-action="doc:${d.id}" ${doc?.id === d.id ? 'aria-current="true"' : ""}><span class="source-mark" title="${d.type}">${d.type === "Notion" ? "N" : "#"}</span><span class="doc-row-copy"><strong title="${esc(d.title)}">${esc(d.title)}</strong><span class="doc-row-meta" title="${d.type} / ${esc(d.collection)} / ${d.updated}">${d.type} / ${esc(d.collection)} / ${d.updated}</span></span>${!M.available(s, d.id) ? '<span class="doc-reference-state unavailable">사용 불가</span>' : ""}</button>`).join("") || '<p class="muted tiny">표시할 자료가 없습니다.</p>'}</aside><article class="doc-detail" data-document-id="${esc(doc?.id || "empty")}">${doc ? `<div class="doc-detail-inner"><div class="row between"><span class="source-type"><span class="source-mark">${doc.type === "Notion" ? "N" : "#"}</span>${doc.type} / ${esc(doc.collection)}</span>${!M.available(s, doc.id) ? badge("사용 불가", true) : ""}</div><h2>${esc(doc.title)}</h2><p class="tiny muted">가져온 시각 / ${doc.updated}</p><div class="doc-toolbar"><span></span>${btn("다시 가져오기", "refresh:" + doc.id, "small")}</div>${!M.available(s, doc.id) ? '<div class="notice" style="margin-top:20px">자료를 가져오지 못했습니다.</div>' : ""}<div class="doc-body">${M.available(s, doc.id) ? esc(doc.body) : ""}</div><div class="source-footer row between"><span></span>${btn("외부 AI로 자료 정리", "ai-open:" + doc.id, "primary", !M.available(s, doc.id))}</div></div>` : `<div class="source-empty"><h2>${ui.search ? "검색 결과가 없습니다" : "표시할 자료가 없습니다"}</h2>${btn(ui.search ? "검색 지우기" : "전체 자료 보기", ui.search ? "source-clear-search" : "source-show-all", "small")}</div>`}</article></div>`;
}
function people() {
  const all = M.peopleFor(s);
  const group = s.groups.find((g) => g.id === ui.selectedGroup);
  const list = group ? all.filter((p) => group.members.includes(p.id)) : all;
  const groupButton = (name, id, count, selected) => `<button type="button" data-action="group-view:${id}" aria-current="${selected ? "page" : "false"}" class="${selected ? "active" : ""}">${icon("users")}<span class="group-name">${esc(name)}</span><span class="group-count">${count}</span></button>`;
  return `<section class="people-page"><header class="library-heading"><h1>사람과 그룹 <span>${all.length}</span></h1>${btn(icon("plus") + "그룹 만들기", "group-new")}</header><div class="people-grid"><nav class="group-list" aria-label="그룹">${groupButton("모든 사람", "all", all.length, !group)}<span class="group-list-label">그룹</span>${s.groups.map((g) => groupButton(g.name, g.id, g.members.length, group?.id === g.id)).join("")}</nav><section class="people-directory"><div class="people-directory-heading"><h2>${esc(group?.name || "모든 사람")} <span>${list.length}명</span></h2>${group ? btn(icon("pencil") + "그룹 편집", "group-edit:" + group.id, "small") : ""}</div><div class="people-directory-list">${list.map((p) => UI.personRow({ name: p.name, role: p.role || "역할 미지정" })).join("") || `<div class="people-directory-empty"><p>그룹에 등록된 사람이 없습니다.</p>${group ? btn("구성원 선택", "group-edit:" + group.id, "small") : ""}</div>`}</div></section></div></section>`;
}
function openParticipants(mode) {
  ui.modal = mode;
  ui.attendees = mode === "new" ? [] : [...s.attendees];
  ui.participantPeople = [...(s.customPeople || [])];
  ui.source = "all";
  ui.peopleSearch = "";
  ui.personName = "";
  ui.personRole = "";
  ui.personRegistration = false;
  ui.newTitle = "";
  ui.newScope = [];
  render();
}
function selectablePeople() {
  const group = s.groups.find((g) => g.id === ui.source);
  const query = personKey(ui.peopleSearch);
  return M.peopleFor({ ...s, customPeople: ui.participantPeople }).filter(
    (p) =>
      (query || ui.source !== "selected" || ui.attendees.includes(p.id)) &&
      (query || !group || group.members.includes(p.id)) &&
      personKey(`${p.name} ${p.role || ""}`).includes(query),
  );
}
function peopleSelector() {
  const list = selectablePeople();
  const all = M.peopleFor({ ...s, customPeople: ui.participantPeople });
  const query = ui.peopleSearch.trim();
  const group = s.groups.find((g) => g.id === ui.source);
  const selectedCount = list.filter((p) => ui.attendees.includes(p.id)).length;
  const allSelected = list.length > 0 && selectedCount === list.length;
  const scopeName = query ? "모든 사람 검색 결과" : ui.source === "selected" ? "선택한 참석자" : group?.name || "모든 사람";
  const draftName = (ui.personName || "").trim();
  const draftRole = (ui.personRole || "").trim();
  const sameDraftName = all.filter((p) => personKey(p.name) === personKey(draftName));
  const duplicate = sameDraftName.some((p) => personKey(p.role) === personKey(draftRole));
  const needsRole = sameDraftName.length > 0;
  const registration = ui.personRegistration
    ? `<section class="participant-registration registration-form" aria-label="새 사람 등록"><div class="registration-heading"><strong>새 사람 등록</strong>${btn("취소", "person-register-cancel", "small quiet")}</div><div class="registration-fields"><label class="field">이름<input data-field="person-name" value="${esc(ui.personName || "")}" placeholder="이름" aria-label="새 사람 이름"></label><label class="field">${needsRole ? "역할 (필수)" : "역할 (선택)"}<input data-field="person-role" value="${esc(ui.personRole || "")}" placeholder="예: 기획, 운영" aria-label="새 사람 역할" ${needsRole ? "required" : ""}></label>${btn("등록하고 선택", "person-add", "small", !draftName || duplicate || (needsRole && !draftRole))}</div>${duplicate ? '<p class="registration-notice" role="status">이름과 역할이 같은 사람이 있습니다. 목록에서 선택하세요.</p>' : needsRole ? '<p class="registration-notice" role="status">같은 이름의 사람과 구별할 역할을 입력하세요.</p>' : ""}</section>`
    : `<div class="participant-registration">${btn(icon("plus") + "새 사람 등록", "person-register-open", "small")}</div>`;
  const sourceButton = (name, id, count) => `<button type="button" data-action="pick-source:${id}" aria-pressed="${!query && ui.source === id}" class="${!query && ui.source === id ? "active" : ""}"><span>${esc(name)}</span><span class="scope-count">${count}</span></button>`;
  return `<div class="participant-search"><label for="participant-search-input">기존 사람 선택</label><div class="participant-search-control">${icon("search")}<input id="participant-search-input" data-field="people-search" value="${esc(ui.peopleSearch)}" aria-label="모든 사람에서 이름 또는 역할 검색" placeholder="모든 사람에서 이름 또는 역할 검색">${query ? `<button type="button" data-action="people-search-clear" class="icon quiet" aria-label="사람 검색 지우기">${icon("x")}</button>` : ""}</div></div><div class="selector-layout participant-selector${query ? " searching" : ""}"><nav class="selector-nav" aria-label="참석자 범위">${sourceButton("모든 사람", "all", all.length)}${sourceButton("선택한 참석자", "selected", ui.attendees.length)}<span class="selector-nav-label">그룹</span>${s.groups.map((g) => sourceButton(g.name, g.id, g.members.length)).join("")}</nav><div class="selector-people"><div class="participant-list-heading"><div><strong>${esc(scopeName)}</strong><span class="tiny muted" role="status">${list.length}명 중 ${selectedCount}명 선택</span></div>${btn(allSelected ? "표시된 사람 해제" : "표시된 사람 선택", "attendee-all", "small quiet", !list.length)}</div><div class="participant-list" tabindex="0" aria-label="참석자 목록">${list.map((p) => UI.personOption({ name: p.name, role: p.role || "역할 미지정", checked: ui.attendees.includes(p.id), attrs: { "data-field": "attendee:" + p.id } })).join("") || `<div class="participant-empty">${icon("users")}<p>${query ? "일치하는 사람이 없습니다." : ui.source === "selected" ? "아직 선택한 참석자가 없습니다." : "그룹에 등록된 사람이 없습니다."}</p></div>`}</div></div></div>${registration}`;
}
function modalBody() {
  if (ui.modal === "finish") return UI.dialogContent({
    title: "녹음을 종료할까요?",
    bodyHTML: `<p><strong>${esc(M.activeRecording(s)?.title || s.title)}</strong></p>`,
    actionsHTML: btn("계속 녹음", "close") + btn("종료하고 회의록 정리", "finish-confirm", "primary"),
  });
  if (ui.modal === "new") return UI.dialogContent({
    title: "새 회의",
    bodyHTML: `<label class="field">회의 이름<input data-field="new-title" placeholder="예: 10월 독서 모임" value="${esc(ui.newTitle)}"></label>`,
    actionsHTML: btn("회의 준비", "pick-save", "primary", !ui.newTitle.trim()),
  });
  if (ui.modal === "attendees")
    return `<h2>회의 참석자</h2>${peopleSelector()}<div class="modal-footer"><span class="tiny muted">${ui.attendees.length}명 선택</span><div class="row">${btn("취소", "close")}${btn("적용", "pick-save", "primary")}</div></div>`;
  if (ui.modal === "group")
    return `<h2>${ui.group ? "그룹 편집" : "그룹 만들기"}</h2><label class="field" style="margin-top:22px">그룹 이름<input data-field="group-name" value="${esc(ui.groupName)}" placeholder="예: 독서 모임"></label><div class="tiny muted">구성원</div>${M.peopleFor(
      s,
    )
      .map(
        (p) =>
          `<label class="selector-person"><input type="checkbox" data-field="group-person:${p.id}" ${ui.groupMembers.includes(p.id) ? "checked" : ""}><span class="avatar">${esc(p.name.slice(0, 1))}</span><span class="person-identity"><strong>${esc(p.name)}</strong><span class="muted">${esc(p.role || "역할 미지정")}</span></span></label>`,
      )
      .join(
        "",
      )}<div class="modal-footer">${ui.group ? btn("그룹 삭제", "group-delete", "quiet") : "<span></span>"}<div class="row">${btn("취소", "close")}${btn("저장", "group-save", "primary", !ui.groupName.trim())}</div></div>`;
  if (ui.modal === "speaker")
    return `<h2>발언자 지정</h2><p class="muted tiny" style="margin:15px 0">${M.turns(s).find((t) => t.id === ui.turn)?.time} 발언에 적용</p><p style="margin-bottom:18px">${esc(M.textAt(s, ui.turn))}</p>${s.attendees.map((id) => `<div class="person-row"><span class="avatar">${esc(personName(id).slice(0, 1))}</span><span class="grow person-identity"><strong>${esc(personName(id))}</strong><span class="muted">${esc(personRole(id) || "역할 미지정")}</span></span>${btn("지정", "speaker-set:" + id, "small")}</div>`).join("")}${btn("이름 미확인으로 두기", "speaker-set:unknown", "quiet small")}`;
  if (ui.modal === "compare")
    return `<h2>전사 비교</h2><div class="compare" style="margin-top:22px"><section><h3>원문</h3><p>${esc(M.turns(s).find((t) => t.id === ui.turn)?.text)}</p></section><section><h3>현재 전사</h3><p>${esc(M.textAt(s, ui.turn))}</p></section></div>${s.finalTranscript?.[ui.turn] ? '<div class="read-note"><strong>최종 전사</strong><p>' + esc(s.finalTranscript[ui.turn]) + "</p></div>" : ""}`;
  if (ui.modal === "scope")
    return `<h2>회의 참고 자료 선택</h2><p class="scope-meeting-title">${esc(s.title)}</p>${M.documents.map((d) => `<label class="selector-person"><input type="checkbox" data-field="scope:${d.id}" ${ui.scopeDraft.includes(d.id) ? "checked" : ""} ${!M.available(s, d.id) && !ui.scopeDraft.includes(d.id) ? "disabled" : ""}><span class="grow">${esc(d.title)}</span>${badge(M.available(s, d.id) ? d.type : "사용 불가", !M.available(s, d.id))}</label>`).join("")}<div class="modal-footer"><span class="tiny muted">${ui.scopeDraft.length}개 선택</span><div class="row">${btn("취소", "close")}${btn("적용", "scope-apply", "primary")}</div></div>`;
  if (ui.modal === "evidence") {
    const a = s.answer;
    return `<h2>답변 당시 근거</h2><p class="muted tiny" style="margin-top:12px">${a.time ? a.time + " 질문에서 참고한 내용" : "자료 질문에서 참고한 내용"}</p>${ui.evidence === "transcript" && a.transcript ? `<div class="read-note">${esc(a.transcript.name)} / ${a.transcript.time}\n\n${esc(a.transcript.text)}</div>` : a.sources.map((d) => (M.available(s, d.id) ? `<div class="read-note"><strong>${esc(d.title)}</strong>\n\n${esc(d.body)}</div>` : '<div class="notice" style="margin-top:18px">현재 접근할 수 없는 자료입니다. 연결을 확인해 주세요.</div>')).join("")}`;
  }
  if (ui.modal === "connections")
    return `<h2>자료 연결</h2>${["Notion", "Slack"].map((name) => `<div class="person-row"><span class="source-mark">${name === "Notion" ? "N" : "#"}</span><strong class="grow">${name}</strong><span class="tiny muted">${M.providerConnected(s, name) ? "연결됨" : "연결 안 됨"}</span>${btn(M.providerConnected(s, name) ? "연결 해제" : "연결", "connection-toggle:" + name, "small")}</div>`).join("")}`;
  if (ui.modal === "ai") return aiModal();
  if (ui.modal === "knowledge")
    return `<h2>자료 노트</h2>${s.knowledgeNotes.length ? s.knowledgeNotes.map((n, index) => `<div class="read-note"><strong>${esc(n.title)}</strong>\n\n${esc(n.body)}<p class="tiny muted">근거: ${n.sources.map((d) => esc(d.title)).join(", ")}</p>${s.page === "meeting" ? btn("회의 메모로 가져오기", "knowledge-attach:" + index, "small") : ""}</div>`).join("") : '<p class="muted" style="margin-top:20px">자료 노트가 없습니다.</p>'}`;
  if (ui.modal === "save") return saveModal();
  return "";
}
function aiModal() {
  const c = s.candidate,
    valid = M.canApply(s);
  return `<h2>외부 AI로 자료 정리</h2><div class="ai-layout"><section><h3>보낼 내용</h3>${M.documents.map((d) => `<label class="selector-person"><input type="checkbox" data-field="ai-select:${d.id}" ${s.aiSelection.includes(d.id) ? "checked" : ""} ${!M.available(s, d.id) ? "disabled" : ""}><span class="tiny">${esc(d.title)}</span></label>`).join("")}<label class="field">요청<textarea data-field="aiRequest" rows="3">${esc(s.aiRequest)}</textarea></label><label class="field">덧붙일 메모<textarea data-field="knowledgeDraft" rows="2">${esc(s.knowledgeDraft)}</textarea></label><label class="checkline"><input type="checkbox" data-field="consent" ${s.consent ? "checked" : ""}>선택한 자료를 외부 AI에 전달하는 데 동의합니다</label><div style="margin-top:16px">${btn("요청문 복사", "ai-prepare", "primary", !s.consent || !s.aiSelection.length || !s.aiSelection.every((id) => M.available(s, id)))}</div>${ui.aiReady ? '<p class="tiny muted" style="margin-top:12px">사용하는 AI에 요청문을 붙여넣어 주세요.</p>' : ""}</section><section><h3>답변 검토</h3>${c ? `<div class="candidate-body">${esc(c.text)}</div><p class="tiny muted">참고 자료: ${c.sources.map((d) => esc(d.title)).join(", ")}</p>${!valid && !c.applied ? '<div class="notice">보낼 내용이 바뀌었습니다. 다시 요청해 주세요.</div>' : ""}<div class="row" style="margin-top:18px">${btn(c.applied ? "저장됨" : "자료 노트에 저장", "ai-apply", "primary", !valid)}${btn("다시 입력", "ai-edit", "small")}</div>` : `<label class="field" style="margin-top:18px">AI 답변 붙여넣기<textarea data-field="ai-response" rows="10" placeholder="AI 답변 붙여넣기">${esc(ui.aiResponse || "")}</textarea></label>${btn("답변 검토", "ai-return", "primary", !ui.aiReady || !ui.aiResponse?.trim())}`}</section></div>`;
}
function saveModal() {
  if (s.saveState === "uncertain")
    return `<h2>저장 결과 확인이 필요합니다</h2><p style="margin:18px 0">중복 저장을 막기 위해 기존 요청의 결과부터 확인합니다.</p><div class="read-note"><strong>${esc(s.saveSnapshot.title)}</strong>\n${esc(s.saveSnapshot.destination)}\n\n${esc(s.saveSnapshot.draft)}\n\n할 일\n${esc(s.saveSnapshot.task)}</div><div class="modal-footer"><span></span>${btn("기존 저장 결과 찾기", "save-recover", "primary")}</div>`;
  if (s.saveState === "saved")
    return `<h2>Notion에 저장된 내용</h2>${M.saveDirty(s) ? '<div class="notice" style="margin-top:16px">현재 회의록이 저장된 내용과 다릅니다.</div>' : ""}<div class="read-note"><strong>${esc(s.receipt.title)}</strong>\n${esc(s.receipt.destination)}\n\n${esc(s.receipt.draft)}\n\n할 일\n${esc(s.receipt.task)}</div><div class="modal-footer"><span class="tiny muted"></span>${btn("완료", "close", "primary")}</div>`;
  if (!M.providerConnected(s, "Notion"))
    return `<h2>Notion 연결이 필요합니다</h2><div class="row">${btn("자료 연결", "connections", "primary")}${btn(icon("copy") + "회의록 복사", "minutes-copy")}</div>`;
  return `<h2>Notion에 저장</h2><label class="field" style="margin-top:24px">제목<input data-field="saveTitle" value="${esc(s.saveTitle)}"></label><label class="field">저장 위치<select data-field="destination">${[...new Set([s.destination, "독서 모임 / 회의 기록", "내 회의록"])].map((destination) => `<option value="${esc(destination)}" ${s.destination === destination ? "selected" : ""}>${esc(destination)}</option>`).join("")}</select></label><div class="read-note">${esc(s.draft)}\n\n할 일\n${esc(s.task)}</div><div class="modal-footer"><span class="tiny muted"></span>${btn("저장", "save-send", "primary", !s.saveTitle.trim())}</div>`;
}
function renderModal() {
  const root = $("#modal-root");
  if (!ui.modal) {
    root.innerHTML = "";
    return;
  }
  root.innerHTML = `<div class="modal-shade"><section role="dialog" aria-modal="true" aria-labelledby="modal-title" class="modal ${ui.modal === "ai" ? "wide" : ""} ${ui.modal === "attendees" ? "attendees-modal" : ""}"><div style="float:right">${btn(icon("close"), "close", "icon quiet")}</div>${modalBody().replace("<h2>", '<h2 id="modal-title">')}</section></div>`;
  root
    .querySelector('[data-action="close"]')
    .setAttribute("aria-label", "닫기");
}
const toggle = (arr, id, on) =>
  on ? [...new Set([...arr, id])] : arr.filter((x) => x !== id);
function handleTextInput(e) {
  const key = e.target.dataset.field;
  if (!key) return;
  if (e.target.isConnected === false) return;
  if (e.isComposing) composingInputs.add(e.target);
  const value = e.target.value;
  if (key === "edit-text") {
    s.editDrafts = { ...(s.editDrafts || {}), [ui.edit]: value };
    persist();
    return;
  }
  if (key === "people-search") {
    ui.peopleSearch = value;
    render();
    return;
  }
  if (key === "library-search") {
    ui.librarySearch = value;
    ui.libraryPage = 0;
    render();
    return;
  }
  if (key === "source-search") {
    ui.search = value;
    render();
    return;
  }
  if (key === "meeting-title") {
    s.title = value;
    s.saveTitle = value;
  } else if (key === "person-name") ui.personName = value;
  else if (key === "person-role") ui.personRole = value;
  else if (key === "ai-response") {
    ui.aiResponse = value;
    s.aiResponseDraft = value;
  } else if (key === "group-name") ui.groupName = value;
  else if (key === "new-title") ui.newTitle = value;
  else if (
    [
      "question",
      "note",
      "draft",
      "task",
      "saveTitle",
      "aiRequest",
      "knowledgeDraft",
    ].includes(key)
  )
    s[key] = value;
  else return;
  const setDisabled = (action, disabled) => {
    const control = document.querySelector(`[data-action="${action}"]`);
    if (control) control.disabled = disabled;
  };
  if (key === "new-title") setDisabled("pick-save", !ui.newTitle.trim());
  if (key === "group-name") setDisabled("group-save", !ui.groupName.trim());
  if (key === "saveTitle") setDisabled("save-send", !s.saveTitle.trim());
  if (key === "person-name" || key === "person-role") {
    const name = personKey(ui.personName);
    const role = personKey(ui.personRole);
    const matches = M.peopleFor({ ...s, customPeople: ui.participantPeople }).filter((p) => personKey(p.name) === name);
    setDisabled("person-add", !name || (matches.length && !role) || matches.some((p) => personKey(p.role) === role));
  }
  if (key === "question") {
    const submit = document.querySelector('#question-form button[type="submit"]');
    if (submit) submit.disabled = !s.question.trim() || !(M.hasTranscript(s) || s.scope.some((id) => M.available(s, id)));
  }
  render();
}
document.addEventListener("input", handleTextInput);
document.addEventListener("compositionstart", (e) => {
  if (e.target.dataset.field) composingInputs.add(e.target);
});
document.addEventListener("compositionend", (e) => {
  composingInputs.delete(e.target);
  settlingComposition = true;
  handleTextInput(e);
  settlingComposition = false;
  setTimeout(flushRender, 0);
});
document.addEventListener("focusout", (e) => {
  if (!composingInputs.delete(e.target)) return;
  settlingComposition = true;
  handleTextInput(e);
  settlingComposition = false;
  setTimeout(flushRender, 0);
});
document.addEventListener("pointerdown", () => { pointerActive = true; }, true);
document.addEventListener("pointerup", () => {
  setTimeout(() => { pointerActive = false; flushRender(); }, 0);
}, true);
document.addEventListener("pointercancel", () => { pointerActive = false; flushRender(); }, true);
document.addEventListener("change", (e) => {
  const field = e.target.dataset.field;
  if (!field) return;
  const [key, id] = field.split(":"),
    on = e.target.checked;
  if (key === "recent-order" || key === "recent-limit") {
    if (key === "recent-order" && ["opened", "date"].includes(e.target.value)) navigation.order = e.target.value;
    if (key === "recent-limit" && [3, 5, 8].includes(Number(e.target.value))) navigation.limit = Number(e.target.value);
    saveNavigation();
  } else if (key === "library-month") {
    ui.libraryMonth = e.target.value;
    ui.libraryPage = 0;
  } else if (key === "library-sort") {
    ui.librarySort = e.target.value;
    ui.libraryPage = 0;
  } else if (key === "library-page") ui.libraryPage = Number(e.target.value);
  else if (key === "attendee") ui.attendees = toggle(ui.attendees, id, on);
  else if (key === "group-person")
    ui.groupMembers = toggle(ui.groupMembers, id, on);
  else if (key === "scope") ui.scopeDraft = toggle(ui.scopeDraft, id, on);
  else if (key === "meeting-doc") s.selectedDoc = e.target.value;
  else if (key === "ai-select") s.aiSelection = toggle(s.aiSelection, id, on);
  else if (key === "consent") s.consent = on;
  else if (key === "destination") s.destination = e.target.value;
  else return;
  render();
});
document.addEventListener("submit", (e) => {
  if (e.target.id !== "question-form") return;
  e.preventDefault();
  if (!s.question.trim()) return;
  M.ask(s);
  render();
});
document.addEventListener("click", async (e) => {
  pointerActive = false;
  const node = e.target.closest("[data-action]");
  if (!node || node.disabled) {
    const popupTrigger = e.target.closest("[popovertarget]");
    if (pendingRender && popupTrigger) {
      const popupId = popupTrigger.getAttribute("popovertarget");
      e.preventDefault();
      render();
      document.getElementById(popupId)?.showPopover();
    } else setTimeout(flushRender, 0);
    return;
  }
  const [action, id] = node.dataset.action.split(":");
  if (action === "desktop") {
    globalThis.NeroDesktopController?.action(id, node.dataset.value);
    return;
  }
  if (action === "library-month" || action === "library-sort") {
    if (action === "library-month") ui.libraryMonth = id;
    else ui.librarySort = id;
    ui.libraryPage = 0;
    const trigger = document.querySelector(`[data-field="${action}-trigger"]`);
    trigger?.focus({ preventScroll: true });
  } else if (action === "library-status") {
    ui.libraryStatus = id;
    ui.libraryPage = 0;
  } else if (action === "library-clear") {
    ui.librarySearch = "";
    ui.libraryStatus = "all";
    ui.libraryMonth = "all";
    ui.libraryPage = 0;
  } else if (action === "library-page")
    ui.libraryPage += id === "next" ? 1 : -1;
  else if (action === "page") {
    s.page = id;
    ui.search = "";
    if (id === "sources") ui.sourceFilter = "all";
    ui.modal = null;
    ui.edit = null;
  } else if (action === "return-transcript") {
    s.page = "meeting";
    s.tab = "transcript";
  } else if (action === "source-clear-search") ui.search = "";
  else if (action === "source-remove")
    s.scope = s.scope.filter((docId) => docId !== id);
  else if (action === "source-filter") ui.sourceFilter = id;
  else if (action === "source-show-all") {
    ui.sourceFilter = "all";
    ui.search = "";
  } else if (action === "source-open") {
    s.selectedDoc = id;
    ui.companion = "question";
    s.page = "meeting";
    s.tab = "sources";
    ui.search = "";
  } else if (action === "meeting-sources") {
    s.page = "meeting";
    s.tab = "sources";
    ui.search = "";
  } else if (action === "tab") s.tab = id;
  else if (action === "new") {
    openParticipants("new");
    return;
  } else if (action === "open-meeting") {
    const next = M.switchMeeting(s, id);
    if (next) {
      s = next;
      s.page = "meeting";
      ui.search = "";
      ui.modal = null;
      ui.edit = null;
      ui.raw = false;
      rememberMeeting();
    } else toast("현재 회의를 먼저 종료해 주세요.");
  } else if (action === "attendees") {
    openParticipants("attendees");
    return;
  } else if (action === "close") {
    ui.modal = null;
  } else if (action === "pause") {
    M.toggleRecording(s);
  } else if (action === "finish") {
    if (desktopSession) { globalThis.NeroDesktopController.action("stop"); return; }
    ui.modal = "finish";
  } else if (action === "finish-confirm") {
    const finishedId = M.finishRecording(s);
    if (finishedId && finishedId !== s.meetingId)
      s = M.switchMeeting(s, finishedId) || s;
    s.page = "meeting";
    s.tab = "summary";
    ui.modal = null;
  } else if (action === "recording-return") {
    const recording = M.activeRecording(s);
    if (recording && recording.meetingId !== s.meetingId)
      s = M.switchMeeting(s, recording.meetingId) || s;
    s.page = "meeting";
    s.tab = "transcript";
    ui.modal = null;
  } else if (action === "record-start") {
    if (desktopSession && !globalThis.NeroDesktopController.beforeRecordingStart()) return;
    if (!M.beginRecording(s)) toast("진행 중인 녹음을 먼저 종료해 주세요.");
  } else if (action === "companion") ui.companion = id;
  else if (action === "minutes-ref") ui.minutesRef = id;
  else if (action === "minutes-mode") ui.minutesMode = id;
  else if (action === "minutes-prepare") M.prepareMinutes(s);
  else if (action === "minutes-add-note") {
    s.draft += (s.draft ? "\n\n" : "") + s.note;
  } else if (action === "knowledge-attach") {
    M.attachKnowledgeNote(s, Number(id));
    ui.modal = null;
    s.page = "meeting";
    s.tab = "transcript";
    ui.companion = "notes";
  } else if (action === "transcript") ui.raw = id === "raw";
  else if (action === "edit") ui.edit = Number(id);
  else if (action === "edit-cancel") {
    if (s.editDrafts) delete s.editDrafts[ui.edit];
    ui.edit = null;
  } else if (action === "edit-save") {
    s.corrections[id] = s.editDrafts?.[id] ?? M.textAt(s, Number(id));
    if (s.editDrafts) delete s.editDrafts[id];
    ui.edit = null;
  } else if (action === "compare" || action === "final-compare") {
    ui.turn = action === "final-compare" ? 5 : Number(id);
    ui.modal = "compare";
  } else if (action === "speaker") {
    ui.turn = Number(id);
    ui.modal = "speaker";
  } else if (action === "speaker-set") {
    s.speaker[ui.turn] = id === "unknown" ? "unknown" : id;
    ui.modal = null;
  } else if (action === "time") toast("오디오 없음");
  else if (action === "suggest") {
    s.question = M.hasTranscript(s)
      ? "누가 장소를 확인하기로 했고, 무엇을 준비하면 되나요?"
      : "모임 전에 무엇을 준비해야 하나요?";
  } else if (action === "scope") {
    ui.scopeDraft = [...s.scope];
    ui.scopeMeetingId = s.meetingId;
    ui.modal = "scope";
  } else if (action === "scope-apply") {
    if (ui.scopeMeetingId === s.meetingId) s.scope = [...ui.scopeDraft];
    ui.modal = null;
  } else if (action === "evidence") {
    ui.evidence = id;
    ui.modal = "evidence";
  } else if (action === "answer-note") {
    if (M.addAnswerToNotes(s)) {
      ui.companion = "notes";
    }
  } else if (action === "doc") s.selectedDoc = id;
  else if (action === "connections") ui.modal = "connections";
  else if (action === "connection-toggle") {
    M.toggleConnection(s, id);
    s.consent = false;
    ui.aiReady = false;
  } else if (action === "refresh") {
    if (!M.providerConnected(s, M.documents.find((d) => d.id === id)?.type)) {
      toast("자료 연결을 먼저 확인해 주세요.");
      return;
    }
    s.sourceStatus[id] = "ready";
  } else if (action === "pick-source") {
    ui.source = id;
    ui.peopleSearch = "";
    ui.personRegistration = false;
  } else if (action === "people-search-clear") {
    ui.peopleSearch = "";
    ui.personRegistration = false;
  } else if (action === "person-register-open") {
    ui.personRegistration = true;
    ui.personName = ui.peopleSearch.trim();
    ui.personRole = "";
  } else if (action === "person-register-cancel") {
    ui.personRegistration = false;
    ui.personName = "";
    ui.personRole = "";
  } else if (action === "attendee-all") {
    const list = selectablePeople();
    const allSelected = list.every((p) => ui.attendees.includes(p.id));
    const ids = list.map((p) => p.id);
    ui.attendees = allSelected
      ? ui.attendees.filter((id) => !ids.includes(id))
      : M.selectGroup(ui.attendees, ids);
  } else if (action === "pick-save") {
    if (ui.modal === "attendees") {
      s.attendees = [...ui.attendees];
      const existingIds = new Set((s.customPeople || []).map((p) => p.id));
      s.customPeople = ui.participantPeople.filter((p) => existingIds.has(p.id) || ui.attendees.includes(p.id));
    }
    if (ui.modal === "new") {
      const next = M.prepareMeeting(
        s,
        ui.newTitle.trim(),
        ui.attendees,
        ui.newScope,
      );
      if (!next) {
        toast("회의를 준비할 수 없습니다.");
        render();
        return;
      }
      s = next;
      rememberMeeting();
    }
    ui.modal = null;
    s.page = "meeting";
  } else if (action === "person-add") {
    const name = (ui.personName || "").trim();
    const role = (ui.personRole || "").trim();
    const matches = M.peopleFor({ ...s, customPeople: ui.participantPeople }).filter((p) => personKey(p.name) === personKey(name));
    if (!name || (matches.length && !role) || matches.some((p) => personKey(p.role) === personKey(role))) return;
    const draft = { customPeople: ui.participantPeople };
    const person = M.addPerson(draft, name, role);
    ui.participantPeople = draft.customPeople;
    if (person)
      ui.attendees = M.selectGroup(ui.attendees, [person.id || person]);
    ui.personName = "";
    ui.personRole = "";
    ui.personRegistration = false;
    ui.peopleSearch = name;
    ui.source = "all";
  } else if (action === "group-view") ui.selectedGroup = id;
  else if (action === "group-new") {
    ui.modal = "group";
    ui.group = null;
    ui.groupName = "";
    ui.groupMembers = [];
  } else if (action === "group-edit") {
    const g = s.groups.find((g) => g.id === id);
    ui.group = g.id;
    ui.groupName = g.name;
    ui.groupMembers = [...g.members];
    ui.modal = "group";
  } else if (action === "group-save") {
    if (!ui.groupName.trim()) return;
    const g = {
      id: ui.group || "group-" + Date.now(),
      name: ui.groupName.trim(),
      members: [...ui.groupMembers],
    };
    s.groups = ui.group
      ? s.groups.map((x) => (x.id === ui.group ? g : x))
      : [...s.groups, g];
    ui.selectedGroup = g.id;
    ui.modal = null;
  } else if (action === "group-delete") {
    s.groups = s.groups.filter((g) => g.id !== ui.group);
    ui.selectedGroup = "all";
    ui.modal = null;
  } else if (action === "ai-open") {
    if (id) {
      s.selectedDoc = id;
      if (s.aiSelection.length !== 1 || s.aiSelection[0] !== id) {
        s.aiSelection = [id];
        s.consent = false;
        ui.aiReady = false;
      }
    }
    ui.modal = "ai";
    ui.aiResponse =
      s.aiResponseDraft ?? ui.aiResponse ?? s.candidate?.text ?? "";
    if (s.aiPrepared?.context === M.candidateContext(s)) {
      ui.aiReady = true;
      ui.preparedContext = s.aiPrepared.context;
    }
  } else if (action === "ai-prepare") {
    const prompt = M.buildAIPrompt(s);
    if (!prompt) return;
    try {
      await navigator.clipboard.writeText(prompt);
    } catch {
      toast("복사할 수 없습니다. 브라우저의 클립보드 권한을 확인해 주세요.");
      return;
    }
    ui.aiReady = true;
    s.candidate = null;
    ui.preparedContext = M.candidateContext(s);
    toast("요청문을 복사했습니다.");
  } else if (action === "ai-return") {
    if (ui.preparedContext !== M.candidateContext(s) || !s.consent) {
      ui.aiReady = false;
      toast("요청 내용이 바뀌었습니다. 다시 복사해 주세요.");
    } else M.prepareCandidate(s, ui.aiResponse);
  } else if (action === "ai-edit") {
    const valid = M.canApply(s);
    ui.aiResponse = s.candidate?.text ?? ui.aiResponse ?? "";
    s.aiResponseDraft = ui.aiResponse;
    if (valid) {
      M.buildAIPrompt(s);
      ui.preparedContext = M.candidateContext(s);
      ui.aiReady = true;
    } else ui.aiReady = false;
    s.candidate = null;
  } else if (action === "ai-apply") {
    M.applyCandidate(s);
  } else if (action === "knowledge-notes") ui.modal = "knowledge";
  else if (action === "minutes-copy") {
    try {
      await navigator.clipboard.writeText(
        s.title + "\n\n" + s.draft + "\n\n할 일\n" + s.task,
      );
      toast("회의록을 복사했습니다.");
    } catch {
      toast("복사할 수 없습니다. 브라우저의 클립보드 권한을 확인해 주세요.");
    }
  } else if (action === "save-open") ui.modal = "save";
  else if (action === "save-send") {
    M.beginSave(s);
  } else if (action === "save-recover") {
    M.recover(s);
  }
  if (s.page === "meeting" && ["page", "recording-return", "finish-confirm", "record-start"].includes(action)) rememberMeeting();
  render();
  if (action === "person-register-open") document.querySelector('[data-field="person-name"]')?.focus({ preventScroll: true });
});
document.addEventListener("beforetoggle", (e) => {
  if (!e.target.hasAttribute?.("data-filter-menu") || e.newState !== "open") return;
  const trigger = document.querySelector(`[data-field="${e.target.dataset.triggerField}"]`);
  const rect = trigger.getBoundingClientRect();
  e.target.style.left = Math.max(8, Math.min(rect.left, innerWidth - 232)) + "px";
  e.target.style.top = (rect.bottom + 6) + "px";
}, true);
document.addEventListener("toggle", (e) => {
  if (!e.target.hasAttribute?.("data-filter-menu")) return;
  const trigger = document.querySelector(`[data-field="${e.target.dataset.triggerField}"]`);
  trigger?.setAttribute("aria-expanded", String(e.newState === "open"));
  if (e.newState === "open") e.target.querySelector('[aria-checked="true"]')?.focus({ preventScroll: true });
}, true);
document.addEventListener("keydown", (e) => {
  if (e.isComposing || composingInputs.size || e.keyCode === 229) return;
  const trigger = e.target.closest?.("[popovertarget]");
  const menu = e.target.closest?.("[data-filter-menu]");
  if (trigger && ["ArrowDown", "ArrowUp"].includes(e.key)) {
    e.preventDefault();
    const popup = document.getElementById(trigger.getAttribute("popovertarget"));
    popup.showPopover();
    const options = [...popup.querySelectorAll('[role="menuitemradio"]')];
    (e.key === "ArrowUp" ? options.at(-1) : options[0])?.focus();
    return;
  }
  if (menu) {
    const options = [...menu.querySelectorAll('[role="menuitemradio"]')];
    const index = options.indexOf(document.activeElement);
    if (["ArrowDown", "ArrowUp", "Home", "End"].includes(e.key)) {
      e.preventDefault();
      const next = e.key === "Home" ? 0 : e.key === "End" ? options.length - 1 : (index + (e.key === "ArrowDown" ? 1 : -1) + options.length) % options.length;
      options[next]?.focus();
    } else if (["Escape", "Tab"].includes(e.key)) {
      if (e.key === "Escape") e.preventDefault();
      menu.hidePopover();
      document.querySelector(`[data-field="${menu.dataset.triggerField}"]`)?.focus({ preventScroll: true });
    }
    return;
  }
  if (e.key === "Escape" && ui.modal) {
    ui.modal = null;
    render();
  }
  if (e.key === "Tab" && ui.modal) {
    const nodes = [
      ...$("#modal-root").querySelectorAll(
        "button:not(:disabled),input:not(:disabled),textarea,select",
      ),
    ];
    const first = nodes[0],
      last = nodes.at(-1);
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last?.focus({ preventScroll: true });
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first?.focus({ preventScroll: true });
    }
  }
});
$("#review-toggle").onclick = () => {
  $("#review").hidden = !$("#review").hidden;
};
$("#review").innerHTML =
  '<label>가상 상황 <select id="scenario"><option value="live">녹음 중</option><option value="paused">일시정지</option><option value="final">최종 전사</option><option value="source-failed">자료 일부 실패</option><option value="disconnected">자료 연결 해제</option><option value="uncertain">Notion 결과 불확실</option></select></label><button id="scenario-apply">상황 적용</button><span class="tiny muted">제품 화면 밖의 검토 도구</span><button id="demo-transcript">대화 예시 추가</button><button id="reset-demo">예시 초기화</button>';
$("#scenario-apply").onclick = () => {
  const v = $("#scenario").value;
  if (["live", "paused", "final"].includes(v) && globalThis.NeroDesktopController) {
    globalThis.NeroDesktopController.open({ live: "recording", paused: "paused", final: "finished" }[v]);
    return;
  }
  ui.modal = null;
  if (["live", "paused", "final"].includes(v)) {
    s.page = "meeting";
    s.tab = "transcript";
    s.phase = { live: "recording", paused: "paused", final: "finished" }[v];
    if (v === "final") M.finishMeeting(s);
  }
  if (v === "source-failed") {
    s.page = "sources";
    s.sourceStatus.venue = "failed";
    s.selectedDoc = "venue";
  }
  if (v === "disconnected") {
    s.page = "sources";
    s.connected = false;
  }
  if (v === "uncertain") {
    s.page = "meeting";
    s.tab = "summary";
    if (s.saveState === "saved") {
      s.saveState = "idle";
      s.receipt = null;
    }
    M.beginSave(s);
  }
  render();
};
$("#demo-transcript").onclick = () => {
  toast(
    M.addDemoTranscript(s)
      ? "녹음 중인 회의에 대화 예시를 추가했습니다."
      : "먼저 녹음을 시작해 주세요.",
  );
  render();
};
$("#reset-demo").onclick = () => {
  s = initialState();
  ui.modal = null;
  ui.search = "";
  ui.sourceFilter = "all";
  ui.selectedGroup = "all";
  ui.librarySearch = "";
  ui.libraryStatus = "all";
  ui.libraryMonth = "all";
  ui.librarySort = "newest";
  ui.libraryPage = 0;
  ui.raw = false;
  ui.edit = null;
  render();
  toast("초기화됨");
};
render();
for (const input of document.querySelectorAll('[data-field="recent-order"],[data-field="recent-limit"]')) input.checked = String(navigation[input.dataset.field === "recent-order" ? "order" : "limit"]) === input.value;
globalThis.NeroDesktopController?.attach({
  model: M,
  getState: () => s,
  setState: (state) => { s = state; },
  begin: beginDesktopPreview,
  end: endDesktopPreview,
  render: () => render(),
  hasModal: () => !!ui.modal,
  clearModal: () => { ui.modal = null; ui.edit = null; },
});
