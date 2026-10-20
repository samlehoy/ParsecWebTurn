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

### Phased live result

The second supplied trial rejects the observer hypothesis: there were 19
report-descriptor requests in the fast phase, 25 in the 4,014-ms quiet phase,
and 47 in the slow phase before teardown. Counts include replies logged as
`control_exchange`; the initial waiting phase had two such requests. The
observer again saw arrival and removal with no enumeration errors.

The first report-descriptor request had a nonzero correlation ID and transfer
capacity 114. The other 92 had correlation ID zero and transfer capacity zero,
while all requested 114 bytes and were returned the same 50-byte descriptor
with matching reply ID, byte count and nonnegative status. This distinguishes
the initial request from the recurring family; it does not establish that
zero is an invalid ID or that these are failed USB transactions. No requests
are suppressed or padded to 114 bytes based on that observation.

The first capability check returned zero-sized Raw Input preparsed data and
no OS error. This is a missing measurement, not evidence of descriptor rejection.
The revised observer obtains the matching HID collection's metadata using
[HidD_GetPreparsedData](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/hidsdi/nf-hidsdi-hidd_getpreparseddata),
then frees it using
[HidD_FreePreparsedData](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/hidsdi/nf-hidsdi-hidd_freepreparseddata).
It opens only the synthetic device interface with zero desired access and
shared read/write, closes it immediately, and never requests input-report
read/write access or retries with more privileges. No descriptor, network
reply or interrupt-read behavior is changed for this follow-up.

### HID collection confirmed; same-PC work can continue

The third supplied trial returned `HIDP_STATUS_SUCCESS` (`0x110000`),
UsagePage 1 / Usage 2, InputReportByteLength 4, one input button-capability
group and two value-capability entries. The host again observed arrival and
removal. This confirms successful parsing of the mouse collection; it does
not prove delivery of input reports, which the live test still never sends.
There were 27 report-descriptor exchanges during the four-second quiet phase.

The requested 114 bytes are the fixture's advertised 50-byte report descriptor
plus 64. Independent static examples in the same stock module show a similar
allowance: at `0x1c9969` a request of 147 is answered with 83, at `0x1c99ed`
a request of 179 is answered with 115, and at `0x1c9a8b` a request of 118 is
answered with 54. All differ by 64. This corroborates leaving a correctly
bounded short descriptor response intact; it does not establish the origin
of the recurring zero-ID requests or a universal Windows rule.

The local documented HIDCLASS ETW provider was inspected read-only. Its
manifest exposes rundown/device-information events rather than the control
transaction attribution needed here. No tracing session was started, no
analytic channel enabled, and no driver or registry setting changed. Two
separate PCs remain useful for separating same-machine interactions, but are
not a prerequisite for preparing the next wire contract offline.

## Offline interrupt-report contract (not a live sender)

Additional field boundaries are independently visible in the stock client
and host. They are wire observations; no reconstructed proprietary function
or internal runtime call is used:

- Client dispatch at `0xbb53b` consumes a nested kind-7 read request: endpoint
  at offset 4, transfer type at 5, request ID at 8 and requested bytes at 16.
  Interrupt endpoints map to libusb transfer type 3.
- Client completion at `0xbcdcd`–`0xbce01` constructs nested kind 8: endpoint
  at 4, transfer type at 5, request ID at 8, USB status at 12, actual byte count
  at 16 and data beginning at 20. The host consumer at `0xc27a0`–`0xc2818`
  reads the same endpoint, status, count and data fields. It does not inspect
  the correlation ID in this consumer, but the fixture preserves it.
- The status mapping jump table at `0x1c8278`, interpreted with the branch
  bodies at `0x1c824b`–`0x1c8271`, maps completed transfer (0) to USB status 0
  and cancellation (3) to `0xc0010000`. These are USB completion statuses,
  not the signed libusb byte-count status used by control kind 10.

The new `usb_mouse_interrupt_fixture.rs` is included only under `cfg(test)`.
It checks the fixed mouse's endpoint 0x81 / interrupt type 3, bounded request
size, preserving ID zero, three-byte USB report sizing (without the Windows
report-ID placeholder), and cancellation with zero input bytes. It rejects
wrong endpoints/directions, other transfer types and malformed lengths. There
is no hook into the live kind-7 handler, no queue, no timer and no report sent
to the host. A future neutral-report test still requires deliberate runtime
integration and actual host observation; passing these tests is not evidence
of working remote input or of resolving descriptor-query repetition.

## Two-PC result and host control-loop investigation (2026-10-20)

The two-PC logs `usb-client-28796-24258.jsonl` and
`usb-host-28790-2762.jsonl` confirm that self-connection is not required for
the recurring requests. There are 117 report-descriptor exchanges: the first
has capacity 114 and a nonzero correlation ID; the remaining 116 have capacity
and ID zero. All request 114 bytes and receive the same 50-byte descriptor,
status 50 and matching ID. Intervals between the zero-ID requests average
126.34 ms (112–161 ms). The earlier observer quiet phase already excluded
that observer's device queries as a necessary cause.

The host observer starts with baseline_count 1, then records 0 → 1 → 0 with
errors 0. Its arrival/removal booleans compare against the initial baseline,
so they are false in this run and no capability query/phase progression is
triggered. These booleans must not override the recorded count transitions.
This run does not provide a fresh parsed-capability measurement.

Read-only inspection of the same hash-pinned stock module identifies a
specific stale-request path that explains the wire pattern without changing
the descriptor. The following are observations about branch conditions and
field lifetimes, not copied function implementations or runtime hooks:

| Location | Observed behavior |
| --- | --- |
| `0x1c2680`–`0x1c26c0` | Wait wrapper uses `WaitForSingleObject`; signaled maps to `0x10000000`, timeout (`0x102`) to `0x10000002`. |
| `0x1c8b80`–`0x1c8b92` | Worker waits 100 ms and tests only bit `0x10000000`. Both mapped results pass; timeout does not skip the read. |
| `0x1c2360`–`0x1c24dd` | Control read uses IOCTL `0x2af00c` and `GetOverlappedResult(TRUE)`. On success it updates setup, request ID and returned data count. On `GetLastError()==0x103` it returns `0x10000001` without updating those caller outputs. |
| `0x1c8b98`–`0x1c8bf3` | Worker clears request ID and data count before the read, but not its eight-byte setup storage. It again tests only bit `0x10000000`, so the no-more-items return also passes. |
| `0x1c8c13`–`0x1c8cf4` | Worker builds and forwards the control request from that setup storage with the cleared count/ID. The setup can therefore be from the preceding transaction. |
| `0xc1db0`–`0xc1f9e` | Network wrapper sends once and waits for a matching reply ID. It does not independently retransmit this control request. |
| `0x1c8d12`–`0x1c8d77` | Nonnegative reply status and actual data count are forwarded to the driver completion path. Status 50 is not treated as an error. |
| `0x1c24f0`–`0x1c2519`, `0x1c2662` | Driver-completion wrapper rejects request ID zero before issuing the completion IOCTL. Answering a recurring zero-ID request therefore does not imply a real driver transaction was completed. |

The PE import table independently confirms the Windows calls at IAT RVAs
`0x2438d0` (WaitForSingleObject), `0x2437a8` (GetOverlappedResult),
`0x243778` (GetLastError) and `0x243918` (DeviceIoControl).
[Microsoft documents timeout as `0x102`](https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-waitforsingleobject)
and [ERROR_NO_MORE_ITEMS as 259 / `0x103`](https://learn.microsoft.com/en-us/windows/win32/debug/system-error-codes--0-499-).
These Win32 values must not be confused with the module's mapped statuses.

This yields a strong, specific explanation: a 100-ms empty-queue iteration
can forward the previous USB setup again, with ID and capacity zero; network
reply time adds to that interval. This also explains why the serial-string
request and SET_IDLE each have an initial real-ID request followed by a zero-ID
copy before the report-descriptor request becomes the last setup in storage.
The observed delay alone does not prove network RTT or this execution path.

Checks performed: the four mapped-status bit predicates and all 121 logged
control reply IDs were checked offline; all 116 recurring report requests
match the predicted zero-ID/zero-capacity shape. No live driver return values
were captured. Consequently this is a statically demonstrated possible path
and a strongly supported explanation of the logs, not dynamic proof that each
request took that path. Request ID zero must not be declared invalid for every
USB message family on this basis.

No descriptor padding, dropped requests, changed replies, input reports,
host patch, debugger hook or production runtime change was introduced.
The existing 50-byte descriptor remains intact. The next dynamic confirmation,
if needed, is to observe the host's empty-queue result through supported
diagnostics; it must not require patching the stock module. A live neutral
interrupt-report test would be a separate experiment, not a proven fix for
this host-loop behavior.

### Optional fixed desktop input experiment

The same isolated branch now implements that separate experiment in
`usb_mouse_input_probe.rs`, gated by `PARSEC_USB_MOUSE_INPUT_PROBE=1` in addition
to the original feature/runtime/log requirements. Its tested framing completes
validated kind-7 reads with a fixed neutral/move/press/release sequence; it does
not capture or relay physical input. See [instructions and limits](usb-mouse-probe.md).
The earlier offline-only fixture now tests the actual optional encoder rather
than maintaining a duplicate implementation. A successful build still does
not establish delivery on the host; both visual desktop confirmation and new
client/host logs are required. In particular, interrupt-read activity and the
host control-request queue must not be treated as the same queue.

### Two-PC live movement and click confirmed (2026-10-20)

The user observed movement and a drawn dot in Paint with
`usb-client-input-1048-29487.jsonl` / `usb-host-1034-19261.jsonl`.
All ten fixed input steps (0–9) were queued, including button press and release
154 ms apart. The host issued twelve interrupt reads, each with capacity three;
there were no transport refusals or interrupt rejections. Its observer started
at baseline zero, confirmed arrival, HIDP_STATUS_SUCCESS with mouse usage and
four Windows input-report bytes, then removal, with zero observer errors.
Together with the visual result, this confirms actual desktop motion/click
delivery through the unmodified stock host, not just network enqueueing.

Descriptor requests continued: 111 total, of which 110 have ID/capacity zero
and the unchanged 114-byte requested length / 50-byte response. Splitting those
zero-ID requests by client elapsed time gives:

| Phase | Requests | Mean interval within phase |
| --- | --- | --- |
| Before first movement (before 4385 ms) | 16 | 127.73 ms |
| Movement/click sequence (4385–9407 ms) | 38 | 133.65 ms |
| After final neutral report (9407–17012 ms) | 56 | 134.71 ms |

These intervals do not support cessation of descriptor queries while input
is flowing. The result excludes absent interrupt-input delivery as a necessary
condition for the repetition. It remains consistent with the independently
identified empty-control-queue/stale-setup path; it does not capture the live
control-driver statuses and therefore is not direct execution proof of that
path. No descriptor change or zero-ID filtering is justified by this test.

### Isolated interactive forwarding

The live desktop launcher adds a separate PID_F002 five-button relative mouse
with vertical wheel and horizontal AC Pan, on the same established wire path.
It adapts the application's existing mouse commands rather than capturing
physical USB devices or changing the proprietary core. Parsec's button numbering
(left 1, middle 2, right 3, side 4/5) and 120-unit wheel convention are corroborated
by the local lowlat research checkout's `include/lowlat.h`; no SDK/injector code
is imported. The Windows bridge already reverses WM_MOUSEWHEEL for the web ABI,
and the pinned `mty_window_scroll` export reverses that Y value again before
application input processing; the existing outgoing wire convention is retained.

Only this explicit mode consumes ordinary outgoing mouse opcodes at the BUD
driver boundary. The normal mouse route stays blocked even without a USB probe
or after timeout/failure; non-mouse commands retain the existing route. Native
focus loss issues release-all, preserving keyboard cleanup and resetting HID
button/motion state. The older fixed/enumeration modes are retained separately.
Bounds, teardown behavior, relative-position limitations and test instructions
are in [the probe guide](usb-mouse-probe.md#live-desktop-mouse-forwarding-2026-10-20).
Automated tests cover routing isolation, button/wheel mappings, split motion,
drag-edge ordering, focus release, capacity/queue/backlog bounds and descriptor
sizes. Actual interactive delivery of the new report layout still needs a new
two-PC test; the earlier three-byte fixed-sequence success is not proof of it.

### Interactive two-PC result (2026-10-20)

The user confirmed movement, button release, scrolling and a continuous Paint
stroke using `usb-client-live-4806-14599.jsonl` / `usb-host-4793-4373.jsonl`.
The client logged 6332 HID reports, 7430 handled commands and 7457 ordinary
mouse commands suppressed, before timed removal at 90005 ms. There were no
transport refusals, interrupt rejections or backlog-limit events. These are
local counters; the user's desktop observations establish actual delivery of
the tested operations, not independent acknowledgement of every report.

The host confirmed mouse usage, HIDP_STATUS_SUCCESS, six Windows input-report
bytes and four value-capability entries, followed by removal. The observer
also logged one early error 122 (insufficient buffer); it recovered and completed
the observation. This must not be reported as an entirely error-free observer
run. The sole control stall was the optional Microsoft OS string `0x03EE`.
Descriptor queries continued: one real-ID request and 754 zero-ID requests,
requesting 129 bytes and receiving the new 65-byte report descriptor.

The remaining visible issue is local/host cursor misalignment, as expected
from absolute-coordinate differences driving a relative HID mouse. The first
sample only establishes an anchor; host pointer acceleration and independently
maintained pointer positions prevent exact absolute correspondence. Existing
cursor handling selects a native cursor at the local position, and only warps
to a host position on specific capture transitions. This is not a measurement
of dropped HID reports. A future cursor-mode change must preserve local UI,
focus release, normal-mouse suppression and the GPU-only video path; blindly
warping on every host update risks feedback or disturbed relative deltas.

### Host-position cursor following implementation

The isolated follow-up separates live HID Raw Input acquisition from the core's
existing relative-mode acquisition. A valid live interrupt read activates a
test-only telemetry bit; the BUD adapter publishes it into a generation-guarded
window state. Expiry, errors and retirement clear it. This bit is local native
state, not a new message to the proprietary host or a change to its protocol.

While remote input is allowed, host position is mapped with the existing
Viewport::unmap geometry to the native cursor's one-pixel ClipCursor region.
The existing custom native cursor/hotspot is selected for a visible desktop
cursor; hidden/relative host intent retains hidden capture. Position updates
are coalesced on the existing UI thread. Absolute cursor-position messages are
ignored while this HID raw acquisition is active, and hardware deltas remain
the only motion source. Local detach, menus, focus and visibility suspend HID
capture and request release, restoring the existing local UI path.

The platform basis is documented [ClipCursor](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-clipcursor)
and [Raw Input](https://learn.microsoft.com/en-us/windows/win32/inputdev/about-raw-input).
The Windows UI regression test checks actual position changes and the absence
of outgoing motion during synthetic host updates, rather than assuming that
every OS-generated move is physical input. No decoded video surface or graphics
backend is changed. The new behavior still needs confirmation in the two-PC
desktop trial; following the latest host report also inherits its latency.

### Live cursor diagnostic result (2026-10-20)

Inputs: `usb-client-live-21026-22861.cursor.jsonl`,
`usb-client-live-21026-22861.jsonl`, `usb-host-21000-2410.jsonl`.
The user also saw Paint's small brush marker moving, which may be an
application-drawn marker in the video rather than the client's native cursor.
Its appearance alone does not identify which cursor was visible.

- The cursor trace counted 72 wire events, 72 delivered events, 66 position
  changes at both boundaries, and 72 posted/applied UI notifications.
- Every one of the 85 sampled states with HID Raw Input acquired matched both
  the latest reported host position and its one-pixel clip. There were 72
  successful clip calls and no clip failures. This establishes sampled client
  behavior, not continuous agreement with the host's actual pointer.
- At elapsed 70,537 through 88,627 ms the wire count stayed at 66 while raw
  motion packets rose from 6296 to 8342. At the latter sample, the most recent
  wire cursor event was already 19,337 ms old. There is therefore no continuous
  cursor-position feed at this observed control-message boundary during motion.
- All 72 cursor messages carried an image. This does not prove that shape
  changes were their only trigger, since the client requests uncached images.
- USB totals: 8392 reports, 9010 handled mouse commands and 9026 suppressed
  ordinary mouse commands. The host confirmed arrival, valid HID capabilities,
  removal and zero observer errors. No automatic ordinary mouse fallback ran.

The BUD driver forwards each successfully parsed control frame unchanged to
`ExternalControl` before its session-only handler; it has no cursor filter or
cursor coalescing there. The new trace begins at the adapter's event boundary.
These results locate the practical limitation upstream of cursor delivery and
Windows positioning; they do not prove why the stock host reports infrequently,
nor exclude a lower transport/protocol issue solely from these logs.

The previous implementation assumed that cursor metadata was a sufficient live
position source. This trial disproves that assumption for this session. Moving
the visible pointer from local deltas between reports would be prediction, not
the requested actual host position: host acceleration, clipping and other input
can differ. No such heuristic, host patch, decoded-frame readback, or blind HID
timing change is introduced. A reliable actual-position source or a separately
evaluated input design is needed before claiming the cursor mismatch is fixed.

Parsec's own historical [browser architecture explanation](https://parsec.app/blog/game-streaming-tech-in-the-browser-with-parsec-5b70d0f359bc)
describes cursor-state messages and deliberately local cursor motion. It does
not promise a continuous authoritative position subscription. This supports
rechecking that assumption, but is not a specification of the current host's
precise notification policy.

### Absolute HID live mapping confirmed (2026-10-20)

Inputs: `usb-client-absolute-27270-28278.jsonl`,
`usb-host-absolute-27254-7304.jsonl`, and its `.mapping.jsonl` companion.

- The stock host requested interrupt reads and the client queued all nine
  click-free absolute grid reports, steps 0 through 8, about one second apart.
  All reported sends were accepted; no transport refusal or interrupt rejection
  appears. Removal was queued at 17,007 ms.
- Windows reported both Generic Desktop X/Y value capabilities as absolute,
  16-bit, logical minimum 0 and maximum 32767. Both queries and mouse collection
  parsing returned HIDP_STATUS_SUCCESS (1114112). The input report size was six
  Windows bytes, including the report-ID placeholder, matching five USB bytes.
- GetCursorPos samples confirmed every grid point in the intended order, within
  the observer's three-pixel per-axis tolerance. There were exactly nine observed
  position transitions; all 135 grid-matching samples agreed between the primary
  and virtual-desktop models. The final point remained stable for about five
  seconds. Fourteen preceding samples matched neither fixed target.
- Arrival and removal were observed with zero observer errors. DPI-awareness
  initialization succeeded.

This proves actual absolute HID positioning through the unmodified stock host
in this session, beyond enumeration or local send acceptance. It does not yet
prove live client-pointer mapping, dragging, local menu transitions or relative
game input with this descriptor: the fixture sends no buttons or live motion.

The user clarified that this was a **single virtual monitor**, not a secondary
monitor in a multi-monitor desktop. The observed Windows session exposed primary
size 1920x1080 and desktop bounds (0,0,1920,1080). This trial therefore confirms
absolute HID positioning on that single virtual display. The two mapping models
coincide here; multi-monitor/offset-display behavior is untested and outside the
current requested scope. Further live input work targets a single physical or
virtual display.
Client/host wall-clock differences are not treated as transport latency; this
trial has neither synchronized clock validation nor a round-trip measurement.
