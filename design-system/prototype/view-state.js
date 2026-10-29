(function (root) {
  function createViewState(doc) {
    const scroll = new Map(),
      details = new Map();
    let focused = null,
      modal = null,
      opener = null;
    const all = (selector) => [...doc.querySelectorAll(selector)];
    const owner = (el) => el?.closest?.("[data-view-state]")?.dataset.viewState;
    const descriptor = (el) => {
      if (!el) return null;
      const field = el.dataset?.field,
        action = el.dataset?.action;
      if (!field && !action) return null;
      let selection = null;
      try {
        if (typeof el.selectionStart === "number")
          selection = [
            el.selectionStart,
            el.selectionEnd,
            el.selectionDirection,
          ];
      } catch {}
      return {
        field,
        action,
        owner: owner(el),
        dialog: !!el.closest?.('[role="dialog"]'),
        selection,
      };
    };
    const fieldKey = (el) => `${owner(el)}|field|${el.dataset.field}`;
    const detailKey = (el, index) =>
      `${owner(el)}|details|${el.id || el.className || index}`;
    const firstModalControl = () =>
      doc.querySelector(
        "#modal-root input:not(:disabled),#modal-root textarea:not(:disabled),#modal-root select:not(:disabled)",
      ) || doc.querySelector("#modal-root button:not(:disabled)");
    function find(ref) {
      if (!ref) return null;
      const matches = all(ref.field ? "[data-field]" : "[data-action]").filter(
        (el) =>
          (ref.field
            ? el.dataset.field === ref.field
            : el.dataset.action === ref.action) &&
          !!el.closest?.('[role="dialog"]') === ref.dialog &&
          !el.disabled,
      );
      return (
        matches.find((el) => owner(el) === ref.owner) ||
        (!ref.field ? matches[0] : null)
      );
    }
    function capture() {
      for (const el of all("[data-view-state]"))
        scroll.set(el.dataset.viewState, [el.scrollTop, el.scrollLeft]);
      for (const el of all("textarea[data-field],input[data-field]"))
        scroll.set(fieldKey(el), [el.scrollTop, el.scrollLeft]);
      all("details").forEach((el, i) => details.set(detailKey(el, i), el.open));
      focused = descriptor(doc.activeElement);
    }
    function restore(bindings, nextModal = null) {
      for (const [selector, key] of bindings) {
        const elements =
          selector === "$document" ? [doc.scrollingElement] : all(selector);
        for (const el of elements)
          if (el)
            el.dataset.viewState = typeof key === "function" ? key(el) : key;
      }
      all("details").forEach((el, i) => {
        const key = detailKey(el, i);
        if (details.has(key)) el.open = details.get(key);
      });
      let target = null,
        selection = null;
      if (nextModal !== modal) {
        if (nextModal) {
          if (!modal) opener = focused;
          target = firstModalControl();
        } else target = find(opener);
      } else {
        target = find(focused);
        selection = focused?.selection;
        if (!target && focused?.action?.startsWith("edit:"))
          target = doc.querySelector('[data-field="edit-text"]');
      }
      if (nextModal && !target?.closest?.('[role="dialog"]')) {
        target = firstModalControl();
        selection = null;
      }
      target?.focus({ preventScroll: true });
      if (selection && target?.setSelectionRange) {
        try {
          target.setSelectionRange(...selection);
        } catch {}
      }
      for (const el of all("[data-view-state]")) {
        const position = scroll.get(el.dataset.viewState) || [0, 0];
        el.scrollTop = position[0];
        el.scrollLeft = position[1];
      }
      for (const el of all("textarea[data-field],input[data-field]")) {
        const position = scroll.get(fieldKey(el));
        if (position) {
          el.scrollTop = position[0];
          el.scrollLeft = position[1];
        }
      }
      modal = nextModal;
    }
    function forget(prefix) {
      for (const key of scroll.keys())
        if (key.startsWith(prefix)) scroll.delete(key);
      for (const key of details.keys())
        if (key.startsWith(prefix)) details.delete(key);
    }
    return { capture, restore, forget };
  }
  if (typeof module === "object" && module.exports)
    module.exports = createViewState;
  else root.NeroViewState = createViewState;
})(typeof globalThis === "object" ? globalThis : this);
