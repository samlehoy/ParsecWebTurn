# Overlay rendering isolation

An Intel HD Graphics user reported coloured noise at the overlay menu's right
edge and along the metrics bar with 0.9.2-dev.1. The decoded picture remained
clean; hiding the native video surface with F8 also removed the noise from the
original OpenGL UI. This narrows the investigation to GUI capture/composition,
but does not by itself prove a particular driver or synchronization bug.

The D3D11 GUI pass now holds the existing device multithread critical section
across its uploads, graphics state setup, draws and binding cleanup. Previously,
individual calls were protected, but another component using the shared
immediate context could interleave between those calls. The pass also explicitly
unbinds geometry, hull and domain shaders: it only needs vertex/pixel shaders.
The RAII guard releases protection on both success and error. It is not held
across decoding, swap-chain presentation, shared application locks or queue
waits. There is no new decoded-frame copy, readback, Flush or presentation delay.

Microsoft documents using Enter/Leave to preserve graphics command ordering:
[multithread protection](https://learn.microsoft.com/en-us/windows/win32/api/d3d11_4/nn-d3d11_4-id3d11multithread).
The existing ID3D10Multithread interface on the D3D11 device is also the interface
recommended for sharing devices with Media Foundation:
[Direct3D 11 decoding integration](https://learn.microsoft.com/en-us/windows/win32/medfound/supporting-direct3d-11-video-decoding-in-media-foundation).

An isolated WARP test checks every pixel of a synthetic 64x48 target. It covers
multiple draw batches, odd-width RGBA atlases, transparent pixels with nonzero
RGB values, clipping, vertex-buffer growth, changed texture versions and reused
frames. An intentionally inherited geometry shader suppresses rendering unless
the GUI establishes its own shader stages. Readback exists only in this test.

These checks establish compositor behavior, not that the reported Intel artifact
has been reproduced or eliminated. Live testing of 162b130 on the affected Intel
machine did not eliminate the flicker.

## Default software overlay

Live testing on the affected Intel HD Graphics machine confirmed that software
overlay rendering eliminated the observed flicker. It is now the default;
launch `ParsecWebTurn.exe --d3d-overlay` to compare the previous path.
The earlier `--software-overlay` argument remains accepted for compatibility. CPU
rasterization consumes the same captured GUI triangles, clip rectangles and RGBA
atlases, with bilinear sampling and premultiplied alpha. The resulting GUI-only
bitmap is presented in an owned, click-through, non-activating layered window
using documented [UpdateLayeredWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-updatelayeredwindow).
The native D3D11 overlay pass is skipped in this mode. Windows still owns desktop
composition, which may use the GPU: this is software **GUI rasterization**, not a
claim that the entire desktop compositor runs on the CPU.

Rasterization runs on the existing GUI render worker, not the input or video
worker. A bounded latest-surface slot passes the result to the UI thread; old
pending images are replaced. No decoded frame is copied or read back. The normal
default is the software overlay, and the original OpenGL account/computer UI is
unchanged. CPU/memory usage can be higher while rendering the overlay.

The comparison points to the D3D11 GUI composition path, but does not establish
a particular driver defect. This Win32 presentation layer is Windows-specific;
other operating systems still need a native presenter. The CPU GUI rasterizer
does not add a copy or readback of decoded video. Color conversion and final
output writes in the existing GPU presenter are still necessary; this is not
a claim of complete end-to-end zero-copy execution.
