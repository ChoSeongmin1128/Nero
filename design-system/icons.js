(function (root) {
  const shapes = {
    "undo-2": "<path d=\"M9 14 4 9l5-5\"/><path d=\"M4 9h10.5a5.5 5.5 0 0 1 5.5 5.5a5.5 5.5 0 0 1-5.5 5.5H11\"/>",
    "folder-input": "<path d=\"M2 9V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H20a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2v-1\"/><path d=\"M2 13h10\"/><path d=\"m9 16 3-3-3-3\"/>",

    "volume-x": "<path d=\"M11 4.702a.7.7 0 0 0-1.203-.498L6.413 7.587A1.4 1.4 0 0 1 5.416 8H3a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2.416a1.4 1.4 0 0 1 .997.413l3.383 3.384A.7.7 0 0 0 11 19.298z\" />\n  <path d=\"m16.5 14.5 5-5\" />\n  <path d=\"m16.5 9.5 5 5\" />",
    "volume-2": "<path d=\"M11 4.702a.705.705 0 0 0-1.203-.498L6.413 7.587A1.4 1.4 0 0 1 5.416 8H3a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2.416a1.4 1.4 0 0 1 .997.413l3.383 3.384A.705.705 0 0 0 11 19.298z\" />\n  <path d=\"M16 9a5 5 0 0 1 0 6\" />\n  <path d=\"M19.364 18.364a9 9 0 0 0 0-12.728\" />",
    "arrow-left": '<path d="m12 19-7-7 7-7" />\n  <path d="M19 12H5" />',
    "arrow-right": '<path d="M5 12h14" />\n  <path d="m12 5 7 7-7 7" />',
    "arrow-up-down":
      '<path d="m21 16-4 4-4-4" />\n  <path d="M17 20V4" />\n  <path d="m3 8 4-4 4 4" />\n  <path d="M7 4v16" />',
    "audio-lines":
      '<path d="M2 10v3" />\n  <path d="M6 6v11" />\n  <path d="M10 3v18" />\n  <path d="M14 8v7" />\n  <path d="M18 5v13" />\n  <path d="M22 10v3" />',
    "book-open":
      '<path d="M12 5v16" />\n  <path d="M20.001 19A2 2 0 0022 17V5a2 2 0 00-1.999-2L16 3.002A5 5 0 0012 5a5 5 0 00-4-2H4a2 2 0 00-2 2v12a2 2 0 001.999 2H8a5 5 0 014 2 5 5 0 014-2z" />',
    "calendar-check":
      '<path d="M8 2v3" />\n  <path d="M16 2v3" />\n  <rect x="3" y="3" width="18" height="18" rx="2" />\n  <path d="M3 9h18" />\n  <path d="m9 15 2 2 4-4" />',
    "calendar-days":
      '<path d="M8 2v3" />\n  <path d="M16 2v3" />\n  <rect x="3" y="3" width="18" height="18" rx="2" />\n  <path d="M3 9h18" />\n  <path d="M8 13h.01" />\n  <path d="M12 13h.01" />\n  <path d="M16 13h.01" />\n  <path d="M8 17h.01" />\n  <path d="M12 17h.01" />\n  <path d="M16 17h.01" />',
    "check-check":
      '<path d="M18 6 7 17l-5-5" />\n  <path d="m22 10-7.5 7.5L13 16" />',
    check: '<path d="M20 6 9 17l-5-5" />',
    "chevron-down": '<path d="m6 9 6 6 6-6" />',
    "chevron-left": '<path d="m15 18-6-6 6-6" />',
    "chevron-right": '<path d="m9 18 6-6-6-6" />',
    "chevron-up": '<path d="m18 15-6-6-6 6" />',
    "chevrons-left":
      '<path d="m11 17-5-5 5-5" />\n  <path d="m18 17-5-5 5-5" />',
    "chevrons-right":
      '<path d="m6 17 5-5-5-5" />\n  <path d="m13 17 5-5-5-5" />',
    "circle-check":
      '<circle cx="12" cy="12" r="10" />\n  <path d="m16 9-5.5 5.5L8 12" />',
    "circle-pause":
      '<circle cx="12" cy="12" r="10" />\n  <line x1="10" x2="10" y1="15" y2="9" />\n  <line x1="14" x2="14" y1="15" y2="9" />',
    circle: '<circle cx="12" cy="12" r="10" />',
    clock: '<circle cx="12" cy="12" r="10" />\n  <path d="M12 6v6l4 2" />',
    copy: '<rect width="14" height="14" x="8" y="8" rx="2" ry="2" />\n  <path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2" />',
    download:
      '<path d="M12 15V3" />\n  <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />\n  <path d="m7 10 5 5 5-5" />',
    ellipsis:
      '<circle cx="12" cy="12" r="1" />\n  <circle cx="19" cy="12" r="1" />\n  <circle cx="5" cy="12" r="1" />',
    "external-link":
      '<path d="M15 3h6v6" />\n  <path d="M10 14 21 3" />\n  <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" />',
    "file-text":
      '<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z" />\n  <path d="M14 2v5a1 1 0 0 0 1 1h5" />\n  <path d="M10 9H8" />\n  <path d="M16 13H8" />\n  <path d="M16 17H8" />',
    folder: '<path d="M20 20H4a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h5l2 2h9a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2Z"/>',
    inbox: '<path d="m4 4-2 10v6h20v-6L20 4Z"/><path d="M2 14h6l2 3h4l2-3h6"/>',
    "trash-2": '<path d="M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7M14 10v7"/>',
    "folder-open":
      '<path d="m6 14 1.5-2.9A2 2 0 0 1 9.24 10H20a2 2 0 0 1 1.94 2.5l-1.54 6a2 2 0 0 1-1.95 1.5H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H18a2 2 0 0 1 2 2v2" />',
    headphones:
      '<path d="M3 14h3a2 2 0 0 1 2 2v3a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-7a9 9 0 0 1 18 0v7a2 2 0 0 1-2 2h-1a2 2 0 0 1-2-2v-3a2 2 0 0 1 2-2h3" />',
    link: '<path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71" />\n  <path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71" />',
    "list-filter":
      '<path d="M2 5h20" />\n  <path d="M6 12h12" />\n  <path d="M9 19h6" />',
    list: '<path d="M3 5h.01" />\n  <path d="M3 12h.01" />\n  <path d="M3 19h.01" />\n  <path d="M8 5h13" />\n  <path d="M8 12h13" />\n  <path d="M8 19h13" />',
    "messages-square":
      '<path d="M16 10a2 2 0 0 1-2 2H6.828a2 2 0 0 0-1.414.586l-2.202 2.202A.71.71 0 0 1 2 14.286V4a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2z" />\n  <path d="M20 9a2 2 0 0 1 2 2v10.286a.71.71 0 0 1-1.212.502l-2.202-2.202A2 2 0 0 0 17.172 19H10a2 2 0 0 1-2-2v-1" />',
    mic: '<path d="M12 19v3" />\n  <path d="M19 10v2a7 7 0 0 1-14 0v-2" />\n  <rect x="9" y="2" width="6" height="13" rx="3" />',
    "notebook-pen":
      '<path d="M13.4 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-7.4" />\n  <path d="M2 6h4" />\n  <path d="M2 10h4" />\n  <path d="M2 14h4" />\n  <path d="M2 18h4" />\n  <path d="M21.378 5.626a1 1 0 1 0-3.004-3.004l-5.01 5.012a2 2 0 0 0-.506.854l-.837 2.87a.5.5 0 0 0 .62.62l2.87-.837a2 2 0 0 0 .854-.506z" />',
    "notebook-tabs":
      '<path d="M2 6h4" />\n  <path d="M2 10h4" />\n  <path d="M2 14h4" />\n  <path d="M2 18h4" />\n  <rect width="16" height="20" x="4" y="2" rx="2" />\n  <path d="M15 2v20" />\n  <path d="M15 7h5" />\n  <path d="M15 12h5" />\n  <path d="M15 17h5" />',
    "panel-left": '<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M9 3v18"/>',
    "panel-left-close":
      '<rect width="18" height="18" x="3" y="3" rx="2" />\n  <path d="M9 3v18" />\n  <path d="m16 15-3-3 3-3" />',
    pause:
      '<rect x="14" y="3" width="5" height="18" rx="1" />\n  <rect x="5" y="3" width="5" height="18" rx="1" />',
    pencil:
      '<path d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z" />\n  <path d="m15 5 4 4" />',
    play: '<path d="M5 5a2 2 0 0 1 3.008-1.728l11.997 6.998a2 2 0 0 1 .003 3.458l-12 7A2 2 0 0 1 5 19z" />',
    plus: '<path d="M5 12h14" />\n  <path d="M12 5v14" />',
    "refresh-cw":
      '<path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8" />\n  <path d="M21 3v5h-5" />\n  <path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16" />\n  <path d="M8 16H3v5" />',
    search: '<path d="m21 21-4.34-4.34" />\n  <circle cx="11" cy="11" r="8" />',
    send: '<path d="M14.536 21.686a.5.5 0 0 0 .937-.024l6.5-19a.496.496 0 0 0-.635-.635l-19 6.5a.5.5 0 0 0-.024.937l7.93 3.18a2 2 0 0 1 1.112 1.11z" />\n  <path d="m21.854 2.147-10.94 10.939" />',
    settings:
      '<path d="M9.671 4.136a2.34 2.34 0 0 1 4.659 0 2.34 2.34 0 0 0 3.319 1.915 2.34 2.34 0 0 1 2.33 4.033 2.34 2.34 0 0 0 0 3.831 2.34 2.34 0 0 1-2.33 4.033 2.34 2.34 0 0 0-3.319 1.915 2.34 2.34 0 0 1-4.659 0 2.34 2.34 0 0 0-3.32-1.915 2.34 2.34 0 0 1-2.33-4.033 2.34 2.34 0 0 0 0-3.831A2.34 2.34 0 0 1 6.35 6.051a2.34 2.34 0 0 0 3.319-1.915" />\n  <circle cx="12" cy="12" r="3" />',
    square: '<rect width="18" height="18" x="3" y="3" rx="2" />',
    "triangle-alert":
      '<path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3" />\n  <path d="M12 9v4" />\n  <path d="M12 17h.01" />',
    upload:
      '<path d="M12 3v12" />\n  <path d="m17 8-5-5-5 5" />\n  <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />',
    users:
      '<path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2" />\n  <path d="M16 3.128a4 4 0 0 1 0 7.744" />\n  <path d="M22 21v-2a4 4 0 0 0-3-3.87" />\n  <circle cx="9" cy="7" r="4" />',
    x: '<path d="M18 6 6 18" />\n  <path d="m6 6 12 12" />',
  };
  const aliases = {
    meeting: "notebook-tabs",
    sources: "folder-open",
    people: "users",
    close: "x",
    arrow: "arrow-right",
    ask: "messages-square",
    doc: "file-text",
    calendar: "calendar-days",
    filter: "list-filter",
    "more-horizontal": "ellipsis",
  };
  root.GroveIcons = (name) => {
    const id = aliases[name] || name;
    return `<svg class="lucide lucide-${id}" viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">${shapes[id] || shapes["file-text"]}</svg>`;
  };
})(typeof globalThis === "object" ? globalThis : this);
