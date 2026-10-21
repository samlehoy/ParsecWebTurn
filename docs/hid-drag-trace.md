# Temporary cursor visibility diagnostics

This test branch includes the current dev cursor fixes without changing their
behavior. The normal build has no trace code enabled; only this branch's
`hid-input-trace` test executable records cursor diagnostics.
No new dependency is added. No video/GPU path is instrumented or changed.

Run the test EXE with the usual settings.json and BUD HID mouse enabled, so the
existing host-cursor/button probes are active. Reproduce the brief disappearance after releasing a dragged window
three or four times, then close the client normally. A second short trial may
exercise camera movement and its return to a visible pointer. Send
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

Schema 3 stages (all use the same process monotonic clock):

- `cursor_transport`: the BUD transport has delivered a reassembled control
  message to the session driver; X/Y, relative and hidden flags only.
- `cursor_worker_poll`: the application worker has collected that event, before
  processing media in the same loop.
- `cursor_control_queued`: the worker queues the control message for the retained
  application, after that loop's media processing.
- `cursor_backend_decode`: the application removes the control message from the
  queue and begins decoding it.

- `cursor_request`: kind (1 visibility, 2 default cursor, 3 image), boolean value.
- `relative_request`: previous request, new request, authoritative, suspended.
- `cursor_selected`, `cursor_applied`, `capture_applied`: non-null Win32 cursor
  handle, acquired relative capture, pending position restore, requested capture.
- `host_cursor`: host X/Y, relative flag, hidden flag.
- `win_button`: pressed, button, local X/Y.

A non-null selected handle describes local cursor selection; it does not prove
that the cursor was visible in a presented video frame. There is no readback.
The first new timestamp is after reliable message reassembly, not at raw socket
arrival. A delay before it still combines input delivery, host handling, network
transit/retransmissions and transport reassembly. No host clock is compared with
the client clock. These probes do not change mouse/camera policy, wakeups,
queues, transport timers or media behavior.
