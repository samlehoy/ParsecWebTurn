# Local driver binding patch

MIT source from nomi-san/lowlat revision
`5f8db5689e55de0d396b0520824caa9fc053dd86`, previously used as a Git dependency.
This local patch preserves that driver API and package identity for lowlat-decode.
No new SDK, host, encoder or runtime dependency is introduced.

Only functional change: AMF `decoder_with_layout` selects the documented
`AMFVideoDecoderHW_H265_MAIN10` component for HEVC/P010. The existing `decoder`
entry point retains its NV12 behavior. This follows upstream finding
`eb2fdd7684cc3e70f1d64ae5206c95ecd948f16b`: AMF 1.4.31 needs the separate
Main10 component, while newer runtimes also accept it. Failure remains a normal
backend error; EvoParseq retains Media Foundation fallback.

Cargo workspace metadata/lints are flattened for local use. All other modules
are unchanged and remain available to the existing decoder dependency.
