# Desktop application updates

This describes the native rewrite, not the archived Swift updater. Both supported
targets use Tauri's official updater, pinned to `2.14.1`. Apple Developer ID and
notarization establish macOS distribution trust; the separate updater signature
authenticates downloaded packages on both operating systems. Windows Authenticode
is independent of this signature and is not configured by this change.

## Runtime policy

- The shared UI controller checks 45 seconds after initialization, then every six
  hours. An available update downloads automatically. Installation requires the
  user's restart action, after the consuming screen has saved all open editors.
  Settings can also invoke a manual check, download or installation retry.
- Native commands are restricted to the main window. No frontend permission grants
  direct access to the updater plugin's download/install commands. The UI cannot
  supply a signing key, endpoint, destination or installer arguments.
- The running executable embeds its product version and independent build number
  from `tauri.conf.json`. Feed/signature versions use `<product>+<build>`, e.g.
  `0.0.1+41`. Builds are canonical positive `u64` integers, compared exactly;
  JavaScript receives build strings. A candidate must increase the build number
  and must not decrease the product version. Ordinary development retains `0.0.1`.
- Both official verification (`requireSignedVersion`) and the shared policy verify
  package bytes and the authenticated trusted-comment version. The latter checks
  the entire literal version, including build metadata. An unsigned manifest
  cannot relabel a valid old package as a newer build. Missing signed versions,
  malformed builds and payload tampering are rejected.
- The feed is pinned to the existing public Nero GitHub repository's
  `releases/latest/download/latest.json`. Initial artifact URLs must be HTTPS assets
  under that repository's `releases/download/`, without credentials/query strings.
  Downloads follow bounded HTTPS redirects, have a 15-minute timeout and a 512 MiB
  cap. The public key is embedded; private keys and passwords never enter Git or
  the app. No repository, release or feed is published by the application/tooling.
- The verified package and raw manifest occupy one app-cache slot. Restoring it
  uses the running application's trusted updater configuration, then re-verifies
  the exact signature/build before offering installation. Interrupted/invalid
  downloads are never installed. A fresh check remains available.

## Installation and failure handling

The UI's `saveEdits` callback must finish or explicitly refuse all open drafts before
the native command runs. Confirmation dialogs remain interactive while this callback
runs. Existing save ownership locks the editors while their saves drain; after the
callback succeeds, `setInstalling` locks app interaction until the native command
returns or the app restarts. A failed or declined save prevents installation.

Native installation serializes update operations, marks installation in progress,
and atomically fences new processing/capture admissions using the existing worker
lock. Running work, pending jobs, capture or live speech cause a refusal without
cancellation. `CoreHandle.prepare_shutdown(None)` then checks capture ownership on
the actor and fences all later requests, including calls queued by earlier IPC or
agent clients. Only snapshot/shutdown preparation/failure recovery bypass this
actor gate. This avoids a check-then-start race. The native gate is authoritative;
browser state alone is not an idle check.

The application writes a small pending-install receipt outside the meeting library
and calls the official installer with the already verified bytes. It never copies
or replaces the user's meeting/audio/model directory. If preparation or installer
launch fails, processing/core admission is reopened and the downloaded package is
retained for retry. The plugin's destructive pre-exit window cleanup is disabled
so a Windows launcher error leaves the current app usable.

On macOS, the pinned plugin stages on the application's filesystem and uses an
app-only rename/swap with restoration on swap failure, then Nero restarts. A system
authorization prompt can still be required by filesystem ownership. Normal UI
permission requests are not added by the controller.

On Windows, the official updater launches the current-user NSIS installer in
passive mode, exits and asks NSIS to relaunch Nero. The existing Restart Manager
guard never force-closes an application. `/UPDATE` has a bounded five-second grace
period for the old process to exit naturally; an active app or an unverifiable
process state still aborts. Manual installation/uninstallation retain their
immediate running-app guard. User data removal is not part of updating.

NSIS is not a transactional package manager. A successful installer launch is not
proof that every later installation step succeeded. The receipt is checked on
the next app launch: the exact target reports completion, the previous version
reports an incomplete update with retry, and an unexpected version is reported
separately. If the executable cannot start after an external interruption, run the
retained verified installer manually. Whole-install rollback and power-loss
acceptance are separate from the tested refusal/launch-error paths; do not claim
they have been established by unit tests.

## Shared frontend integration

`src/app-updater.js` exports `createAppUpdater` and `appUpdateAction`. Supply the
existing native `invoke`/`listen`, an awaited `saveEdits` callback, `onChange(status)`
and `setInstalling(boolean)`. Call `start()` once after the app is initialized and
`dispose()` on teardown. Settings/theme markup belongs to the consuming screen
and canonical shared design system, not this controller.

`app-update-view.js` connects this controller to the desktop bridge and the General
settings row. `main.js` starts it after initialization and supplies
`prepareAppLeave(() => true)` as the shared leave/save guard. The view owns the
installation interaction lock; main owns its contribution to `state.editLocks`.
Available updates download automatically after both scheduled and manual checks.
Installation is only invoked by the explicit restart action in settings or the
notification center. Ready updates and recovery messages use the `app-update`
service in the common notice collection, separately from integration notices.

Commands: `app_update_status`, `app_update_check`, `app_update_download`,
`app_update_install`. Status events use `nero://app-update`. Public status fields:
`phase`, `currentVersion`, `currentBuild`, `availableVersion`, `availableBuild`,
`downloadedBytes`, `totalBytes`, `downloadReady`, `message`, `recoveryMessage`.
Phases: idle, checking, current, available, downloading, ready, waiting,
installing, error. Waiting means work must finish before another user install
action; it does not cancel work or install silently later.

## Preparing a release without publishing

1. Build each platform from the same clean source commit and final version/build.
   Keep the actual packaged `build-info.json`, native build and signing receipts.
   Use an NSIS `*-setup.exe` on Windows and a signed/notarized `.app.tar.gz` on Mac.
   The existing DMG remains the manual Mac installer; it is not an updater package.
2. Sign the final bytes on the designated signing machine using the dedicated
   Nero updater key. Run the installed Tauri CLI `signer sign` with
   `--app-version 0.0.1+<build>`, `--private-key-path <private-vault-key>` and the
   package path. Supply its password through the existing private signing process.
   Do not expose it in terminal output, command history, Git, this document or app
   configuration. Automatic `tauri build` signing binds only the product version;
   re-signing with the independent build is required. Windows can transfer its
   final installer to the signing machine; it does not need the private key.
3. Run the native executable's headless verifier:
   `nero-desktop --nero-verify-app-update <package> <package.sig> 0.0.1+<build>`.
   On macOS use the executable inside `Nero.app/Contents/MacOS/`. Success prints
   the authenticated payload SHA256 and exits zero. This mode opens no library,
   models, window or network connection. It uses the production embedded key.
4. Generate a local combined feed with
   `node apps/desktop/scripts/app-update-feed.mjs --version 0.0.1 --build <build>
   --tag <release-tag> --verifier <native-executable> --mac <app.tar.gz>
   --mac-provenance <mac-build-info.json> --windows <setup.exe>
   --windows-provenance <windows-build-info.json> --out <new-folder/latest.json>`.
   Each signature must be beside its package as `<package>.sig`. The tool requires
   both supported platforms, matching clean provenance and the native verifier's
   authenticated hash. It creates the feed plus a local provenance receipt and
   refuses overwrites. It does not upload anything or include local paths/tokens
   in the public feed.
5. Publication of the feed/assets to a GitHub release is a separate explicit
   action. The release's actual tag must match the generated asset URLs. Keep the
   latest-release policy consistent with the pinned `/releases/latest/` endpoint.
   Validate actual delivery/installation separately when publication is authorized.

Installed build40 and earlier have no updater. Install an updater-enabled build
once through the manual installer before this channel can update that machine.
An absent/unpublished GitHub feed must not be described as an operational public
update channel.

## Validation boundaries

Tests cover exact ordering, actual synthetic Minisign signatures, signed-version
relabeling, missing signed versions, tampered bytes/comments, bounded cache reads,
cache replacement, incomplete-install receipts, actor admission/recovery,
processing refusal without cancellation, saved-editor ordering, duplicate clicks,
automatic download without auto-install, and local feed provenance/verification
failures. Windows guard tests compile and execute real NSIS helpers against
synthetic processes/files, including natural exit and timeout preservation.

These are not production signing/publishing, a real Windows-to-Mac acceptance,
power-loss rollback, microphone/model quality or OS authorization acceptance.
Physical-device acceptance is optional by the current user's instruction; record
what was performed without relabeling unperformed checks as release blockers.

Primary references: [Tauri updater guide](https://v2.tauri.app/plugin/updater/),
[pinned updater source](https://docs.rs/tauri-plugin-updater/2.14.1/src/tauri_plugin_updater/updater.rs.html).
