# EvoParseq changelog

## Unreleased — native client development

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
