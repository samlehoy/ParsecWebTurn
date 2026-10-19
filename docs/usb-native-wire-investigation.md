# Native USB control-message investigation — 2026-10-19

Static interoperability research, not a working USB implementation. The stock
core, driver and application were not modified or loaded into an analysis
process. No packet or synthetic input was sent to a host. No proprietary source
or reconstructed function implementation is included in this document.

## Audited inputs

- Installed native module `parsecd-150-105c.dll`, SHA-256
  `dbd59739cfb84a239860a30480b91281f0ebd316dfb5bf58bd4539e82c92c425`.
- Original web module 150-104a, SHA-256
  `d663dd96df477c65479fb93eb88756c7fcafff581cc93be563625cd195a4b4a6`.
- lowlat `1d2bb76d3cab31ab73dbd88ca07d2e071683a7bb`, especially
  `crates/core/src/control.rs` and `crates/core/src/microphone.rs` for the common
  control envelope. Microphone-specific fields must not be applied to USB.

Local PE parsing and Capstone disassembly were read-only. RVAs below identify
evidence in the exact native module above, not public APIs or callable SDK
contracts. Inspection tooling and binary-derived output remain in ignored
`dist/usb-hid-research/`. Do not use these RVAs as runtime hooks.

## Common envelope: statically established

The native application passes its USB callback through the manager constructor
at RVA `0x97967`/`0xbb8f0`. That callback is at `0xa1880` and delegates to
`0x4a5f0`, which selects opcode 32 and a fixed 1932-byte body. The common sender
at `0xa5a70` converts the three 32-bit header arguments to network byte order.

| Field | Observed encoding |
| --- | --- |
| Control channel | 0 |
| Header | Three big-endian u32 arguments, followed by one opcode byte |
| Opcode | 32, shared virtual-device family |
| Argument 0 | Body length, 1932 |
| Argument 1 | Two callback byte arguments packed as `(first << 8) | second`; observed USB lifecycle calls use 0 and 1, yielding 1 |
| Argument 2 | `(USB vendor ID << 16) | USB product ID` |
| Body | Fixed 1932 bytes; USB records use little-endian scalar fields |

This is a 1945-byte control message before transport-specific framing. It is
not a raw USB-over-IP standard packet, a UDP datagram size or a complete TCP
frame. The header selectors are not sufficient to identify a microphone/USB
message without considering the device identifier and body kind.

## Nested USB carrier

The USB lifecycle and transfer records inspected here use:

| Body offset | Bytes | Meaning |
| --- | --- | --- |
| 0 | 4 | Outer virtual-device kind 14 |
| 4 | 4 | Nested record length 1570 (`0x622`) |
| 8 | 1570 | Nested USB record, beginning with its own little-endian u32 kind |
| 1578 | 354 | Zero-filled remainder in inspected senders |

Evidence: lifecycle builder `0xbd1d0`/`0xbd41d`, teardown `0xbd4a0`,
configuration reply `0xbb7a8`/`0xbc890`, and host handler `0xc29f0`.
The outer kind 14 and nested record kind 15 are different fields, not competing
names for one opcode.

## Device identity record: nested kind 15

The client builder and host consumer agree on these boundaries. Offsets below
are relative to the nested 1570-byte record, not the network header/body.

| Offset | Capacity | Meaning |
| --- | --- | --- |
| 0 | 4 | Nested kind 15 |
| 4 | 18 | USB device descriptor |
| 22 | 255 | Manufacturer text storage |
| 277 | 255 | Product text storage |
| 532 | 10 | Language/string-descriptor bytes queried by the inspected sender |
| 542 | 1024 | Raw configuration-descriptor storage |
| 1566 | 4 | Actual raw configuration byte count |

Evidence: client `0xbd1d0` through `0xbd449`; host `0xc2370` through `0xc247b`.
String termination/encoding and every failure case are not fully validated.
A future sender must reject out-of-range counts rather than imitate unchecked
copies found in a native binary. These capacities are not permission to send
arbitrary lengths or malformed descriptors.

## Lifecycle: independently visible on both sides

1. Client sends outer kind 14 / nested kind 15 with device identity/descriptors.
2. The host stores descriptor information and replies with nested kind 2,
   asking for structured configuration information.
3. The client replies with nested kind 11. This contains a status and a packed
   representation of the configuration/interfaces/endpoints, not just the raw
   configuration-descriptor bytes from the identity record.
4. The host extracts IN and OUT endpoints and attempts driver device creation.
5. Driver enumeration generates control requests that the host forwards to the
   client. Replies must carry the matching correlation identifier and status.
6. Client teardown sends nested kind 16; the host dispatcher closes its USB
   manager on that record.

Evidence: client configuration handling `0xbb7a8`/`0xbc890`, host configuration
request/creation `0xc2370`, control request/reply `0xc1db0`/`0xbcf40`, teardown
`0xbd4a0` and `0xc2bcb`. This sequence is static evidence, not a captured exchange.

## Structured configuration reply

The client serializer at `0xbc890` and host reader at `0xc20e0` establish that
the configuration reply is not interchangeable with the standard descriptor:

- Nested kind 11 at offset 0, signed operation status at offset 4.
- Nine configuration-header bytes at offset 8.
- Up to four interface groups, starting at offset 17, stride 184 bytes.
- Each group has space for four alternate settings, stride 45 bytes, followed
  by its 32-bit alternate-setting count at group offset 180.
- Each alternate contains nine interface bytes followed by four nine-byte
  endpoint descriptions. These include the extended endpoint fields used by
  the native library, rather than assuming all endpoint records are seven bytes.
- The inspected host considers bulk and interrupt endpoints and separates their
  IN/OUT directions. Counts above the host's bounds must be rejected locally.

These observations suffice to define structural bounds. They do not yet validate
all class-specific descriptor requests, alternate-setting changes or device
reset behavior.

## Control transactions and transfer records

The inspected request has nested kind 1 and carries the USB setup request,
bounded payload and correlation ID. The client returns nested kind 10 with a
signed operation status, data length, data and the same ID. The host waits for
that ID; unrelated replies are not evidence of a successful transaction.

Other inspected records include nested kinds 6 and 8 for asynchronous endpoint
results, and 12/13 for descriptor-query results. Their complete behavior and
status mapping remain to be validated before use. Do not replace every nonzero
return with a generic success: a device may enumerate partway and still fail
its HID report descriptor or interrupt-read requests.

## Host acceptance: stronger evidence, not a completed device test

At `0xc29f0`, the host separates known tablet/microphone identifiers from the
generic USB branch. That generic branch requires its enabled state, then routes
the nested USB records to the USB manager. No YubiKey vendor/product allowlist
was observed in this dispatcher or the inspected descriptor/configuration path.
The identity record supplies the device descriptor that later reaches the
driver creation attempt.

This narrows the question: generic mouse admission appears plausible, rather
than requiring a device to pretend to be a security key. It does not prove
the absence of restrictions elsewhere, owner-session authorization, acceptance
by the installed driver, successful HID enumeration or working mouse reports.
No restrictions or paid-feature checks were bypassed.

## libvirtualhid as a design reference

[libvirtualhid](https://github.com/LizardByte/libvirtualhid) is useful for device
lifecycle, capability checks, normalized mouse state and report handling. Its
scope explicitly excludes the network protocol. The documented Windows
driver-backed mouse path requires a local driver and machine license; using it
on our client would not make a stock Parsec host consume that driver's reports.
It must not be treated as a Parsec USB serializer or a substitute for host
integration. No dependency or code from it was added.

## What is needed for a meaningful next prototype

- An isolated, opt-in USB control-message sender/receiver, not the normal input
  queue: our current BUD external-control path is limited to 256 bytes whereas
  the established USB control message is 1945 bytes.
- Strict structural validation and bounded storage for both directions.
- A minimal virtual test mouse with its own truthful identity, descriptors and
  responses; never a YubiKey identity used to get through a filter.
- Correlated control replies and interrupt-read completion, plus timeouts,
  cancellation and nested-kind-16 cleanup.
- Host-side observation of device arrival, descriptor-request completion and
  removal. A queue accepting a local send or an unresponsive host is not an
  acceptance/rejection result.

First success criteria are enumeration and a controlled relative movement in
Windows outside games. Game compatibility remains a separate question.

## Isolated enumeration probe

The local `usb-mouse-probe` feature now supplies a fixed mouse identity,
configuration reply, bounded correlated enumeration responses and timed
teardown through the existing authenticated BUD control channel. It deliberately
does not complete interrupt input reports or send movement/clicks. Its separate
host observer checks Windows mouse arrival/removal. See
[probe status and instructions](usb-mouse-probe.md). The first live trial
observed mouse arrival/removal through the stock host over BUD / TURN TLS.
The feature is excluded from default builds.

One additional static detail used by the probe: at RVA `0xbcfee`–`0xbd0d1`,
the native client places the nonnegative `libusb_control_transfer` byte count
in both the kind-10 status field (offset 4) and data-length field (offset 1032).
Negative libusb errors remain negative status with zero data length. Success
with input data must therefore not always be encoded as status zero.

## Report-descriptor repetition follow-up

The supplied first-trial logs show one configuration request, two interrupt
read requests, and 91 GET_DESCRIPTOR requests with wValue `0x2200`. After the
first pair, 89 inter-request intervals average 160.07 ms (155–169 ms). The
observer sleeps 100 ms after each device-list/name query, so its own metadata
polling is a plausible source of the repetitions. This is a hypothesis, not a
proven cause. The single stalled request was string index `0xEE` (`0x03EE`),
not the HID report descriptor. The mouse was observed from 18,126 to 32,583 ms
on the observer clock and its removal request was sent at 17,000 ms on the
client's separately started session clock.

The native host wrapper at `0xc1db0` sends a control transaction once, waits
for matching correlation ID, then copies the returned data. It has no resend
loop in that transaction handler. Its driver bridge at `0x1c8cf7`–`0x1c8d3b`
accepts nonnegative libusb status/count and passes the returned bytes on; it
does not require status zero. This supports keeping the current byte-count
status semantics, rather than speculatively changing them.

The follow-up leaves the 50-byte HID report descriptor and three-byte USB
input report unchanged. It adds setup/reply sizes, type/index, correlation IDs
and status logging, plus observer fast/quiet/slow phases and a single parsed
HID capability query using documented Windows APIs:

- [GetRawInputDeviceInfoW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getrawinputdeviceinfow), including `RIDI_PREPARSEDDATA`.
- [HidP_GetCaps](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/hidpi/nf-hidpi-hidp_getcaps) and [HIDP_CAPS](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/hidpi/ns-hidpi-_hidp_caps).

No physical mouse is opened or captured. Windows' input-report byte length
includes the report-ID byte even when the descriptor uses no report ID;
the expected three-byte USB mouse payload therefore corresponds to four bytes
in HIDP_CAPS.InputReportByteLength. New live results are still needed to decide
whether the observer causes the repeated queries.
