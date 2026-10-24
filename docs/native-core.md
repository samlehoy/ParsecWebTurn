# Experimental native Rust core

Work branch: `codex/native-core`. This is a BUD-only bootstrap client, separate
from dev/main and from published releases. It is not a feature-complete replacement.

Build: `cargo build --release --manifest-path src-native/Cargo.toml --features native-core`.
The build does not fetch, compile, embed or execute `parsecd.wasm`. All new runtime
code is Rust, including the native Win32 sign-in UI. The shared source tree still
contains the existing WASM bridge and its build dependencies for the normal build;
removing that compile-time coupling is not yet completed.

## First live check

Run `ParsecWebTurn-native-core.exe` beside a copy of your usual `settings.json`,
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

Only BUD is used. The existing direct/TURN UDP -> TLS -> TCP policy, rustls
verification and Force TURN option are reused. BUD is fixed on in Network settings.
Settings and Connection stats are available from the native window menu.
Save and reconnect after changing connection/video/audio settings.

The **Session** button or **Ctrl+Shift+O** opens a small native control panel over
the video. It offers Keep host / Match client display / editable fixed resolution,
a host bandwidth limit in Mbps, requested FPS and Constant FPS, Default / Rotated
host orientation, mouse/keyboard/both immersive capture, and fullscreen. Apply
saves preferences for later connections. Host video values are requests and remain
subject to host permissions, supported display modes and encoder capacity.
Constant FPS does not duplicate frames or add pacing on the client. Keep host
leaves unset bitrate/FPS/pacing/orientation fields absent from the request.

The slightly translucent panel uses an owned layered Win32/GDI window, so the
Windows compositor keeps it above the DXGI video child without global topmost. It it does not read back or copy
video pixels. Local controls release remote held input while focused. BUD rotation
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
WebRTC remains outside this branch's first scope; the normal client is unchanged.

ParsecWebTurn remains independent of lowlat. This work does not change the
stock host or turn it into an open-source server.

Signaling upgrade errors show a bounded stage and HTTP status when available.
If event logging is enabled, `native_signaling_upgrade_failed` records the same
safe fields; request URLs, account tokens and server response bodies are excluded.

Host settings are queried using application sub-ID 9. Updates use opcode 17,
application sub-ID 11, and a JSON **object containing the `video` array**, not a
bare array. The host's returned object, untouched streams, output identity and
other fields are preserved. After an update the client requests the actual
configuration again; the panel and optional event log show the host's reply.
This contract was rechecked against the pinned core's native UI and lowlat's
daemon app.rs. The patched fields are:
`resolutionX`, `resolutionY`, `encoderFPS`, `encoderMaxBitrate`, `fullFPS`,
`rotated`. No original code or proprietary core file is imported by this work.

Windows CI runs the full native test suite. Its two PNGs are offline GDI control
fixtures without accounts, tokens or a video stream; they check layout, not live
DXGI/DWM composition. Live overlay ordering, host setting changes and opacity need
rechecking with the new build.
