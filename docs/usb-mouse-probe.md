# Isolated USB mouse probe

## Live absolute desktop HID mouse (2026-10-20)

Use **`Start-Client-Live-Absolute-Mouse-Probe.cmd`** with experimental BUD and
the **new host observer binary** (`Start-Host-Observer.cmd`). Copy your existing
settings.json to the extracted client directory. Enable host USB/Yubikey
passthrough, end prior probe sessions, and test on one physical or virtual host
display. The nine-point fixture already proved actual absolute positioning on
one virtual 1920x1080 display; live interaction still needs remote validation.

The new PID_F004 descriptor has five buttons, absolute 16-bit X/Y (0..32767),
and signed relative vertical/horizontal wheels. Seven USB bytes become eight
Windows HID input bytes including the report-ID placeholder. No report is sent
at enumeration until the first real mapped position is known. A button/wheel
without a valid position is ignored rather than clicking or scrolling at zero.
Mouse-button and wheel events update the core's local pointer location first;
the unmodified core continues to decide whether an event belongs to local UI
or the remote session. Only its remote absolute-motion output is normalized
through the existing video viewport and consumed by the BUD USB interceptor.
The internal normalized tag never reaches the host as a Parsec mouse command.

The native pointer remains local and moves normally; there is no confinement
to a stale host position and no host-position return warp while active. This
explicit desktop experiment keeps native relative acquisition off, including
when the host requests relative input. This does **not** implement relative game
mouselook. The older relative-HID experiment remains a separate launcher.
Activation requires the current BUD session and a valid HID read; selecting
WebRTC does not enable the absolute UI mode or alter its mouse transport.

Press/release edges retain FIFO order and their own target positions. Only
compatible motion samples coalesce; a queued wheel stays at its target. The
existing 64-sample, eight-read and four-ms pacing limits remain. Focus loss,
local menus/settings and hidden video release held remote buttons and discard
stale queued motion; the next interaction needs fresh coordinates. Teardown
releases at the last *sent* position, never an invented origin or a discarded
queued target. The 90-second limit and prohibition on normal mouse fallback
remain in force. Restart the client after expiry for another test.

Check Paint pointer/brush alignment across the video (including letterboxing),
click/double-click, continuous drawing and dragging/release; then both scroll
axes and side buttons where available. Open/close local Settings/overlay and
switch focus while holding a button to verify release and local navigation.
Send `usb-client-live-absolute-*.jsonl` and `usb-host-*.jsonl`. These logs contain
aggregate counters, descriptor metadata and lifecycle events, not coordinates,
button histories or credentials. The old host-position cursor trace is disabled
for this mode because equality with stale host metadata is not its display rule.

There is no video-path change, GPU/CPU frame copy, host/WASM patch, additional
SDK or dependency. This feature remains isolated to the USB test branch/build.

## Absolute HID mapping experiment (2026-10-20)

The relative-HID cursor diagnostic established that the current stock-host
cursor messages are not a continuous position source in the tested session.
No supported continuous-position subscription has been established. This
next experiment checks standard absolute HID input independently, before
attempting live routing or claiming that the visible cursor mismatch is fixed.

1. End earlier probe sessions. Keep host USB/Yubikey passthrough enabled.
2. On the host run **`Start-Host-Absolute-Mapping-Observer.cmd`** from this build.
3. On the client run **`Start-Client-Absolute-Mapping-Probe.cmd`**, connect using
   experimental BUD, and leave connected for at least 20 seconds. Do not move
   either physical mouse during the sequence; open a plain desktop/Paint first.
4. Send `usb-client-absolute-*.jsonl`, `usb-host-absolute-*.jsonl`, and the host's
   additional `usb-host-absolute-*.mapping.jsonl` file.

This enumerates a distinct `VID_FFFF / PID_F003` non-boot Mouse/Pointer with
five released buttons and two **absolute 16-bit axes, logical 0..32767**.
Each five-byte report is `[buttons=0, X_le16, Y_le16]`. Nine fixed reports visit
the 3x3 grid at 10%, 50%, 90%, left-to-right and then top-to-bottom. They start
1.5 seconds after the first valid interrupt read, at one-second intervals.
No button press, wheel, live movement or normal Parsec mouse command is sent.
The device is removed after the existing 17-second bounded experiment. Ordinary
remote mouse transmission stays blocked for this process even after expiry;
restart with another launcher to restore the corresponding input mode.

The host observer reads the matching device's parsed X/Y capabilities, reporting
absolute/relative, bit width and logical range. Its optional mapping sampler
uses documented GetCursorPos/GetSystemMetrics, with a DPI-awareness result,
to compare the real Windows pointer against those nine known positions under
both **primary-screen** and **virtual-desktop** mappings. The mapping file saves
grid-step matches, position-change booleans and desktop bounds, not actual cursor
coordinates, trajectories, screen images or account/device identifiers. It does
not hook input, inject events, change display settings or require elevation.

The completed trial used a **single virtual monitor**, and confirmed all nine
actual cursor positions on it. Current live input work targets one physical or
virtual display; multiple monitors are outside the requested scope. The mapping
observer retains both models for measurement, without claiming that the HID
descriptor guarantees either mapping on other configurations. Host cursor
clamping, other users' physical input and display changes can invalidate a sample.

The existing enumeration, relative input and live-relative launchers remain
separate and explicitly clear the new mode. No WASM/host patch, video processing,
new SDK, live network listener or dependency is added.

References: [USB HID 1.11](https://www.usb.org/sites/default/files/hid1_11.pdf),
[GetCursorPos](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getcursorpos),
[GetSystemMetrics](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getsystemmetrics),
[HidP_GetSpecificValueCaps](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/hidpi/nf-hidpi-hidp_getspecificvaluecaps).

## Live desktop mouse forwarding (2026-10-20)

Live two-PC result: movement, button release, scrolling and continuous drawing
were confirmed. The remaining reported issue is local/host cursor misalignment
in this relative-HID experiment. The new five-byte layout was parsed as six
Windows HID input bytes. See the [result and limits](usb-native-wire-investigation.md#interactive-two-pc-result-2026-10-20).

Use `Start-Client-Live-Mouse-Probe.cmd` with the **new host observer** for an
interactive, 90-second experiment on your own desktop. Enable stock host
USB/Yubikey passthrough, end earlier probe sessions, start the observer on the
host, then connect through Experimental BUD. Test movement, click/double-click,
drag/release, left/right/middle and side buttons, both wheels where available,
and focus loss/re-entry using a blank Paint canvas and a scrollable document.
The keyboard and local overlay/settings remain on their existing paths.

This launcher selects a distinct unassigned test identity `VID_FFFF / PID_F002`
with a 65-byte report descriptor and five-byte interrupt reports: five buttons,
relative X/Y, vertical wheel, horizontal AC Pan. Windows' HID capability input
length should be six bytes including its report-ID placeholder. The original
enumeration/fixed-sequence launchers still select PID_F001 and three-byte reports.
All launchers explicitly clear the other input-mode environment options.

Ordinary Parsec mouse button, wheel and motion opcodes are consumed **before
network transmission** while live HID mode is selected. They never also reach
the host's normal mouse-injection path. Keyboard, gamepad, video and WebRTC
behavior is unchanged. The unmodified WASM still handles local UI and determines
which mouse commands belong to the remote session. Release-all also resets the
HID state while retaining normal keyboard cleanup. Native focus loss explicitly
requests release and clears queued mouse motion; failure to enqueue that cleanup
ends the BUD attempt rather than silently leaving it active.

The HID mouse is relative. In normal desktop mode, the first absolute video
coordinate establishes an anchor and later differences become relative motion;
there is no absolute cursor snap or position guarantee. Existing native relative
capture/immersive mode uses its original deltas. Windows pointer acceleration
still applies. Raw wheel units accumulate to 120 per HID detent. Motion splits
into signed eight-bit chunks; consecutive samples with the same button state
coalesce, while button-state transitions retain FIFO order. There are at most
64 queued samples, eight validated host reads and 8192 accumulated counts per
axis; reports are paced to at most one per 4 ms. Bounds or send failures stop
the USB probe. There is no unbounded backlog or automatic ordinary mouse fallback.

The mouse is removed after 90 seconds or on graceful disconnect. **Normal
Parsec mouse transmission stays blocked for that live-test process**, including
when USB setup fails or the probe expires; restart with the ordinary/enumeration
launcher to restore the original behavior. Host/session cleanup is still needed
if a connection is lost before release/removal can arrive. No physical USB device
is opened, no global input hook is installed, and the host/core are not patched.

Logs are `usb-client-live-*.jsonl` plus `usb-host-*.jsonl`. The live log records
aggregate report/command/suppression counts once per second and on removal,
not actual movement coordinates, button histories, keystrokes or arbitrary USB
payloads. Counts prove local handling only; confirm actual host behavior visually.
The observer recognizes both diagnostic PIDs and otherwise remains read-only.
This is still an isolated test branch, not a production input feature.

### Host-position cursor following

The cursor-following build uses the **same live launcher**. After the host has
issued a valid live HID read and provided cursor metadata, focused remote input
uses Raw Input deltas. The native Windows cursor is confined to the latest
host-reported position mapped through the existing video letterbox geometry.
Host cursor updates refresh that position through one coalesced UI notification;
their absolute WM_MOUSEMOVE echoes are not sent back as motion. No extra video
render pass, decoded-frame copy, framebuffer readback, dependency or custom
cursor-composition window is added. The native cursor image/hotspot stays on
the existing supported Win32 cursor path.

This separate HID capture does not change the core's relative-mode request.
Local core detach/overlay menus, native settings, menu loops, lost focus,
minimization and hidden video suspend it and request button release. Local
absolute UI input resumes while suspended. A hidden/relative host cursor keeps
the existing hidden-cursor policy. Probe expiry/failure/disconnect disables the
following state; generation checks prevent an old connection from controlling
a new connection's cursor. Ordinary remote mouse commands remain blocked as
before, so expiry still requires restarting the client for a normal mouse route.

The visible position is the **latest reported host position**, not a prediction
of the next one: host/network reporting latency can therefore be felt at the
cursor. Test alignment in Paint, drawing, drag/release, both scroll axes and
focus changes; open the local overlay/settings and verify free local navigation
and correct re-acquisition on returning. Cursor-capture transitions are written
to the optional application event log as `hid_cursor_capture`, with requested
and active booleans only. USB report/count logs remain unchanged.

Windows CI additionally exercises actual cursor positioning, zero outgoing
motion from position corrections, retained relative deltas, local menu input,
capture release/re-entry and stale-generation rejection in a native UI test.
That test does not establish performance or correctness on every remote machine.

### Cursor-following diagnostic follow-up

The first remote trial still showed intermittent cursor following. Its USB log
confirmed 6698 reports, 8653 handled/suppressed mouse commands and no transport
refusals or input backlog failures. The observer confirmed arrival, valid mouse
capabilities and removal without errors. These logs did **not** measure cursor
events, so they do not distinguish infrequent stock-host position reports from
client delivery, visibility or positioning problems. The native UI test alone
does not prove a continuous live host-position feed.

The follow-up keeps the same behavior and live launcher, and automatically adds
`usb-client-live-*.cursor.jsonl` beside the ordinary USB log. It samples once per
second for at most 120 seconds: wire cursor event/image/position-change counts,
delivered event/change counts and ages, UI notification counts, Raw Input motion
counts, ClipCursor successes/failures, capture/UI state, native cursor visibility,
and whether the actual cursor and clip match the latest host position. It saves
no coordinates, trajectories, button history, images or credentials. A new file
is required; file errors disable this diagnostic instead of interrupting input.
All disk writes occur on the existing BUD worker, never the UI/input thread.

For comparison, move continuously for 10 seconds inside one Paint canvas area
without changing the cursor shape, then move across an area that changes the
cursor shape, and finally open/close local Settings. Send the new `.cursor.jsonl`
and the ordinary live USB log. Host observation remains optional and unchanged.
This build adds evidence; it does not claim the live cursor problem is fixed.

## Fixed desktop input experiment (2026-10-20)

Live result: a two-PC trial confirmed visible movement and a click mark in
Paint. All ten steps were queued; press/release were 154 ms apart. The host
confirmed mouse capabilities, arrival and removal without observer errors.
Descriptor repetition continued before, during and after input. See the
[measured result and its limits](usb-native-wire-investigation.md#two-pc-live-movement-and-click-confirmed-2026-10-20).

The separate `Start-Client-Input-Probe.cmd` launcher now enables a fixed,
bounded interrupt-input sequence. `Start-Client-Probe.cmd` still performs only
enumeration and explicitly disables the additional input opt-in. Both require
the optional `usb-mouse-probe` build feature and mandatory dedicated JSONL log.
Neither is enabled in ordinary builds or on dev/main.

On the host enable USB/Yubikey passthrough, close earlier probe sessions and
observers, open a blank **Paint canvas**, and place the cursor well inside it.
Start one fresh `Start-Host-Observer.cmd`, then run the input launcher on the
other PC. Connect through Experimental BUD and do not move/click your physical
mouse during the roughly seven-second sequence. Remain connected 25 seconds.
Use the Windows desktop only, outside games or sensitive application windows.

Times below start at the first valid host interrupt-read request:

| Time | Fixed USB report |
| --- | --- |
| 0 s | Released buttons, zero motion |
| 2 / 2.25 / 2.5 s | Relative X +12, buttons released |
| 3 / 3.25 / 3.5 s | Relative X -12, buttons released |
| 5 s | Left button down, zero motion |
| At least 120 ms later | All buttons released, zero motion |
| 7 s | Final neutral report; subsequent reads stay pending |

Windows pointer acceleration can affect visible movement; returning exactly
to the initial pixel is not a success criterion. The click should leave a dot
on the canvas. The fixed reports contain three USB bytes, with no extra Windows
HID report-ID placeholder. No physical-device capture, input forwarding, new
driver, dependency, WASM modification or host patch is introduced.

Only actual, validated kind-7 requests on endpoint 0x81 / interrupt type 3 are
completed. The pending queue is limited to eight entries, request capacity to
3–1550 bytes, the whole probe to 512 incoming records and seventeen seconds
from BUD establishment. At least 120 ms separates actual reports, even after a
late tick. A button press requires two pending reads so one is reserved for
release; otherwise the click is skipped. Graceful termination queues a release
if necessary before removal. Connection loss cannot guarantee remote release
or removal; local send acceptance is never claimed as host execution.

The client log adds `fixed_input_enabled`, `interrupt_read_pending` (ID and
capacity), and `fixed_report_queued` (step index 0–9), plus best-effort
`button_release_queued` on early exit. Existing `control_exchange` records
remain unchanged, including zero-ID requests. No arbitrary USB payload is logged.
Compare descriptor-request intervals before, during and after the input
sequence. Changes are evidence to investigate, not proof of cause: the empty
**control-request** queue identified in the host analysis is distinct from
pending **interrupt input** reads. Delivering input need not stop stale control
requests. The host observer proves arrival/removal and parsed capabilities;
movement/click delivery must also be confirmed visually in Paint.

Return both the fresh `usb-client-input-*.jsonl` and `usb-host-*.jsonl`, and say
whether movement and the canvas click were visible. The earlier offline wire
tests now exercise the same framing functions as this optional live experiment;
additional tests cover timing, bounded pending reads and reserved release.

## Earlier enumeration follow-up

Status (2026-10-19): the initial Windows CI build passed four USB tests and
Clippy. The live test through BUD / TURN TLS observed mouse arrival and removal
on the stock host, with zero observer errors. It also recorded repeated report
descriptor requests. The current follow-up build adds correlated setup/reply
metadata and controlled observation phases to investigate those repetitions;
it does not change descriptors or send interrupt input reports. A separate
`codex/usb-mouse-enumeration-probe` branch supplies the test builds; the default
client and dev/main branches do not enable the probe. The first phased run
ruled out this observer's polling as the cause of the repetitions (25 report
descriptor requests during the four-second quiet phase). Raw Input returned
no preparsed data for the mouse, so the current observer uses the documented
HID metadata API on a zero-access collection handle instead. This capability
check has now been confirmed by a live follow-up: HIDP_STATUS_SUCCESS,
mouse usage (page 1 / usage 2), four Windows input-report bytes, one input
button-capability group and two value-capability entries. Arrival and removal
were again observed. Descriptor-query repetition remains unexplained.

This isolated build tests generic USB admission through an authenticated BUD
connection to an **unmodified stock Parsec host**. It does not access the host
driver directly. This is private-protocol interoperability research, not a
supported Parsec USB API or a production input feature.

1. Enable USB passthrough in the stock host. Use your own host/account and no
   other USB passthrough device in this test session.
2. Run `Start-Host-Observer.cmd` on the **host**, before connecting. It observes
   the diagnostic mouse through documented, read-only Windows Raw Input device
   enumeration for at most two minutes. It requires no administrator account.
3. Place the test client alongside the same `settings.json`/profile files used
   by your working client, or configure/login separately. No credentials are
   included in the package. Select **Experimental BUD** in Network settings;
   keep your known working direct/TURN settings. WebRTC does not run this probe.
4. Run `Start-Client-Probe.cmd`, connect to the host and remain connected at
   least 25 seconds. The first established BUD session announces the fixed
   mouse after two seconds and requests removal at seventeen seconds. A new
   process is needed for another probe. Start the host observer again as well.
5. Keep both `usb-client-*.jsonl` and `usb-host-*.jsonl` logs. The observer must
   run on the host; client-side enumeration cannot establish host acceptance.

After the first arrival the host observer checks Windows' parsed HID
capabilities once via `HidD_GetPreparsedData` / `HidP_GetCaps`, with a shared
collection handle opened with **zero desired access**. The handle and the
allocated preparsed data are released immediately. There is no input-report
read/write or elevated-permission retry. It then uses three phases: three seconds of 100-ms device
polling, four seconds with **no device enumeration/metadata calls**, then
1000-ms polling until removal. The host log marks each phase. The client log
records setup fields, transfer sizes, response status and correlation IDs,
with wall-clock timestamps on both sides. No USB payload or device path is
logged. Keep Device Manager and other USB inspection tools closed for this
trial, because those could trigger their own descriptor queries.

If repetitions pause during the quiet interval and track the slow polling
afterwards, that supports an observer-induced query rather than a failed
enumeration retry. If they continue unchanged, the setup/reply fields and
HID capabilities provide the next evidence. The phase experiment does not by
itself attribute queries from other processes or prove working mouse input.
In the first phased trial, the repetitions continued unchanged; do not treat
the observer-induced explanation as established.

The original interrupt-read preparation was offline only. The input launcher
now uses the tested encoder for the fixed live sequence described above;
the enumeration launcher still leaves interrupt reads pending.

The synthetic identity is `VID_FFFF / PID_F001`, with product name
`USB mouse enumeration probe`. These are unassigned diagnostic identifiers,
not a claim to an allocated vendor ID or an impersonation of another device.
No physical USB device is captured. The enumeration launcher sends no input;
the input launcher sends only the fixed desktop sequence. An interrupted connection can prevent the
removal message being delivered; host/session cleanup is then required and
must be checked with the observer/Device Manager. Do not replace or uninstall
the Parsec USB adapter or other existing devices.

The responder implements fixed descriptors and recognized enumeration requests;
unknown requests stall or terminate the probe. With the enumeration launcher,
interrupt reads remain pending.
It is deliberately incomplete as a general USB implementation. A host request
or a locally accepted transport send alone is **not** proof of enumeration.
An arrival and subsequent removal observed as a Windows mouse is the first
success criterion; lack of arrival does not by itself prove a host allowlist.

The feature is absent from default builds. It has no new dependency, does not
use the regular 256-byte input queue and does not change WASM, media, decoding
or WebRTC. The optional BUD diagnostic uses fixed 1945-byte frames, a 512-request
limit and a 15-second active window, with no retries or automatic rearming.

Build on a Windows machine with the MSVC C++ Build Tools and Windows SDK:

```powershell
cargo test --locked --manifest-path src-native/crates/bud/Cargo.toml -p lowlat-client --features usb-mouse-probe usb_mouse_probe -- --test-threads=1
cargo clippy --locked --manifest-path src-native/crates/bud/Cargo.toml -p lowlat-client --features usb-mouse-probe --all-targets -- -D warnings
cargo build --locked --release --manifest-path src-native/Cargo.toml --features usb-mouse-probe
rustc --edition 2024 -C opt-level=s -C target-feature=+crt-static src-native/probes/usb-mouse-observer.rs -o USB-Mouse-Observer.exe
```

Copy the resulting `parsec-web-turn.exe` as `ParsecWebTurn-USB-Probe.exe`, the
observer and the four `scripts/usb-mouse-probe/*.cmd` launchers into a separate
test directory. Do not replace the normal client executable.
