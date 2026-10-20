# Experimental USB HID input

The successful live absolute probe is now an optional Settings → Input feature
on the isolated USB branch. It is disabled by default and requires BUD. Save
and reconnect after changing it. Enable USB/Yubikey passthrough on the stock
Parsec host. The client needs only its normal EXE and settings.json, without
CMD launchers, environment variables, host observers or dedicated USB logs.

The Input tab also offers an independent experimental USB HID keyboard. Both
devices reuse `usb_hid.rs` for creation/removal, framing, enumeration, standard
control transfers and failure handling. Only descriptors and input state differ.
Both options are BUD-only, disabled by default, and apply after reconnecting.

For the keyboard, select the same active layout on host and client (including
Hungarian–Hungarian). Physical usage codes are forwarded without guessing text
or changing the host's layout. Modifiers and simultaneous keys are represented
in a bitmap input report, without a six-key array limit. Duplicate typematic
events are ignored; the host handles repetition. LED output reports are accepted
as device state, without changing the client's physical keyboard LEDs. Queues
are bounded and ordered; focus loss/local menus/disconnect release held keys.
Normal Parsec keyboard packets are suppressed while the HID keyboard is selected,
even on setup failure. No keystrokes or typed text are logged. Keyboard host
acceptance and character behavior await the user's live test; no intermediate
enumeration/descriptor/input probe program is introduced.

## Supported scope

The PID_F004 descriptor describes five buttons, absolute 16-bit X/Y (0..32767),
and signed relative vertical/horizontal wheels. Seven USB bytes become eight
Windows HID input bytes including the report-ID placeholder. A single physical
or virtual display is supported. Relative game aiming and multi-monitor
coordinate mapping are outside the validated scope. No game/anti-cheat
compatibility is claimed.

The unmodified core still routes events between local UI and the remote session.
Remote absolute positions are normalized through the existing video viewport
and consumed by the BUD USB adapter. The internal normalized command never
reaches the host as a normal Parsec mouse command. Normal remote mouse commands
are suppressed whenever HID is selected, including setup failure. Keyboard
input retains its existing path unless the separate HID keyboard option is
selected. WebRTC does not enable either HID device.

No report is fabricated at the origin during enumeration. Buttons and wheels
require a known position. Press/release edges keep their positions and order;
compatible motion coalesces and wheels retain their target. Queues are bounded
to 64 samples and eight pending reads, with at least 4 ms between reports.
Focus loss and local menus release held buttons, discard old motion and require
fresh coordinates. Session teardown releases/removes the device. There is no
diagnostic session-duration or request-count cutoff.

## Logging and implementation

The optional existing event logger records HID enablement, active/inactive
transitions and setup, transfer, queue or send failures. It never records
continuous input, coordinates, click history or USB payloads. Expected stalls
for unsupported optional descriptor requests are not logged as failures.

- `src-native/src/hid_input.rs`: viewport normalization and lifetime cleanup.
- `src-native/src/window.rs`: desktop input/focus and local menu integration.
- `src-native/crates/bud/vendor/lowlat-client/src/usb_hid.rs`: shared USB
  control replies and device lifecycle.
- `hid_mouse_input.rs` in the same directory: bounded input reports.
- `hid_keyboard_input.rs`: keyboard descriptor, key state/queue and LED/idle state.

The former enumeration, fixed-input, relative and host-cursor-follow experiments,
their observers, launchers and diagnostic file writers have been removed.
No dependency was added. WASM, host binary, GPU and decoder code are unchanged.
USB HID descriptors follow the standard; their carriage through Parsec's
proprietary USB protocol is based on recorded interoperability research,
not a publicly supported Parsec API.

## Validation evidence

The nine-point absolute trial matched actual Windows cursor positions to within
three pixels on one virtual 1920×1080 display. The following live trial was
reported as working normally and recorded 5417 reports, no transport refusal,
interrupt rejection or backlog fault. See the
[research record](usb-native-wire-investigation.md#live-absolute-desktop-input-confirmed-2026-10-20).

Regression checks cover descriptors/correlations, invalid reads, queue limits,
long-lived pacing, click/drag/release positions, wheels, focus cleanup, normal
mouse suppression, viewport mapping and native Settings persistence/tab layout.
Tests use isolated synthetic profiles. Historical probes remain in Git history
and the research record, not in the runnable client.

The [Windows CI run](https://github.com/vampywiz17/ParsecWebTurn/actions/runs/39403643625)
for the earlier mouse-only `b3316a7` passed all six protocol/input tests, three settings/viewport/window
tests, the native Settings UI test and strict vendored-client Clippy. The Input
tab screenshot was visually checked. The standalone release build contains only
`ParsecWebTurn.exe` (18,096,640 bytes), SHA-256
`cfabf49fb6d6872067e55be0a72161a31a78916cf7ecc0c2d38fdd306b739d8b`.
The packaged settings-based activation still needs a live host/client trial;
the earlier launcher-based absolute input has already been live-tested.
