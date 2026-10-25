> Historical investigation results. One-off comparison executables and test-only
> build gates have been retired; current behavior and build instructions are in
> the [project README](../README.md).

# AMD HEVC presentation validation

The test build uses AMF for normal HEVC playback and the existing Direct D3D11
decoder with Windows Presentation API for optional original-surface presentation.
H.264 remains Media Foundation. Neither transport nor the embedded WASM changes.
The `amd-video-test` feature enables AMD only in explicitly built test binaries.

## Copy and ownership policy

Normal AMF output feeds the existing video processor on the decoder's own device.
`SurfaceCopy` and SmartAccessVideo are disabled. The existing color conversion /
output write remains, with no application decoded-frame intermediate copy,
plane export or CPU readback. AMD documents the decoder's low-latency reorder
mode and `SurfaceCopy` in its [decoder interface](https://github.com/GPUOpen-LibrariesAndSDKs/AMF/blob/master/amf/public/include/components/VideoDecoderUVD.h).

Optional original-surface presentation uses independently allocated, displayable
NV12 decoder textures and the same presentation/lease implementation as NVIDIA
and Intel. It does not wrap or copy AMF output. The existing runtime checks for
displayable textures, presentation support and independent decoder surfaces are
retained. At most two presentation leases remain outstanding; busy display
updates are skipped while every encoded picture continues to be decoded.

The supported allocation flags and NV12 format are documented by Microsoft in
[Displayable surfaces](https://learn.microsoft.com/en-us/windows/win32/direct3d11/displayable-surfaces).
This removes the application's conversion pass, but does not prove direct
scanout or the absence of internal Windows/driver copies, including on hybrid
GPU display configurations. Unsupported paths retain the existing GPU recovery.

## Local synthetic results, 2026-10-15

GPU: AMD Radeon 780M Graphics, driver 32.0.31041.1004.
Fixture: 120 HEVC Main 8-bit 4:2:0 pictures at 1280 x 720; no account or network.

- Normal AMF: build `a9021fa`; 120/120 decoded outputs and direct processor input
  views on the same D3D11 device, zero later inputs required for output. One
  conversion/output write, no application decoded-frame copies or CPU readback.
- Original NV12 presentation: build `ea3581b`; `display-probe --amd` submitted
  120/120 pictures at 60 fps, with zero busy display skips.
- Unpaced burst: all 120 pictures decoded; 5 display updates submitted and 115
  skipped because presentation was busy. No unbounded queue or decoder failure.
  These presentation counts depend on scheduling and are not fixed expectations.
- Empty close and deliberately truncated SPS startup failure: both closed safely.

The presentation probe verifies successful API submission and bounded ownership,
not visual correctness, scanout mode or end-to-end latency. Live motion, colors,
overlays, resize and reconnect still need verification independently with both
WebRTC and BUD using the full test client; see [current client instructions](../README.md).
