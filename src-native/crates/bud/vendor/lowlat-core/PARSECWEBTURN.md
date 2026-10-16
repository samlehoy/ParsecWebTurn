# Pinned native protocol core with isolated TURN stream additions

MIT-licensed nomi-san/lowlat core at
`5f8db5689e55de0d396b0520824caa9fc053dd86`. The original license is retained.
This is the existing open protocol dependency, not the proprietary Parsec WASM.

Only `relay.rs`, `turn.rs` and new `turn/stream.rs` receive the TURN changes from
[lowlat PR #2](https://github.com/nomi-san/lowlat/pull/2), pinned to
`befd5a29c7a331111f11378c0b5623de788e9769`. All other retained protocol code
stays at the original pin, avoiding unrelated newer upstream changes. Unused
cursor/PNG decoding, controller report conversion, microphone and inactive
PMTU-probe modules and their fixtures are omitted. Existing framed input remains
forwarded untouched by the transport-only client.

The changes distinguish reliable client-to-TURN transactions from UDP retries,
bound their deadlines, preserve nonce/authentication recovery and frame TURN
STUN/ChannelData on streams. Allocation and peer traffic remain UDP. The existing
240-second permission/channel renewal policy is retained. Tests cover both modes.
No SDK ABI, decoder, host encoder, renderer or account API is imported.
