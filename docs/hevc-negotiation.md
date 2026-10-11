# HEVC negotiation experiment

Investigated 2026-10-11. The embedded Parsec WASM 150-104a is unchanged
(SHA-256 `d663dd96df477c65479fb93eb88756c7fcafff581cc93be563625cd195a4b4a6`).

## Host-facing request

The private Parsec control channel's opcode 11 carries a NUL-terminated JSON
object. Argument 0 is its UTF-8 byte length including NUL; arguments 1 and 2 are
zero. The three arguments are big-endian i32 values, followed by the opcode byte.
**Bit 0 (`0x01`) of `_flags` advertises HEVC decoding support.** This is a
Parsec application protocol field, not WebRTC RTP/SDP codec negotiation.

The existing web adapter sends `_flags: 0`. The experiment changes that to `1`
when explicitly enabled and locally available. Other pinned fields, including
`mediaContainer: 0` and `_VideoProtocolVersion`, are preserved. No extra guessed
`h265`, `codec`, or `decoderH265` JSON properties are added.

Two independent observations establish the bit assignment:

- The [preserved native SDK header](https://github.com/mbroemme/parsec-sdk/blob/14222ac25be3bc28c49b4858213a737642031513/sdk/parsec.h)
  defines `ParsecClientVideoConfig.decoderH265` at byte 13 of its 16-byte per-stream
  configuration. Static inspection of the matching Linux SDK traces this through
  `ParsecClientConnect` (virtual address `0x1a310`, configuration copied into
  context + `0xac`), the setup copy (`0x11ee6`, into video context + `0x110`), and
  the capability builder (`0xdd50`). At `0xdf10`, byte `0x11d` is tested; when
  enabled and the selected decoder reports HEVC support, `0xdfae` ORs `1` into
  the stream flags at + `0x24`. The initializer reads those flags at `0x18c07`
  and serializes `_flags` at `0x18c63` / `0x18c6c`, then sends opcode 11 at
  `0x18d04` onward. ELF SHA-256:
  `55e01e04edc27f2db44e7d239121f53d51cadf17b4840eea791880cc72892835`.
- [lowlat's protocol description, section 11.5](https://github.com/nomi-san/lowlat/blob/7b4dc4ae79990ea8c59a108c80b3c89b695c3cb4/docs/01-protocol.md#115-session-initialization)
  independently identifies HEVC as bit 0 and explains opcode 13's equivalent
  per-stream flags. Its newer native-peer base flag `0x08` is **not** added to
  our older pinned web contract. The original web adapter works with zero base
  flags; flags from different peer generations must not be mixed blindly.

## Rust implementation and boundaries

Settings adds **Prefer H.265 / HEVC**, off by default (`preferHevc` in the existing
JSON file). Before advertising it, the transport worker checks the selected
D3D11 GPU's HEVC Main/NV12 support and an installed synchronous D3D11-aware
Media Foundation HEVC decoder. If unavailable, the request stays H.264.

The compressed stream inspector identifies HEVC from a validated Annex B
sequence containing a VPS; H.264 SPS remains an accepted host fallback. Decoder
selection follows observed syntax, never the requested preference alone. The
first codec is fixed for this connection; unannounced codec changes are refused.
VPS/SPS/PPS and BLA/IDR/CRA are distinguished from delta pictures. The experiment
requests neither 10-bit nor 4:4:4.

HEVC decoder discovery uses [MFTEnumEx](https://learn.microsoft.com/en-us/windows/win32/api/mfapi/nf-mfapi-mftenumex),
and the [documented HEVC transform](https://learn.microsoft.com/en-us/windows/win32/medfound/h-265---hevc-video-decoder)
accepts Annex B elementary stream input. The existing NV12 D3D11 texture /
VideoProcessor / DXGI presentation path is reused. No live CPU readback or
intermediate decoded-frame copy is added. Capability checks do not establish
actual hardware execution; existing output telemetry retains that distinction.

This is an interoperability experiment: matching native SDK flags and an
independent host implementation do **not** prove a stock Parsec host will honor
HEVC on its WebRTC transport. An end-to-end host test is still required. A host
may send H.264 despite the preference. Local decoder initialization/driver failure
after a HEVC stream starts is reported as video failure; there is no unproven
automatic renegotiation/reconnect. Disable the option and reconnect in that case.

The shared WASM configuration reader contains `client_decoder_h265`, but the
active `parsec_web_new_attempt` ABI does not pass it out. Consequently this option
is owned by the native Rust settings/transport, with no WASM patch or JavaScript
injection. Analysis tooling and downloaded SDK binaries remain outside the build.
