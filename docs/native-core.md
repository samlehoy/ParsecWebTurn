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
preferably in a separate test folder. Enter your Parsec email/password, optional
2FA code and the host's peer ID. The peer ID is what lowlat's client demo also
takes; no new machine-list endpoint is guessed. Passwords and account tokens
are not written to settings or logs. This initial client signs in on each run
and attempts to revoke that session when it exits. Failed revocation is logged
without the credential. No Python, launcher, additional SDK or Parsec install
is needed on the client.

Only BUD is used. Retain your ordinary Network settings for direct/TURN routing;
the existing UDP -> TLS -> TCP policy, rustls verification and Force TURN option
are reused. Enable BUD in Network settings for the existing HID preferences.
Settings and Connection stats remain available from the native window menu.
Save and reconnect after changing connection/video/audio settings.

Stream shortcuts: Ctrl+Shift+W toggles fullscreen, Ctrl+Shift+R releases or
reacquires the remote pointer, Ctrl+Shift+Q disconnects and exits. Closing the
window exits too. Physical key usages follow the existing lowlat/USB contract;
the host keyboard layout interprets them. Camera motion, absolute HID input,
focus-release and local Settings use the existing Win32 input owner.

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
authenticated WebSocket URL contract; our existing BUD adapter constructs the
wire offers and candidates. `crates/core/src/cursor.rs` and the cache portion
of `crates/client/src/cursor.rs` supply cursor flags, cached-image and forget
semantics. No upstream PNG decoder, SDK/C ABI, daemon, installer, renderer or
decode backend is added. zeroize was already in the dependency graph and is
used directly for the new password/2FA/session owners.

## Validation limits and remaining scope

First live interoperability is pending. Compilation and fixture tests cannot
prove successful account login, current stock-host behavior, audio sync, GPU
performance or HID behavior on a real connection. Existing experimental labels
and hardware limitations still apply.

There is no machine-list UI, saved account session, account recovery flow,
gamepad integration, host video-settings overlay or native updater integration
in this first version. WebRTC is deliberately outside this branch's first scope.
The lowlat native client also rejects its web transport; our existing WebRTC
code has not been converted into a WASM-independent signaling implementation.

ParsecWebTurn remains independent of lowlat. This work does not change the
stock host or turn it into an open-source server.
