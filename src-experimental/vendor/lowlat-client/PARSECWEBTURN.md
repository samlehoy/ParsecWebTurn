# Narrow compressed-video seam

MIT-licensed source copied from nomi-san/lowlat `feat/win-seams`, revision
`5f8db5689e55de0d396b0520824caa9fc053dd86`, `crates/client/src`.
Copyright (c) 2026 Nguyen Duy; original license in `LICENSE`.

Only `src/seam.rs` and `src/sound.rs` have behavioral changes. The standalone Cargo manifest pins
all sibling lowlat crates to the same revision; upstream integration-test-only
dependencies and workspace lint inheritance are not copied. Upstream unit tests
inside source files are preserved. There is no GPL codec feature enabled.
`src/input.rs` only adjusts four test-fixture include paths; the corresponding
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
`experimental-bud` feature. This is
not yet account/UI integration or proof of stock-host media compatibility.
