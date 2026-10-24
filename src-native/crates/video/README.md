# GPU video backends

HEVC normal: NVIDIA Direct D3D11, AMD AMF, Intel VPL. H.264 stays Media Foundation.
Optional original-surface presentation uses Direct D3D11 and Windows Presentation
API on all three supported vendors. Runtime capability checks and the existing
GPU fallback/recovery remain. No transport or account code lives in this crate.

Live decoded pixels stay on the GPU; no plane exporter, intermediate decoded-frame
copy or CPU readback is used. Normal output needs the existing conversion/scaling
write; original-surface presentation omits that application pass. Windows/driver
internal copies and direct scanout are not established.

`direct.rs` derives parameter staging from MIT-licensed lowlat revision
5f8db5689e55de0d396b0520824caa9fc053dd86. Exporters and CPU pixel paths were omitted. Native-core full chroma uses
AYUV (8-bit) or Y410 (10-bit) original decoder textures on the existing normal
GPU conversion/output pass. Original-surface presentation remains NV12-only. Driver bindings and parser dependencies
remain pinned; the original license is in LICENSE-lowlat.

The earlier direct H.264 comparison path was removed: this backend is HEVC-only.
Its unused H.264 parser, parameter staging and codec selection are not allocated
or compiled here. HEVC retains its existing 18 surfaces, 32 slice slots and
submission/presentation path. H.264 playback remains in Media Foundation.

Ownership is expressed through self_cell and bounded surface leases. VPL retains
at most two renderer reads until a GPU fence completes; original presentation
retains at most two display leases. Busy display updates can be skipped without
dropping encoded reference pictures or adding GPU waits/copies. AMF's existing
low-latency and no-SurfaceCopy settings remain unchanged.
