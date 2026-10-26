<p align="center">
  <img src="assets/evoparseq-wordmark.svg" width="360" alt="EvoParseq">
</p>

<p align="center"><strong>A Windows Parsec client built on a new, native Rust core.</strong></p>

EvoParseq connects to your existing, unmodified Parsec hosts. Its new native core
handles sign-in, computer discovery, session signaling and connection control;
the Rust client provides networking, GPU video, audio and input directly.
**EvoParseq implements its client directly in native Rust and does not bundle
proprietary Parsec client code.** No browser is required.

**EvoParseq is an independent community project. It is not affiliated with,
endorsed by, or developed by Parsec or Unity.** It is also independent of lowlat.
An existing Parsec account and a compatible Parsec host are required.

This README describes the **`codex/native-core` EvoParseq development branch**.
The repository's `main` and `dev` branches and their releases are separate.

## What it can do

- **Native sign-in and computer picker:** optional two-factor authentication,
  encrypted remembered sessions, manual host peer ID, and background list refresh.
- **WebRTC and BUD:** direct connections or custom TURN relays over UDP, TLS or
  TCP; Force TURN, STUN-only and Cloudflare TURN credentials. Transport selection
  is explicit, with no silent switch between WebRTC and BUD. Optional experimental
  Windows TCP/TLS QoS marks outgoing TURN traffic; off by default.
- **GPU video:** H.264 and preferred H.265/HEVC, selectable GPU, D3D11 rendering
  and Media Foundation decoding, plus experimental NVIDIA/AMD/Intel HEVC paths.
- **Session controls:** draggable overlay icon, resolution requests including
  Keep host / Match client, bandwidth limit, frame rate, constant FPS,
  orientation, fullscreen and mouse/keyboard immersive mode. A host display
  selector switches the current stream between listed physical/virtual outputs,
  using Keep host resolution on the selected display.
- **Audio:** stereo Opus, adjustable playback buffer and optional raw PCM over
  BUD. Experimental multichannel audio can request a compatible 5.1/7.1 layout.
- **Input and clipboard:** normal mouse/keyboard forwarding, two-way Unicode text
  clipboard, and experimental USB HID mouse/keyboard over BUD.
- **Diagnostics:** connection route, RTT, traffic, FPS, decoder/output format,
  CPU/GPU/memory use, audio queues, host timing and local video timings.
  Optional event logs record failures without continuous mouse/key logging.

Host permissions, encoder capabilities and driver support determine which
requests can be fulfilled. A preference is not proof of the received stream;
**Connection stats reports the actual decoder, bit depth and output**.

## Try it

1. Download the **EvoParseq-win64** artifact from a successful
   [native client CI run](https://github.com/vampywiz17/ParsecWebTurn/actions/workflows/native-core.yml?query=branch%3Acodex%2Fnative-core).
2. Extract it to a writable folder and run **EvoParseq.exe**. You can reuse your
   existing `settings.json`; the name change does not change its data format.
3. Sign in with the account that owns the host, select a computer and connect.
   Owner connections are the currently supported authorization path.
4. Use **Settings** for Network, Video, Audio and Input. Save and reconnect when
   changing transport or connection capabilities; use the session menu for live
   host video requests.

Use **Help > About EvoParseq / Check for updates** for application information
and stable updates. See [migration from ParsecWebTurn](MIGRATION.md) for existing
settings, sign-in and transition-package details.

Requires Windows x64, a D3D11-capable GPU and suitable graphics drivers.
Original-surface zero-copy also requires Windows 11 and driver support.
Ordinary client use does not require administrator rights or helper programs.
The host must already be configured for Parsec hosting.

| Shortcut | Action |
| --- | --- |
| Ctrl+Shift+M / Ctrl+Shift+O | Open the session menu and restore a hidden icon |
| Ctrl+Shift+W | Toggle fullscreen |
| Ctrl+Shift+R | Release / recapture remote input |
| Ctrl+Shift+Q | Disconnect |
| Ctrl+Alt+End | Request Ctrl+Alt+Del on the host |

Secure attention requires the host's installed Parsec service and permissions;
a portable host may not execute it.

## Experimental features and limits

**Vendor HEVC decoding, zero-copy, 10-bit HEVC, HEVC 4:4:4, display timing,
USB HID input and multichannel audio remain experimental.** Use H.264 with
experimental options disabled for the most conservative configuration.

- **Zero-copy:** presents original 8-bit NV12 decoder surfaces through Windows
  Presentation API. Normal rendering uses a GPU conversion/scaling pass. Neither
  path reads decoded frames back to the CPU. Internal driver/compositor copies
  and direct scanout are not independently verified.
- **10-bit HEVC:** the Media Foundation path can produce 10-bit RGB SDR.
  An optional experimental AMD AMF Main10 path reuses P010 GPU output;
  it is off by default and live-validated on Radeon 780M (performance A/B pending). Live
  Main 10 host encoding has been confirmed with NVIDIA; AMD configurations have
  not consistently worked in testing. This is not HDR. An initial host encoder
  error asks you to disable 10-bit and reconnect; no automatic 8-bit retry occurs.
- **HEVC 4:4:4:** available experimentally on compatible NVIDIA and Intel
  client GPUs, with compatible host encoding. **Intel is not live-validated**;
  its decoder and output support must pass runtime checks. The 8-bit GPU path
  has been confirmed live over BUD on NVIDIA RTX 3060; other configurations
  remain unverified. Combining 4:4:4 with 10-bit HEVC is implemented
  experimentally with direct Y410-to-RGB10 GPU conversion; live 1920x1080
  playback is confirmed on RTX 3060 over BUD/TURN-UDP. AMD cannot enable 4:4:4 in Settings.
  The tested AMD Radeon 780M driver did not expose the required D3D11 profile.
  H.264 remains 4:2:0; original-NV12 zero-copy is not combined with
  this path. Host/account terms and permissions still apply.
- **USB HID:** BUD only, with USB/Yubikey passthrough enabled on the stock host.
  Supports mouse buttons, dragging, scrolling, relative camera input and keyboard
  forwarding. HID mouse input disables the display selector. Use the host primary
  display (or one active physical/virtual display) and matching keyboard layouts.
  On stock Windows hosts the client requests the primary output when necessary,
  and pauses HID mouse forwarding until the host reports that output selected.
  Absolute desktop reports remain bounded; relative camera input is unchanged.
  The primary-output restriction has synthetic coverage; live multi-monitor
  validation is still required.
  No host patch is required; the host needs its supported USB driver.
- **Raw PCM:** BUD only; the host must allow it (`network_raw_audio=1`). WebRTC
  audio remains stereo Opus. Multichannel playback has **synthetic test coverage
  only**, without confirmed live surround playback/channel ordering.
- **Timing:** local values measure CPU durations, not GPU completion, physical
  scanout or end-to-end latency. Traffic includes transport overhead rather than
  pure video bitrate. New format-change handling still needs live GPU validation.

Gamepads, microphone/camera forwarding, HDR, multi-monitor HID mapping and
friend/invitation authorization are not part of the current native client.

## Settings and troubleshooting

Preferences and the optionally remembered session live in **`settings.json`
beside the EXE**. Session tokens and saved server credentials use Windows
CurrentUser DPAPI; passwords and two-factor codes are not saved. Keep this file
when replacing the EXE. `--data-dir "C:\YourFolder"` selects another writable folder.

Enable **Event logging** in Network settings to create **`client-events.jsonl`**
in the same folder. Logs contain UTC timestamps, connection transitions and
media errors, without credentials, clipboard text, media content or continuous
input events. When reporting a problem, include the build, GPU/driver, transport,
reproduction steps and a log or screenshot. **Do not publish `settings.json`
or account tokens.**

[Native implementation notes](docs/native-core.md) ·
[Settings comparison](docs/native-settings-comparison.md) ·
[Protocol research references](docs/NATIVE_REFERENCES.md)

## Build

Use the Rust MSVC toolchain and Windows C++ Build Tools/SDK:

```powershell
cargo build --locked --release --manifest-path src-native/Cargo.toml
```

The binary is `src-native/target/x86_64-pc-windows-msvc/release/evoparseq.exe`.
CI packages it as **EvoParseq.exe**, checks the native dependency graph and runs
native regression tests. Application source is in `src-native/src`;
transport/video adapters are in `src-native/crates`. Only the current native
runtime and its supported components are retained in this branch.

## Credits and independence

Thanks to **Duy (Nguyen Duy)** and [lowlat](https://github.com/nomi-san/lowlat)
for inspiration, technical feedback and open-source components. EvoParseq reuses
selected, pinned MIT-licensed protocol, transport and GPU components, retaining
their licenses and attribution. It is independently developed and maintained;
it is not an official lowlat client.

The project's own code is provided under the [MIT license](LICENSE). Third-party
licenses and applicable Parsec service terms still apply. This native client
was developed without access to Parsec's proprietary source code and does not
patch the official host or client. Parsec and Unity names and marks belong to
their respective owners; no affiliation or endorsement is implied.
