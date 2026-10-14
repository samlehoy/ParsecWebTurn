# Next development: vendor decoding and optional BUD

Scope agreed 2026-10-14. These are planned work, not shipped capabilities.
Reference: lowlat `feat/win-seams`, pinned research revision
`5f8db5689e55de0d396b0520824caa9fc053dd86`.

## 1. Vendor decoder backends, starting with Intel VPL

Priority updated at the user's request: Intel first, targeting the integrated
GPU in the Core Ultra 5 235U test laptop. The independent `src-experimental`
crate now binds current VPL to a caller-owned D3D11 device and checks original
NV12 surfaces with an offline fixture. It refuses legacy host-memory output.
It is not a live playback integration or evidence of a latency improvement.
See [implementation gates](../src-experimental/README.md).

Intel's current runtime supports caller-owned GPU surfaces. The legacy path in
the reviewed lowlat revision uses CPU planes and is excluded. Lowlat itself
does not automatically prefer VPL over its direct D3D11 decoder on Intel;
vendor APIs alone are not a reason to replace our working path. The new path
must use no more decoded-frame GPU copies than our existing renderer, with no
CPU video processing or fallback. Device/surface lease ownership and GPU ordering
must be proven before any live switch is exposed.

### Subsequent AMD evaluation

AMF is a subsequent candidate because lowlat documents a measured HEVC benefit and
we can test an AMD Radeon 780M locally. The public AMD decoder API exposes
`AMF_VIDEO_DECODER_LOW_LATENCY` (`LowLatencyDecode`) separately from low-delay
output ordering (`ReorderMode`). Lowlat queries support and requests both;
availability and accepted properties must be established on the actual driver.

Primary references:

- [AMD decoder API](https://github.com/GPUOpen-LibrariesAndSDKs/AMF/blob/master/amf/doc/AMF_Video_Decode_API.md)
- [AMD public decoder header](https://github.com/GPUOpen-LibrariesAndSDKs/AMF/blob/master/amf/public/include/components/VideoDecoderUVD.h)
- [lowlat AMF backend](https://github.com/nomi-san/lowlat/blob/5f8db5689e55de0d396b0520824caa9fc053dd86/crates/decode/src/amf.rs)
- [lowlat Windows measurements](https://github.com/nomi-san/lowlat/blob/5f8db5689e55de0d396b0520824caa9fc053dd86/docs/impl-plan-windows.md)
- [NVIDIA NVDEC API](https://docs.nvidia.com/video-technologies/video-codec-sdk/13.0/nvdec-video-decoder-api-prog-guide/index.html)
- [Intel VPL](https://www.intel.com/content/www/us/en/docs/oneapi/programming-guide/2023-2/intel-oneapi-video-processing-library-onevpl.html)

For AMD, build an isolated AMF vs existing Media Foundation comparison with identical
synthetic 8-bit H.264 and HEVC access units, the same selected GPU, and paced
input at both 30 and 60 FPS. Measure CPU submission/output availability separately
from GPU completion and any conversion/export work. The previous H.264 MFT test
already establishes that our existing low-latency mode avoids extra output hold
on this AMD GPU; AMF must beat that enabled baseline, not a deliberately disabled
decoder property. Compare warm-up separately, preserve input/output correlation,
and verify complete output and correct pixels in an offline-only readback check.

Use the rendering D3D11 device where supported. Live output must remain on the
GPU, without a CPU plane fallback presented as equivalent. AMF output availability
can precede GPU completion: order dependent work on the documented device/context
and account for fences and outstanding work. Do not introduce an extra decoded
texture copy unless the interoperability requirement is demonstrated and measured.
Lowlat's shared plane textures involve a compute split and sometimes a preliminary
GPU copy; that export design is not automatically preferable to our direct NV12
VideoProcessor input.

Only after a repeatable benefit is established should an opt-in decoder adapter
be connected to the existing media path. Keep Media Foundation usable. NVIDIA
NVDEC follows with its own hardware tests; SDK names alone do not
prove lower delay, and Intel's driver issue is not assumed fixed by a new API.
This work is separate from BUD: a vendor decoder can receive the same compressed
WebRTC media without changing its network transport. Ten-bit and 4:4:4 remain
outside this initial integration scope.

## 2. Experimental BUD, including UDP TURN

**BUD is not limited to direct connections.** Lowlat's client allocates a standard
UDP TURN relay itself, advertises its relayed address and wraps connectivity and
media traffic on the client's leg. The host sends to that reachable address and
does not need to implement TURN. Supplying `Config::relay` makes lowlat's attempt
relay-only; its model differs from our WebRTC automatic ICE choice.

References:

- [Client relay design](https://github.com/nomi-san/lowlat/blob/5f8db5689e55de0d396b0520824caa9fc053dd86/docs/10-client.md#11-the-relay)
- [Relay connectivity](https://github.com/nomi-san/lowlat/blob/5f8db5689e55de0d396b0520824caa9fc053dd86/docs/03-connectivity.md#7-relay)
- [Client configuration](https://github.com/nomi-san/lowlat/blob/5f8db5689e55de0d396b0520824caa9fc053dd86/crates/client/src/config.rs)
- [SDK API](https://github.com/nomi-san/lowlat/blob/5f8db5689e55de0d396b0520824caa9fc053dd86/crates/sdk/src/abi/client.rs)

The reviewed configuration takes one resolved UDP relay endpoint and credentials.
Multiple-endpoint selection and Cloudflare credential generation are application
work. Do not silently reinterpret TCP/TLS-only settings as UDP. Start with an
isolated coturn fixture, then confirm the same mechanism with Cloudflare and a
stock Parsec host. Include allocation, permission, channel-bind and stale-nonce
renewals, a long session, disconnect and reconnect. Upstream's recorded stock-host
tests are encouraging, but do not prove our signaling or public relay setup.

### Required separation

- WebRTC stays the default. BUD is explicit opt-in and labeled experimental.
- Keep `attempt.rs`, its SDP/ICE credentials, candidate gates, readiness rules,
  TURN tiers, renewal policy and SCTP ordering/reliability on the existing path.
  Add a separate BUD session/signaling adapter instead of injecting BUD conditions
  throughout that implementation.
- Do not pass native BUD media framing or crypto credentials into web parsers.
  Verify how the retained WASM account/UI can supply native signaling before
  promising a small API-only migration. The SDK does not supply account signaling.
- The SDK decodes media and exports frame leases; it does not merely swap the
  socket below our current decoder. Audit the frame/handle interface, GPU identity,
  fence ownership and lease lifetime before any renderer integration. CPU planes
  must not replace the existing GPU path as an unnoticed fallback.
- Share only deliberate application interfaces and read-only provider data.
  Separate attempt state, cancellation, profiles and credentials. No implicit
  transport fallback or retries that change a user's choice.
- Preserve working WebRTC TURN/UDP, TURN/TCP and TURN/TLS. TLS is important on
  networks where UDP is blocked. BUD's current UDP relay is an additional option,
  not a substitute for these restricted-network routes.
- Run the unchanged WebRTC direct/relay regression suite with BUD disabled, then
  validate BUD direct and relay separately. A usable UI switch comes after these
  gates, not before an actual BUD connection exists.

Initial acceptance is a working stock-host session over each BUD route with
correct input, audio, GPU video and teardown, plus no WebRTC regressions. Compare
the two protocols on the same host/path under controlled loss and jitter; do not
assume every BUD loss recovers within one RTT. A unified layered UI is a separate,
deferred change, not a prerequisite allowed to modify the stable WebRTC path.
