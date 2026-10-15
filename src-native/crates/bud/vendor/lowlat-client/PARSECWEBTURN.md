# Transport-only lowlat client seam

MIT-licensed source derived from nomi-san/lowlat `feat/win-seams`, revision
`5f8db5689e55de0d396b0520824caa9fc053dd86`, `crates/client/src`.
Copyright (c) 2026 Nguyen Duy; original license in `LICENSE`.
Sibling common/core/net/crypto crates remain pinned to that revision.
This is a ParsecWebTurn-specific subset, not the upstream SDK's complete API.

The subset retains native credentials, AES-GCM, candidate/readiness exchange,
BUD rings, acknowledgements/NACKs, congestion control, liveness, UDP TURN,
allocation/permission renewal and clean departure. Packet/window sizes and the
native retransmission/congestion policies are unchanged. The application owns
account signaling and chooses direct or relay attempts separately from WebRTC.

Only bounded compressed-video/audio pools cross into the application. A sole,
non-cloneable media consumer borrows each pool lease synchronously outside the
input-command mutex. Network arrivals unpark it without a GUI/frame timer.
Stop the media consumer before teardown/reconnect; never retain a lease's pointer.
Input already framed by the retained WASM UI enters a fixed-size bounded ring.
A full ring refuses input explicitly; session initialization/reports remain
transport-owned. Native AES keys never enter the WASM guest.

Removed because ParsecWebTurn supplies these functions already: SDK decoder
selection and threads, vendor/software video backends, decoded frame exporters,
CPU plane/readback paths, SDK audio decode and PCM staging, input remapping,
controller report conversion, cursor cache/PNG decode, semantic UI events and
microphone capture/encoding. Their tests/fixtures and unused dependencies were
removed with them. The unused port-mapping and separate NAT-classification APIs
were removed; normal STUN gathering and host candidates remain. There is no
second renderer, decoder, UI or account WebSocket in this subset.

Original native controls are forwarded once to the retained UI. Only session
status, encoder generation/encode time and the client's roster number are also
read by the transport. Initialization requests uncached cursor updates to match
the existing UI contract. Decoder/keyframe recovery stays in the existing
application's MF pipeline; its keyframe request wakes the native protocol loop.
Periodic protocol reports retain the previous zero decode timings until actual
application timings are supplied; these zeros are not displayed as measured data.

Remaining upstream config/pool tests and the application's encrypted direct and
isolated UDP TURN tests validate this subset. Stock-host playback was confirmed
by the user over direct and relay paths; new automatic direct-to-relay retry
still needs acceptance on a network where direct access is unavailable.
The normal WebRTC build does not link this subset. Offline Intel probes remain
a separate feature and are never part of the live BUD decoder path.

UDP totals are published from the native IO shell, independently of delivered
media/control counters. The narrow socket-accounting copy and its exact scope
are documented in ../lowlat-net/PARSECWEBTURN.md. Browser/SCTP transport code is
omitted from this optional native-only dependency; our WebRTC path is unchanged.

The 10-bit video preference/capability fields and associated tests were removed
on 2026-10-14 at the application owner's request. Initialization remains 8-bit.
