# EvoParseq changelog

## Unreleased — native client development

- Add opt-in experimental Windows AudioVideo QoS for BUD and WebRTC TURN
  TCP/TLS, using one shared portable adapter with a no-op fallback elsewhere.
  Mark once per connected socket; refusal is nonfatal and optionally logged.
  Preserve existing TLS verification, transport I/O, UDP and GPU media paths.
  Synthetic congestion results are promising; real TURN benefit remains to test.

- Size native remote cursor images to the client's Windows arrow cursor canvas,
  preserving shape aspect ratio and scaling/clamping the hotspot together.
  Reuse the cursor until its shape or Windows size changes; retain hidden-camera
  behavior and add opt-in shape/mode metadata without movement/pixel logging.

- Group Settings into routing, video-format and HID input choices; use dropdowns for
  presentation/chroma/AMF Main10 and audio buffer/codec/speaker modes. Preserve
  existing settings.json flags, defaults and media/input behavior.
- Record live AMD Radeon 780M AMF Main10 4:2:0 / RGB10 SDR validation.

- Add an opt-in AMD AMF Main10 decoder using the separate HEVC Main10
  component and original P010 GPU textures. Default remains Media Foundation;
  existing conversion/output and MF recovery are reused. Live AMD performance
  comparison is pending; no decoded-frame copy/readback or extra conversion pass.

- Initialize the existing AMD AMF decoder with SPS visible dimensions for
  cropped HEVC, retaining coded-size checks and the existing GPU rendering path.

- Refresh Tokio to 1.53.3, rustls-webpki to 0.103.16 and webpki-roots to
  1.0.10 in application/BUD lockfiles for published dependency fixes.
- Keep BUD video encode timing separate from audio timing reports, preserving
  the last valid nonzero video value within each connection.

- Independent native Rust account sign-in, optional two-factor authentication,
  DPAPI-protected remembered sessions and a background-refreshing computer picker.
- Explicit WebRTC/BUD transport selection; direct and custom TURN UDP/TLS/TCP
  routes with bounded native IO and no silent transport switch.
- Shared GPU video pipeline, optional original-surface presentation, display
  timing and host/local timing diagnostics.
- Experimental HEVC 4:4:4 at 8/10-bit with direct AYUV/Y410 GPU surfaces and a
  single RGB output conversion. NVIDIA RTX 3060 playback is live-validated;
  Intel is capability-gated and not live-validated; AMD remains disabled.
- Native session controls, draggable icon, fullscreen/immersive shortcuts,
  two-way text clipboard and experimental combined HID mouse/keyboard over BUD.
- Stereo Opus, adjustable audio buffers, BUD raw PCM and experimental
  multichannel audio (synthetic coverage only).
- EvoParseq branding throughout application-owned packages and portable builds.
- Retired the previous client runtime, its adapters, diagnostic launchers,
  build dependencies and obsolete prototype documentation from this branch.

This changelog describes the EvoParseq development branch. Published releases
from the repository's other branches are separate products.
