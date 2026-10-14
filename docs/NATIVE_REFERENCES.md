# Native development research references

These are research inputs, not dependencies or compatibility guarantees. Check
the relevant source revision and validate against our pinned core and actual
Parsec hosts before adopting behavior. Platform integrations remain based on
documented APIs; reverse-engineered Parsec contracts stay isolated and explicit.

## lowlat

[nomi-san/lowlat](https://github.com/nomi-san/lowlat) implements independent Parsec
host and client components. The originally reviewed **main** revision below
contains the Linux host only. The maintainer subsequently pointed out that the
native client SDK and Windows work are on **feat/win-seams**; the main-only
finding must not be generalized to the entire project.

Reviewed revision: `7b4dc4ae79990ea8c59a108c80b3c89b695c3cb4` (2026-10-11).
Its MIT-licensed documentation is useful for future protocol investigations:

- `docs/01-protocol.md`: control opcodes, HEVC flags, media framing, session
  initialization and the separate WebRTC/SCTP path.
- `docs/03-connectivity.md` and `docs/04-signaling.md`: connection/signaling
  details to compare with our captured behavior.
- `examples/web-client`: a small independent browser client for comparison.
- Audio/clipboard, display changes, liveness and encoder generations are useful
  follow-up topics. Native BUD packet descriptions must not be applied directly
  to our WebRTC data channels: their media framing differs.

The HEVC investigation uses it as independent corroboration of `_flags` bit 0;
see [HEVC negotiation evidence and limitations](hevc-negotiation.md). No lowlat
library, host implementation, socket/crypto replacement or copied source is
included in our binary.

The client-side reuse/critical-section changes inspired by the IO design are
documented in [native input and network hand-off](native-input-network-handoff.md).
They do not implement BUD in the WebRTC client. The `pmtu` source explicitly says
its probing mechanism has no callers in this revision; its presence alone is not
evidence of an active optimization. Host encoder settings similarly do not prove
the internal behavior of a stock client's decoder.

Additional reviewed revision on `feat/win-seams` (2026-10-14):
`5f8db5689e55de0d396b0520824caa9fc053dd86`.

- `crates/client`, `crates/sdk`, `include/lowlat.h` and `docs/10-client.md`
  implement/document the native client. Its session API resembles our private
  web imports, but the client accepts BUD and refuses the web transport; this is
  not a drop-in WebRTC replacement. Applications still supply signaling.
- `crates/decode/src/amf.rs`, `nvdec.rs`, `vpl.rs`, and the Windows client seam
  are concrete decoder references. AMF requests `ReorderMode` and
  `LowLatencyDecode`, checks support, and uses the caller's D3D11 device. GPU
  handle exports may still require plane splitting and GPU copies; CPU plane
  output is not equivalent to a GPU-only path.
- `docs/10-client.md` section 11 and `docs/03-connectivity.md` section 7 describe
  BUD through a client-allocated UDP TURN relay, transparent to the host. This
  has documented stock-host testing; compatibility with our Cloudflare setup
  remains to be demonstrated. TURN/TCP and TURN/TLS are not provided by this path.

The next development scope and isolation requirements are recorded in
[BUD and vendor decoder investigation](native-bud-vendor-decoders.md). No lowlat
runtime or vendor decoder has been integrated into the distributed client yet.

## libparsec

[nomi-san/libparsec](https://github.com/nomi-san/libparsec) is another useful
research reference. At the 2026-10-11 review it contains a work-in-progress
README and points to lowlat for the host implementation. Its planned native
SDK, client and GPU/HEVC features are roadmap items, not evidence of an
implemented decoder or a verified client negotiation contract. Revisit it for
future protocol and integration ideas; it is not a runtime dependency.

## OpenParsec

[hugeBlack/OpenParsec](https://github.com/hugeBlack/OpenParsec) is a Swift/SwiftUI
iOS/iPadOS client built around the separately supplied proprietary Parsec SDK.
Its open application source does not implement the SDK's transport or decoder
internals, and does not establish equivalent capabilities in our WASM core.

Reviewed revision: `b49c72014bddd915f5ed801e7ae9392bb330e913` (2026-10-11).
Useful source paths at that revision:

- `OpenParsec/ParsecSDKBridge.swift`: release pressed inputs before disconnect,
  release again after a successful connection, separate audio/event polling,
  and connection generations used to retire old polling loops. These are useful
  lifecycle design references, not proof that its synchronization is suitable
  for our implementation.
- `OpenParsec/GameController.swift`, `TouchHandlingView.swift` and
  `KeyCodeTranslators.swift`: mouse, keyboard, gamepad and touch interaction
  ideas, especially for a future mobile client. Apple-specific APIs require
  platform-appropriate replacements on Windows, Linux and Android.
- `OpenParsec/ParsecUserData.swift` and the SDK bridge: video configuration and
  display enumeration messages, including resolution, bitrate and constant
  frame rate. Treat their SDK message IDs and JSON as research leads; verify
  any corresponding WASM/data-channel contract separately before integration.
- `OpenParsec/audio.c`: preallocated playback buffers and clearing queued audio
  on pause/disconnect. Its Apple AudioQueue buffering and silence policy are
  not a measured low-latency preset for our Windows audio path.

HEVC selection uses the SDK's `decoderH265` configuration field. The Metal
renderer source and Metal rendering bridge are commented out at this revision;
their presence is not evidence of an active Metal or zero-copy pipeline.

The repository declares GPL-3.0 licensing. Keep it as an external design
reference; copying or integrating its code requires a separate license
compatibility review. No OpenParsec source or SDK is included in our binary.
