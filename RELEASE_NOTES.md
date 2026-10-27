EvoParseq v0.9.5-dev.1 - Native Rust core and session controls

This Windows prerelease moves the native-core development into **dev** as
**EvoParseq**, an independent native Rust client for existing, unmodified Parsec
hosts. It replaces the previous client runtime rather than embedding the
proprietary Parsec WebAssembly core. The changes below cover the implemented
changes since the latest stable release, **ParsecWebTurn v0.9.4**.

**This is a development preview. ParsecWebTurn v0.9.4 remains the stable release.**

## What's new and improved

- **Native core:** implement account sign-in, optional two-factor authentication,
  computer discovery, authenticated signaling and owner-session control directly
  in Rust. Remembered sessions use Windows CurrentUser encryption; passwords and
  two-factor codes are not saved. The computer picker refreshes in the background.
- **Native WebRTC and BUD sessions:** retain the existing media and transport
  backends while replacing their WASM-dependent orchestration. Use explicit
  transport selection, direct connections, custom TURN UDP/TLS/TCP, Force TURN,
  STUN-only and Cloudflare credentials without a silent transport switch.
- **New EvoParseq interface:** add application branding, native About/update
  windows, a draggable overlay icon, performance summary and session controls for
  resolution, bandwidth, frame rate, constant FPS, orientation, immersive input
  and fullscreen. Selection fields use dropdowns, host-reported choices where
  available and compatible presets otherwise. Restore hidden controls by shortcut
  and keep local menus available while forwarding remote input.
  Keep the stream canvas black during startup, fix overlay/stats window
  ordering and confirm asynchronous host-settings updates before accepting
  another apply, avoiding stale values and repeated clicks.
- **Host display selection:** switch the current stream between listed physical
  or virtual outputs, requesting the selected display's host resolution. HID
  mouse mode restricts selection to the primary output and pauses forwarding
  until that output is reported, preserving the absolute-positioning contract.
- **Experimental HEVC 4:4:4:** add compatible NVIDIA/Intel 8-bit AYUV and 10-bit
  Y410 decode/output paths with a single GPU RGB conversion and no decoded-frame
  CPU readback. NVIDIA RTX 3060 8/10-bit playback is live-validated; Intel remains
  capability-gated and unverified. AMD selection is disabled. H.264 stays 4:2:0,
  and original-NV12 zero-copy is not combined with 4:4:4.
- **AMD HEVC improvements:** initialize cropped AMF streams with their visible
  dimensions for older runtimes. Add optional Main10 component selection with
  original P010 GPU output and the existing RGB10 conversion; Radeon 780M
  playback is confirmed. Media Foundation remains the default Main10 route.
- **Video format changes:** rebuild HEVC GPU sessions when the host changes
  the encoded format, preserve parameter-set dependency order, and retire old
  codec state during encoder rebuilds without adding frame copies.
- **Clear 10-bit failure handling:** preserve the requested mode. An initial host
  encoder failure prompts the user to disable 10-bit and reconnect instead of
  silently reopening an 8-bit session. Stats retain actual stream/output precision.
- **Presentation catch-up:** extend the reference-safe BUD approach to WebRTC,
  decoding dependencies in order while skipping obsolete display work after a
  burst. Bounded recent queue history adapts WebRTC catch-up to sustained late
  input and resets after recovery. No extra frame copy, rendering thread or
  pacing buffer is introduced.
- **Automatic audio-output recovery:** reopen shared WASAPI after transient
  device errors and Windows default-output changes, without reconnecting video.
  Retire stale queued and in-flight audio, preserve Opus state and existing
  buffer preferences, and expose recovery transitions in logs/stats.
- **Optional TCP/TLS QoS:** add an off-by-default experimental Windows AudioVideo
  marking option for outgoing BUD/WebRTC TURN sockets. Refusal is nonfatal;
  existing TLS verification and transport IO remain intact. It does not set
  Cloudflare/server priorities or prioritize incoming relay traffic.
- **Cursor, settings and diagnostics:** size remote cursor images to the client's
  Windows cursor canvas while preserving shape/hotspot proportions. Group routing,
  video-format, HID and audio choices into dropdowns; retain saved preferences.
  Keep BUD video encode timing separate from audio timing reports and reuse
  existing telemetry for the session overlay and diagnostics.
- **Maintenance and packaging:** separate shared sign-in, settings, session,
  diagnostics and input rules from Win32 adapters while retaining the GPU/media
  path. Remove the old runtime, adapters, diagnostic launchers and obsolete
  prototype dependencies/documentation. Refresh Tokio to 1.53.3,
  rustls-webpki to 0.103.16 and webpki-roots to 1.0.10. Ship portable EvoParseq
  and legacy-named transition packages with retained third-party notices.

## Compatibility and validation

**Vendor HEVC, zero-copy, 10-bit HEVC, HEVC 4:4:4, display timing, USB HID input
and multichannel audio remain experimental.** H.264, stereo Opus and Automatic
presentation are the conservative choices. Current native authorization supports
owner connections; friend/invitation flows are not implemented.

10-bit is SDR, not HDR. Host encoding capability determines the actual stream;
NVIDIA host Main10 is confirmed, while other encoder configurations can differ.
Intel 4:4:4 is not live-validated. Original-surface zero-copy remains 8-bit NV12
and requires Windows 11 plus compatible drivers. Multichannel audio still has
synthetic coverage only. Adaptive catch-up, real TURN QoS benefit and physical
audio-device recovery need broader live validation; no universal latency gain
is claimed. Host virtual-display limitations may delay display switching.

HID requires BUD and USB/Yubikey passthrough with the supported driver on the
stock host. HID mouse uses the primary display; use matching keyboard layouts.
BUD raw PCM requires host permission (`network_raw_audio=1`); WebRTC audio remains
stereo Opus. Ctrl+Alt+Del requires the installed host service and permissions.
No host patch or extra client helper is required.

## Updating

Close the old client and extract **EvoParseq-v0.9.5-dev.1-win64.zip**. Keep your
`settings.json` under the same Windows account. Sign in again: the previous
web-core login is not imported, while native remembered sessions and compatible
preferences remain Windows-user protected. Do not copy protected credentials
between machines/accounts expecting them to decrypt.

For existing shortcuts, **ParsecWebTurn-v0.9.5-dev.1-win64.zip** contains the same
EvoParseq executable under the legacy name. Both ZIPs exclude settings.json.
Run only one client and keep a backup until the transition is verified.

This is a **Windows x64 prerelease**, not an automatic stable-channel upgrade.
The main branch and stable ParsecWebTurn v0.9.4 remain unchanged. See README and
MIGRATION.md for capabilities, limitations and the future stable-update path.
