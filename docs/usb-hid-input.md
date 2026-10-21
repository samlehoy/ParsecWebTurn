# Experimental USB HID input

Settings > Input offers independent USB HID mouse and keyboard options.
Both require BUD, are disabled by default, and apply after saving and
reconnecting. Enable USB/Yubikey passthrough on the stock Parsec host.
Only the normal client EXE and settings.json are needed; no host modification,
launcher, observer or separate USB diagnostic program is required.

## Mouse and keyboard behavior

The mouse uses viewport-normalized absolute positions for desktop navigation
on one physical or virtual display. When the host requests relative input or
hides its cursor, the client uses native Raw Input deltas for camera control
and hides its local pointer. Five buttons, dragging and vertical/horizontal
scrolling are supported. Focus loss and local menus release capture and held
input; returning to the session restores the applicable host-requested mode.

When the host leaves relative/hidden-cursor mode, the local pointer is restored
to that transition's host-reported position through the video viewport. This
also covers desktop window dragging that temporarily hides the host cursor.
Without the restoration, the old local capture anchor can be sent back as an
absolute HID move after release. Ordinary visible cursor reports do not warp
the pointer; relative Raw Input and camera HID reports are unchanged. Focus,
local-menu and Disable Cursor Warp safeguards remain in effect.

Keyboard events carry physical HID usages and modifiers. Select the same
active layout on host and client, for example Hungarian on both computers.
The client does not synchronize layouts or guess characters. Simultaneous
keys use a bitmap without a six-key array limit; the host handles typematic
repetition. Keyboard LED output is accepted as virtual-device state without
changing the local physical keyboard's LEDs.

Multi-monitor coordinate mapping and automatic keyboard-layout synchronization
are not implemented. Application/game compatibility is not universal. The user
confirmed desktop input, simultaneous mouse/keyboard operation and Aion 2
camera control on the tested setup; this does not establish compatibility with
other games or host policies.

## Shared USB device

The stock host's generic USB path has one device manager. Consequently the
client announces exactly one device per session, even when both options are
selected. All HID collections share endpoint 0x81 and one bounded queue of
real host interrupt-read correlation IDs. No read completion is fabricated.

- Report 1: absolute mouse X/Y, 0..32767 (five bytes including report ID).
- Report 3: relative mouse X/Y, -32767..32767, five buttons and signed wheels
  (eight bytes including report ID).
- Report 2 in combined mode: keyboard modifiers and key bitmap (30 bytes
  including report ID). Keyboard-only uses its existing unnumbered report.

Buttons and wheels always belong to the relative mouse collection, including
on the desktop. Switching axes therefore does not transfer a held camera
button between Windows HID devices. Mouse-only uses PID F007, keyboard-only
F005, and combined input F008 under the experimental VID FFFF.

Compatible motion coalesces without reordering button/key edges: absolute
coordinates replace earlier coordinates, while relative deltas add. Oversized
deltas and wheel movement drain through subsequent reports without losing the
remainder. Input queues and pending reads are bounded; reports are spaced by
at least 4 ms and mouse/keyboard scheduling alternates eligible input to avoid
starvation. Focus cleanup discards stale movement and releases held state;
session teardown removes the single device.

The retained core continues to route local UI and remote events. Ordinary
Parsec forwarding is suppressed independently for each selected HID input,
including setup failure, to prevent duplication or silent fallback. WebRTC
does not enable these HID options. GPU decoding and presentation are unchanged.

HID descriptors and native capture use documented interfaces:
[USB HID specification](https://www.usb.org/sites/default/files/hid1_11.pdf),
[Windows top-level collections](https://learn.microsoft.com/en-us/windows-hardware/drivers/hid/top-level-collections)
and [Raw Input motion flags](https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-rawmouse).
Their carriage through Parsec's proprietary USB protocol is based on
interoperability research, not a publicly supported Parsec API. Neither the
original WASM core nor the host binary is modified.

## Logging and maintenance

The optional existing client-events.jsonl records activation/state changes and
setup, transfer, queue or send failures. It excludes continuous movement,
coordinates, clicks, key history, typed text and USB payloads.

The implementation is limited to the BUD transport adapter's usb_hid.rs,
hid_mouse_input.rs and hid_keyboard_input.rs, the native hid_input.rs viewport
adapter, existing desktop input/window lifecycle, and Settings > Input.
No new dependency or upstream SDK/decoder API was added. Historical probe
implementations and investigation notes remain available in Git history.
Regression tests for the shipped behavior run in the normal native/backend
validation workflows and release build; no separate test-branch workflow remains.
