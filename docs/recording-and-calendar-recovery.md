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
