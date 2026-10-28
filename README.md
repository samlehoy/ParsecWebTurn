<p align="center">
  <img src="assets/evoparseq-wordmark.svg" width="360" alt="EvoParseq">
</p>

<p align="center"><strong>An independent Windows Parsec client with a native Rust core.</strong></p>

EvoParseq connects to existing, **unmodified Parsec hosts**. Native Rust handles
sign-in, computer discovery, signaling, transport, GPU video, audio and input.
It does not bundle or run the proprietary Parsec WebAssembly client core and
requires no browser, WebView or helper executable.

**This is an independent community project, not affiliated with or endorsed by
Parsec, Unity or lowlat.** A Parsec account and a compatible host are required.

The **`dev` branch now contains EvoParseq**. The current preview is
[**v0.9.5-dev.2**](https://github.com/vampywiz17/ParsecWebTurn/releases/tag/v0.9.5-dev.2).
The `main` branch and stable **ParsecWebTurn v0.9.4** remain available separately.
This prerelease does not replace the stable release in the updater.

## Features

- **Native account and sessions:** optional 2FA, encrypted remembered sign-in,
  background-refreshing computer picker and manual peer ID. Owner connections
  are supported; friend/invitation authorization is not implemented.
- **WebRTC or BUD:** direct or custom TURN connections over UDP/TLS/TCP,
  Force TURN, STUN-only and Cloudflare TURN credentials. No silent transport
  switch. Optional experimental Windows TCP/TLS QoS marks outgoing TURN traffic.
- **GPU video:** H.264 and H.265/HEVC, GPU selection, Media Foundation and
  experimental vendor HEVC decoders, 10-bit SDR output, original-surface
  zero-copy, and compatible NVIDIA/Intel HEVC 4:4:4 at 8 or 10 bits.
- **Session overlay:** draggable icon; resolution, bandwidth, FPS, constant FPS,
  orientation, fullscreen and immersive input controls. Switch the current stream
  between host-listed physical or virtual displays.
- **Audio:** stereo Opus, configurable buffering, BUD raw PCM and experimental
  multichannel output. Automatically reopen the Windows audio output after
  transient device errors or default-device changes, retiring stale audio.
- **Input:** two-way Unicode text clipboard; normal mouse/keyboard forwarding;
  experimental BUD USB HID mouse and keyboard, including relative camera input.
- **Diagnostics:** route, RTT, traffic, FPS, actual decoder/format, audio queues,
  host/local timings and system resource use. Optional bounded event logs record
  failures without continuous mouse/key activity.
- **Presentation:** Automatic, Low latency, VSync and experimental DXGI VRR,
  with reference-safe catch-up after bursts. VRR requires an enabled compatible
  display/driver; DXGI support alone does not verify panel activation. Zero copy
  retains Windows-managed timing. WebRTC uses bounded recent queue history.

Host permissions, encoders and graphics drivers determine the actual stream.
**Connection stats shows the received depth/chroma and application RGB output**;
a selected preference alone does not prove that the host accepted it.

## Download and run

1. Download `EvoParseq-v0.9.5-dev.2-win64.zip` from the
   [preview release](https://github.com/vampywiz17/ParsecWebTurn/releases/tag/v0.9.5-dev.2).
   Development artifacts are also available from successful
   [dev CI runs](https://github.com/vampywiz17/ParsecWebTurn/actions/workflows/native-core.yml?query=branch%3Adev).
2. Extract to a writable folder and run **EvoParseq.exe**. Settings and optional
   encrypted sessions are stored beside it in `settings.json`.
3. Sign in with the host owner's account, choose a computer and connect.
   Configure Network, Video, Audio and Input in Settings. Save and reconnect
   after changing transport or connection capabilities; session video requests
   can be applied while connected.

**Client GPU requirement: Direct3D feature level 11.0 or newer**, with compatible
D3D11 video-support drivers, on Windows x64. The DirectX version shown on
dxdiag's System tab is the installed runtime version; check **Feature Levels**
on its Display tab for the GPU's capabilities. DirectX 10.x hardware, such as
the GeForce 210, is unsupported. Support below feature level 11.0 is not planned,
as it would require developing and maintaining a separate video path in parallel.

Zero-copy
requires Windows 11 and suitable drivers. Ordinary client use needs no admin
rights. The stock host must already be configured for hosting.

**Upgrading from ParsecWebTurn:** keep `settings.json` under the same Windows
account and sign in again. The old web-core login is not imported. A legacy-named
transition ZIP supports existing executable shortcuts. See [migration](MIGRATION.md).
Help > Check for updates offers newer **stable** EvoParseq packages; prerelease
installation is manual.

| Shortcut | Action |
| --- | --- |
| Ctrl+Shift+M / Ctrl+Shift+O | Open session menu and restore hidden icon |
| Ctrl+Shift+W | Toggle fullscreen |
| Ctrl+Shift+R | Release / recapture remote input |
| Ctrl+Shift+Q | Disconnect |
| Ctrl+Alt+End | Request host Ctrl+Alt+Del |

Ctrl+Alt+Del requires the installed Parsec host service and permissions; a
portable host may not execute it.

## Experimental features and limits

**Vendor HEVC, zero-copy, 10-bit HEVC, 4:4:4, display timing, USB HID and
multichannel audio remain experimental.** For conservative operation, use H.264,
stereo Opus and Automatic timing with experimental options disabled.

- **Video:** live decoded frames remain on the GPU. Normal rendering uses the
  conversion/output pass; zero-copy presents original 8-bit NV12 surfaces.
  Driver/compositor copies and direct scanout are not independently verified.
- **10-bit:** RGB10 output is SDR, not HDR. NVIDIA host encoding is confirmed;
  other host encoders may refuse it or send 8-bit. An initial encoder failure
  asks you to disable 10-bit and reconnect. Optional AMF Main10 output is
  live-validated on AMD Radeon 780M; Media Foundation remains its default.
- **4:4:4:** 8/10-bit playback is live-validated on NVIDIA RTX 3060. Intel support
  is capability-gated and **not live-validated**. AMD selection is disabled.
  H.264 stays 4:2:0; 4:4:4 does not combine with original-NV12 zero-copy.
  Compatible host encoding, account permissions and service terms still apply.
- **HID:** requires BUD and the stock host's USB/Yubikey passthrough plus its
  supported USB driver. HID mouse uses the host primary display and disables
  display switching; forwarding pauses until that selection is reported.
  Use matching host/client keyboard layouts. No host patch is needed.
- **Audio:** raw PCM is BUD-only and requires host permission
  (`network_raw_audio=1`). WebRTC remains stereo Opus. Multichannel audio has
  synthetic coverage only; live surround playback/channel order is unconfirmed.
- **Displays and timing:** host virtual-display limitations can delay switching.
  Reported timings are CPU/host durations, not GPU completion, scanout or total
  latency. Adaptive catch-up, QoS benefit and physical audio-device recovery
  still require broader live testing; no measured latency gain is claimed.

Gamepads, microphone/camera forwarding, HDR and multi-monitor HID mapping are
not implemented. See the [changelog](CHANGELOG.md) for shipped changes.

## Troubleshooting and development

Enable **Event logging** in Network settings for `client-events.jsonl` beside
settings.json. Include the build, GPU/driver, transport and reproduction steps
with a log or screenshot when reporting issues. Logs omit credentials, media,
clipboard text and continuous input. **Do not publish settings.json or tokens.**
Saved credentials use Windows CurrentUser DPAPI; copying them to another machine
or Windows account does not transfer access. `--data-dir "C:\YourFolder"` selects
another writable data directory.

Build with Rust MSVC and Windows C++ Build Tools/SDK:

```powershell
cargo build --locked --release --manifest-path src-native/Cargo.toml
```

CI packages `src-native/target/x86_64-pc-windows-msvc/release/evoparseq.exe` as
EvoParseq.exe. Application logic lives in `src-native/src`; platform, transport
and video adapters reuse the existing bounded workers and GPU path.

[Architecture](docs/native-core.md) · [Settings comparison](docs/native-settings-comparison.md) ·
[Development references](docs/NATIVE_REFERENCES.md)

## Credits and independence

Thanks to **Duy (Nguyen Duy)** and [lowlat](https://github.com/nomi-san/lowlat)
for inspiration, feedback and selected MIT-licensed protocol, transport and GPU
components. Their licenses and local-change notices are retained. EvoParseq is
independently developed and maintained; it is not an official lowlat client.

Our code uses the [MIT license](LICENSE). Applicable Parsec service terms and
third-party licenses still apply. Development did not use Parsec's proprietary
source code, and EvoParseq does not patch the official host or client. Parsec and
Unity names and marks belong to their owners; no affiliation is implied.

### Optional AI upscaling

The Video tab offers one **AI upscaling (experimental)** selector: NVIDIA RTX
VSR has Off/Low/Medium/High/Ultra; Intel VPL AI has Off/On. Other GPUs disable the
control. Each vendor's saved preference is retained.

Both paths upscale on the GPU in normal DXGI mode. They add GPU processing,
resources and an output copy, may increase latency, and are **not Zero copy**.
Unsupported configurations retain ordinary rendering. No CPU pixel readback
or generated frames are introduced.

- **NVIDIA:** RTX Video SDK 1.1.0; RTX 3060 playback is live-confirmed.
  The runtime is outside the EXE. On first launch with an NVIDIA GPU, setup asks
  permission before downloading the pinned official NVIDIA payload. Archive
  and DLL hashes are checked; verified caches are reused. The DLL lives under
  `%LOCALAPPDATA%/EvoParseq/vsr/1.1.0`, without admin rights. Declining leaves
  normal video available; enabling VSR later can authorize setup on next launch.
  SDK-enabled packages include NVIDIA's license; no CUDA toolkit is required.
- **Intel:** documented VPL 2.17 API with the installed Intel driver runtime,
  without a bundled/downloaded DLL or model. **Hardware operation is unverified.**
  Strict checks require the AI filter and same-device D3D11 surfaces; rejected
  runtime/configuration details remain in event logs. Currently limited to
  8-bit SDR upscaling; 10-bit, 4:4:4 and Zero copy keep their existing paths.
