# ParsecWebTurn

**A portable Rust client for connecting to your Parsec computers.**

ParsecWebTurn runs the original, pinned Parsec WebAssembly core in Wasmtime,
with native Windows video, audio, input and networking. It needs no browser,
WebView2 or installer: launch a single **ParsecWebTurn.exe**.
It is an independent community project, not affiliated with Parsec or Unity.

**The embedded Parsec WASM core is completely unmodified.** ParsecWebTurn is
not a Rust rewrite of the original Parsec client. Rust provides the native
runtime and platform framework around the existing core, implementing the
services it needs outside a browser and the additional integrations described below.

`main` contains the stable native client. This `dev` branch includes the newer
features described below; check a release's notes for what its binary includes.
The earlier Tauri/WebView2 client is preserved on the
[legacy branch](https://github.com/vampywiz17/ParsecWebTurn/tree/legacy).

**Development release:** BUD, vendor-specific HEVC decoding, Zero copy, 10-bit HEVC
and custom Audio options are experimental. For stability-first use, select WebRTC with H.264 and leave the
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
- Optional **experimental BUD**, with direct UDP, automatic TURN fallback and
  forced relay over UDP, TLS or TCP. It uses the existing native video/audio paths.
- Live Connection stats: active route, RTT, incoming/outgoing traffic, decoded
  FPS, codec, decoder, GPU, host encode time when available, and app resource use.
  Transport-specific statistics adapt to WebRTC or BUD; unknown values stay unknown.
- Saved GPU selection, optional original-surface HEVC presentation, persistent
  sign-in/preferences, and graceful disconnect/reconnect.

Not every setting in the embedded Parsec interface has a native implementation.
Gamepad support still needs live validation. 4:4:4 streaming is not implemented.
10-bit HEVC is an opt-in experiment requiring host and client GPU support.
Live Main 10 streaming is currently verified only with NVIDIA host encoding;
the tested AMD Radeon 780M host sent 8-bit HEVC despite the 10-bit request.
This is an observed host-path limitation, not a lack of hardware Main 10 support.
Linux/Android clients and Vulkan/D3D12 renderers are future work.

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

The **Network**, **Video**, **Audio** and **Input** tabs share one **Save settings** action. Switching
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
  Direct connections use UDP. TURN uses the same configured URLs as WebRTC,
  preferring UDP, then TLS, then plain TCP (port 80 first among TCP URLs).
  Failed connection attempts advance to the next configured relay URL; an
  established stream is not migrated automatically. TLS verifies the original
  server identity. The relay-to-host leg remains UDP. TCP/TLS support is
  experimental and can suffer TCP head-of-line blocking. There is no silent
  switch to WebRTC and no separate BUD relay setting.
- **Enable event logging:** write `client-events.jsonl` beside `settings.json`
  (also when using `--data-dir`). Logs connection/route changes, connection
  failures, decoder errors/recovery and profile-save failures as JSON Lines with
  UTC timestamps. No credentials, IP addresses, packet contents or media are
  recorded. A background writer keeps disk I/O off the media workers; the bounded
  queue reports dropped log events. Rotation retains the current file and one
  `client-events.previous.jsonl`, each approximately 5 MiB. Disabled by default;
  changes apply immediately on Save. Disabling leaves existing files intact.

Examples: `turn:relay.example.com:3478?transport=udp`,
`turn:relay.example.com:3478?transport=tcp`,
`turns:relay.example.com:443?transport=tcp`. TLS needs a TLS-enabled TURN server
with a trusted certificate; changing only the port is insufficient.

### Video

- **Video GPU:** adapter for native decoding/rendering. Automatic uses Windows'
  default. Re-select a missing adapter or choose Automatic before reconnecting.
- **Prefer H.265 / HEVC:** request HEVC when supported. H.264 remains the default;
  the host decides the stream actually sent. Connection stats reports the result.
- **10-bit HEVC (experimental):** with HEVC enabled, request Main 10 only after
  a selected-GPU P010 decode/conversion capability check. Both transports retain
  the declaration on later encoder/keyframe requests. **Currently verified only
  with NVIDIA host encoding.** The live test used AMD Radeon 780M client decoding
  and 10-bit RGB output; the same client received 8-bit HEVC with AMD host encoding.
  The host encoder restriction is distinct from the client's GPU vendor.
  Main 10 uses the existing
  D3D11-aware Media Foundation GPU path, with P010 fed directly to the normal
  conversion pass, writing directly into a 10-bit RGB SDR swap chain when the
  GPU supports the conversion and presentation. No extra decoded-frame copy or
  conversion pass is added; unsupported output falls back to 8-bit RGB. Stats
  report RGB output depth separately from decoded depth. This does not enable
  HDR or verify the physical monitor/link's bit depth. Original-surface presentation is currently
  8-bit only. The host may still choose 8-bit; inspect the actual profile and
  decoded color depth in Connection stats, not just the preference.
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

### Audio (experimental)

- **Custom audio buffer:** optional minimum/maximum in milliseconds, initially
  75/150 to match the pinned core's automatic limits. Saved custom values are
  preserved. Off uses the core's actual playback limits and original WASAPI policy;
  the disabled fields retain the custom values for the next time you enable them.
  On uses the shortest supported shared WASAPI engine period when available,
  with the existing initialization as fallback. It starts with real PCM, without
  prefilled silence, and keeps the newest staged stereo audio within the output
  budget including device padding. Smaller values can cause audible gaps.
  Limits must satisfy 0 <= minimum <= maximum <= 500 ms, with maximum above zero.
- **Allow raw PCM audio:** experimental **BUD-only** permission, disabled by
  default. The host must also enable `network_raw_audio=1`; other guests can
  affect what the host sends. Both PCM and Opus remain accepted according to
  each BUD packet's codec header. PCM is 48 kHz stereo, signed 16-bit, about
  1.54 Mbps before transport overhead. WebRTC remains Opus because its current
  headerless audio contract cannot safely distinguish PCM from an Opus fallback.
- Save and reconnect to apply audio changes. Connection stats shows the actual
  codec, playback buffer, device capacity, decoded PCM waiting and underruns.
  These local queue measurements exclude transit and Bluetooth/device delay;
  they are not end-to-end audio/video synchronization measurements.

See [audio-path audit and implementation](docs/native-audio-options.md).

### Input (experimental)

- **USB HID mouse:** optional absolute desktop mouse over **BUD**, disabled by
  default. Enable BUD on the Network tab, then select the mouse on the Input
  tab, save and reconnect. Enable USB/Yubikey passthrough on the stock host.
- Supports pointer movement, five buttons, dragging and vertical/horizontal
  scrolling on one physical or virtual display. Relative game aiming and
  multi-monitor mapping are not supported. Local menus remain accessible.
- **USB HID keyboard:** a separate experimental BUD option, also disabled by
  default. Uses the same USB transport/device lifecycle as the mouse. Physical
  key presses, releases and modifiers are forwarded; select the same active
  keyboard layout on host and client (for example Hungarian on both). The client
  does not change the host layout. Normal Parsec keyboard forwarding is suppressed
  while selected. Focus loss, local menus and disconnect release held keys.
  The first keyboard build awaits live host/client validation.
- Only the usual EXE and settings.json are needed. No launcher or observer is
  required. While selected, ordinary Parsec mouse messages stay suppressed,
  including if HID setup fails; disable it and reconnect to restore normal input.
- Optional event logging records setup failures and state changes in the existing
  client-events.jsonl beside settings.json, without mouse movement, coordinates
  or click history, keystrokes or typed text. Video decoding and rendering are unchanged.

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
| HEVC Main 10 and 10-bit RGB SDR output | Experimental; live verified with NVIDIA host encoding and AMD 780M client decoding; tested AMD host sends 8-bit |
| Original-surface HEVC presentation on NVIDIA/AMD/Intel | Experimental; live tested on selected GPUs |
| Optional BUD direct UDP and TURN over UDP/TCP/TLS, automatic relay fallback | Experimental; direct/UDP/TLS live tested; TCP covered by isolated relay tests |
| Custom audio buffering and BUD raw PCM permission | Experimental; Opus and automatic buffering remain default |
| Transport-aware connection and playback statistics | Implemented |
| Network/Video/Audio settings tabs | Implemented |
| Gamepad verification and further native menu integration | Pending |
| Vulkan rendering and Linux client | Planned |
| D3D12, Android and Android TV | Exploring |

No delivery dates are promised. Other platforms also need native codec, audio,
window and input integration; adding a renderer alone will not make the client
portable to them.

## The Parsec core, beyond the browser

The original, pinned Parsec WebAssembly binary is embedded without modifying its
code. It is precompiled for Wasmtime at build time; this prepares the same core
for native execution rather than rewriting or patching it. No browser is involved.

Rust acts as the surrounding runtime and platform framework. It implements the
core's host interfaces for windows, graphics, GPU video decoding/presentation,
audio, input and networking, and adds native settings and statistics. These are
substantial native integrations, but they do not replace the original Parsec
core with a newly written client. Features added through this framework, including
the optional BUD transport and vendor-specific decoding, do not modify the WASM.

Research suggests shared foundations between Parsec's web core and its official
native client, but does not establish identical source code or feature sets.
Running the web core outside a browser should not be read as reconstructing the
official native client.

In WebRTC mode, Parsec's own video/audio/control messages travel through data
channels using ICE, DTLS and SCTP, rather than ordinary WebRTC media tracks/RTP
playback. This retains TURN's UDP/TCP/TLS connectivity. Optional BUD provides an
independent native UDP media transport, while the same client handles decoding,
sound, input and the embedded interface. A regular Parsec host supports both paths.

## Legal notice and original Parsec code

**This project was developed without access to Parsec's proprietary source code.
No proprietary Parsec source code has been edited or incorporated into the Rust
implementation. The embedded original Parsec WASM binary is unmodified.**

ParsecWebTurn uses the publicly served Parsec web-client WASM binary as a pinned,
closed-source component. The build verifies its SHA-256 against the pinned
reference before precompiling it for Wasmtime. Precompilation does not involve
editing Parsec's source or patching the WASM. The binary is included in the
distributed application; it is not an open-source component of this project.

The Rust code provides the native runtime, platform services and integrations
around that component. It does not constitute a rewrite of the original Parsec
client. The experimental BUD adapter is a separate, lowlat-based protocol
implementation, not a modification of Parsec's WASM or proprietary source.

The project's MIT license applies to its own code; it does not relicense the
proprietary Parsec component. Third-party components remain subject to their
respective licenses and applicable terms. Binary integrity checks establish
identity with the pinned reference, not permission to redistribute it.
ParsecWebTurn is independent and is not affiliated with or endorsed by Parsec
or Unity. Parsec names and marks belong to their respective owners.

## Acknowledgements: lowlat

Special thanks to Duy (Nguyen Duy) and the
[lowlat project](https://github.com/nomi-san/lowlat) for the inspiration,
open-source work and helpful technical feedback. Duy's suggestions and bug
reports have also helped improve ParsecWebTurn.

ParsecWebTurn reuses and adapts selected MIT-licensed lowlat components for its
experimental BUD transport, TURN connectivity and vendor-specific GPU video
backends. The reused code retains its license and attribution; pinned revisions,
local adaptations and integration boundaries are documented in the backend
crates' notices and [research references](docs/NATIVE_REFERENCES.md).

**ParsecWebTurn is an independently developed and maintained project. It is not
part of lowlat or an official lowlat client or release.** These acknowledgements
and the reuse of lowlat code do not imply affiliation with or endorsement by
lowlat or its maintainers.

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
