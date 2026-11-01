Generated with `npm run prepare:icons` in `apps/desktop`.

- Source: `../../brand/nero-cat.png`, the approved application artwork.
- Outputs: `icon.icns`, `icon.ico` and the 512px `icon.png`.
- Preserve the original artwork; resize and encode it without adding a background,
  redrawing, cropping or combining it with the interface logo.

No separate menu-bar or system-tray icon is generated.

Windows clips the ICO's outer corners to transparency. The icon generator also
regenerates ../windows/shortcut-icon-resource.nsh with its SHA-256 filename.
NSIS packages that exact ICO, preserves existing owned shortcuts and points them
to this file, so a replaced EXE cannot reuse Explorer's cached older artwork.
The versioned icon manifest retires the previous owned resource on update;
uninstallation removes the packaged icon and its matching manifest.
