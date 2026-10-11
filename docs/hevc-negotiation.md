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
  per-stream flags. The verified H.264 and eight-bit HEVC requests retain the
  older web contract without its native-peer base flag `0x08`. The experimental
  Main10 combination is investigated separately below; flags from different
  peer generations must not be mixed blindly.

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
does not request 4:4:4. A separate experimental 10-bit preference is described below.

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

## Experimental 10-bit path

`preferHevc10bit` is off by default, independent of the existing `preferHevc`
preference. It is effective only with HEVC enabled. The next experimental startup
uses `_flags: 0x19`: HEVC (`0x01`), native base (`0x08`), ten-bit (`0x10`).
The initial `0x11` experiment passed local preflight and SCTP send, but the live
stock host returned Main/8-bit. User verification of build `091ca62` also
returned Main/8-bit with `Sent (startup flags 0x19)` and zero local dropped
frames at capture time. Adding the base bit therefore did not resolve Main10
negotiation in this setup. Stock-client Main10 on the same host has not yet
been tested. The WASM core remains unchanged. No 4:4:4 request is added; H.264
and 8-bit HEVC retain their verified flags of 0 and 1 respectively.

Before requesting 10-bit, the worker checks the selected GPU's HEVC Main10/P010
decode capability, a D3D11-aware P010 transform, P010-to-RGB10 SDR conversion and
RGB10 render/blend/display support. A failed capability check retains the 8-bit
HEVC request. Failures after stream startup are reported rather than claiming
automatic renegotiation; disable 10-bit and reconnect if needed.

Connection stats distinguishes the 10-bit request from the observed decoded
depth. `10-bit HEVC request` reports disabled, the failing local API stage and
HRESULT (when available), or the actual startup flags only after the startup
message has been accepted by the SCTP send API. This does not prove remote
acceptance. A sent request followed by Main/8-bit output means the received
stream remains 8-bit; the cause on the remote side is not established.

### Additional stock-client and lowlat evidence (2026-10-11)

Read-only static inspection of the installed stock `parsecd-150-105c.dll`
(SHA-256 `dbd59739cfb84a239860a30480b91281f0ebd316dfb5bf58bd4539e82c92c425`)
independently confirms bit 4. At RVA `0x49980` the capability builder initializes
flags to 8, adds HEVC at `0x499a0`, tests a requested/supported ten-bit pair at
`0x499c6`/`0x499ce`, and ORs `0x10` at `0x499d4`. The flags reach the opcode-11
serializer through `0x49b29`/`0x49b50`; `_flags` is written at `0xa5bb1` onward.
This establishes the native client's `0x19` combination, not successful ten-bit
negotiation over WebRTC.

At lowlat revision `7b4dc4ae79990ea8c59a108c80b3c89b695c3cb4`,
`crates/host/src/session.rs` normalizes both INIT and ENCODER_CONFIG declarations
with `flags | FLAG_BASE`. Thus its browser example does not establish whether a
stock host requires the base bit: lowlat supplies it on the receiver. Stock host
code also tests `0x08` separately from `0x10` in the capture/encoder setup
(RVAs `0xac18b`, `0xac5aa`, `0xac5bf`); the complete meaning of that base bit
has not been established. Do not treat it as a universally harmless flag or
change the verified eight-bit requests on this evidence alone.

lowlat supports restating per-stream flags through opcode 13 (stream index,
flags, reinitialize). The stock client sends that frame at RVA `0x49b69` onward
for subsequent declarations. No additional reconfiguration message is added in
the base-bit experiment, to keep that test to one variable. The older public
SDK header exposes HEVC and 4:4:4, but no documented ten-bit configuration field;
its padding remains unused. Research scripts and disassemblies stay outside
the application build.

The stock opcode-13 path at `0x49b69` is an alternative branch to the initial
opcode-11 path, not evidence that both must always be sent for the first
stream. Its observed third argument is zero (`0x49b8a`), so a forced
reinitialization must not be attributed to that trace. No speculative extra
message or retry loop is added after the failed base-bit test. Stock encoder
setup separately gates ten-bit and contains a ten-bit fallback after encoder
creation failure (`0xac84b` onward). These static branches do not establish
which path this live host took. A same-host stock-client Main10 comparison
is needed to separate host availability from the WebRTC declaration path.

Actual SPS profile/depth chooses Main/NV12 or Main10/P010, independently of the
preference. Both HEVC modes prefer synchronous D3D11-aware transforms accepting
`CODECAPI_AVLowLatencyMode` via ICodecAPI or the documented `MF_LOW_LATENCY`
transform attribute; public enumeration order breaks equal-capability
ties. This is a supported DXVA-capable path, not proof that one transform is
measurably faster or that every frame executes in hardware. Async MFTs need a
different event-driven pipeline and are not substituted into this synchronous
worker.

P010 decoder textures feed the VideoProcessor directly and render into an
`R10G10B10A2_UNORM` swapchain with full-range gamma-2.2 BT.709 SDR output.
Matrix/range are preserved through documented video color-space APIs and
conversion support is checked. Overlay shaders draw directly into that same
backbuffer. No intermediate decoded-frame texture or CPU readback is added.
Eight-bit streams retain NV12/BGRA8 output. Connection stats reports decoded
depth only after output with the matching texture format has been observed.
Ten-bit SDR does not imply HDR or guarantee the physical monitor's output depth.

An isolated local test decoded a synthetic 1920x1080 Main10 Annex B fixture
through both enumerated Windows transforms into P010 D3D11 decoder-bound
surfaces. The GPU also reported Main10/P010 decode and P010-to-RGB10 conversion
support. Both transforms rejected ICodecAPI queries but accepted MF_LOW_LATENCY;
this is why both documented configuration routes are supported. This test is
not live Parsec-host negotiation or an end-to-end latency measurement. Probe
code, encoder tools and synthetic fixtures remain ignored outside the binary.

[MF_LOW_LATENCY documentation](https://learn.microsoft.com/en-us/windows/win32/medfound/mf-low-latency)
specifies GetAttributes/SetUINT32 for MFTs and explains its shared property GUID.
