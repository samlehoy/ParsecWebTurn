> Historical investigation results. One-off comparison executables and test-only
> build gates have been retired; current behavior and build instructions are in
> the [project README](../README.md).

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

## Failed-startup teardown correction

A stock-host WebRTC attempt with this option enabled closed the entire
process on the test RTX 3060. WER recorded nvwgf2umx.dll 32.0.16.1742,
exception 0xc0000005 at offset 0x2417ff. Offline PE unwind and the exact
matching client executable locate the call in Presenter::drop:
SetRoot(None), Commit(), SetBuffer(None), then IPresentationManager::Present.
Two dumps also preserve the earlier decoder fallback error; the fatal GPU
call is a secondary failure while recovering, not proof of an SCTP fault.

Teardown now only detaches the composition visual and commits that change;
it never submits an empty/new GPU present. Windows owns retirement of
outstanding buffer references. Normal presentation, copy counts, transports
and the WASM core are unchanged. Parser errors retain their original stage
and category instead of masquerading as HRESULT 0xffffffff. The standalone
probe has explicit --empty-close and --failed-start regression cases.
The initial live WebRTC decoder rejection still needs identification; a
successful safe fallback must not be reported as working original-surface
presentation for that stream.

### Startup burst backpressure

The normal direct D3D11 decoder was confirmed working in WebRTC without
fallback. Unlike its immediate processor read, the new presentation path
retains DPB slots until scanout/composition releases them. Unbounded display
submission can exhaust the 18-slot HEVC DPB before the first retirement.
A synthetic test with the actual HEVC parser/DPB models no retirement and
compares the old unbounded submission against the new two-buffer limit.
The limit applies only to the experimental presenter. Every encoded picture
still decodes, including reference pictures; a busy display update is skipped
without pinning its slot, copying pixels or waiting for scanout. The existing
presentations_busy counter accounts for these skips. No network/encoded queue
policy, decoder algorithm or normal renderer pacing changes. The standalone
--burst GPU probe verifies 120 decoded pictures with no producer pacing.
Live-host confirmation remains necessary to establish whether this is the
initial WebRTC rejection observed in the crash dumps.
