# Experimental native Rust core

Work branch: `codex/native-core`. This is an independent native client with selectable BUD/WebRTC transport, separate
from dev/main and from published releases. It is not a feature-complete replacement.

Build: `cargo build --release --manifest-path src-native/Cargo.toml --no-default-features --features native-core,vendor-video`.
When reusing an older build directory, first run
`cargo clean --manifest-path src-native/Cargo.toml --release -p libopus_sys` to discard
Opus objects built with the old MSVC runtime. Fresh builds need no extra step.
The build does not fetch, compile, embed or execute `parsecd.wasm`. All new runtime
code is Rust, including the native Win32 sign-in UI. The native runtime/build
dependency graph excludes Wasmtime, Cranelift and glow; CI checks this. The normal
WASM client retains its separate default `wasm-core` feature. Synthetic regression
tests deliberately keep the old ABI test dependencies; these do not enter the
native release. See [cleanup audit](native-core-cleanup.md).

## First live check

Run `EvoParseq.exe` beside a copy of your usual `settings.json`,
preferably in a separate test folder. Sign in with your Parsec email/password and
optional 2FA code, then choose an available computer. Refresh updates the list;
manual peer-ID entry remains available if discovery returns no recognized hosts.
The discovery endpoint is the pinned core's `GET /v2/hosts?mode=desktop`; its
native reader takes the `data` array and each record's `peer_id` and `name`.
The new discovery UI still needs a live check; no device list is fabricated.
The bootstrap connection authorization remains for your own stock Parsec hosts.

**Remember sign-in** stores only the session token, encrypted using Windows
CurrentUser DPAPI, in `nativeCore.encryptedSession` in `settings.json`. Passwords
and 2FA codes are never saved. No extra account database, launcher or helper file
is required. Moving the file to another Windows user does not make the session
usable there. Expired/rejected saved sessions return to sign-in. **Sign out**
removes the saved token and attempts server-side revocation. A remembered session
is retained on ordinary exit; an ephemeral session is revoked when its final
owner exits. Revocation errors are logged without credentials.

Network settings select WebRTC or BUD. Both reuse their existing direct/TURN
policy, TLS verification and Force TURN option. Changing transport requires
reconnecting; no failure silently switches protocols.
Settings and Connection stats are available from the native window menu.
Save and reconnect after changing connection/video/audio settings.

The draggable **Session icon** or **Ctrl+Shift+M** (also **Ctrl+Shift+O**) opens a small native control panel over
the video. It offers Keep host / Match client display / editable fixed resolution,
a host bandwidth limit in Mbps, requested FPS and Constant FPS, Default / Rotated
host orientation, mouse/keyboard/both immersive capture, and fullscreen. Apply
saves preferences for later connections. Host video values are requests and remain
subject to host permissions, supported display modes and encoder capacity.
Constant FPS does not duplicate frames or add pacing on the client. Keep host
leaves the host's existing bitrate/FPS/pacing/orientation fields unchanged.

The slightly translucent panel uses an owned layered Win32/GDI window, so the
Windows compositor keeps it above the DXGI video child without global topmost.
The panel does not read back or copy video pixels. Local controls release remote held input while focused. BUD rotation
metadata selects a D3D11 VideoProcessor rotation in the existing conversion pass;
no extra intermediate frame is introduced. Rotated streams use normal GPU
presentation, with an explicit Zero copy status, because original-surface rotation
is not wired. Rotation changes use the existing keyframe/decoder recovery path.
The input extents follow the rotated display, matching lowlat's mapper contract.
Hardware rotation support and real pointer alignment still require live testing.

Ctrl+Shift+W toggles fullscreen, Ctrl+Shift+R releases/reacquires remote input,
and Ctrl+Shift+Q disconnects. Disconnecting or closing the stream returns to the
computer picker; closing the picker exits. Physical keys use the existing USB
usage mapping and host layout. Existing HID camera capture remains on the same
Win32 input owner; no continuous input logging is added.

## Reused components

- Existing native BUD session worker, offer/candidate handling, route retries,
  TURN TCP/TLS carrier and bounded queues; no second transport implementation.
- Existing H.264/HEVC MF/D3D11/AMF/VPL decode, presentation timing and optional
  original-surface Zero copy. No extra decoded-frame copy or CPU pixel readback
  is introduced by this entry point.
- Existing Opus/raw PCM decoder and WASAPI playback, with a direct PCM handoff
  instead of a guest-memory poll. Input, signaling and decoded audio wake the
  native consumer; the timeout is lifecycle maintenance, not media pacing.
- Existing combined USB HID mouse/keyboard, clipboard platform services,
  settings storage, event logging and diagnostic window.

## Upstream provenance

lowlat revision `e548d48c81ae50937940b2b47f700557cd9d50a4` (MIT):
`scripts/kessel-login.py` provides the auth request, 200/201 response, email
confirmation, optional 2FA and logout contracts. They are adapted to Rust using
our existing reqwest/TLS stack. `examples/client/signaling.cc` supplies the
authenticated WebSocket fields; `crates/kessel/src/url.rs` supplies the Rust
root-path requirement (the C++ demo's slash workaround is Matoya-specific).
Our existing BUD adapter constructs the
wire offers and candidates. `crates/core/src/cursor.rs` and the cache portion
of `crates/client/src/cursor.rs` supply cursor flags, cached-image and forget
semantics. No upstream PNG decoder, SDK/C ABI, daemon, installer, renderer or
decode backend is added. zeroize was already in the dependency graph and is
used directly for the new password/2FA/session owners.

## Validation limits and remaining scope

Owner connections to an **unmodified stock Parsec host** were confirmed live on
2026-10-24 using the earlier bootstrap build: two TURN/UDP and one direct BUD
session, H.264 8-bit and stereo Opus. Those tests do not establish HEVC, 10-bit,
TURN TCP/TLS, the new picker/persistence/panel or rotated-display behavior.
Friend/invitation authorization, gamepads, account recovery and native updater
integration are outside this version. Existing experimental labels apply.

The newer UI/control code has synthetic regression coverage; live validation is
still needed for host discovery, saved-session resume/expiry, requested display
changes, rotated input, immersive mode and reopening connections. No GPU
performance improvement or physical monitor bit depth is inferred from fixtures.
WebRTC now has a native account adapter in this branch; the normal WASM client remains separate.

EvoParseq remains independent of lowlat. This work does not change the
stock host or turn it into an open-source server.

The picker refreshes in the background every 30 seconds without disabling its
controls. Unchanged lists are not rebuilt; changed lists preserve selection,
typed peer ID and scroll position. Settings hides the complete picker/sign-in
view and restores it on Back, including during a background refresh.

Video settings include **Hide session button**; the session menu also has a
**Hide button** action. This preference is saved in settings.json. Ctrl+Shift+M
still opens the menu while the button is hidden, following the stock Windows
[menu shortcut](https://support.parsec.app/hc/en-us/articles/32381778420372-Configure-Hotkeys).
The shortcut also clears the saved hidden-button preference, so the button
reappears immediately and remains visible on the next launch.
**Send Ctrl+Alt+Del** / **Ctrl+Alt+End** sends the stock application sub-ID 14
request, independently corroborated by the pinned core UI and lowlat daemon.
The unmodified host checks permissions and needs its installed service for
Windows secure attention; sending a request does not confirm its execution.

While the menu is visible, a compact 1 Hz header reads existing telemetry:
decoder API CPU time, host encode time, network RTT, incoming transport Mbps
and decoded FPS. Decode CPU excludes GPU completion; RX includes transport
overhead rather than being a video-only bitrate. Unknown values show N/A.
The header does not enable detailed host timing reports or GPU readbacks.

Signaling upgrade errors show a bounded stage and HTTP status when available.
If event logging is enabled, `native_signaling_upgrade_failed` records the same
safe fields; request URLs, account tokens and server response bodies are excluded.

Host settings are queried using application sub-ID 9. Updates use opcode 17,
application sub-ID 11, and a JSON **object containing the `video` array**, not a
bare array. The host's returned object, untouched streams, output identity and
other fields are preserved. After an update the client requests the actual
configuration again; the optional event log records the host's reply. The panel
shows pending/error states without dumping host configuration values.
The stock host can answer before its settings task applies the update. The
client sends each write once, then polls configuration reads at most four times
per second for up to five seconds. Apply remains pending until the requested
fields match, or a failure is shown. Old replies are not displayed as successful
confirmation and cannot be used for another concurrent edit. Bandwidth, FPS,
frame pacing and orientation have concrete menu choices; only resolution offers
Keep host. Missing preferences in older profiles use current host values where
usable, otherwise 10 Mbps / 60 FPS / adaptive / default orientation.
The session button is more translucent than the panel. Both the button and an
open panel stay visible when the client loses focus, retaining normal owned-window
ordering (never system-wide topmost). The native stream window paints black
until video is available; this adds no decoded-frame operation or startup delay.
This contract was rechecked against the pinned core's native UI and lowlat's
daemon app.rs. The installed stock Windows host (150-105c) additionally confirms
that its serializer and reader support five video slots; all five are preserved,
including unused slots. The older web core reads only the first three. The patched fields are:
`resolutionX`, `resolutionY`, `encoderFPS`, `encoderMaxBitrate`, `fullFPS`,
`rotated`. No original code or proprietary core file is imported by this work.

Windows CI runs the full native test suite. Its PNGs are offline GDI control
fixtures without accounts, tokens or a video stream; they check layout, not live
DXGI/DWM composition. Live overlay ordering, host setting changes and opacity need
rechecking with the new build.

## Experimental 4:4:4 and restored WebRTC

The WebRTC account adapter uses the old client's native Rust ICE/DTLS/SCTP
engine and its pinned channel/initialization contract, without the WASM core.
It still needs live direct, TURN/UDP, TURN/TLS and TURN/TCP verification in this
independent build. BUD-only HID and raw/multichannel audio stay BUD-only.

Prefer 4:4:4 is currently supported on NVIDIA client GPUs only (experimental).
It offers 8-bit HEVC after selected-GPU Main444/AYUV decoding and processor
input checks. With 10-bit HEVC also enabled, it checks Main10 4:4:4/Y410 decode,
Y410-to-RGB10 conversion and the existing Main/Main10 4:2:0 fallback rows. Existing pinned lowlat DXVA range-extension
parameters are reused in the GPU-only direct decoder. AYUV goes directly to the
existing conversion/presentation pass, with no CPU pixel readback or intermediate
video copy. This mode uses normal GPU rendering, not original-NV12 zero copy;
The two preferences can be combined; the host remains responsible for the
actual stream format. A 10-bit 4:4:4 surface uses the same single processor
write directly into the RGB10 swapchain, with no intermediate RGB texture.
This combined mode still requires live validation. A rejected Y410/RGB10
conversion is an error, rather than a silent retry in RGB8. The host can still send
4:2:0; actual decoded chroma is shown separately in Connection stats. Live BUD
8-bit 4:4:4 decoding/presentation has been confirmed on NVIDIA GeForce RTX 3060.
The tested AMD Radeon 780M driver rejects the Main444 profile query with
`0x80070057`, so that configuration advertises 4:2:0 only. This is not a blanket
claim about all AMD hardware or APIs. AMD and Intel 4:4:4 client configurations
are not currently supported; Settings disables and clears the 4:4:4 checkbox
for non-NVIDIA adapters, including Automatic when Windows selects one. The
connection capability check independently enforces this policy. Other NVIDIA
configurations remain unverified.

See [stock settings comparison](native-settings-comparison.md) for existing
features and possible future additions; unimplemented options were not added.

For a requested Main 10 connection, a host encoder error `-15002` before the
first decoded frame displays a message asking the user to disable 10-bit HEVC
in Settings > Video and reconnect. The client does not automatically retry
in 8-bit or change saved preferences. Other host errors retain their numeric
status and the existing error handling.

In the native build, Connection stats is an independent top-level window so
normal activation can place it in front of or behind the stream. Session stays
owned by the stream. Closing the stream explicitly closes its stats window.

HEVC format changes are handled before compressed input reaches the old GPU
decoder. The client rebuilds the selected decode/presentation path, retains
current parameter sets and the arriving keyframe, and requests a new keyframe
only when needed. A size change alone does not disable vendor decoding or
zero-copy, and does not consume the decoder-error retry limit. Live validation
with remote hosts and the affected GPU paths is still required.

Full-chroma requests are retained on BUD per-stream encoder updates as well as
initialization and native keyframe requests. Capability logs distinguish
`chroma_444_requested` from the effective `chroma_444` declaration. When a
requested AYUV path is unavailable, `video_chroma_444_unavailable` records the
bounded GPU probe stage/result. Decoder state logs include the actual adapter,
decoded chroma and bit depth; no media data or continuous input is recorded.

The common video processor accepts AYUV decoder textures directly, alongside
NV12/P010. This fixes an initial 4:4:4 integration bug where the renderer's old
texture guard rejected otherwise accepted packed 4:4:4 input. No CPU repacking,
intermediate decoded texture or additional color conversion pass is introduced.
The regression test validates the format/array/geometry boundary without
claiming successful hardware 4:4:4 decoding. Actual GPU/driver output still needs
live confirmation. Failures record the vendor fallback reason for diagnosis.
