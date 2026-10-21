> Historical investigation results. One-off comparison executables and test-only
> build gates have been retired; current behavior and build instructions are in
> the [project README](../README.md).

# 8-bit GPU decoder comparison

The standard client retains Media Foundation. The optional vendor-video test
build uses measured HEVC backends with the existing D3D11 processor/DXGI renderer.
The proprietary WASM is unchanged. No NVDEC exporter is used.

## Copy contract

H.264 and 8-bit HEVC use the decoder's NV12 texture. The texture is fed directly to the existing video processor on
the same selected D3D11 device. There is no application decoded-frame copy,
CPU pixel readback or additional plane export/split. The existing single color
conversion/scaling write into the BGRA8 presentation buffer remains necessary.
BGRA8 remains the output format. Driver-internal work/copies cannot be counted by these probes.

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
The retained **36 8-bit trials / 4320 frames** passed, including direct GPU conversion from
NV12, with no application decoded-frame copies or CPU readback.

60 FPS submission-to-output p50 ranges across three rounds, in milliseconds:

| GPU | Codec | Existing low-latency MF | Direct AMF |
| --- | --- | ---: | ---: |
| Radeon 780M | H.264 | 0.223–0.243 | 0.198–0.206 |
| Radeon 780M | HEVC Main | 16.851–16.867 | 0.516–0.561 |
| RTX 3060 | H.264 | 0.605–0.651 | Not applicable |
| RTX 3060 | HEVC Main | 17.217–17.334 | Not applicable |

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

## Direct D3D11 comparison and integration (2026-10-14)

[Raw three-way results](measurements/gpu-direct-2026-10-14.json), source
`a4ff06d92c2156a7c0c013cf07a42f79ee7ca664`: **60 trials / 7200 frames**, all
passed on the same Radeon 780M and RTX 3060. H.264 and HEVC were each tested
at 30/60 FPS, three alternating rounds per backend. Original decoder NV12
array slices feed the same processor directly. The same visible 1280x720 area
is consumed regardless of padded decoder allocation height. No decoded-frame
copy, shader plane split or CPU pixel access is used.

60 FPS p50 ranges across rounds, submission through offline GPU conversion
completion, milliseconds (includes repeated diagnostic resource setup):

| GPU | Codec | MF | AMF | Direct D3D11 |
| --- | --- | ---: | ---: | ---: |
| RTX 3060 | H.264 | 3.206–3.266 | — | 3.182–3.289 |
| RTX 3060 | HEVC | 19.217–19.257 | — | 2.574–2.617 |
| Radeon 780M | H.264 | 5.248–5.406 | 3.716–5.112 | 4.800–6.158 |
| Radeon 780M | HEVC | 21.676–23.320 | 4.152–5.418 | 5.144–6.254 |

For HEVC both direct D3D11 and AMF eliminate the MF one-later-input output lag
on these clips; the lag also appears at 30 FPS in MF. Direct D3D11's AMD output
availability is faster (0.175–0.194 ms p50) than AMF's (0.515–0.576 ms), but
that alone is misleading: through conversion completion, AMF is generally the
better AMD candidate. NVIDIA direct D3D11 improves both availability and
completion compared with MF. H.264 has no comparable one-frame lag; overlapping
rounds/tails do not justify changing its existing MF path. These are synthetic
screening results, not a guarantee of equal savings on every host/driver.

The optional `vendor-video` native build feature therefore selects **AMD AMF
for AMD HEVC** and **direct D3D11 for NVIDIA HEVC**. H.264, Intel and other GPUs
retain MF. The GPU module is independent of BUD/WebRTC and feeds the existing
single video-processor/DXGI path. Normal builds keep the original MF decoder.
The BUD test artifact enables both optional features so either
transport can be tested with the new HEVC paths.

Initialization failures retain MF. A live vendor failure or unsupported format
change enters the existing keyframe recovery and disables vendor acceleration
for the remainder of that connection. The actual decoder and any fallback are
reported in Connection stats. No transport rerouting accompanies decoder fallback.
AMF refuses reordered, non-8-bit or non-4:2:0 input; unsupported streams stay on
MF. Surfaces are leased through queuing of the processor read on the same GPU.
No per-frame CPU wait for GPU completion is used in playback.

Before normal distribution, verify stock-host picture colors/cropping, sustained
motion, resolution changes and disconnect/reconnect on both GPUs. Offline API
success/frame completeness is not a pixel-correctness or real-host acceptance
claim. The existing output remains BGRA8. High-depth experiments were removed.

Source: [documented D3D11 decode ownership and operations](https://learn.microsoft.com/en-us/windows/win32/medfound/supporting-direct3d-11-video-decoding-in-media-foundation).

[Integrated ownership/metadata validation](measurements/gpu-integrated-2026-10-14.json)
from `7c2c0a053a3a23955aa28a58cccdda06a8991183` also passed 120 HEVC frames on
AMD AMF and 120 on NVIDIA direct D3D11, using the exact owned module linked by
the test client. It verified timestamp correlation, original-device NV12 input
and the same single GPU conversion, with zero later-input lag and no pixel
readback. This is a separate functionality/ownership check, not an additional
controlled backend speed comparison.
