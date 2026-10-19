# Enumeration-only USB mouse probe

Status (2026-10-19): the initial Windows CI build passed four USB tests and
Clippy. The live test through BUD / TURN TLS observed mouse arrival and removal
on the stock host, with zero observer errors. It also recorded repeated report
descriptor requests. The current follow-up build adds correlated setup/reply
metadata and controlled observation phases to investigate those repetitions;
it does not change descriptors or send interrupt input reports. A separate
`codex/usb-mouse-enumeration-probe` branch supplies the test builds; the default
client and dev/main branches do not enable the probe.

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
capabilities once, then uses three phases: three seconds of 100-ms device
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

The synthetic identity is `VID_FFFF / PID_F001`, with product name
`USB mouse enumeration probe`. These are unassigned diagnostic identifiers,
not a claim to an allocated vendor ID or an impersonation of another device.
No physical USB device is captured. No movement, clicks, game interaction or
interrupt input reports are sent. An interrupted connection can prevent the
removal message being delivered; host/session cleanup is then required and
must be checked with the observer/Device Manager. Do not replace or uninstall
the Parsec USB adapter or other existing devices.

The responder implements fixed descriptors and recognized enumeration requests;
unknown requests stall or terminate the probe. Interrupt reads remain pending.
It is deliberately incomplete as a general USB implementation. A host request
or a locally accepted transport send alone is **not** proof of enumeration.
An arrival and subsequent removal observed as a Windows mouse is the first
success criterion; lack of arrival does not by itself prove a host allowlist.

The feature is absent from default builds. It has no new dependency, does not
use the regular 256-byte input queue and does not change WASM, media, decoding
or WebRTC. The optional BUD diagnostic uses fixed 1945-byte frames, a 128-request
limit and a 15-second active window, with no retries or automatic rearming.

Build on a Windows machine with the MSVC C++ Build Tools and Windows SDK:

```powershell
cargo test --locked --manifest-path src-native/crates/bud/Cargo.toml -p lowlat-client --features usb-mouse-probe usb_mouse_probe -- --test-threads=1
cargo clippy --locked --manifest-path src-native/crates/bud/Cargo.toml -p lowlat-client --features usb-mouse-probe --all-targets -- -D warnings
cargo build --locked --release --manifest-path src-native/Cargo.toml --features usb-mouse-probe
rustc --edition 2024 -C opt-level=s -C target-feature=+crt-static src-native/probes/usb-mouse-observer.rs -o USB-Mouse-Observer.exe
```

Copy the resulting `parsec-web-turn.exe` as `ParsecWebTurn-USB-Probe.exe`, the
observer and the two `scripts/usb-mouse-probe/*.cmd` launchers into a separate
test directory. Do not replace the normal client executable.
