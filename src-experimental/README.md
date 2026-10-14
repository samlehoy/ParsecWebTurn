# Isolated native development gates

This crate is **not linked or packaged by the WebRTC client**. It pins lowlat's
MIT-licensed client/driver libraries to `5f8db5689e55de0d396b0520824caa9fc053dd86`.
No lowlat GPL codec feature is enabled. These are isolated implementation gates and optional BUD test builds, not a
claim that a faster Intel decoder or BUD has shipped in the default client.
The native client's optional `experimental-bud` feature links the transport/media
adapter only. Offline decoder probes require the separate `gpu-probes` feature;
they are not part of that BUD feature or the normal client.

## Intel first

Run `intel-gpu-probe.exe` on the Intel test laptop. It discovers physical Intel
adapters by PCI vendor and LUID, loads their installed current VPL runtime, binds
the caller-owned, multithread-protected D3D11 device and queries codec support.
Older runtimes that require host-memory planes are explicitly refused.

The H.264 fixture submits 80 pictures, checks native NV12 output on the same
D3D11 device, requires all 80 outputs and records CPU output availability
separately from GPU completion. It does not map pixels, export planes, create
staging textures or copy decoded frames. The completion wait is offline-only;
it must not be copied into a playback loop. Runtime-internal copies are unknown.
An additional 120-frame, 8-bit HEVC fixture checks the same native texture
contract. Neither fixture validates live playback, image correctness or
presentation yet; the two different clips are not a codec speed comparison.

The first target is the user's Core Ultra 5 235U integrated Intel GPU. Its actual
adapter and driver report, rather than the generic Windows display name, decide
compatibility. No Intel hardware is available on the development machine.

`Run-Intel-GPU-Comparison.cmd` also runs `mf-baseline-probe.exe`, built from
the existing offline Media Foundation latency probe without duplicating its
source. Compare only H.264 **output availability**, the same 1920x1080 fixture,
the same adapter, and MFT trials with `low_latency_requested: true` and
`request_accepted: true`. Both exclude the first 16 outputs as warm-up. The MFT
probe does not measure GPU completion; do not compare it to VPL's GPU completion
array. Disabled-low-latency MFT trials are controls, not the production baseline.
VPL runs five trials at the same 60 FPS input cadence. Output timestamps must
correlate to their original submissions. Match the adapter LUID from both
reports; names alone are insufficient. Compare upper-middle p50 and nearest-rank
p95 over the 64 post-warm-up outputs, then summarize the five trials. This is
screening evidence; neither report measures presentation, and it cannot establish
end-to-end playback improvement or internal driver copy counts.

Acceptance for live integration: at most the existing GPU output/conversion
write, no CPU video path, no added decoded-frame GPU copy, correctly retained
surfaces/fences, and repeatable latency no worse than enabled low-latency Media
Foundation. A capability probe alone cannot satisfy that acceptance.

## AMD/NVIDIA 8-bit screening

`vendor-gpu-probe.exe` runs the same 120-frame synthetic 720p H.264 and HEVC Main clips on each physical AMD/NVIDIA adapter. It compares AMF
against the current low-latency MF requests on AMD; NVIDIA uses MF plus separate
NVDEC decoder-creation checks. NVDEC's pinned D3D11 plane exporter adds copies
and is deliberately not used. This is not a measured NVDEC decoding comparison.

Every decoded output must be GPU-only NV12 on the caller's D3D11 device.
Its original texture is the video processor input; the final probe also executes
the existing single BGRA output conversion. No decoded pixels are mapped to CPU,
no intermediate decoded-frame textures or plane split/export are used. Offline
fence waits and repeated view/output-resource construction are diagnostic work,
not changes to the live decoder. Output availability is measured before that
work; completion timings include it and must not be treated as pure decode time.
See [method, results and limitations](../docs/gpu-vendor-comparison.md).

## BUD direct and UDP TURN

`bud::Session` implements an isolated attempt lifecycle, native credentials,
answer/candidate exchange, readiness and teardown on the lowlat client. An
attempt chooses `Route::Direct` or `Route::TurnUdp` explicitly. TURN credentials
use lowlat's redacted, zeroized relay configuration. There is no WebRTC fallback,
TCP/TLS reinterpretation or shared WebRTC state.

`signaling::Signaling` emits native offers (`mode: 1`, including the native media
key), scopes incoming answers/candidates by attempt and bounds message size.
It does not authenticate an account itself. The native adapter deliberately
shares the existing certificate-verified account WSS interface: only active BUD
attempt messages are adapted, and AES keys stay outside the WASM guest. Never
log native offer/answer material. An unused standalone WSS prototype was removed.

`Session::external_video` hands borrowed compressed video/audio leases
to the application: the SDK's video decoder/export and audio decoder do not run.
The narrow vendored client seam is documented in `vendor/lowlat-client/PARSECWEBTURN.md`.
The optional `src-native` feature `experimental-bud` connects these payloads to
the existing MF/D3D11 video and libopus pipelines. It adds no alternate decoder,
plane-splitting renderer or GPU copy path. Normal builds do not include it.
The optional test client has explicit account/UI lifecycle wiring and a Settings
checkbox. The user confirmed stock-host playback over direct and UDP TURN routes.
It now retries an unestablished direct connection once through UDP TURN, using a
fresh native attempt and cancelling the old offer. This retry needs real-network
acceptance. Force TURN starts on relay immediately; STUN-only never retries.
Native BUD statistics hide ICE/DTLS and display actual transport counters,
including incoming/outgoing UDP payload traffic with protocol overhead.

The vendored client is now a transport-only subset: unused decoder/exporter,
audio-decode, microphone, cursor-cache, input-mapper and port-mapping code and
dependencies have been removed. The app retains its existing MF/D3D11 and Opus
paths, with no additional GPU processing or decoded-frame copy.

Tests run real UDP connectivity between two native SDK peers, directly and
through an isolated coturn instance, including encrypted control delivery. These tests do not by themselves prove stock-host media,
stock-host compatibility or Cloudflare interoperability. Manual
acceptance remains separate from these automated transport gates. The production WebRTC regression workflow is unchanged.

Build/test (explicit target avoids the repository's Windows default on Linux):

```text
cargo test --locked --manifest-path src-experimental/Cargo.toml --target x86_64-pc-windows-msvc
cargo build --release --locked --manifest-path src-experimental/Cargo.toml --target x86_64-pc-windows-msvc --features gpu-probes
```

The ignored `turn_udp_connectivity` test runs only against the isolated CI coturn
fixture; its credentials are dummy data. Never point it at a production relay.

### Optional BUD test client

The `experimental-bud` native feature now provides a Settings checkbox and binds
BUD to the existing computer picker, account socket, input UI, MF/D3D11 and Opus
pipelines. The opaque authenticated account WebSocket is a deliberately shared
interface: only active BUD attempt messages are adapted; normal WebRTC messages
are unchanged. Native AES credentials never enter the WASM guest. Direct UDP and
UDP TURN use separate native attempts; automatic relay retry stays within BUD.
There is no TCP/TLS reinterpretation or automatic WebRTC fallback. See [test instructions](../docs/BUD-TEST.txt).

Automated direct/isolated TURN tests include encrypted control delivery in both
directions. Stock-host direct and relay playback were confirmed by the user; automatic
relay retry remains to be checked with the new test client. This is an optional test artifact, not the distributed default client.
