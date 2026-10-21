# Temporary cursor visibility diagnostics

This test branch includes the current dev cursor fixes and a candidate BUD
cursor-event scheduling change described below. The normal build has no trace code enabled; only this branch's
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

Schema 5 stages (all use the same process monotonic clock):

- `core_button`: the retained core prepared a mouse button message (type,
  button index, pressed, reserved). Continuous core motion is filtered out.
- `button_submit`, `button_session_locked`, `button_ring_queued`: entry to the
  BUD sender, acquisition of its existing session mutex, successful enqueue in
  the existing bounded input ring. Values are the unchanged opcode/arguments.
- `hid_button_command`: the transport thread has consumed the button command;
  the same opcode/arguments as the ring probes.
- `hid_read_pool`, `hid_read_pool_refilled`: shared HID device ready flag,
  pending host interrupt-read count, device error code, reserved. Sampled at a
  button command and when an empty read pool receives a valid host read.
- `hid_button_report`: mouse button bitmask, protocol-queue acceptance boolean,
  reserved, reserved. Only button-state changes or send failures are recorded;
  absolute/relative movement and keyboard reports are excluded.
- `control_pressure_out`, `control_pressure_in`: control send-window fragment
  count (queued/unacknowledged), cumulative NACK retransmissions, cumulative
  timeout retransmissions and smoothed BUD acknowledgement RTT in microseconds
  (-1 when unavailable). Sampled at a changed mouse-button report and a received
  cursor message. Counts are saturated to i32::MAX. These are aggregate channel
  values, not an ACK or retransmission proof for a particular mouse report.

- `cursor_transport`: the BUD transport has delivered a reassembled control
  message to the session driver; X/Y, relative and hidden flags only.
- `cursor_worker_poll`: the application worker has collected that event, before
  processing media in the same loop.
- `cursor_control_queued`: the worker queues the control message for the retained
  application, after that loop's media processing.
- `cursor_backend_decode`: the application removes the control message from the
  queue and begins decoding it.
- `cursor_callback_wake`, `cursor_callback_run`: a host capture-state transition
  queued a local wake hint, and the input loop consumed that hint.

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
the client clock. The probes themselves do not change mouse/camera policy,
queues, transport timers or media behavior.
The outgoing report timestamp is protocol-queue acceptance, not OS/socket send
completion, host USB completion or input application. The interval after it
still combines protocol/carrier output, network/retransmissions, host handling
and return delivery. The RTT estimate must not be subtracted as an exact
one-way network delay. No timestamp is taken from another machine's clock.

## Candidate event scheduling

After queuing a validated BUD host cursor message, the worker wakes the existing
input loop only if `relative || hidden` changed (or for the first cursor event).
One hint covers a whole received batch. Repeated shapes/positions with the same
capture state do not create more callbacks. The shared worker lock is released
before the wake; a full input queue falls back to the normal periodic callback.

The same immutable WASM callback consumes the same FIFO in its original order.
No cursor event is processed out of band, no visibility request is overridden,
and no button release predicts host intent. An early callback postpones the next
regular tick so it does not create an immediate redundant callback. Ordinary
mouse input keeps its existing scheduling. The separate graphics/audio workers,
video presentation and transport timers are unchanged. Camera capture still uses
the existing authoritative `relative || hidden` rule and focus/menu guards.

Synthetic tests cover both host capture flags, malformed input rejection,
the local-only wake hint and callback scheduling without a catch-up burst.
The previous foreground capture regression still covers hidden-only camera
capture and return to absolute input. Live improvement/camera compatibility of
this candidate remains to be checked; it is not merged into dev.

The event-scheduling candidate remains enabled in this measurement build. The
schema-4 live trace showed average BUD delivery-to-native-cursor time fall from
19.24 ms to 1.36 ms across the respective drag samples; network/host conditions
changed between runs, so this does not establish end-to-end improvement of that
size. Schema 5 adds observation only and does not change the mouse, camera,
HID pacing/read pool, retransmissions, transport waits, audio or video behavior.

## Candidate post-turn output drain (schema 6)

The client driver can enqueue HID reports and control replies after the network
shell has drained its output. Previously the next shell pass waited before
draining those messages, potentially until a protocol deadline. The candidate
conditionally drains again after the driver successfully queues output, using
the existing UDP batch or native TCP/TLS carrier and its backpressure rules.
It does not wait, receive, add a wake, change ACK/retransmission deadlines or
change the HID report cadence. Idle passes do not perform this extra drain.
The initial drain stays ahead of the driver's media handoff work.

`button_flush_begin` and `button_flush_end` bracket this additional drain only
when it contains a newly queued mouse-button state change. The end record's
first value counts carrier submissions in that drain, including any other
ready protocol output; it is not a per-button send completion or host receipt.
Motion-only output does not generate these records. The filename, bounded
capacity and normal event-log behavior remain unchanged.

A real loopback regression queues ordered messages after a completed shell
turn, flushes without another sender turn and checks delivery and accounting
without an added wake. A driver regression distinguishes accepted enqueues
from missing-channel and oversized-message rejection. Live cursor improvement
and camera behavior still require verification with the test executable.
