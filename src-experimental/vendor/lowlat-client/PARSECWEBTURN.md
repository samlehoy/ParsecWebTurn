# Narrow compressed-video seam

MIT-licensed source copied from nomi-san/lowlat `feat/win-seams`, revision
`5f8db5689e55de0d396b0520824caa9fc053dd86`, `crates/client/src`.
Copyright (c) 2026 Nguyen Duy; original license in `LICENSE`.

The external-media seam changes `src/seam.rs`, `src/sound.rs`, and the feature-gated
control/consumer-wakeup plumbing in `src/driver.rs`, `src/shell.rs`, `src/event.rs`
and `src/input.rs`. The standalone Cargo manifest pins
all sibling lowlat crates to the same revision; upstream integration-test-only
dependencies and workspace lint inheritance are not copied. Upstream unit tests
inside source files are preserved. There is no GPL codec feature enabled.
`src/input.rs` also adjusts four test-fixture include paths; the corresponding
upstream controller reports are copied under `tests/data/pad` for unit tests.
The six cursor PNG/RGBA fixture pairs remain under `tests/data` for the preserved
upstream cursor tests. Neither fixture set is runtime application data.

The opt-in `external-video` feature adds `Client::new_external_video(caps, port)`.
It retains the upstream session, crypto, candidate/readiness handling, BUD rings,
congestion/retransmission, TURN and renewal behavior. It does not start the SDK
video decoder thread; the application consumes the existing bounded `Units`
pool with exactly one consumer. No decoded textures, plane export, staging or
GPU readback is involved. `caps` must come from the application's verified
decoder. The normal constructor retains its previous behavior.

Decoder/frame-export switching is refused in external mode. The application
can request a keyframe through the existing protocol loop. It must stop its
consumer before teardown/reconnect and must not keep borrowed compressed units
beyond the pool lease. ParsecWebTurn's `Session::with_video` provides synchronous
lease ownership; no `Units` handle is exposed from that adapter.

External mode also lends compressed audio packet leases and disables the SDK's
decoded-audio consumer. The application's existing Opus decoder remains in use;
native audio headers are removed without re-encoding or copying the payload.

This patch is not a new decoder or renderer. It is isolated to the experimental
crate; the standard WebRTC client does not depend on this copy. The compressed
sink feeds the existing Media Foundation/D3D11 pipeline under the optional native
`experimental-bud` feature. The optional test client now binds account/UI lifecycle and explicit BUD selection;
stock-host media compatibility still requires acceptance testing.

External mode forwards original control frames to the existing application UI.
The SDK still owns session initialization, cryptography, retransmission,
congestion control and keyframe recovery. Already-framed application input uses
a bounded fixed-size request ring; SDK-managed initialization and reports are
refused. Network activity unparks the media consumer; media does not wait for a
GUI/frame polling timer. Normal constructors leave this forwarding disabled.

The media consumer can be detached exactly once from a session's command path.
Its non-cloneable wrapper owns the sole video/audio consumers, lends only
synchronous pool leases, and must be dropped before session teardown. This lets
MF input inspection/queueing proceed without holding the SDK input-command lock.
External initialization requests uncached cursor updates because the retained
WASM UI does not implement native checksum cache references.
