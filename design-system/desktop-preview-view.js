(function (root) {
  const escape = (value) => String(value ?? "").replace(/[&<>"']/g, (character) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
  })[character]);
  const icon = (name) => root.NeroIcons ? root.NeroIcons(name) : "";
  const button = (action, label, options = {}) => `<button type="button" data-action="desktop:${action}"${options.value ? ` data-value="${escape(options.value)}"` : ""} class="desktop-button ${options.className || ""}"${options.pressed !== undefined ? ` aria-pressed="${options.pressed}"` : ""}${options.label ? ` aria-label="${escape(options.label)}" title="${escape(options.label)}"` : ""}${options.expanded !== undefined ? ` aria-expanded="${options.expanded}" aria-controls="${escape(options.controls || "desktop-menubar-panel")}"` : ""}${options.disabled ? " disabled" : ""}>${options.icon ? icon(options.icon) : ""}${label ? `<span>${label}</span>` : ""}</button>`;
  const isActive = (capture) => ["recording", "paused"].includes(capture.phase);
  const phaseLabel = (phase) => ({ idle: "녹음 대기", recording: "녹음 중", paused: "일시정지", finished: "녹음 종료", error: "녹음 중단" })[phase] || "녹음 대기";
  const status = (capture, error) => `<span class="desktop-capture-state is-${escape(error ? "error" : capture.phase)}"><span class="desktop-state-dot" aria-hidden="true"></span>${error === "permission" ? "마이크 접근 필요" : error ? "녹음 중단" : phaseLabel(capture.phase)}</span>`;
  const timer = (capture) => `<time class="desktop-timer" data-desktop-timer>${escape(capture.elapsedLabel || "00:00")}</time>`;

  function sceneContent(scene, meeting) {
    if (scene === "app") return "";
    if (scene === "call") return `<div class="desktop-work-window desktop-call-window" aria-label="화상회의 화면 예시">
      <div class="desktop-window-chrome"><span class="desktop-window-dots" aria-hidden="true"><i></i><i></i><i></i></span><span>화상회의</span><span class="desktop-window-context">연결됨</span></div>
      <div class="desktop-call-heading"><span>진행 중인 통화</span><strong>${escape(meeting.title)}</strong></div>
      <div class="desktop-call-grid">${[1, 2, 3, 4].map((number) => `<div class="desktop-call-tile"><span class="desktop-call-avatar" aria-hidden="true">${icon("users")}</span><span class="desktop-call-name">참석자 ${number}</span></div>`).join("")}</div>
      <div class="desktop-call-footer"><span>${icon("mic")} 마이크 켜짐</span><span>카메라 꺼짐</span><span class="desktop-context-example">다른 앱 화면 예시</span></div>
    </div>`;
    return "";
  }

  function reminder(view) {
    const { meeting, capture, notification } = view;
    if (!["upcoming", "finished"].includes(notification)) return "";
    const finished = notification === "finished";
    const title = finished ? capture.title || meeting.title : meeting.title;
    const conflicting = !finished && view.otherRecording;
    const blockedPreparation = !finished && !conflicting && !!view.error;
    const titleDisabled = conflicting || blockedPreparation;
    const titleAction = finished ? "open-app" : "start";
    return `<section class="desktop-notification" data-desktop-title-preview aria-label="${finished ? "녹음 완료 알림" : "다가오는 회의 알림"}"><button type="button" class="desktop-title-trigger" data-desktop-title-trigger data-action="desktop:${titleAction}" aria-describedby="desktop-full-meeting-title" aria-label="${escape(title)}${titleDisabled ? "" : " 열기"}"${titleDisabled ? " disabled" : ""}><span class="desktop-notice-symbol" aria-hidden="true">${icon(finished ? "circle-check" : "calendar-days")}</span><span class="desktop-notice-title">${escape(title)}</span></button><span class="desktop-reminder-time">${escape(finished ? "녹음 종료" : meeting.countdownLabel || "곧 시작")}</span><div class="desktop-notice-actions">${conflicting ? button("open-app", "녹음 중인 회의", { className: "is-quiet" }) : ""}${!finished ? button("snooze-reminder", "", { icon: "clock", label: "5분 뒤 알림", className: "is-icon is-quiet" }) : ""}${button("dismiss-reminder", "", { icon: "x", label: "알림 닫기", className: "is-icon is-quiet" })}</div><span class="desktop-title-tooltip" data-desktop-title-tooltip><span class="desktop-full-title" id="desktop-full-meeting-title" role="tooltip">${escape(title)}</span></span></section>`;
  }

  function captureActions(capture) {
    return `${button(capture.phase === "paused" ? "resume" : "pause", capture.phase === "paused" ? "녹음 계속" : "일시정지", { icon: capture.phase === "paused" ? "play" : "pause" })}${button("stop", "종료", { icon: "square", className: "is-stop" })}`;
  }

  function recordingControl(view) {
    const { capture, control, error } = view;
    if (control === "hidden" || (view.scene === "app" && !view.showControlInApp) || (!isActive(capture) && !error)) return "";
    const overlay = view.overlay || { placement: "notch", expanded: control === "expanded" };
    const floating = overlay.placement === "floating";
    const expanded = !!overlay.expanded || !!error;
    const title = capture.title || view.meeting.title;
    const drag = `<div class="desktop-overlay-grip" data-overlay-drag title="드래그해 ${floating ? "이동" : "분리"}"><span class="desktop-overlay-state is-${escape(error ? "error" : capture.phase)}" aria-label="${escape(error ? "녹음 중단" : phaseLabel(capture.phase))}"><span class="desktop-state-dot" aria-hidden="true"></span></span>${timer(capture)}</div>`;
    const controls = !error ? `<div class="desktop-overlay-controls"><div class="desktop-overlay-primary" id="desktop-overlay-detail"${expanded ? "" : " hidden"}>${button(capture.phase === "paused" ? "resume" : "pause", "", { icon: capture.phase === "paused" ? "play" : "pause", label: capture.phase === "paused" ? "녹음 계속" : "일시정지", className: "is-icon is-quiet" })}${button("stop", "", { icon: "square", label: "녹음 종료", className: "is-icon is-quiet is-stop" })}${button("toggle-menu", "", { value: "overlay", icon: "ellipsis", label: "녹음 메뉴", expanded: view.menuOpen && view.menuOrigin === "overlay", className: "is-icon is-quiet" })}</div>${button("toggle-expanded", "", { icon: expanded ? "chevron-up" : "chevron-down", label: expanded ? "녹음 제어기 줄이기" : "녹음 제어기 펼치기", expanded, controls: "desktop-overlay-detail", className: "is-icon is-quiet" })}</div>` : "";
    const recovery = error ? `<div class="desktop-overlay-detail" id="desktop-overlay-detail"><button type="button" data-action="desktop:open-app" class="desktop-recorder-title" title="${escape(title)}" aria-label="${escape(title)} 열기"><span>${escape(title)}</span></button><div class="desktop-recorder-error" role="status"><strong>${error === "permission" ? "마이크 접근이 꺼져 있습니다" : "마이크 연결이 끊겼습니다"}</strong><p>${error === "permission" ? "접근을 허용한 뒤 다시 시도해주세요." : "지금까지 녹음한 내용은 유지됩니다."}</p><div class="desktop-recovery-actions">${button("retry", error === "permission" ? "마이크 접근 허용" : "다시 연결", { className: "is-primary" })}${button("hide-control", "", { icon: "x", label: "녹음 제어기 숨기기", className: "is-icon is-quiet" })}</div></div></div>` : "";
    return `<section class="desktop-recorder is-${floating ? "floating" : "notch"}${expanded ? " is-expanded" : " is-compact"}${error ? " has-error" : ""}" data-desktop-overlay aria-label="${floating ? "부유 녹음 제어기" : "노치 녹음 제어기"}"><div class="desktop-overlay-row">${drag}${controls}</div>${recovery}</section>`;
  }

  function menubarPanel(view) {
    if (!view.menuOpen) return "";
    const { capture } = view;
    const meeting = { ...view.meeting, title: capture.title || view.meeting.title };
    const canControl = isActive(capture) || view.error;
    const floating = view.overlay?.placement === "floating";
    const hidden = view.control === "hidden" || (view.scene === "app" && !view.showControlInApp);
    return `<section class="desktop-menubar-panel${view.menuOrigin === "overlay" ? " is-overlay-origin" : ""}" id="desktop-menubar-panel" aria-label="Nero 메뉴"><div class="desktop-menu-heading"><strong>Nero</strong>${status(capture, view.error)}</div><p class="desktop-menu-meeting">${escape(meeting.title)}</p>${canControl ? `<p class="desktop-menu-source">${icon("mic")}<span>${escape(capture.microphoneLabel || "기본 마이크")}</span></p>` : ""}${view.error ? button("retry", view.error === "permission" ? "마이크 접근 허용" : "마이크 다시 연결", { icon: "refresh-cw" }) : isActive(capture) ? `<div class="desktop-menu-time">녹음 시간 ${timer(capture)}</div><div class="desktop-menu-actions">${captureActions(capture)}</div>` : button("start", "회의 준비", { icon: "arrow-right" })}<div class="desktop-menu-links">${button("open-app", canControl ? "회의 열기" : "Nero 열기", { icon: "arrow-right", className: "is-quiet" })}${canControl ? `${button(floating ? "dock" : "detach", floating ? "노치에 붙이기" : "부유창으로 분리", { icon: floating ? "chevron-up" : "external-link", className: "is-quiet" })}${button(hidden ? "restore-control" : "hide-control", hidden ? "녹음 제어기 보기" : "녹음 제어기 숨기기", { icon: hidden ? "audio-lines" : "x", className: "is-quiet" })}` : ""}</div></section>`;
  }

  function stopConfirmation(view) {
    if (!view.confirmStop) return "";
    return `<div class="desktop-stop-shield"><section class="desktop-stop-dialog" id="desktop-stop-dialog" role="alertdialog" aria-modal="true" aria-labelledby="desktop-stop-title" aria-describedby="desktop-stop-detail" tabindex="-1"><span class="desktop-stop-icon">${icon("square")}</span><h2 id="desktop-stop-title">녹음을 종료할까요?</h2><p class="desktop-stop-meeting">${escape(view.capture.title || view.meeting.title)}</p><p id="desktop-stop-detail">녹음 시간 ${escape(view.capture.elapsedLabel || "00:00")}</p><div class="desktop-stop-actions">${button("cancel-stop", "돌아가기")}${button("confirm-stop", "녹음 종료", { className: "is-primary" })}</div></section></div>`;
  }

  function render(input) {
    if (!input || !input.open) return "";
    const view = { scene: "document", notification: null, control: "expanded", menuOpen: false, confirmStop: false, error: null, ...input, meeting: { title: "새 회의", ...(input.meeting || {}) }, capture: { phase: "idle", elapsedLabel: "00:00", ...(input.capture || {}) } };
    const phase = view.capture.phase;
    const scenario = view.otherRecording ? "other-recording" : view.error ? view.error : view.notification === "upcoming" ? "upcoming" : phase;
    return `<section class="desktop-preview" id="desktop-preview-panel"><header class="desktop-preview-header"><div class="desktop-preview-heading"><h1 id="desktop-preview-title">데스크톱 미리보기</h1><span class="desktop-preview-badge">시연</span></div>${button("close", "시연 닫기", { icon: "arrow-left", className: "is-quiet" })}</header><div class="desktop-preview-toolbar"><div class="desktop-tool-group" role="group" aria-label="데스크톱 상황"><span class="desktop-tool-label">상황</span>${[["upcoming", "회의 알림"], ["recording", "녹음 중"], ["paused", "일시정지"], ["permission", "마이크 권한"], ["mic-disconnected", "마이크 끊김"], ["other-recording", "다른 회의 녹음"], ["finished", "종료 후"]].map(([value, label]) => button("scenario", label, { value, pressed: scenario === value, className: "is-scenario" })).join("")}${button("advance", "5분 경과", { value: "300", icon: "clock", className: "is-quiet" })}</div><div class="desktop-tool-group desktop-title-case-picker" role="group" aria-label="회의 제목 예시"><span class="desktop-tool-label">제목 예시</span>${[["upcoming", "기본 제목"], ["long-title", "긴 제목"], ["unbroken-title", "공백 없는 제목"]].map(([value, label]) => button("scenario", label, { value, pressed: value === "upcoming" ? !view.exampleCase : view.exampleCase === value, className: "is-scenario" })).join("")}</div><div class="desktop-tool-group desktop-scene-picker" role="group" aria-label="배경 화면"><span class="desktop-tool-label">배경</span>${button("scene", "데스크톱", { value: "document", pressed: view.scene === "document", className: "is-scenario" })}${button("scene", "화상회의", { value: "call", pressed: view.scene === "call", className: "is-scenario" })}${button("scene", "Nero", { value: "app", pressed: view.scene === "app", className: "is-scenario" })}</div></div><div class="desktop-stage" data-desktop-phase="${escape(phase)}"><div class="desktop-menubar"><strong class="desktop-menubar-app">${view.scene === "app" ? "Nero" : view.scene === "call" ? "화상회의" : "데스크톱"}</strong><div class="desktop-menubar-items" aria-hidden="true"><span>파일</span><span>편집</span><span>보기</span></div><div class="desktop-menubar-status">${button("toggle-menu", `${isActive(view.capture) ? `<span class="desktop-menu-dot is-${escape(phase)}"></span>${escape(view.capture.elapsedLabel)}` : "Nero"}`, { icon: "audio-lines", label: "Nero 메뉴", expanded: view.menuOpen, className: "desktop-menu-trigger is-quiet" })}<span class="desktop-menubar-clock"><span>시안</span> ${escape(input.clockLabel || "")}</span></div></div><div class="desktop-workspace">${sceneContent(view.scene, view.meeting)}<div class="desktop-dock" aria-label="Nero 앱 바로가기">${button("open-app", "Nero", { icon: "audio-lines", className: "is-dock" })}</div></div><div class="desktop-dock-target" data-desktop-dock-target aria-hidden="true"><span>노치에 붙이기</span></div>${reminder(view)}${menubarPanel(view)}${recordingControl(view)}${view.notification === "snoozed" ? '<div class="desktop-reminder-status" role="status">5분 뒤 알림</div>' : ""}${stopConfirmation(view)}</div><footer class="desktop-preview-footer"><span>브라우저 안의 데스크톱 시연</span><span>마이크와 알림 권한을 사용하지 않습니다.</span></footer></section>`;
  }

  root.NeroDesktopView = { render };
})(typeof globalThis === "object" ? globalThis : this);
