# Nero UI primitives

`select.jsx`, `popover.jsx`, `dialog.jsx`, `calendar.jsx` adapt the MIT shadcn/ui Base UI registry at commit `96da9cc7f1aa10580a3741664e2a77c721206c00` (2026-10-28).

Upstream source: https://github.com/shadcn-ui/ui/tree/96da9cc7f1aa10580a3741664e2a77c721206c00/apps/v4/registry/bases/base/ui

`provenance.json` records original file hashes. `LICENSE.md` preserves the upstream MIT notice. Structural parts, `data-slot` naming, Base UI composition and Calendar ownership follow the original. Nero replaces the Tailwind style strings and configurable icon placeholders with its shared semantic CSS tokens and Lucide icons, adds the native-dialog portal container, and uses Korean calendar labels. React DayPicker owns calendar navigation rather than Nero reimplementing date-grid keyboard behavior.

The application imports the canonical React entry through Vite and shares its React runtime with React feature dialogs. `build-controls.mjs` emits the same source as a standalone bundle for the static component catalog and prototypes. Do not edit generated controls.js under `.generated` or `.work`.

The imperative `GroveControls` adapter alone owns native select/date enhancement, event forwarding and cleanup. Each React root owns only its dedicated host, never the original native form element or the consuming screen state.
