# GPU comparison and opt-in BUD HEVC Main 10

The live test client retains Media Foundation and its existing D3D11 video
processor/DXGI renderer. The proprietary WASM is unchanged. AMF and NVDEC are
offline research dependencies, never linked by the BUD or default WebRTC client.

## Copy contract

H.264/8-bit HEVC use the decoder's NV12 texture; opt-in BUD Main 10 uses its
P010 texture. The texture is fed directly to the existing video processor on
the same selected D3D11 device. There is no application decoded-frame copy,
CPU pixel readback or additional plane export/split. The existing single color
conversion/scaling write into the BGRA8 presentation buffer remains necessary.
P010 decode does not make the display output HDR or 10-bit: BGRA8 remains the
output format. Driver-internal work/copies cannot be counted by these probes.

## Offline procedure

Build `src-experimental` with `gpu-probes`, run `vendor-gpu-probe.exe` and retain
its JSON stdout and progress stderr. No account, profile, network or host is
used. Lowlat and fixtures are pinned to
`5f8db5689e55de0d396b0520824caa9fc053dd86` (MIT).

Each codec uses the same 120-frame synthetic 1280x720 clip for both backends.
The compressed HEVC SPS declares no picture reordering. Tests run at paced
30/60 FPS, three rounds with alternating MF/AMF order, excluding the first
16 outputs. Timestamps must match original inputs and all 120 frames must
arrive. p50 is the upper middle; p95 is nearest rank. GPU adapter LUID/driver,
output format and ownership, processor input acceptance and GPU conversion
are checked. Decoder output is never mapped/read on CPU.

Both backends query output immediately; MF is polled between submissions too,
so a transient NEED_MORE_INPUT is not automatically charged as a 16/33 ms
application submission delay. Earlier screening without this extra polling is
not the final comparison. A separate completion metric waits on a device fence
after GPU-view validation/conversion: it includes offline setup, excludes
network/presentation, and is not a pure decode or end-to-end latency metric.
These synchronous diagnostic waits must not be copied into the playback loop.

AMF requests `LowLatencyDecode` and low-latency `ReorderMode`, reporting each
property's acceptance. `SurfaceCopy` and `EnableDecoderSmartAccessVideo` are
explicitly disabled. The probe refuses CPU or cross-device output.
The baseline uses the live client's supported low-latency MF requests, including
the HEVC `MF_LOW_LATENCY` alternative, rather than an intentionally slow baseline.

## Results on the development machine (2026-10-14)

[Raw fixture-only results](measurements/gpu-vendor-2026-10-14.json), source
`9fc18fac7904cb4c638b4d46ff0b05d4b23af56b`. AMD Radeon 780M driver
32.0.31041.1004; NVIDIA GeForce RTX 3060 driver 32.0.16.1742.
All **54 trials / 6480 frames** passed, including direct GPU conversion from
NV12/P010, with no application decoded-frame copies or CPU readback.

60 FPS submission-to-output p50 ranges across three rounds, in milliseconds:

| GPU | Codec | Existing low-latency MF | Direct AMF |
| --- | --- | ---: | ---: |
| Radeon 780M | H.264 | 0.223–0.243 | 0.198–0.206 |
| Radeon 780M | HEVC Main | 16.851–16.867 | 0.516–0.561 |
| Radeon 780M | HEVC Main 10 | 16.856–16.864 | 0.509–0.520 |
| RTX 3060 | H.264 | 0.605–0.651 | Not applicable |
| RTX 3060 | HEVC Main | 17.217–17.334 | Not applicable |
| RTX 3060 | HEVC Main 10 | 19.764–19.987 | Not applicable |

All HEVC MF trials, at both 30 and 60 FPS, required one later input before
output; the AMF trials required none. This persisted when MF output was polled
between inputs, so it is not explained by our submission-tick polling alone.
H.264 has no comparable one-frame delay in either backend. AMF HEVC is a useful
candidate for further integration; H.264's small difference is insufficient
reason to replace a working path. These results are for these synthetic clips,
GPU drivers and offline setup, not a stock-host/end-to-end speed guarantee.
Repeated processor/view/output allocation and per-frame fence waits in the
conversion validation also perturb scheduling; the earlier availability-only
control is consistent with the HEVC one-input lag, but absolute NVIDIA timings
changed when conversion was enabled. Do not present those numbers as isolated
GPU decode times or compare unlike diagnostics as a renderer benchmark.
No live AMF/NVDEC backend was enabled by these measurements.

## NVIDIA boundary

On RTX 3060 the driver creates H.264, HEVC Main and Main 10 NVDEC decoders.
This is a capability/creation check, not an NVDEC decode latency measurement.
Lowlat's pinned Windows NVDEC exporter copies decoded planes into D3D textures;
that implementation does not meet our current no-additional-copy acceptance.
It is not integrated. NVIDIA hardware is tested through our existing MF path.
This is not evidence that every possible NVDEC renderer must add a copy.

Sources: [lowlat's pinned Windows exporter](https://github.com/nomi-san/lowlat/blob/5f8db5689e55de0d396b0520824caa9fc053dd86/crates/decode/src/nvdec/windows.rs),
[NVIDIA D3D11 interop format restrictions](https://docs.nvidia.com/cuda/cuda-driver-api/cuda_driver_api/group__CUDA__D3D11.html),
[AMD decoder API](https://github.com/GPUOpen-LibrariesAndSDKs/AMF/blob/master/amf/doc/AMF_Video_Decode_API.md),
[AMD decoder properties](https://github.com/GPUOpen-LibrariesAndSDKs/AMF/blob/master/amf/public/include/components/VideoDecoderUVD.h).

## BUD Main 10 test

The optional BUD test build adds **Request 10-bit HEVC (BUD only)**, disabled by
default and enabled in Settings only with BUD and Prefer HEVC selected. The
selected GPU must advertise Main 10/P010 decode and video-processor input, and
a synchronous D3D11-aware MF HEVC transform must be installed. The native BUD
initialization advertises only supported 4:2:0 formats; 4:4:4 stays disabled.
The existing 8-bit host fallback remains supported.

Actual SPS/profile/depth selects NV12 or P010; 10-bit input is refused on the
default WebRTC policy. Decoded texture metadata supplies **Decoded color depth**
in BUD stats. A preference or capability check alone is not proof of a 10-bit
stream. Confirm **Main 10 + 10-bit** from a stock host after saving/reconnecting,
compare both GPUs and toggle the preference off to recheck 8-bit HEVC. No stock
host Main 10 playback claim is made before this manual test.
