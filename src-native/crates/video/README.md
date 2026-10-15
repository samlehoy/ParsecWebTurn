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
5f8db5689e55de0d396b0520824caa9fc053dd86. Exporters, CPU pixel paths, full chroma
and high-depth profiles were omitted. Driver bindings and parser dependencies
remain pinned; the original license is in LICENSE-lowlat.

Ownership is expressed through self_cell and bounded surface leases. VPL retains
at most two renderer reads until a GPU fence completes; original presentation
retains at most two display leases. Busy display updates can be skipped without
dropping encoded reference pictures or adding GPU waits/copies. AMF's existing
low-latency and no-SurfaceCopy settings remain unchanged.
