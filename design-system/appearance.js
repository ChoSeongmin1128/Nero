/* Appearance never reads, writes or renders meeting state. Loaded before paint. */
(() => {
  const windowsDesign = new URLSearchParams(location.search).get("platform") === "windows" && new URLSearchParams(location.search).get("session") === "windows-design";
  const nativeKey = document.documentElement.dataset.appearanceStorage;
  const key = nativeKey || (windowsDesign ? "nero-prototype-v3-windows-design-appearance" : "nero-design-system-v1-appearance");
  const colors = new Set(["green"]);
  const modes = new Set(["light", "dark", "system"]);
  const legacy = {
    nero: { color: "green", mode: "light" },
    slate: { color: "blue", mode: "light" },
    forest: { color: "green", mode: "dark" },
    graphite: { color: "purple", mode: "dark" },
    system: { color: "green", mode: "system" },
  };
  function decode(saved) {
    if (Object.hasOwn(legacy, saved)) return { ...legacy[saved], color: "green" };
    try {
      const value = JSON.parse(saved);
      if (modes.has(value?.mode))
        return { color: "green", mode: value.mode };
    } catch {}
    return { color: "green", mode: nativeKey ? "system" : "light" };
  }
  const media = window.matchMedia("(prefers-color-scheme: dark)");
  let preference = decode(null);
  try {
    preference = decode(localStorage.getItem(key));
  } catch {
    // Browsers that block storage can still switch for this page session.
  }
  function apply() {
    const root = document.documentElement;
    root.setAttribute("data-appearance-changing", "");
    const mode =
      preference.mode === "system"
        ? media.matches
          ? "dark"
          : "light"
        : preference.mode;
    root.dataset.color = preference.color;
    root.dataset.mode = mode;
    root.dataset.appearance = `${preference.color}-${mode}`;
    root.dataset.appearancePreference = `${preference.color}-${preference.mode}`;
    for (const axis of ["color", "mode"]) {
      document
        .querySelectorAll(`input[name="appearance-${axis}"]`)
        .forEach((input) => {
          input.checked = input.value === preference[axis];
        });
    }
    document.querySelectorAll("[data-theme-preview]").forEach((preview) => {
      preview.dataset.mode = mode;
    });
    // Commit the new palette without animating through mismatched text/surfaces.
    // Restore ordinary hover transitions only after this style update is resolved.
    void root.offsetWidth;
    root.removeAttribute("data-appearance-changing");
  }
  window.NeroAppearance = { apply };
  apply();
  media.addEventListener("change", () => {
    if (preference.mode === "system") apply();
  });
  window.addEventListener("storage", (event) => {
    if (event.key !== key && event.key !== null) return;
    preference = decode(event.newValue);
    apply();
  });
  document.addEventListener("DOMContentLoaded", () => {
    apply();
    const dialog = document.getElementById("settings-dialog");
    if (!dialog) return;
    let opener = null;
    document.addEventListener("click", (event) => {
      const trigger = event.target.closest("[data-open-settings]");
      if (!trigger || dialog.open) return;
      opener = trigger;
      dialog.showModal();
      dialog
        .querySelector('input[name="appearance-mode"]:checked')
        ?.focus({ preventScroll: true });
    });
    dialog.addEventListener("change", (event) => {
      const input = event.target;
      const axis =
        input.name === "appearance-color"
          ? "color"
          : input.name === "appearance-mode"
            ? "mode"
            : null;
      if (!axis || !(axis === "color" ? colors : modes).has(input.value))
        return;
      preference = { ...preference, [axis]: input.value };
      apply();
      try {
        localStorage.setItem(key, JSON.stringify(preference));
      } catch {
        // Keep the chosen appearance usable when persistence is unavailable.
      }
    });
    if (nativeKey) return;
    document
      .getElementById("settings-close")
      .addEventListener("click", () => dialog.close());
    dialog.addEventListener("close", () => {
      const target = opener?.isConnected
        ? opener
        : document.querySelector("[data-open-settings]");
      target?.focus({ preventScroll: true });
      opener = null;
    });
  });
})();
