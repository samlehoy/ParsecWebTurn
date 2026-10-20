# Transport-only lowlat client seam

MIT-licensed source derived from nomi-san/lowlat `feat/win-seams`, revision
`5f8db5689e55de0d396b0520824caa9fc053dd86`, `crates/client/src`.
Copyright (c) 2026 Nguyen Duy; original license in `LICENSE`.
Common/crypto remain pinned to that revision; local net/core copies carry the
isolated TURN stream additions described below.
This is a ParsecWebTurn-specific subset, not the upstream SDK's complete API.

The application may supply a validated audio channel count/mask in the existing
initialization contract. Stereo remains the default. This adds no SDK decoder,
audio device, network scheduler or session-layer dependency.

Application-verified Main 10 capability is retained for external GPU decoding.
The stream-0 encoder-config bridge preserves the negotiated HEVC/range/depth
flags instead of allowing the embedded UI to erase them; it does not enable
4:4:4. No packet scheduling, retransmission or congestion policy is changed.

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
by the user over direct and relay paths, including automatic direct-to-relay
retry where direct access was unavailable.
WebRTC sessions do not use this subset. Removed offline Intel investigations
are not part of the live BUD decoder path.

UDP totals are published from the native IO shell, independently of delivered
media/control counters. The narrow socket-accounting copy and its exact scope
are documented in ../lowlat-net/PARSECWEBTURN.md. Browser/SCTP transport code is
omitted from this optional native-only dependency; our WebRTC path is unchanged.

TURN stream integration: native IO/configuration and established-carrier telemetry
from lowlat PR #2, revision befd5a29c7a331111f11378c0b5623de788e9769.
No SDK ABI or decoder API is imported. Common/crypto remain at the original pin;
core is the sibling pinned copy with TURN stream additions and unused host-only
modules omitted, as recorded in its PARSECWEBTURN.md. See
../../../../../../docs/bud-turn-stream.md for scope and verification.

ParsecWebTurn's optional USB HID input is local adapter code, not an imported
lowlat SDK input implementation. One device lifecycle and bounded interrupt
queue serve mouse-only, keyboard-only or combined collections. The application
retains local UI routing; selected HID inputs suppress their normal Parsec
counterparts. Protocol/control, descriptor, relative/absolute mouse and physical
keyboard state are isolated from media and crypto. No new dependency, host
patch, probe launcher or per-input-event logging is introduced. See
../../../../../../docs/usb-hid-input.md for behavior, scope and validation.
