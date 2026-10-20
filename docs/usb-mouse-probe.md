# Experimental USB HID input

The successful live absolute probe is now an optional Settings â†’ Input feature
on the isolated USB branch. It is disabled by default and requires BUD. Save
and reconnect after changing it. Enable USB/Yubikey passthrough on the stock
Parsec host. The client needs only its normal EXE and settings.json, without
CMD launchers, environment variables, host observers or dedicated USB logs.

The Input tab also offers an independent experimental USB HID keyboard. Both
functions reuse `usb_hid.rs` for creation/removal, framing, enumeration, standard
control transfers and failure handling. Only descriptors and input state differ.
Exactly one USB device is announced per session: mouse-only, keyboard-only or
a combined device with three top-level HID application collections (absolute
mouse report ID 1, relative mouse report ID 3, keyboard report ID 2). All
collections share one interrupt endpoint and
one bounded queue of real host read correlations. The Windows HID class driver
exposes the collections separately; see [Microsoft's collection model](https://learn.microsoft.com/en-us/windows-hardware/drivers/hid/top-level-collections).
Both options are BUD-only, disabled by default, and apply after reconnecting.

For the keyboard, select the same active layout on host and client (including
Hungarianâ€“Hungarian). Physical usage codes are forwarded without guessing text
or changing the host's layout. Modifiers and simultaneous keys are represented
in a bitmap input report, without a six-key array limit. Duplicate typematic
events are ignored; the host handles repetition. LED output reports are accepted
as device state, without changing the client's physical keyboard LEDs. Queues
are bounded and ordered; focus loss/local menus/disconnect release held keys.
Normal Parsec keyboard packets are suppressed while the HID keyboard is selected,
even on setup failure. No keystrokes or typed text are logged. Keyboard-only
host acceptance and typing, including combined mouse/keyboard operation, have
been confirmed by the user. No intermediate
enumeration/descriptor/input probe program is introduced.

## Supported scope

The mouse descriptor provides absolute 16-bit X/Y (0..32767) on report ID 1
and signed relative 16-bit X/Y (-32767..32767), five buttons and two signed
wheels on report ID 3. Both are standard Mouse top-level collections within
one USB device. Buttons and wheels always belong to the relative collection,
including on the desktop, so a mode switch does not release/repress a held
camera button or migrate its state between devices. Mouse-only uses PID_F007;
combined mouse/keyboard uses PID_F008 to avoid reusing cached older descriptors.
A single physical or virtual display is supported. Multi-monitor mapping is
outside the validated scope. Game compatibility needs live testing.

The unmodified core still routes events between local UI and the remote session.
Remote absolute positions are normalized through the existing video viewport
and consumed by the BUD USB adapter. The internal normalized command never
reaches the host as a normal Parsec mouse command. Normal remote mouse commands
are suppressed whenever HID is selected, including setup failure. Keyboard
input retains its existing path unless the separate HID keyboard option is
selected. WebRTC does not enable either HID device.

No report is fabricated at the origin during enumeration. Absolute moves and
button/wheel edges keep their FIFO order. Compatible moves coalesce: absolute
coordinates replace older coordinates; relative deltas add without applying
Windows pointer acceleration. Signed 16-bit overflow is split into later reports,
not clipped or repeated. Queues stay bounded to 64 samples and eight pending
reads, with at least 4 ms between reports. Focus loss/local menus release held
buttons and discard old motion. Teardown releases/removes the single USB device.

The host cursor's existing relative/hidden flags select native Raw Input capture
and hide the local cursor, using the same documented Win32 path as ordinary
Parsec input. When capture ends, the desktop again uses viewport-normalized
absolute coordinates. Adapter-only mode changes never go to the stock host as
normal Parsec mouse commands. No button-based guess of camera mode is added.
See [USB HID report definitions](https://www.usb.org/sites/default/files/hid1_11.pdf)
and [Windows Raw Input motion flags](https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-rawmouse).

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
three pixels on one virtual 1920Ă—1080 display. The following live trial was
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

The [combined mouse/keyboard build](https://github.com/vampywiz17/ParsecWebTurn/actions/runs/39413531882)
from `40d9bc9` passed the same ten existing mouse/shared-lifecycle, viewport,
settings and native UI regression checks plus strict Clippy. No separate
keyboard enumeration/input probe or new diagnostic executable was added.
The Input-tab screenshot was visually checked. Its standalone EXE is
18,104,832 bytes (8,192 bytes above the mouse-only build), SHA-256
`52652ea304460048605f2ba9c695bf8cf11f044b9da1ae5cae429bc648ba14f4`.
The new keyboard forwarding still awaits live host acceptance and typing tests,
with matching active host/client layouts. Existing tests are not proof of that
new live keyboard behavior.

## Simultaneous mouse/keyboard routing correction

The 2026-10-20 client log confirms that each standalone option activates, but
enabling both produces `invalid_interrupt_request` for the keyboard and then
`host_setup_timeout` for the mouse. Read-only inspection of the previously
identified stock host module confirms a single generic USB manager in this
dispatch path: the generic branch reuses its one manager without indexing it
by the outer VID/PID. Two device announcements overwrite the shared descriptor
state; distinct outer IDs do not create independent host managers. The initial
integration's two-announcement design was incorrect.

The correction uses one combined HID interface and two standard report IDs.
Standalone descriptors and identities stay unchanged. The shared device owns
all pending interrupt requests and completion framing; device input modules
only produce payloads. The combined scheduler alternates eligible report types
so continuous pointer movement cannot starve keys. Real correlation IDs are
consumed once, both held input states are released before removal, and input
contents remain absent from the event log. A focused regression checks the
single announcement, descriptor IDs/lengths, request correlation for both
payload sizes, keyboard LED output and both release reports before teardown.

The [corrected build](https://github.com/vampywiz17/ParsecWebTurn/actions/runs/39418023083)
from `6c01715` passed seven HID protocol/input checks, three settings/desktop
checks, the native Settings UI check and strict vendored-client Clippy. The
Input-tab screenshot was visually checked. Its standalone EXE is 18,105,856
bytes, SHA-256
`f80d506d1f474db959277f111e4c9ca97a938fa9b483aae1e0a265ce8f631631`.
This verifies the implementation's synthetic correlations and lifecycle;
simultaneous mouse/keyboard acceptance on the real host still needs the user's
live test. The user has separately confirmed working keyboard-only forwarding.

## Relative camera input correction

The user confirmed the combined-device fix works for both inputs. The initial
absolute-only mouse still jumped in Aion 2 camera control and left its pointer
visible. HID mode had explicitly disabled native relative capture. That block
is removed; the adapter now accepts raw relative motion using report ID 3 while
retaining report ID 1 for desktop mapping. Button state has a single owner
across both modes. A focused regression covers signed/coalesced motion, overflow
remainder, stale-motion removal, wheel-only reports, mode transitions while a
button is held, and focus cleanup. The existing native-window test checks
hidden-host-cursor capture and suppresses absolute synchronization in that mode.
Live game behavior of this new relative path remains to be tested.

The [Windows build](https://github.com/vampywiz17/ParsecWebTurn/actions/runs/39421160915)
from `bc06fe9` passed eight HID protocol/input tests, three settings/window
tests, the native Settings UI test and strict vendored-client Clippy. The
updated Input tab was visually checked. The standalone EXE is 18,107,392 bytes,
SHA-256 `d80c141c75b7a75fdaf44c264655b83bffd928fb7d13bacd64f906ab63bdc49a`.
A separate descriptor check confirmed report 1 contains two unsigned absolute
16-bit axes (five bytes including ID), and report 3 contains signed relative
16-bit axes, buttons and wheels (eight bytes including ID). These offline
checks do not prove host HID binding or game acceptance of the new descriptors.
