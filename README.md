# ParsecWebTurn

**A portable Rust client for connecting to your Parsec computers.**

ParsecWebTurn runs the original, pinned Parsec WebAssembly core in Wasmtime,
with native Windows video, audio, input and networking. It needs no browser,
WebView2 or installer: launch a single **ParsecWebTurn.exe**.
It is an independent community project, not affiliated with Parsec or Unity.

`main` contains the stable native client. This `dev` branch includes the newer
features described below; check a release's notes for what its binary includes.
The earlier Tauri/WebView2 client is preserved on the
[legacy branch](https://github.com/vampywiz17/ParsecWebTurn/tree/legacy).

**Development release:** BUD, vendor-specific HEVC decoding and Zero copy are
experimental. For stability-first use, select WebRTC with H.264 and leave the
experimental options disabled, or use the
[latest stable release](https://github.com/vampywiz17/ParsecWebTurn/releases/latest).

## Current features

- Sign in, choose a computer and connect to a regular Parsec host.
- H.264 and optional H.265 / HEVC Main, 8-bit 4:2:0 video.
- Opus sound, keyboard/mouse input, immersive capture and relative mouse mode.
- The familiar Parsec overlay, drawn in software for compatibility while the
  streamed video stays on the GPU.
- WebRTC with direct connections and custom STUN/TURN, including Cloudflare and
  TURN over UDP, TCP and TLS.
- Optional **experimental BUD**, with direct UDP, automatic UDP TURN fallback,
  and forced UDP relay. It uses the same native video/audio paths as WebRTC.
- Live Connection stats: active route, RTT, incoming/outgoing traffic, decoded
  FPS, codec, decoder, GPU, host encode time when available, and app resource use.
  Transport-specific statistics adapt to WebRTC or BUD; unknown values stay unknown.
- Saved GPU selection, optional original-surface HEVC presentation, persistent
  sign-in/preferences, and graceful disconnect/reconnect.

Not every setting in the embedded Parsec interface has a native implementation.
Gamepad support still needs live validation. 10-bit and 4:4:4 streaming are not
implemented. Linux/Android clients and Vulkan/D3D12 renderers are future work.

## Getting started

1. Download and extract a Windows x64 ZIP from the
   [releases page](https://github.com/vampywiz17/ParsecWebTurn/releases).
   The [latest stable release](https://github.com/vampywiz17/ParsecWebTurn/releases/latest)
   may contain fewer features than this development branch.
2. Run **ParsecWebTurn.exe**, sign in and select your host.
3. Choose the **web** launch option in the computer list. Native integration is
   built around that core entry point, including when experimental BUD is selected.

Windows x64, D3D11/OpenGL support and compatible GPU drivers are required.
Original-surface presentation additionally needs Windows 11 and driver support.
Administrator rights are not required for normal use.

**Shortcuts:** F11 toggles fullscreen; Ctrl+, opens Settings; Ctrl+Shift+S opens
Connection stats. F8 toggles the video layer to reach the underlying interface.
Custom shortcuts configured in the embedded Hotkeys page take priority.
Background global hotkeys are not implemented by this core.

## Settings

The **Network** and **Video** tabs share one **Save settings** action. Switching
tabs preserves unsaved edits. Changes apply to the next connection.

### Network

- **STUN servers:** one URL per line; blank uses Parsec's default discovery server.
- **STUN only:** excludes TURN and does not request Cloudflare credentials.
- **TURN provider:** custom server URLs, username/password, or Cloudflare TURN
  Key ID/API token. Custom URLs also accept one server per line. Existing saved
  secrets can be kept, replaced or explicitly forgotten.
- **Credential lifetime/cache:** control generated Cloudflare credentials.
- **Force TURN relay:** use a configured relay even when a direct route exists.
  It cannot be combined with STUN only.
- **Experimental BUD:** select BUD instead of the default WebRTC transport.
  BUD supports UDP only, including its relay connection. Keep WebRTC for networks
  that require TURN over TCP or TLS. There is no silent transport switch.

Examples: `turn:relay.example.com:3478?transport=udp`,
`turn:relay.example.com:3478?transport=tcp`,
`turns:relay.example.com:443?transport=tcp`. TLS needs a TLS-enabled TURN server
with a trusted certificate; changing only the port is insufficient.

### Video

- **Video GPU:** adapter for native decoding/rendering. Automatic uses Windows'
  default. Re-select a missing adapter or choose Automatic before reconnecting.
- **Prefer H.265 / HEVC:** request HEVC when supported. H.264 remains the default;
  the host decides the stream actually sent. Connection stats reports the result.
- **Zero copy (experimental, HEVC):** present the original NV12 decoder surface
  using Windows Presentation API. Unsupported configurations retain the normal
  GPU rendering/recovery path. Check the actual renderer and status in stats.

| Codec / mode | NVIDIA | AMD | Intel |
| --- | --- | --- | --- |
| H.264 | Media Foundation | Media Foundation | Media Foundation |
| HEVC normal | Direct D3D11 | AMD AMF | Intel VPL |
| HEVC original-surface presentation | Direct D3D11 | Direct D3D11 | Direct D3D11 |

Both HEVC modes have been tested with live Parsec hosts using WebRTC and BUD on
the tested NVIDIA, AMD and Intel systems. This does not guarantee every GPU or
driver. Hardware/firmware-disabled HEVC, including affected Dell configurations,
cannot be enabled by selecting a different client decoder.

Decoded pixels stay on the GPU. Normal HEVC feeds the decoder surface directly
to the existing GPU conversion/scaling output pass. Original-surface presentation
removes that application pass and adds no intermediate decoded-frame copy or
CPU readback. It does not establish direct scanout or eliminate internal Windows/
driver transfers; hybrid GPU/display configurations can still introduce them.

## Connection stats

Traffic means measured usage, not available bandwidth. RTT is transport-specific,
not ICMP ping; local dropped frames are distinct from network packet loss.
Audio format stays visible during silence. BUD displays its delivery counters
instead of irrelevant ICE/DTLS data. Decoder recovery and Zero copy status use
short descriptions; detailed error information remains available to diagnostics.
GPU decoder surfaces are evidence of a GPU video path, not independent proof of
hardware execution, so uncertain hardware decoding is labelled **Likely**.

## Saved data and updates

`settings.json` beside the executable stores native connection/video settings
and Windows-protected server credentials. Use `--data-dir "C:\YourFolder"` for a
different writable location. Sign-in and embedded Parsec preferences are stored
separately under `%LOCALAPPDATA%\ParsecWebTurn\Native`, protected for your Windows
user. Replacing the executable preserves them. Moving from the legacy client
requires signing in again; its separate browser profile is left untouched.

**Help > Check for updates** checks stable GitHub releases. A background startup
check can also notify you without taking focus. Download the ZIP in your browser,
close the app and replace the executable, keeping your settings. Updates are never
installed automatically; prereleases are not offered by the stable updater.

**Help > About** shows the embedded Parsec core version and checks the version
currently served by Parsec. This read-only check never installs, executes or
replaces the core. Unavailable/unrecognized versions stay unknown. Core upgrades
require compatibility testing and a new application build.
`PARSECWEBTURN_NO_UPDATE_CHECK=1` disables background update/core-version checks.

## Milestones and roadmap

Statuses below describe this branch, not necessarily the latest stable release.

| Milestone | Status |
| --- | --- |
| Native Windows client without WebView2 | Implemented |
| H.264, Opus, keyboard/mouse, overlay and saved profiles | Implemented |
| WebRTC direct and TURN over UDP/TCP/TLS, including forced relay | Implemented |
| HEVC: NVIDIA Direct D3D11, AMD AMF, Intel VPL | Experimental; live tested on selected GPUs |
| Original-surface HEVC presentation on NVIDIA/AMD/Intel | Experimental; live tested on selected GPUs |
| Optional BUD direct and UDP TURN, automatic relay fallback | Experimental; live tested |
| Transport-aware connection and playback statistics | Implemented |
| Network/Video settings tabs | Implemented |
| Gamepad verification and further native menu integration | Pending |
| Vulkan rendering and Linux client | Planned |
| D3D12, Android and Android TV | Exploring |

No delivery dates are promised. Other platforms also need native codec, audio,
window and input integration; adding a renderer alone will not make the client
portable to them.

## The Parsec core, beyond the browser

The pinned WebAssembly core runs in a native Rust application, precompiled at
build time. Rust supplies its platform integration; no browser is involved.
Research suggests shared foundations with Parsec's native client, but does not
establish identical code or feature sets. The original core remains unmodified.

In WebRTC mode, Parsec's own video/audio/control messages travel through data
channels using ICE, DTLS and SCTP, rather than ordinary WebRTC media tracks/RTP
playback. This retains TURN's UDP/TCP/TLS connectivity. Optional BUD provides an
independent native UDP media transport, while the same client handles decoding,
sound, input and the embedded interface. A regular Parsec host supports both paths.

## Build and contribute

The native application and its backend crates live under `src-native`:

- `src/`: application, WebRTC, platform integration and settings/stats UI.
- `crates/bud/`: transport-only BUD adapter and its trimmed vendored dependencies.
- `crates/video/`: GPU decoder and original-surface presentation backends.
- `vendor/`: pinned Parsec core and WebRTC/ICE/TURN compatibility patches.
- `fixtures/` and backend test fixtures: offline regressions, excluded from normal
  execution. Removed one-off comparison tools are available in Git history.

Build on Windows with the Rust MSVC toolchain and C++ build tools:
`./scripts/build.ps1`. The normal build includes the current backends, with BUD
and original-surface presentation still selected explicitly in Settings.
`diagnostics` remains a separate opt-in build feature. See the
[technical guide](docs/NATIVE_TECHNICAL.md),
[configuration compatibility](docs/NATIVE_CONFIG_COMPATIBILITY.md) and
[research references](docs/NATIVE_REFERENCES.md).

Please [open an issue](https://github.com/vampywiz17/ParsecWebTurn/issues) with the
version, Windows/GPU/driver, selected transport/route and steps to reproduce.
Never publish credentials or sign-in tokens. Included third-party license
notices must accompany redistributed binaries.
