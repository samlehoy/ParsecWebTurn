# HEVC negotiation experiment

> Historical investigation below. Correction from the lowlat maintainer and the
> client branch, 2026-10-16: `0x08` means full color range, not a generation/base
> flag. Current native declarations are `0x08` (H.264), `0x09` (HEVC Main), or
> `0x19` (opt-in Main 10 with verified GPU capability). Later stream-0 encoder
> requests preserve the same negotiated codec/range/depth flags. The old failure
> to obtain Main 10 does not establish a web-core or host limitation. Live Main
> 10 negotiation was verified on 2026-10-17 with NVIDIA host encoding and AMD
> 780M client decoding/RGB10 output. The same client received 8-bit HEVC from
> the tested AMD host encoder. Other host encoders are not established as
> working Main 10 paths. 4:4:4 remains deliberately unsupported.

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

The HEVC input type includes the coded width/height and profile parsed from the
actual SPS, before output negotiation. A local isolated Media Foundation probe
reproduced `SetOutputType(NV12)` failing with `0xC00D36E6` without input geometry,
and succeeding with that geometry supplied. Separate compressed VPS headers are
retained within a bounded startup buffer until SPS arrives, then submitted in
their original order. This adds no decoded-frame copy or ongoing frame delay.
SPS reading handles emulation prevention, sublayer profile/level fields and
conformance-window bounds; the decoder remains responsible for full validation.

HEVC decoder discovery uses [MFTEnumEx](https://learn.microsoft.com/en-us/windows/win32/api/mfapi/nf-mfapi-mftenumex),
and the [documented HEVC transform](https://learn.microsoft.com/en-us/windows/win32/medfound/h-265---hevc-video-decoder)
accepts Annex B elementary stream input. The existing NV12 D3D11 texture /
VideoProcessor / DXGI presentation path is reused. No live CPU readback or
intermediate decoded-frame copy is added. Capability checks do not establish
actual hardware execution; existing output telemetry retains that distinction.

HEVC first requests `CODECAPI_AVLowLatencyMode` through ICodecAPI. If that
interface or property is rejected, it requests
[MF_LOW_LATENCY](https://learn.microsoft.com/en-us/windows/win32/medfound/mf-low-latency)
through the transform's attributes as UINT32=1, before setting media types or
starting the stream. If both routes are rejected, decoding remains available
and the existing acceptance flag stays false. Acceptance does not establish
a measured latency improvement. H.264 setup, video queues, GPU presentation
and eight-bit-only HEVC support remain unchanged.

End-to-end user verification on 2026-10-11 confirmed visible video from a Parsec
host over this WebRTC path: HEVC Main, 1920x1080, Media Foundation HEVC decoding,
D3D11 VideoProcessor / DXGI presentation on AMD Radeon 780M Graphics. The supplied
statistics showed D3D11 decoder-surface output and zero local dropped frames at
capture time. This establishes interoperability for the tested setup, not
universal host/driver support, sustained stability or a measured latency claim.
The hardware status remains "Likely" based on decoder-surface evidence, not an
execution counter. A host may still send H.264 despite the preference.
Local decoder initialization/driver failure
after a HEVC stream starts is reported as video failure; there is no unproven
automatic renegotiation/reconnect. Disable the option and reconnect in that case.

The shared WASM configuration reader contains `client_decoder_h265`, but the
active `parsec_web_new_attempt` ABI does not pass it out. Consequently this option
is owned by the native Rust settings/transport, with no WASM patch or JavaScript
injection. Analysis tooling and downloaded SDK binaries remain outside the build.
