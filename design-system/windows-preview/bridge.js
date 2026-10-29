(function (root) {
  const parameters = new URLSearchParams(location.search);
  if (parameters.get("platform") !== "windows" || parameters.get("session") !== "windows-design") return;
  document.body.classList.add("windows-design");
  let adapter, engine, timer;
  let appearance = { color: "green", mode: "light" };
  const media = matchMedia("(prefers-color-scheme: dark)");
  const scenarios = { upcoming: "upcoming", recording: "recording", paused: "paused", error: "mic-disconnected", permission: "permission", finished: "finished", long: "long-title", unbroken: "unbroken-title" };
  const send = (message) => { if (parent !== root) parent.postMessage(message, location.origin); };
  function applyAppearance(value) {
    if (!["green", "blue", "purple", "mono", "amber", "rose"].includes(value?.color) || !["light", "dark", "system"].includes(value?.mode)) return;
    appearance = { color: value.color, mode: value.mode };
    const mode = appearance.mode === "system" ? media.matches ? "dark" : "light" : appearance.mode;
    document.documentElement.dataset.color = appearance.color;
    document.documentElement.dataset.mode = mode;
    document.documentElement.dataset.appearance = appearance.color + "-" + mode;
  }
  function emit() {
    if (!engine) return;
    const view = engine.snapshot();
    document.querySelectorAll(".recordingbar .time").forEach((node) => { node.textContent = view.capture.elapsedLabel; });
    send({ type: "windows-nero:state", phase: view.capture.phase, title: view.capture.title,
      elapsedLabel: view.capture.elapsedLabel, ownerId: view.capture.id,
      error: view.error, confirmStop: view.confirmStop, notification: view.notification,
      countdownLabel: view.meeting.countdownLabel, color: document.documentElement.dataset.color,
      mode: document.documentElement.dataset.mode });
  }
  function update() {
    adapter.setState(engine.store);
    adapter.render();
  }
  function requestStop() {
    const view = engine.snapshot();
    if (!["recording", "paused"].includes(view.capture.phase)) return;
    send({ type: "windows-nero:request-stop", ownerId: view.capture.id, title: view.capture.title });
  }
  function command(message) {
    if (!engine) return;
    engine.sync(adapter.getState());
    if (message.action === "appearance") { applyAppearance(message); emit(); return; }
    if (message.action === "scenario") {
      if (!Object.hasOwn(scenarios, message.value)) return;
      engine.apply("scenario", scenarios[message.value]);
      adapter.clearModal();
    } else if (message.action === "prepare") engine.apply("start");
    else if (message.action === "snooze") engine.apply("snooze-reminder");
    else if (message.action === "dismiss") engine.apply("dismiss-reminder");
    else if (message.action === "advance") engine.advance(300);
    else if (message.action === "start") {
      if (!engine.state.error) adapter.model.beginRecording(engine.store);
      engine.sync(engine.store);
    } else if (["pause", "resume", "retry"].includes(message.action)) engine.apply(message.action);
    else if (message.action === "open") engine.apply("open-app");
    else if (message.action === "stop") {
      const current = engine.snapshot().capture;
      if (!message.ownerId || message.ownerId !== current.id || !["recording", "paused"].includes(current.phase)) { emit(); return; }
      engine.apply("stop"); engine.apply("confirm-stop");
    } else return;
    update();
  }
  root.NeroDesktopController = {
    attach(value) {
      adapter = value;
      engine = root.NeroDesktopModel.create(adapter.model, { now: Date.now() });
      adapter.begin(engine.store);
      engine.apply("start");
      update();
      send({ type: "windows-nero:ready" });
      timer = setInterval(() => { engine.sync(adapter.getState()); engine.advance(1); emit(); }, 1000);
    },
    sync() { if (engine) { engine.sync(adapter.getState()); emit(); } },
    action(name) {
      if (!engine) return;
      engine.sync(adapter.getState());
      if (name === "stop") requestStop();
      else if (["pause", "resume", "open-app", "retry"].includes(name)) { engine.apply(name); update(); }
    },
    beforeRecordingStart: () => !!engine && !engine.state.error,
    elapsed: (id) => engine?.snapshot().capture.id === id ? engine.snapshot().capture.elapsedLabel : "00:00",
    open(scenario) { command({ action: "scenario", value: scenario || "upcoming" }); },
    close() {},
  };
  window.addEventListener("message", (event) => {
    if (event.source !== parent || event.origin !== location.origin || event.data?.type !== "windows-nero:command") return;
    command(event.data);
  });
  document.addEventListener("keydown", (event) => {
    if (event.isComposing || event.keyCode === 229 || !event.ctrlKey || !event.shiftKey || event.altKey || event.metaKey || event.key.toLowerCase() !== "g") return;
    event.preventDefault();
    send({ type: "windows-nero:open-window" });
  }, true);
  media.addEventListener("change", () => { if (appearance.mode === "system") applyAppearance(appearance); });
  window.addEventListener("pagehide", () => clearInterval(timer));
})(window);
