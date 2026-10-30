# Nero brand assets

`nero-cat.png` is the approved application icon artwork. Preserve its composition,
background and colors. Desktop icons use this PNG directly, with only the resizing
and native icon encoding provided by the pinned Tauri CLI.

`logo/` contains the transparent cat symbol and Nero lettering for application
surfaces, including light and dark variants. These SVG assets are separate from
the application icon artwork.

Run `npm run prepare:icons` from `apps/desktop` to regenerate the checked-in
ICNS, ICO and 512px PNG. No additional image package is required. The application
does not create a separate macOS menu-bar or Windows system-tray icon.
