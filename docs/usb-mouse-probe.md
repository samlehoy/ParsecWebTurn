# Isolated USB mouse probe

## Fixed desktop input experiment (2026-10-20)

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
observer and the three `scripts/usb-mouse-probe/*.cmd` launchers into a separate
test directory. Do not replace the normal client executable.
