# 10-bit SDR RGB output

## Live host compatibility

Main 10 streaming is currently verified only with **NVIDIA host encoding**.
In the 2026-10-17 live comparison, the monitor was connected to the GPU being
tested; virtual-display registry settings were not changed. With the same BUD
client and `0x19` capability declaration, NVIDIA encoding produced 10-bit HEVC
and 10-bit RGB SDR output on AMD Radeon 780M client decoding. AMD 780M host
encoding produced 8-bit HEVC instead. The stock host log confirmed `encoder =
amd` versus `encoder = nvidia`; both logged `input = rgba10`, which describes
input format and does not prove encoded bit depth.

This does not establish an AMD hardware limitation or a universal Parsec AMD
restriction. The reason for the tested host path's 8-bit output is not reported
in the host log. Other host encoders, including Intel, remain unverified for
live Main 10. The client option remains experimental and available; no encoder
vendor is blocked. Client decode/presentation capability is separate from host
encoding, and AMD client decoding is verified in the successful live test.

## Output implementation

Actual P010 decoder output selects `DXGI_FORMAT_R10G10B10A2_UNORM` when
`CheckVideoProcessorFormat` reports output support and
`ID3D11VideoProcessorEnumerator1::CheckVideoProcessorFormatConversion` accepts
the actual input range/matrix and full-range gamma-2.2 BT.709 RGB output.
`IDXGISwapChain3` must accept that SDR color space for presentation.

The existing `VideoProcessorBlt` writes directly into the swap-chain buffer.
There is no additional decoded-frame texture, copy, CPU readback or conversion
pass. Both RGB formats occupy 32 bits per pixel. NV12 input retains BGRA8;
requesting Main 10 does not force RGB10 if the host actually sends 8-bit.
The original-surface NV12 presentation path is unchanged.

Backbuffer views are released and context bindings cleared before ResizeBuffers
changes dimensions or format. Cache identity includes input format and color
metadata. Unsupported RGB10 conversion/presentation uses the existing BGRA8
conversion. A rejected RGB10 resize, color-space setup, output view or blit is
retried once with BGRA8 using the same P010 surface. Device removal/reset/hang
remains an error for normal recovery. Fallback is retained for that renderer's
lifetime rather than retried on every frame.

Stats and optional event logs distinguish decoded depth from RGB swap-chain
depth and keep a detailed fallback reason. RGB10 is application precision, not
proof of 10-bit DWM composition, scanout, a display cable or panel. This remains
SDR; no PQ/BT.2020 signaling, HDR metadata or Windows HDR switch is used.

## Verification

Regression tests cover input matrix/range mapping, independently reported output
depth, and actual D3D11 overlay rendering into BGRA8 and RGB10 targets (including
transparency, clipping and inherited GPU state). The overlay tests use WARP and
read only their synthetic targets.

On a PC with D3D11 video processing, the separate diagnostics executable accepts:

```text
parsec-native-diagnostics.exe video-hardware-probe rgb10-output.json --rgb10
```

It feeds only generated NV12/P010 grey ramps through the real renderer, changes
8-bit/10-bit input and full/studio range, reads the synthetic target **before**
Present, and requires more than 256 distinct red levels when RGB10 is active.
It also forces the unsupported-output fallback. Unsupported conversion is
reported as 8-bit/fallback, not treated as proof of successful RGB10. Live
decoding never enables this readback. The existing `--main10` probe separately
checks actual HEVC Main 10 decoding to P010.

Measured on 2026-10-17 using build `8ede7e7` on the local AMD Radeon 780M:

- [Generated ramp](measurements/rgb10-output-2026-10-17.json): RGB10 retained
  1,022 distinct red levels for full-range P010 and 875 for studio-range P010.
  NV12 returned to 8-bit RGB, and the forced unsupported-output case retained
  P010 input with 8-bit RGB output and an explicit fallback reason.
- [Actual Main 10 fixture](measurements/main10-rgb10-2026-10-17.json): Media
  Foundation decoded and presented 119 frames as 10-bit P010 / 10-bit RGB with
  no fallback or failure. This probe performs no pixel readback.
- [Windows CI](https://github.com/vampywiz17/ParsecWebTurn/actions/runs/39027421355)
  passed normal/diagnostic tests, strict Clippy, packaging, original-WASM probes
  and independent WebRTC UDP/TCP/TLS relay tests. Hardware measurements above
  were local, not hosted-runner hardware or proof of physical 10-bit scanout.

Intel and NVIDIA use this same documented renderer, but these measurements
establish RGB10 behavior only on the tested AMD configuration.

Documented APIs: [swap-chain formats](https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/ns-dxgi1_2-dxgi_swap_chain_desc1),
[conversion support](https://learn.microsoft.com/en-us/windows/win32/api/d3d11_1/nf-d3d11_1-id3d11videoprocessorenumerator1-checkvideoprocessorformatconversion),
[SDR color-space selection](https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_4/nf-dxgi1_4-idxgiswapchain3-setcolorspace1).
