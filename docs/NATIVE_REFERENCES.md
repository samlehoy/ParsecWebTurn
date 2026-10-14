# Native development research references

These are research inputs, not dependencies or compatibility guarantees. Check
the relevant source revision and validate against our pinned core and actual
Parsec hosts before adopting behavior. Platform integrations remain based on
documented APIs; reverse-engineered Parsec contracts stay isolated and explicit.

## lowlat

[nomi-san/lowlat](https://github.com/nomi-san/lowlat) is an independent Rust Linux
**host** implementation serving stock Parsec clients and browser clients. It is
not a Linux replacement client connecting to a Parsec host.

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
