# Experimental NVIDIA original-surface presentation

Disabled by default and only included with `vendor-video`. The Settings
checkbox is named "Zero copy (experimental, NVIDIA HEVC)"; it is a name for
the application path, not a claim about driver-internal work or direct scanout.
The proprietary core and both transport implementations are unchanged.

The existing direct D3D11 HEVC decoder can optionally allocate individual
NV12 textures with shared/displayable/exclusive-writer flags rather than one
texture array. This requires a decoder configuration advertising independent
textures (ConfigDecoderSpecific bit 14). The separate presentation device
uses PREVENT_INTERNAL_THREADING_OPTIMIZATIONS as required by the API;
ordinary decoder device creation is unchanged.

The original textures are registered with IPresentationManager, attached to
an IPresentationSurface and a DirectComposition visual on the video HWND.
Crop, BT.709 range and the existing aspect-preserving viewport are applied
as presentation properties. There is no application VideoProcessorBlt,
RGB render target, CopyResource, decoded-pixel upload or CPU pixel readback.
The existing separate software overlay is preserved. DWM may still compose;
displayable/independent-flip capability is not proof of active scanout.

The decoded-picture buffer keeps an output slot pinned until
IPresentationBuffer::IsAvailable says that presentation has released it.
Before every decoder write a second ownership guard prevents reuse even if
an IDR reset has cleared the parser's pending-output bookkeeping. No CPU
scanout wait is introduced. Unavailability, unsupported allocation, device
loss or format changes use existing keyframe recovery and the normal GPU
renderer for the remainder of the connection. No network route changes.

Windows 11/WDDM 3.0 capability checks gate this experiment; the new Windows
entry point is resolved dynamically so older systems can use the DXGI path.
The opt-in path is independent of WebRTC/BUD. H.264, AMD and Intel stay on
their existing paths. Initialization failure falls back immediately; a later
presentation/decoder error disables only this option for that connection.

Validation includes default/features builds, persisted checkbox state,
presentation ownership checks and a standalone 120-frame synthetic HEVC
presentation probe. Actual desktop colors, overlay interaction and latency
still need live-host acceptance; no latency benefit is assumed from fewer
application writes alone.

Supported API references:
- https://learn.microsoft.com/en-us/windows/win32/direct3d11/displayable-surfaces
- https://learn.microsoft.com/en-us/windows/win32/comp_swapchain/comp-swapchain
- https://learn.microsoft.com/en-us/windows/win32/comp_swapchain/comp-swapchain-examples
