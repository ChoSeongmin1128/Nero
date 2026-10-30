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
the native command runs. During this interval the consuming screen must honor
`setInstalling` and stop new user edits. A failed save prevents installation.

Native installation serializes update operations, marks installation in progress,
and atomically fences new processing/capture admissions using the existing worker
lock. Running work, pending jobs, capture or live speech cause a refusal without
cancellation. `CoreHandle.prepare_shutdown(None)` then checks capture ownership on
the actor and fences all later requests, including calls queued by earlier IPC or
agent clients. Only snapshot/shutdown preparation/failure recovery bypass this
actor gate. This avoids a check-then-start race. The native gate is authoritative;
browser state alone is not an idle check.

Normal quit and update installation also share one process-lifetime lifecycle
owner. Each must acquire an exclusive lease before its first shutdown side effect;
passing an earlier window check does not grant ownership. Successful quit/restart
commits that lease until the process actually exits, while failed preparation
releases only its own token after reopening its own gates. Side-effecting tasks
own their leases even if the IPC future is dropped. This prevents a previously
admitted normal quit from exiting during an update or undoing its preparation.

Recording changes have transition leases. In particular, stop, completed-snapshot
publication and automatic-transcription registration/error run in one blocking
task under a single transition lease. Quit/update admission refuses while that
transition is present, including the interval where capture has finished but no
processing job has been registered yet. The processing and actor gates still
check real capture/jobs after the lifecycle lease is acquired.

The application writes a small pending-install receipt outside the meeting library
and calls the official installer with the already verified bytes. It never copies
or replaces the user's meeting/audio/model directory. Ordinary preparation/launch
failures reopen processing/core admission and retain the download for retry.
`PreviousAppNotRestored` is a separate critical failure described below and never
uses that rollback/resume path. The plugin's destructive pre-exit window cleanup is disabled
so a Windows launcher error leaves the current app usable.

On macOS, the pinned plugin stages on the application's filesystem and uses an
app-only rename/swap and attempts restoration on swap failure, then Nero restarts.
If installing and restoring both fail, the official plugin preserves the previous
app at a separate path and returns the typed `PreviousAppNotRestored(path)` error.
Nero retains that path, commits its closed lifecycle/core/processing gates and
publishes `phase: recoveryRequired`; it does not resume work or restart automatically.
The path is saved best-effort to one small `app-update/recovery.json` cache receipt.
Even if saving that receipt fails, the live status retains the path and explicitly
asks the user to copy it. The previous app itself is never moved or removed by
Nero's failure handler.

The consuming screen must show a recovery dialog outside the inert editor/settings
area, display/copy `previousAppPath` as text, and offer the guarded
`app_update_recovery_quit` command. This exits with the gates still closed. It also
works when the original install IPC response has not arrived. Ordinary quit,
editing, downloads and installation retry remain blocked in the failed process.
A new manually launched process reads the historical receipt as a warning and
backup path, without claiming that the previous restoration succeeded or that a
manual repair was verified. Only the already-failed process retains that live fence.

A system
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

Commands: `app_update_status`, `app_update_check`, `app_update_download`,
`app_update_install`. Status events use `nero://app-update`. Public status fields:
`phase`, `currentVersion`, `currentBuild`, `availableVersion`, `availableBuild`,
`downloadedBytes`, `totalBytes`, `downloadReady`, `message`, `recoveryMessage`,
`recoveryRequired` (boolean), `previousAppPath` (string or null), and
`recoveryReceiptSaved` (boolean).
Phases: idle, checking, current, available, downloading, ready, waiting,
installing, error, recoveryRequired. Waiting means work must finish before another user install
action; it does not cancel work or install silently later. Recovery-required status
keeps `setInstalling(true)` and exposes only `quitForRecovery()`; the consuming view
owns its recovery dialog and copy-path action. Register `app_update_recovery_quit`
alongside the other commands. Do not route that action through normal `request_quit`.

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
Typed restore-failure injection tests verify that the backup marker/path survives,
all admission stays closed, receipt-write failure does not unfreeze work, ordinary
failure still resumes, and recovery exit remains callable during a pending install
IPC response. These do not reproduce real disk/rename/restore failure on macOS.

These are not production signing/publishing, a real Windows-to-Mac acceptance,
power-loss rollback, microphone/model quality or OS authorization acceptance.
Physical-device acceptance is optional by the current user's instruction; record
what was performed without relabeling unperformed checks as release blockers.

Primary references: [Tauri updater guide](https://v2.tauri.app/plugin/updater/),
[pinned updater source](https://docs.rs/tauri-plugin-updater/2.14.1/src/tauri_plugin_updater/updater.rs.html).
