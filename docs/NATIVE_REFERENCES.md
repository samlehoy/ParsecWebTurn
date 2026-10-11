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

## libparsec

[nomi-san/libparsec](https://github.com/nomi-san/libparsec) is another useful
research reference. At the 2026-10-11 review it contains a work-in-progress
README and points to lowlat for the host implementation. Its planned native
SDK, client and GPU/HEVC features are roadmap items, not evidence of an
implemented decoder or a verified client negotiation contract. Revisit it for
future protocol and integration ideas; it is not a runtime dependency.
