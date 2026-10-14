# Isolated native development gates

This crate is **not linked or packaged by the WebRTC client**. It pins lowlat's
MIT-licensed client/driver libraries to `5f8db5689e55de0d396b0520824caa9fc053dd86`.
No lowlat GPL codec feature is enabled. These are implementation gates, not a
claim that a playable BUD client or a faster Intel backend has shipped.

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
A single VPL trial remains screening evidence, not a repeatability guarantee.

Acceptance for live integration: at most the existing GPU output/conversion
write, no CPU video path, no added decoded-frame GPU copy, correctly retained
surfaces/fences, and repeatable latency no worse than enabled low-latency Media
Foundation. A capability probe alone cannot satisfy that acceptance.

## BUD direct and UDP TURN

`bud::Session` implements an isolated attempt lifecycle, native credentials,
answer/candidate exchange, readiness and teardown on the lowlat client. An
attempt chooses `Route::Direct` or `Route::TurnUdp` explicitly. TURN credentials
use lowlat's redacted, zeroized relay configuration. There is no WebRTC fallback,
TCP/TLS reinterpretation or shared WebRTC state.

`signaling::Signaling` emits native offers (`mode: 1`, including the native media
key), scopes incoming answers/candidates by attempt and bounds message size.
It neither authenticates an account nor opens a websocket. Its offer output is
sensitive: send it only to authenticated account signaling; never log it.

The current session constructor is **transport-only** and deliberately drops
video. It is not a remote desktop implementation. This prevents the SDK's
split-plane export from being adopted as equivalent to our existing direct NV12
renderer. A playable integration still needs the GPU output contract, account
connection, input/audio/UI wiring and stock-host acceptance for both routes.

Tests run real UDP connectivity between two native SDK peers, directly and
through an isolated coturn instance. This does not prove encrypted media,
stock-host compatibility or Cloudflare interoperability. Those remain explicit
acceptance gates. The production WebRTC regression workflow is unchanged.

Build/test (explicit target avoids the repository's Windows default on Linux):

```text
cargo test --locked --manifest-path src-experimental/Cargo.toml --target x86_64-pc-windows-msvc
cargo build --release --locked --manifest-path src-experimental/Cargo.toml --target x86_64-pc-windows-msvc
```

The ignored `turn_udp_connectivity` test runs only against the isolated CI coturn
fixture; its credentials are dummy data. Never point it at a production relay.
