# USB passthrough and HID mouse feasibility — 2026-10-19

Research only. No application code, proprietary core, driver, device binding,
account setting or live input path was changed. No device was created and no
game or anti-cheat was tested. This report is not proof of mouse compatibility.

## Result

The existing Parsec USB passthrough mechanism is a plausible research lead for
exposing an actual HID mouse on a stock host. Neither arbitrary HID acceptance
by the stock host nor a working USB bridge in our pinned web core is established.
A protocol opcode or a driver capable of accepting descriptors locally does not
prove that the remote host will admit and service an arbitrary descriptor.

Two distinct designs must not be conflated:

- Physical mouse passthrough: forward a dedicated physical USB device and its
  control/interrupt transfers. On Windows, obtaining that device's raw transfers
  is a separate problem from receiving ordinary Raw Input events.
- Synthetic mouse: construct an independent virtual mouse and answer its USB/HID
  requests locally. This requires a USB/HID device implementation, not just
  encoding X/Y motion as a keyboard/mouse protocol event. It is not proven to be
  accepted by the Parsec host, and must not impersonate a YubiKey to gain access.

## Evidence from the unchanged web core

Pinned core: 150-104a, SHA-256
`d663dd96df477c65479fb93eb88756c7fcafff581cc93be563625cd195a4b4a6`.
Verified against the current vendor file and the retained inspect report for the
same hash. The archived weblib.js snapshot gives these parameter names; their
in-memory layouts, callback contract, sizes and ownership are not specified:

| Import | Parameter names in weblib.js | WASM result |
| --- | --- | --- |
| usb_devices_create | cb, opaque, cur_client_usb_id, mode, filtered | i32 |
| usb_devices_destroy | ctx | none |
| usb_device_submit | ctx, data | none |
| usb_devices_set_config | ctx, mode, device, enabled | none |
| usb_devices_get | ctx, devices | i32 |

All parameters are i32. This is not enough information to dereference them as
particular structures or invoke a callback safely. `filtered` is only a parameter
name: its meaning and any effect on host policy have not been established.
Every JavaScript body is empty. Our `src-native/src/host.rs::disabled_web_stub`
preserves that unavailability; returning a fabricated non-null handle would not
implement the service.

Selected public strings in the same WASM mention `client_usb_passthrough`,
`host_virtual_usb_passthrough`, YubiKey selection, LibUSB, owner connections and
generation 5 YubiKeys. Shared configuration/help strings do not establish that
all native USB paths remain executable in the web build.

## External evidence and its limits

- [Parsec YubiKey documentation](https://support.parsec.app/hc/en-us/articles/33164294687380-Yubikey-Passthrough)
  documents full-device USB-over-IP for YubiKey 5, Windows 11 on both ends, and
  host vUSB 3.10.0 or newer. The device is unavailable locally during passthrough.
  It does not document generic mouse/keyboard passthrough.
- [parsec-vusb research](https://github.com/nomi-san/parsec-vusb/tree/c5f8bca6805ba3ae1e09504829d29dd8c7464968)
  describes client-provided descriptors and endpoint traffic, and a host driver
  that can create devices from descriptors. It is reverse-engineered evidence
  from native parsecd-150-101.dll, not a supported API contract or a mouse test
  against our web core/current host. No binary from that repository was run.
  Its presence-only virtual mouse is distinct from passthrough.
- [lowlat Windows injection](https://github.com/nomi-san/lowlat/blob/1d2bb76d3cab31ab73dbd88ca07d2e071683a7bb/crates/inject/src/windows.rs)
  uses SendInput for mouse/keyboard. The reviewed client/protocol sources have
  virtual-device framing and controller reports, but no ready general USB mouse
  passthrough implementation was found.
- [Microsoft HID architecture](https://learn.microsoft.com/en-us/windows-hardware/drivers/hid/hid-architecture)
  documents exclusive system access to mouse and keyboard collections. Metadata
  access is not the same as opening their input reports for arbitrary I/O.
- [libusb Windows restrictions](https://github.com/libusb/libusb/wiki/Windows)
  likewise exclude ordinary keyboard/mouse access through the native HID backend.
  Rebinding an active mouse to a different driver is not a transparent client
  feature and was not attempted.

## Application integration implications

The current BUD adapter's external-control handoff accepts at most 256 bytes per
message. Full USB descriptors/transfers cannot be assumed to fit. A larger USB
service would need its own audited size/lifetime/backpressure contract; blindly
enlarging the input queue is not a justified integration. WebRTC and BUD also
require independent bidirectional delivery validation.

A client-only implementation could avoid a new host helper only if the stock
host already accepts the complete device lifecycle through its existing USB
path. Otherwise a host-side HID component is needed. Installing a driver only
on the client cannot change which input API the stock host uses.

## Gates before any live mouse prototype

1. Establish the supported device scope and any host restrictions. Do not infer
   acceptance from an IOCTL list, a UI flag or a parameter named filtered.
2. Establish the callback/data structures, ownership, cancellation and reverse
   transfer path of the core bridge; record the exact compatible version.
3. Validate the baseline with an explicitly supported device in a disposable
   owner session, using metadata/counters without authentication payload logs.
4. Only if mouse support is established, test device enumeration, relative
   movement, button releases, unplug/replug and disconnect cleanup in Windows
   outside games. Keep the normal input path available to recover control.

OS enumeration, functioning input reports, and acceptance by a particular game
are separate results. None was established by this read-only investigation.

## Follow-up: actual callback inspection and local host inventory

The unchanged pinned WASM was inspected statically with WABT 1.0.42. Generated
inspection text and a findings JSON stay under ignored `dist/usb-hid-research/`;
no reconstructed proprietary implementation is added to the source repository.

The active element segment starts at table index 1. At the application USB
creation call, callback slot 73 resolves to `ParsecClientSendVirtualUSB`.
That function has six i32 parameters and one i32 result. Its entire body returns
the constant zero: it does not read a descriptor, invoke a transport, enqueue
output, call an import or service a transfer. This is a concrete counterexample
to assuming that the USB import names imply a complete native USB path in the
web build. Other USB-management call sites do exist; their existence does not
restore this absent outgoing operation.

Static call sites also show bounded device-list storage and fixed submission
records. These boundaries alone do not specify the field meanings, callback
ownership, USB status/error mapping or full duplex protocol. They must not be
turned into guessed device-create or submit structures.

Consequences:

- Implementing only `usb_devices_create/get/submit/set_config/destroy` and
  invoking the supplied callback cannot provide functional forwarding in this
  build. A supported YubiKey baseline cannot simply be assumed to work after
  returning non-null handles.
- The WASM does not have to be patched to write an independent USB transport in
  Rust, but that would be additional protocol/backend implementation, including
  reverse requests, lifecycle and admission, rather than a thin platform binding.
  No complete compatible contract for that implementation was established here.
- This finding does not show that the host or driver refuses arbitrary mice.
  Host acceptance and live enumeration remain untested. A locally created device
  through a separate helper would not prove remote acceptance through Parsec.

Read-only local inventory on 2026-10-19 confirmed:

- Parsec Virtual USB Adapter: signed, Parsec Cloud Inc., version 0.3.10.0,
  installed as `oem55.inf`.
- Parsec service: running under LocalSystem.
- Stock host log: the virtual USB adapter is available and selected for gamepad
  input. This does not establish HID mouse report delivery.

No IOCTL device creation, driver installation/rebinding, settings mutation or
synthetic input was attempted. The immediate obstacle is not a missing local
driver. The proposed thin WASM USB bridge lacks its actual send operation;
the host's arbitrary-HID policy remains a separate unanswered question.

The subsequent [native wire-format investigation](usb-native-wire-investigation.md)
identifies the fixed envelope, nested identity/configuration exchange and the
generic host USB dispatch path. Those static results supersede the earlier
absence of wire-format evidence; live mouse enumeration remains untested.
