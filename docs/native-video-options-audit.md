# Native video options: feasibility audit

Reviewed on 2026-10-11. Research only: no new runtime options or changes to the
Parsec core are introduced by this audit.

## Result

| Option | Evidence | Native client today | Next step |
| --- | --- | --- | --- |
| HEVC 10-bit | Pinned core reads `client_decoder_10bit`; lowlat requests it with `_flags` bit 4 (`0x10`). | Main profile, 8-bit 4:2:0 only; NV12 decoder output and an 8-bit BGRA swapchain. | Implement and validate Main10/P010 decoding and appropriate presentation before advertising support. |
| Zero Copy | Core reads `client_zero_copy`; official option describes retaining decoded output in video memory for D3D11 rendering. | Live decoder textures already feed the D3D11 VideoProcessor directly, without an intermediate decoded-frame texture or CPU readback. | Keep this GPU path; adding a switch alone would not improve it. |
| 4:4:4 | Core reads `client_decoder_444`; SDK binary and lowlat independently identify `_flags` bit 1 (`0x02`). | Current Media Foundation HEVC path supports 4:2:0. | Investigate actual stock-host payload representation and a suitable decoder before attempting support. |
| Alpha Channels | Screenshot describes early access to upcoming releases. | This is a release-channel preference, not video transparency. | No video integration implied. |
| New UI Preview | Official setting `client_web_ui` selects the official application's alternate UI. | Our native shell owns its platform integration. | This setting does not add a codec or renderer to Rust. |

## Evidence and confidence

The unchanged pinned WASM core is version 150-104a, SHA-256
`d663dd96df477c65479fb93eb88756c7fcafff581cc93be563625cd195a4b4a6`.
Static inspection of `config_get_client` shows separate reads for HEVC, 4:4:4,
10-bit and Zero Copy. These are real configuration reads, rather than merely
labels in UI strings. They do not establish that every corresponding native
renderer or codec path is usable through our imported WASM interface.

Our Rust host currently builds the stream initialization request itself. Its
`control::Config` carries the HEVC preference, but no 10-bit preference. The
active `parsec_web_new_attempt` boundary does not supply a ready-made native
decoder or renderer selection. Writing a core configuration key by itself would
therefore not extend the Rust video pipeline.

[lowlat](https://github.com/nomi-san/lowlat), reviewed at
`7b4dc4ae79990ea8c59a108c80b3c89b695c3cb4`, is a Linux **host** implementation.
Its `crates/core/src/init.rs` defines HEVC as `0x01`, 4:4:4 as `0x02`, and 10-bit
as `0x10`. Its host stream code actually uses these preferences to select codec,
depth and chroma, subject to encoder support. Some protocol prose still describes
4:4:4 as refused; the current implementation has more specific capability gates.
Its example web client is not proof of stock-host 10-bit interoperability.

For our working startup contract, HEVC plus 10-bit would suggest `_flags: 0x11`.
That is a research candidate, **not a live-verified request to a stock Parsec
host**. Do not automatically import lowlat's separate native base flag `0x08`.
Likewise, its native video packet header uses a different bit for depth; those
headers must not be applied to our WebRTC media framing.

The preserved [Parsec SDK](https://github.com/mbroemme/parsec-sdk), revision
`14222ac25be3bc28c49b4858213a737642031513`, exposes `decoderH265` and
`decoder444` in `ParsecClientVideoConfig`. Binary inspection corroborates their
startup flag bits. This SDK header does **not** name a 10-bit or Zero Copy field;
its padding must not be reinterpreted as a public 10-bit API.
[libparsec](https://github.com/nomi-san/libparsec) remains a work-in-progress
reference, not independent proof of an implemented 10-bit client.

## What 10-bit would require

The current SPS parser can extract bit depth, but `video_windows.rs` rejects
anything other than Main profile, 8-bit and 4:2:0. The decoder capability check,
transform enumeration and texture validation currently select NV12. Presentation
uses an 8-bit `B8G8R8A8_UNORM` swapchain.

A complete integration would need:

1. An opt-in request, capability checks for the selected GPU and installed
   decoder, and actual SPS verification of Main10/10-bit output from the host.
2. Media Foundation Main10 input and P010 output, with P010 decoder textures
   passed directly to a compatible D3D11 VideoProcessor. Resource/cache decisions
   must distinguish the formats.
3. Suitable output precision and color handling, including the overlay. Keeping
   the present 8-bit swapchain would reduce the final image to 8-bit precision.
4. Live tests with a stock host, fallback to the existing 8-bit path, GPU changes,
   reconnection and sustained streaming.

An isolated local probe enumerated two Media Foundation transforms and both
accepted HEVC Main10 input with a 1920x1080 frame size and P010 output
(`SetInputType`/`SetOutputType` succeeded). **No compressed 10-bit frames were
decoded by this probe.** It establishes type negotiation on this machine, not
hardware execution, compatibility on other machines or stock-host negotiation.
The probe remains ignored research tooling outside the application binary.

10-bit SDR and HDR are separate features. HDR would additionally require the
correct transfer function, primaries, metadata and output/display color-space
handling. A 10-bit option must not be labelled HDR automatically. This is a
quality option, not an established latency optimization.

## What Zero Copy means here

The live path obtains the decoder's `IMFDXGIBuffer` texture, creates an input view
on that resource, and uses `VideoProcessorBlt` to convert/present it into the
swapchain backbuffer. There is no live CPU readback or extra intermediate
decoded-frame copy in this application path. Diagnostic staging copies are
isolated behind test/diagnostic code and are not enabled in live sessions.

The necessary YUV-to-RGB conversion still writes the output surface. Compressed
input buffers also have their own copies. Driver internals and cross-adapter
behavior are not fully observable here. Thus the accurate description is a
GPU-resident decoded-frame path, not an absolute zero-copy guarantee. The
official switch's stated goal is already met by this design; forwarding its
configuration key would not make the current Rust path faster by itself.

## Platform and product documentation

- [Parsec advanced options](https://support.parsec.app/hc/en-us/articles/32381443626516-All-Advanced-Configuration-Options): meanings of 10-bit, Zero Copy and New UI Preview.
- [Microsoft HEVC decoder](https://learn.microsoft.com/en-us/windows/win32/medfound/h-265---hevc-video-decoder): Main/Main10, NV12/P010, 4:2:0 support and Annex B input.
- [D3D11 VideoProcessorBlt](https://learn.microsoft.com/en-us/windows/win32/api/d3d11/nf-d3d11-id3d11videocontext-videoprocessorblt): documented video processing into an output surface.
- [Windows HDR rendering](https://learn.microsoft.com/en-us/windows/win32/direct3darticles/high-dynamic-range): output precision and color-space requirements.

See also [HEVC negotiation](hevc-negotiation.md) and
[native research references](NATIVE_REFERENCES.md).
