# GPU-only 8-bit video backends

Independent of signaling, WebRTC/BUD, audio and UI. Optional native feature
`vendor-video`; the standard build retains Media Foundation.

HEVC: AMD AMF with low-latency decode/reorder and SurfaceCopy/SmartAccess Video
disabled; NVIDIA direct D3D11 Video through our trimmed parameter staging.
H.264 and other GPUs retain MF because no convincing improvement was measured.
Original NV12 decoder textures go directly to the existing D3D11 processor on
the same device, with no CPU pixel output, plane exporter or intermediate copy.
Only the existing conversion/scaling write to the BGRA presentation target stays.

`direct.rs` derives parameter staging from MIT-licensed lowlat d3d11.rs at
5f8db5689e55de0d396b0520824caa9fc053dd86. Its split, GPU/CPU copy exporters,
staging/readback, full chroma and high-depth profiles were omitted. Standard
bounded H.264/HEVC readers and Windows ABI bindings remain pinned dependencies.

self_cell owns driver/device/context/component lifetimes without leaked devices
or lifetime transmutation. AMF output polling waits only for an available surface
(maximum 20 ms before recovery), never for GPU completion. Both backends keep
the output leased through queueing of the existing processor read. Caller must
use the returned device and render synchronously inside the frame callback.

A vendor initialization failure uses MF. A live error/format change requests a
keyframe and retries on the existing MF path for the remainder of the connection.
AMF accepts non-reordered 8-bit 4:2:0 HEVC only. It does not decode pixels on CPU.
This is an opt-in test feature pending stock-host visual/teardown acceptance.
