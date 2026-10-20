# Experimental absolute USB HID mouse

The successful live absolute probe is now an optional Settings → Input feature
on the isolated USB branch. It is disabled by default and requires BUD. Save
and reconnect after changing it. Enable USB/Yubikey passthrough on the stock
Parsec host. The client needs only its normal EXE and settings.json, without
CMD launchers, environment variables, host observers or dedicated USB logs.

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
input retains its existing path. WebRTC does not enable the HID adapter.

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
- `src-native/crates/bud/vendor/lowlat-client/src/usb_hid_mouse.rs`: fixed USB
  descriptors, control replies and device lifecycle.
- `hid_mouse_input.rs` in the same directory: bounded input reports.

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
