# Recording diagnostics and Calendar recovery

Recording observations and user actions have separate owners. `CaptureQuality` retains native
observations and frame positions; `recording-quality.js` maps those facts to presentation and
recovery. The rendering layer does not infer missing audio from counters or observed positions.

- Driver xrun observations are informational. They do not establish missing audio or a gap length.
  The macOS CPAL backend forwards the device-global processor-overload notification as xrun.
- Route changes ask users to check their microphone. Queue, writer, device and format faults
  retain their specific recovery instructions and interrupted state.
- Diagnostics remain available in a collapsed disclosure. Old counters remain readable without
  invented positions. Stored PCM, raw transcript and prior edits are not changed by display policy.

`CalendarService` owns the active authorization ID, cancellation and browser URL. The URL stays
in memory, is not supplied by the UI, and is removed after callback receipt, cancellation or
completion. The native command rejects another attempt's ID before reopening the browser.

`createCalendarSettingsFlow` owns request lifetimes and serializes cancellation before retry.
Closing the panel settles the exact pending begin request, including a failed browser launch.
Failed cancellation retains ownership for another cancellation attempt. Late results cannot
replace the current view. `calendarConnectionView` selects labels/actions separately from React.

- Approval pending: progress, reopen the same browser request, cancel.
- Failed or expired attempt: retry after previous cancellation settles.
- Successful completion: clear previous browser errors and show the connected account.

Automated checks cover recorded positions, diagnostic severity, interruption recovery,
late begin/finish results, cancellation failure, reopening and the rendered Calendar controls.
Microphone/device-load acceptance and real provider authorization remain device checks.

## Recorder visibility

The recorder close action hides the window for the current recording only. The recording
toolbar provides an explicitly labelled icon to show it again during recording or pause.
An explicit show validates the current session ID and does not change the saved automatic
presentation preference. A manual presentation ends with that session.

## Quitting

On macOS, Cmd+Q, Dock quit, logout, restart and shutdown ask the app delegate first.
Nero answers later and runs the same quit flow as its own Quit command: an active
recording shows the save-and-quit confirmation, otherwise the app saves pending edits
and quits. Logout waits for that answer. Keeping the recording, a declined editor
leave or a failed shutdown cancels the request, and the system termination with it.

## Storage space

A new capture needs room for its playback WAV, 10 seconds of further audio, metadata
and a 1 GiB reserve left to macOS, which grows swap in 1 GiB files. Recording does
not start without it. During recording, each checkpoint compares free space with the
same requirement for the frames saved so far and stops the capture as interrupted,
with its saved audio and playback WAV, before the disk fills. Creating a playback WAV
checks the same reserve first. A half-written playback file is removed on failure and
on the next attempt; an interrupted recording without playback retries at launch.

## Unreadable meetings

Entries in `meetings/` whose names are not meeting IDs, such as Finder's `.DS_Store`,
are ignored. A meeting folder that cannot be read (damaged or newer `meeting.json`,
recording folders without meeting information, links or files) stays on disk, is left
out of the library and is reported once at launch. Cleanup never treats it as deleted.
A meeting whose interruption recovery cannot be saved keeps the recovered state in
memory and is recovered again at the next launch.

## Calendar meeting preparation

The picker starts with today, including ended events. Previous/next day and the shared
calendar select one local civil date; the backend resolves both day boundaries in the
requested IANA time zone, including daylight-saving transitions. Queries are bounded and
fully paged. They do not replace the separate reminder cache or change its horizon.
Preparation refetches the selected occurrence and validates account/calendar generation
before the core creates or reuses its meeting. Existing recordings and manual fields are
not replaced, and trashed meetings are not silently restored. An untouched generated
meeting title adopts the selected event title. Its start date in the picker's time zone
fills an unset meeting date, including events that span midnight; reminder recording uses
the system time zone. Creation/recording timestamps remain separate. Title origin is
persisted on new drafts, cleared by title edits and not guessed for older drafts.
Reference and defaults are committed together. Reopening an already linked occurrence
never reapplies provider defaults. A stale API authentication
response cannot invalidate a refreshed access token. An API call that rejects the
current access token drops only that token and retries once with a refreshed one;
reconnection is requested only when the refresh token itself is rejected.

## Cancelled processing

Cancelled notifications have explicit retry and dismiss actions. Dismissal acknowledges
the notification; it never deletes audio or results. Import requires selecting the source
again. Other retries retain source checks, template identity and valid stage caches.
Recognition's saved baseline and its speaker continuation share the cancellation boundary.
A cancelled optional terminology pass is skipped on retry; saved text is used for the
speaker stage. Legacy jobs without a validated continuation retain conflict protection
and can be replaced by an explicit operation from the meeting.

## Integration notifications

The notification center includes confirmed agent-package updates and connection issues
for an already configured Google Calendar account. Optional unconnected services, active
authorization, restore/check states and prior conversation sessions do not create alerts.
Each service/target version has one entry. Resolved entries disappear; confirming an entry
clears its unread badge for the current application session without changing the connection.
The action opens integration settings, never installs or reconnects without a user action.

Agent checks run at startup, on focus and at a bounded interval while the document is
visible. Calendar snapshots share the settings flow and revision guard. Late background
responses cannot restore an update superseded by an explicit connection operation.
