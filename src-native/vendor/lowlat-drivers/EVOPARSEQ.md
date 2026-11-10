# Local driver binding patch

MIT source from nomi-san/lowlat revision
`5f8db5689e55de0d396b0520824caa9fc053dd86`, previously used as a Git dependency.
This local patch preserves that driver API and package identity for lowlat-decode.
No new SDK, host, encoder or runtime dependency is introduced.

AMF `decoder_with_layout` selects the documented
`AMFVideoDecoderHW_H265_MAIN10` component for HEVC/P010. The existing `decoder`
entry point retains its NV12 behavior. This follows upstream finding
`eb2fdd7684cc3e70f1d64ae5206c95ecd948f16b`: AMF 1.4.31 needs the separate
Main10 component, while newer runtimes also accept it. Failure remains a normal
backend error; EvoParseq retains Media Foundation fallback.

Optional `vpl::ai` binds only the public Intel VPL v2.17.0 C API needed for
AI Super Resolution. The stable filter ABI was added in API 2.13; the runtime
version, strict Query/Init/GetVideoParam results and native D3D11 handles are
checked. It opens its own hardware VPP session on the caller's device; existing
VPL decode functions and runtime discovery are unchanged. No dispatcher, model,
Intel runtime binary or vendor sample implementation is bundled. AI processing
uses one persistent input/output surface pair and bounded completion; no CPU
mapping, frame interpolation or software/ordinary-scaler substitution.

API references: https://github.com/intel/libvpl/releases/tag/v2.17.0 and
https://intel.github.io/libvpl/latest/API_ref/VPL_structs_vpp.html#mfxextvppaisuperresolution

Cargo workspace metadata/lints are flattened for local use. Other modules remain
available to the existing decoder dependency.
