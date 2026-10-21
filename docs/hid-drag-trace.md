# Temporary cursor visibility diagnostics

This test branch includes the current dev cursor fixes without changing their
behavior. The normal build has no trace code enabled; only this branch's
`hid-input-trace` test executable records cursor diagnostics.
No new dependency is added. No video/GPU path is instrumented or changed.

Run the test EXE with the usual settings.json. Connect in either normal or HID
mouse mode, reproduce the brief disappearance after releasing a dragged window
two or three times, then close the client normally. Send
`cursor-visibility-trace.json` from beside
settings.json. Administrator rights, host changes and helpers are not needed.

The capture contains host cursor positions/flags, button edges, capture changes,
the core's cursor image/visibility requests and the applied native cursor state.
Continuous movement, adapter commands and USB reports are filtered out.
It records no keyboard usages, text, clipboard, credentials, addresses, media
or arbitrary USB payloads. It is separate from client-events.jsonl.

At most 4,096 numeric records are kept in memory and written once on shutdown;
the omitted count identifies saturation. Reproduce promptly after connecting.
Timing is diagnostic CPU arrival order, not host receipt/completion proof.
The temporary tracing branch is not intended for merging into dev.

Schema 2 stages:

- `cursor_request`: kind (1 visibility, 2 default cursor, 3 image), boolean value.
- `relative_request`: previous request, new request, authoritative, suspended.
- `cursor_selected`, `cursor_applied`, `capture_applied`: non-null Win32 cursor
  handle, acquired relative capture, pending position restore, requested capture.
- `host_cursor`: host X/Y, relative flag, hidden flag.
- `win_button`: pressed, button, local X/Y.

A non-null selected handle describes local cursor selection; it does not prove
that the cursor was visible in a presented video frame. There is no readback.
