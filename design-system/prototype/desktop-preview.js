(function (root) {
  let adapter, engine, timer, opened = false, opener, placements = [], outside = [], showControlInApp = false;
  let pinned = false, approachBlocked = false, menuOrigin = "menubar", menuOpener, collapseTimer, pointer = { x: -1, y: -1 }, drag = null, suppressClickUntil = 0, stopOpener, resizeObserver;
  const element = (id) => document.getElementById(id);
  const overlay = () => element("desktop-view")?.querySelector("[data-desktop-overlay]");
  const stage = () => element("desktop-view")?.querySelector(".desktop-stage");
  const clamp = (number, minimum, maximum) => Math.max(minimum, Math.min(maximum, number));
  const uiActions = new Set(["expand", "collapse", "toggle-expanded", "toggle-control", "detach", "move", "dock", "hide-control", "restore-control", "toggle-menu", "stop", "cancel-stop", "dismiss-reminder", "snooze-reminder"]);
  function focusable(container) {
    return [...container.querySelectorAll('button:not(:disabled),input:not(:disabled),textarea,select,[tabindex="0"]')].filter((node) => node.getClientRects().length && !node.closest("[hidden]"));
  }
  function focusRef(node) {
    if (node?.matches("[data-desktop-title-trigger]")) return { title: true };
    return node?.dataset?.action ? { action: node.dataset.action, value: node.dataset.value, group: node.closest('[role="group"]')?.getAttribute("aria-label"), overlay: !!node.closest("[data-desktop-overlay]"), menu: !!node.closest("#desktop-menubar-panel") } : null;
  }
  function restoreFocus(ref) {
    if (!ref) return;
    if (ref.title) {
      const target = element("desktop-view").querySelector("[data-desktop-title-preview]:not([hidden]) [data-desktop-title-trigger]") || element("desktop-root");
      const preview = target.closest("[data-desktop-title-preview]"), dismissed = preview?.dataset.titleDismissed;
      target.focus({ preventScroll: true });
      if (dismissed) preview.dataset.titleDismissed = dismissed;
      return;
    }
    const nodes = [...element("desktop-root").querySelectorAll("[data-action]")].filter((node) => !node.disabled && node.getClientRects().length);
    const match = (name) => nodes.find((node) => node.dataset.action === name && node.dataset.value === ref.value && (!ref.group || node.closest('[role="group"]')?.getAttribute("aria-label") === ref.group) && !!node.closest("[data-desktop-overlay]") === ref.overlay);
    const alternate = ({ "desktop:pause": "desktop:resume", "desktop:resume": "desktop:pause", "desktop:detach": "desktop:dock", "desktop:dock": "desktop:detach" })[ref.action] || ref.action;
    const overlayTrigger = nodes.find((node) => node.closest("[data-desktop-overlay]") && ["desktop:toggle-menu", "desktop:toggle-expanded"].includes(node.dataset.action));
    const menuTrigger = nodes.find((node) => !node.closest("[data-desktop-overlay]") && node.dataset.action === "desktop:toggle-menu");
    const fallback = ref.overlay || (ref.menu && menuOrigin === "overlay") ? overlayTrigger || menuTrigger : ref.menu && menuTrigger;
    (match(ref.action) || match(alternate) || fallback || element("desktop-root")).focus({ preventScroll: true });
  }
  // Preserve the stage and overlay identity during hover and pointer capture.
  function patchView(markup) {
    const host = element("desktop-view"), template = document.createElement("template");
    template.innerHTML = markup;
    const current = host.querySelector(".desktop-preview"), incoming = template.content.firstElementChild;
    if (!current) { host.replaceChildren(incoming); return; }
    [".desktop-preview-header", ".desktop-preview-toolbar", ".desktop-preview-footer"].forEach((selector) => current.querySelector(selector).replaceWith(incoming.querySelector(selector)));
    const currentStage = stage(), incomingStage = incoming.querySelector(".desktop-stage");
    currentStage.dataset.desktopPhase = incomingStage.dataset.desktopPhase;
    const previousOverlay = overlay(), nextOverlay = incomingStage.querySelector("[data-desktop-overlay]");
    if (previousOverlay && nextOverlay) {
      previousOverlay.className = nextOverlay.className;
      previousOverlay.setAttribute("aria-label", nextOverlay.getAttribute("aria-label"));
      if (previousOverlay.innerHTML !== nextOverlay.innerHTML) previousOverlay.innerHTML = nextOverlay.innerHTML;
    }
    [...currentStage.children].forEach((node) => { if (node !== previousOverlay) node.remove(); });
    [...incomingStage.children].forEach((node) => {
      if (!(previousOverlay && node === nextOverlay)) currentStage.append(node);
    });
    if (previousOverlay && !nextOverlay) previousOverlay.remove();
  }
  function travel(node = overlay()) {
    const container = stage();
    if (!container || !node) return null;
    return { left: 8, top: 41, width: Math.max(0, container.clientWidth - node.offsetWidth - 16), height: Math.max(0, container.clientHeight - node.offsetHeight - 49) };
  }
  function layoutOverlay() {
    const node = overlay();
    if (!opened || !node || drag?.moved) return;
    const info = engine.snapshot().overlay;
    if (info.placement !== "floating") { node.style.left = node.style.top = node.style.transform = ""; return; }
    const bounds = travel(node), position = info.position || { x: .75, y: .2 };
    node.style.left = bounds.left + bounds.width * position.x + "px";
    node.style.top = bounds.top + bounds.height * position.y + "px";
    node.style.transform = "none";
  }
  function layoutPanels() {
    layoutOverlay();
    const panel = element("desktop-menubar-panel"), anchor = overlay(), container = stage();
    if (opened && container) {
      const preview = container.querySelector("[data-desktop-title-preview]");
      if (preview) {
        const title = preview.querySelector(".desktop-notice-title");
        const truncated = title.scrollWidth > title.clientWidth;
        preview.dataset.titleTruncated = String(truncated);
        const trigger = preview.querySelector("[data-desktop-title-trigger]");
        if (truncated) trigger.setAttribute("aria-describedby", "desktop-full-meeting-title");
        else trigger.removeAttribute("aria-describedby");
      }
      const offset = container.getBoundingClientRect().top - element("desktop-root").getBoundingClientRect().top;
      element("desktop-app-host").style.top = offset + container.querySelector(".desktop-menubar").offsetHeight + 16 + "px";
    }
    if (!opened || !panel?.classList.contains("is-overlay-origin") || !anchor || !container) return;
    panel.style.maxHeight = Math.max(80, container.clientHeight - 16) + "px";
    panel.style.overflowY = "auto";
    const bounds = container.getBoundingClientRect(), rect = anchor.getBoundingClientRect();
    const left = clamp(rect.left - bounds.left + (rect.width - panel.offsetWidth) / 2, 8, Math.max(8, container.clientWidth - panel.offsetWidth - 8));
    const below = rect.bottom - bounds.top + 6, above = rect.top - bounds.top - panel.offsetHeight - 6;
    panel.style.left = left + "px";
    panel.style.right = "auto";
    panel.style.top = clamp(below + panel.offsetHeight <= container.clientHeight - 8 ? below : above, 8, Math.max(8, container.clientHeight - panel.offsetHeight - 8)) + "px";
  }
  function draw() {
    if (!opened || drag?.moved) return;
    const active = document.activeElement, ref = active?.closest("#desktop-view") ? focusRef(active) : null;
    const wasConfirm = !!element("desktop-stop-dialog"), view = { ...engine.snapshot(), showControlInApp, menuOrigin };
    const previousTitle = element("desktop-view").querySelector("[data-desktop-title-preview]");
    const titleState = previousTitle && { key: previousTitle.dataset.titleKey, dismissed: previousTitle.dataset.titleDismissed, hovered: previousTitle.dataset.titleHovered, scroll: previousTitle.querySelector("#desktop-full-meeting-title")?.scrollTop || 0 };
    patchView(root.NeroDesktopView.render(view));
    const titlePreview = element("desktop-view").querySelector("[data-desktop-title-preview]");
    if (titlePreview) {
      const meeting = view.notification === "finished" ? view.capture : view.meeting;
      titlePreview.dataset.titleKey = JSON.stringify([view.notification, meeting.id, meeting.title]);
      if (titleState?.key === titlePreview.dataset.titleKey) {
        if (titleState.dismissed) titlePreview.dataset.titleDismissed = titleState.dismissed;
        if (titleState.hovered === "true" && titlePreview.matches(":hover")) titlePreview.dataset.titleHovered = "true";
        const body = titlePreview.querySelector("#desktop-full-meeting-title");
        if (body) body.scrollTop = titleState.scroll;
      }
    }
    element("desktop-app-host").hidden = view.scene !== "app";
    element("desktop-root").classList.toggle("desktop-modal-active", adapter.hasModal() || !!element("settings-dialog")?.open);
    resizeObserver?.disconnect();
    if (overlay()) resizeObserver?.observe(overlay());
    layoutPanels();
    if (view.confirmStop && !wasConfirm) element("desktop-stop-dialog").querySelector('[data-action="desktop:cancel-stop"]').focus({ preventScroll: true });
    else if (ref && !active.isConnected) restoreFocus(ref);
    refreshClock(view);
  }
  function refreshClock(view) {
    element("desktop-root").querySelectorAll("[data-desktop-timer]").forEach((node) => { node.textContent = view.capture.elapsedLabel; });
    const time = element("desktop-root").querySelector(".recordingbar .time");
    if (time) time.textContent = view.capture.elapsedLabel;
    const menuTime = element("desktop-root").querySelector(".desktop-menu-trigger > span");
    if (menuTime?.querySelector(".desktop-menu-dot") && menuTime.lastChild) menuTime.lastChild.textContent = view.capture.elapsedLabel;
    const clock = element("desktop-root").querySelector(".desktop-menubar-clock");
    if (clock) clock.textContent = "시안 " + (view.clockLabel || "");
    const countdown = element("desktop-root").querySelector(".desktop-reminder-time");
    if (countdown && view.notification !== "finished" && view.meeting.countdownLabel) countdown.textContent = view.meeting.countdownLabel;
  }
  function isOverOverlay() {
    const node = overlay();
    return !!node && node.contains(document.elementFromPoint(pointer.x, pointer.y));
  }
  function scheduleCollapse() {
    clearTimeout(collapseTimer);
    collapseTimer = setTimeout(() => {
      if (!opened || !overlay() || pinned || drag || engine.state.confirmStop || engine.state.error || (engine.state.menuOpen && menuOrigin === "overlay") || isOverOverlay() || overlay().contains(document.activeElement)) return;
      engine.apply("collapse"); draw();
    }, 180);
  }
  function expandOnApproach() {
    clearTimeout(collapseTimer);
    if (!opened || engine.state.control === "hidden" || drag || approachBlocked || engine.state.confirmStop || engine.state.overlay.expanded) return;
    engine.apply("expand"); draw();
  }
  function open(scenario) {
    if (!adapter) return;
    if (opened) { if (scenario) action("scenario", scenario); return; }
    opener = document.activeElement;
    engine = root.NeroDesktopModel.create(adapter.model, { now: Date.now() });
    if (scenario) engine.apply("scenario", scenario);
    adapter.begin(engine.store);
    const host = element("desktop-app-host");
    placements = ["app", "modal-root", "settings-dialog", "toast"].map((id) => {
      const node = element(id), marker = document.createComment("desktop-preview:" + id);
      node.before(marker); host.append(node);
      return { node, marker };
    });
    outside = [...document.body.children].filter((node) => node !== element("desktop-root") && node.tagName !== "SCRIPT").map((node) => ({ node, inert: node.inert }));
    outside.forEach(({ node }) => { node.inert = true; });
    opened = true; pinned = false; approachBlocked = false; menuOrigin = "menubar"; menuOpener = null; showControlInApp = false;
    element("desktop-root").hidden = false;
    document.body.classList.add("desktop-preview-active");
    adapter.render();
    element("desktop-root").focus({ preventScroll: true });
    timer = setInterval(() => {
      if (!opened) return;
      engine.sync(adapter.getState());
      const before = engine.snapshot().notification, next = engine.advance(1);
      if (before !== next.notification) draw(); else refreshClock(next);
    }, 1000);
  }
  function clearDrag() {
    const current = drag;
    drag = null;
    if (current && element("desktop-root").hasPointerCapture(current.id)) element("desktop-root").releasePointerCapture(current.id);
    stage()?.classList.remove("is-overlay-dragging");
    overlay()?.classList.remove("is-dragging", "is-dock-target");
    stage()?.querySelector("[data-desktop-dock-target]")?.classList.remove("is-dock-target");
    return current;
  }
  function cancelDrag() {
    const current = clearDrag();
    if (!current) return;
    suppressClickUntil = performance.now() + 250;
    draw();
  }
  function close() {
    if (!opened) return;
    clearTimeout(collapseTimer); cancelDrag(); clearInterval(timer); resizeObserver?.disconnect();
    opened = false;
    element("settings-dialog")?.close();
    placements.forEach(({ node, marker }) => marker.replaceWith(node));
    outside.forEach(({ node, inert }) => { node.inert = inert; });
    element("desktop-view").innerHTML = "";
    element("desktop-app-host").hidden = true;
    element("desktop-root").hidden = true;
    document.body.classList.remove("desktop-preview-active");
    adapter.end(); opener?.focus({ preventScroll: true }); engine = null;
  }
  function action(name, value) {
    if (name === "open") { open(); return; }
    if (!opened) return;
    if (name === "close") { close(); return; }
    const scenarioFocus = name === "scenario" ? focusRef(document.activeElement) : null;
    if (name !== "advance") {
      const titlePreview = element("desktop-view").querySelector("[data-desktop-title-preview]");
      if (titlePreview) titlePreview.dataset.titleDismissed = "true";
    }
    cancelDrag(); clearTimeout(collapseTimer);
    engine.sync(adapter.getState());
    const wasMenuOpen = engine.state.menuOpen;
    if (name === "toggle-menu") {
      const nextOrigin = value === "overlay" ? "overlay" : "menubar";
      if (!wasMenuOpen) menuOpener = focusRef(document.activeElement);
      if (wasMenuOpen && menuOrigin !== nextOrigin && (value || document.activeElement.dataset.action === "desktop:toggle-menu")) {
        menuOpener = focusRef(document.activeElement); menuOrigin = nextOrigin;
        draw(); focusable(element("desktop-menubar-panel"))[0]?.focus({ preventScroll: true }); return;
      }
      menuOrigin = wasMenuOpen ? menuOrigin : nextOrigin;
    } else if (wasMenuOpen) engine.state.menuOpen = false;
    if (name === "stop") stopOpener = focusRef(document.activeElement);
    if (["scenario", "dock", "collapse", "hide-control"].includes(name)) pinned = false;
    if (["dock", "collapse"].includes(name)) approachBlocked = true;
    if (["scenario", "expand", "restore-control", "detach"].includes(name)) approachBlocked = false;
    if (["expand", "restore-control", "detach"].includes(name)) pinned = true;
    if (["toggle-expanded", "toggle-control"].includes(name)) { pinned = !engine.state.overlay.expanded; approachBlocked = !pinned; }
    if (name === "detach" && !value) value = { x: .75, y: .2 };
    if (name === "hide-app") {
      engine.apply("scene", "document"); showControlInApp = false;
    } else {
      if (["scenario", "scene", "open-app", "start"].includes(name)) showControlInApp = false;
      if (["restore-control", "detach"].includes(name)) showControlInApp = true;
      engine.apply(name, value);
    }
    if (["scenario", "open-app", "start", "retry", "hide-app"].includes(name)) adapter.clearModal();
    if (uiActions.has(name)) draw();
    else { adapter.setState(engine.store); adapter.render(); }
    if (scenarioFocus) restoreFocus(scenarioFocus);
    if (name === "toggle-menu") {
      if (engine.state.menuOpen) focusable(element("desktop-menubar-panel"))[0]?.focus({ preventScroll: true });
      else { restoreFocus(menuOpener); scheduleCollapse(); }
    }
    if (name === "cancel-stop") { restoreFocus(stopOpener); stopOpener = null; scheduleCollapse(); }
  }
  function sync() {
    if (!opened) return;
    engine.sync(adapter.getState());
    if (drag && drag.owner !== engine.snapshot().capture.id) cancelDrag();
    draw();
  }
  function beforeRecordingStart() {
    if (!opened) return true;
    if (engine.state.error) {
      engine.apply("expand"); showControlInApp = true; draw(); return false;
    }
    return true;
  }
  function moveDrag(event) {
    pointer = { x: event.clientX, y: event.clientY };
    if (!drag || event.pointerId !== drag.id) return;
    const node = overlay(), container = stage();
    if (!node || !container) { cancelDrag(); return; }
    if (!drag.moved && Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY) < 6) return;
    event.preventDefault();
    if (!drag.moved) {
      drag.moved = true;
      node.classList.remove("is-notch"); node.classList.add("is-floating", "is-dragging");
      node.style.transform = "none";
      container.classList.add("is-overlay-dragging");
    }
    const bounds = travel(node), rect = container.getBoundingClientRect();
    const left = clamp(event.clientX - rect.left - drag.offsetX, bounds.left, bounds.left + bounds.width);
    const top = clamp(event.clientY - rect.top - drag.offsetY, bounds.top, bounds.top + bounds.height);
    node.style.left = left + "px"; node.style.top = top + "px";
    drag.position = { x: bounds.width ? (left - bounds.left) / bounds.width : 0, y: bounds.height ? (top - bounds.top) / bounds.height : 0 };
    const target = container.querySelector("[data-desktop-dock-target]"), targetRect = target.getBoundingClientRect();
    drag.dock = event.clientX >= targetRect.left && event.clientX <= targetRect.right && event.clientY >= targetRect.top && event.clientY <= targetRect.bottom;
    node.classList.toggle("is-dock-target", drag.dock); target.classList.toggle("is-dock-target", drag.dock);
  }
  function endDrag(event) {
    if (!drag || event.pointerId !== drag.id) return;
    const current = clearDrag();
    if (current.moved) {
      event.preventDefault(); suppressClickUntil = performance.now() + 250;
      pinned = !current.dock;
      approachBlocked = !!current.dock;
      engine.apply(current.dock ? "dock" : "detach", current.position);
      draw();
    } else { pinned = true; engine.apply("expand"); draw(); }
  }
  function attach(value) {
    adapter = value;
    resizeObserver = typeof ResizeObserver === "function" ? new ResizeObserver(layoutPanels) : null;
    const desktop = element("desktop-root");
    desktop.addEventListener("pointerover", (event) => {
      pointer = { x: event.clientX, y: event.clientY };
      const titlePreview = event.target.closest("[data-desktop-title-preview]");
      const titleTrigger = event.target.closest("[data-desktop-title-trigger]");
      if (titlePreview && titleTrigger && !titleTrigger.contains(event.relatedTarget)) {
        titlePreview.dataset.titleHovered = "true";
        delete titlePreview.dataset.titleDismissed;
      }
      const node = event.target.closest("[data-desktop-overlay]");
      if (node && !node.contains(event.relatedTarget)) expandOnApproach();
    });
    desktop.addEventListener("pointerout", (event) => {
      pointer = { x: event.clientX, y: event.clientY };
      const titlePreview = event.target.closest("[data-desktop-title-preview]");
      if (titlePreview && !titlePreview.contains(event.relatedTarget)) delete titlePreview.dataset.titleHovered;
      if (event.target.closest("[data-desktop-overlay]") && !overlay()?.contains(event.relatedTarget)) {
        const rect = overlay()?.getBoundingClientRect();
        if (!rect || event.clientX < rect.left || event.clientX > rect.right || event.clientY < rect.top || event.clientY > rect.bottom) approachBlocked = false;
        scheduleCollapse();
      }
    });
    desktop.addEventListener("focusin", (event) => {
      const titlePreview = event.target.closest("[data-desktop-title-preview]");
      if (titlePreview && event.target.matches("[data-desktop-title-trigger]")) delete titlePreview.dataset.titleDismissed;
    });
    desktop.addEventListener("focusout", (event) => { if (event.target.closest("[data-desktop-overlay]")) scheduleCollapse(); });
    desktop.addEventListener("pointerdown", (event) => {
      if (!opened || event.button !== 0 || !event.isPrimary || engine.state.confirmStop || adapter.hasModal() || element("settings-dialog")?.open) return;
      const handle = event.target.closest("[data-overlay-drag]");
      if (!handle || event.target.closest("button,input,a")) return;
      clearTimeout(collapseTimer);
      if (engine.state.menuOpen) { engine.apply("toggle-menu"); draw(); }
      const node = overlay(), rect = node.getBoundingClientRect();
      drag = { id: event.pointerId, startX: event.clientX, startY: event.clientY, offsetX: event.clientX - rect.left, offsetY: event.clientY - rect.top, moved: false, owner: engine.snapshot().capture.id };
      desktop.setPointerCapture(event.pointerId); event.preventDefault();
    });
    desktop.addEventListener("pointermove", moveDrag);
    desktop.addEventListener("pointerup", endDrag);
    desktop.addEventListener("pointercancel", cancelDrag);
    desktop.addEventListener("lostpointercapture", () => { if (drag) cancelDrag(); });
    desktop.addEventListener("click", (event) => {
      const titlePreview = desktop.querySelector("[data-desktop-title-preview]");
      if (titlePreview && !titlePreview.contains(event.target)) titlePreview.dataset.titleDismissed = "true";
      if (event.detail > 0 && performance.now() < suppressClickUntil) { event.preventDefault(); event.stopImmediatePropagation(); }
      else if (opened && engine.state.menuOpen && !event.target.closest('[data-action^="desktop:"],#desktop-menubar-panel')) {
        engine.apply("toggle-menu");
        queueMicrotask(() => {
          if (!opened || engine.state.menuOpen) return;
          if (element("desktop-menubar-panel")) draw();
          scheduleCollapse();
        });
      }
    }, true);
    desktop.addEventListener("transitionend", (event) => { if (event.target.matches("[data-desktop-overlay]")) layoutPanels(); });
    window.addEventListener("blur", () => { if (opened) { pointer = { x: -1, y: -1 }; cancelDrag(); scheduleCollapse(); } });
    window.addEventListener("resize", layoutPanels);
    document.addEventListener("keydown", (event) => {
      if (!opened || event.isComposing || event.keyCode === 229 || adapter.hasModal() || element("settings-dialog")?.open || root.NeroControls?.isOpen()) return;
      if (event.key === "Escape") {
        event.preventDefault(); event.stopImmediatePropagation();
        const titlePreview = desktop.querySelector("[data-desktop-title-preview]");
        const tooltip = titlePreview?.querySelector("[data-desktop-title-tooltip]");
        if (tooltip && getComputedStyle(tooltip).visibility !== "hidden" && tooltip.getClientRects().length) titlePreview.dataset.titleDismissed = "true";
        else if (drag) cancelDrag();
        else if (engine.state.confirmStop) action("cancel-stop");
        else if (engine.state.menuOpen) action("toggle-menu", menuOrigin);
        else if (engine.state.overlay.expanded && !engine.state.error) { action("collapse"); element("desktop-root").focus({ preventScroll: true }); }
        else close();
      } else if (event.key === "Tab") {
        const container = element("desktop-stop-dialog") || element("desktop-root"), nodes = focusable(container);
        const first = nodes[0], last = nodes.at(-1);
        if (!nodes.length) { event.preventDefault(); return; }
        if (event.shiftKey && (document.activeElement === first || document.activeElement === container || !container.contains(document.activeElement))) { event.preventDefault(); last.focus(); }
        else if (!event.shiftKey && (document.activeElement === last || !container.contains(document.activeElement) || document.activeElement === container)) { event.preventDefault(); first.focus(); }
      }
    }, true);
  }
  root.NeroDesktopController = { attach, open, close, action, sync, beforeRecordingStart, elapsed: (id) => opened && engine.snapshot().capture.id === id ? engine.snapshot().capture.elapsedLabel : "00:00" };
})(typeof globalThis === "object" ? globalThis : this);
