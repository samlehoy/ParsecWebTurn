# EvoParseq architecture and validation

EvoParseq is an independent native Rust Windows client for compatible, unmodified
Parsec hosts. It is not affiliated with Parsec/Unity or lowlat.

## Build and package

The default build is the complete native application:

```powershell
cargo test --locked --manifest-path src-native/Cargo.toml
cargo build --locked --release --manifest-path src-native/Cargo.toml
./scripts/package-native.ps1
```

The Windows SDK/MSVC toolchain and Rust are build-time requirements. The portable
application uses EvoParseq.exe and settings.json; ordinary use does not require
administrator rights. The configured static CRT and bundled Opus remain in place.

## Native components

- `native_core/account.rs`: account sign-in, optional two-factor authentication,
  computer discovery and session management. Remembered sessions are protected
  using Windows CurrentUser DPAPI in settings.json.
- `native_core/sign_in.rs`: window-API-independent sign-in/sign-out request
  lifecycle, busy/error state and remembered-sign-in preference. Password and
  optional 2FA code remain zeroizing request-owned values, without a retry cache.
  The Win32 adapter retains the existing DPAPI save policy; authentication APIs,
  session revocation, protected storage and polling cadence are unchanged.
- `native_core/picker.rs`: window-API-independent computer list and discovery
  controller. It keeps one refresh in flight, distinguishes manual/background
  results, preserves selection by peer ID and avoids updates for identical lists.
  Cancelled replies cannot update a new account/view. The Win32 adapter retains
  its existing 30-second timer and 80/500 ms completion polling; discovery uses
  the same per-request HTTP worker, endpoint, validation and timeouts. Sign-in,
  two-factor authentication and protected-session storage remain unchanged.
- `native_core/signaling.rs` and `transport.rs`: authenticated session signaling
  and explicit BUD/WebRTC selection. Failure never silently changes transport.
- `attempt.rs`: native ICE/DTLS/SCTP and data channels for WebRTC.
- `crates/bud`: isolated BUD carrier, protocol and compressed-media handoff.
  Both transports use the same native video/audio workers and input handling.
- `crates/video`: GPU decoder backends and original-surface presentation.
  Normal H.264 and Main 10 4:2:0 use Media Foundation; 8-bit vendor HEVC uses
  AMF/VPL/direct D3D11. Full-chroma uses the common direct D3D11 path.
- `video_windows.rs`: decoder surfaces feed the existing D3D11 video processor
  directly, followed by DXGI presentation. No CPU pixel readback or intermediate
  decoded-frame export/copy is added. Original NV12 presentation uses Windows
  Presentation API and explicit surface leases.
- `audio_stream.rs` and `audio_windows.rs`: bounded decoded PCM queues, reusable
  buffers and native WASAPI playback. Stereo Opus works on both transports;
  BUD can also request raw PCM and experimental 5.1/7.1 audio. Multichannel
  support has synthetic test coverage only.
- `window.rs`, `input_state.rs`, `native_core/session_ui.rs`: Win32/GDI UI,
  immersive capture, relative camera input and local controls. GPU video remains
  separate from UI painting. HID mouse/keyboard share one bounded BUD device
  lifecycle; ordinary Parsec input for a selected HID device is suppressed.
- `native_core/session_model.rs`: window-API-independent Session commands and
  shared host-video/display/pending state. The Win32 panel reads typed host
  preferences and uses the common saved/host/default selection rules. Existing
  window events and notifications still deliver commands and view updates;
  there is no additional queue, thread, frame processing or dependency. This
  is an initial UI separation, not a complete cross-platform application port.
- `native_core/session_video.rs`: host-settings control depends on the small
  `session_model::View` interface (state, client display size and typed status/
  display notifications), rather than the Win32 Window or Session panel. The
  Win32 adapter alone translates these notifications to its existing messages.
  Transport and input/media services retain their existing platform adapters.
- Settings, cursor, clipboard, diagnostics and optional event logging are native.
  Logging excludes continuous mouse/key activity and authentication secrets.

## Session behavior

The computer picker refreshes in the background without rebuilding unchanged
controls. Its draggable session icon opens resolution, bandwidth, FPS, pacing,
orientation, immersive mode and fullscreen controls. Video options are host
requests, not local frame duplication. Applied host settings and actual decoded
formats are reported separately. A host service and authorization are required
for the secure-attention request; portable hosts may not execute it.

UI, decoder, route and audio state are generation-scoped. Reconnect/cancellation
retires the previous attempt; original-surface presentation retains decoder
allocations until Windows releases them. Queue pressure may skip display updates
but must not discard reference dependencies or create unbounded latency.

Immersive mouse confinement follows the Windows cursor hotspot (the click
position); the cursor image can extend beyond the right/bottom edge. No cursor-
size inset is applied, so remote edge pixels remain reachable. Relative camera
input retains its existing Raw Input and one-pixel-anchor behavior. CI runs the
native capture lifecycle separately to prevent other UI tests stealing focus.
Destroying a stream window also terminates its own message loop, releasing the
UI thread's window reference instead of leaving it blocked in GetMessage.

## Validation scope

CI runs native unit/integration tests, GPU surface ownership/metadata tests,
BUD control/HID regressions and Win32 UI captures. Offline/synthetic tests do
not establish physical GPU capability, screen scanout, HDR or host compatibility.
Current live validation is summarized below and in the root README.

## Experimental 4:4:4 and WebRTC

The WebRTC account adapter uses the native Rust ICE/DTLS/SCTP
engine and its pinned channel/initialization contract, using native signaling.
It still needs live direct, TURN/UDP, TURN/TLS and TURN/TCP verification in this
independent build. BUD-only HID and raw/multichannel audio stay BUD-only.

Prefer 4:4:4 is available on NVIDIA and Intel client GPUs (experimental).
Intel 8-bit and combined 10-bit 4:4:4 playback are not live-validated. Intel
uses the same direct D3D11 AYUV/Y410 path as NVIDIA; the existing Intel VPL
4:2:0 path is unchanged. No extra decoded-frame copy or dependency is added.
It offers 8-bit HEVC after selected-GPU Main444/AYUV decoding and processor
input checks. With 10-bit HEVC also enabled, it checks Main10 4:4:4/Y410 decode,
Y410-to-RGB10 conversion and the existing Main/Main10 4:2:0 fallback rows. Existing pinned lowlat DXVA range-extension
parameters are reused in the GPU-only direct decoder. AYUV goes directly to the
existing conversion/presentation pass, with no CPU pixel readback or intermediate
video copy. This mode uses normal GPU rendering, not original-NV12 zero copy;
The two preferences can be combined; the host remains responsible for the
actual stream format. A 10-bit 4:4:4 surface uses the same single processor
write directly into the RGB10 swapchain, with no intermediate RGB texture.
Live 1920x1080 BUD/TURN-UDP playback of this combined mode was confirmed on
NVIDIA GeForce RTX 3060 on 2026-10-25: both request flags were declared
(`flags=27`), decoded output was 10-bit 4:4:4 and RGB output was 10-bit SDR,
with no decoder fallback/error recorded in that attempt. This validates that
configuration, not other GPUs/transports, physical panel depth or HDR. A rejected Y410/RGB10
conversion is an error, rather than a silent retry in RGB8. The host can still send
4:2:0; actual decoded chroma is shown separately in Connection stats. Live BUD
8-bit 4:4:4 decoding/presentation has been confirmed on NVIDIA GeForce RTX 3060.
The tested AMD Radeon 780M driver rejects the Main444 profile query with
`0x80070057`, so that configuration advertises 4:2:0 only. This is not a blanket
claim about all AMD hardware or APIs. AMD 4:4:4 remains disabled; Settings
clears and disables the checkbox for AMD/unknown adapters, including Automatic
when Windows selects one. NVIDIA and Intel still need actual decoder and
processor support; vendor eligibility alone does not advertise 4:4:4. The
connection capability check independently enforces this policy. Other NVIDIA
configurations and all Intel configurations remain unverified by live tests.

See stock settings comparison for existing
features and possible future additions; unimplemented options were not added.

For a requested Main 10 connection, a host encoder error `-15002` before the
first decoded frame displays a message asking the user to disable 10-bit HEVC
in Settings > Video and reconnect. The client does not automatically retry
in 8-bit or change saved preferences. Other host errors retain their numeric
status and the existing error handling.

In the native build, Connection stats is an independent top-level window so
normal activation can place it in front of or behind the stream. Session stays
owned by the stream. Closing the stream explicitly closes its stats window.

HEVC format changes are handled before compressed input reaches the old GPU
decoder. The client rebuilds the selected decode/presentation path, retains
current parameter sets and the arriving keyframe, and requests a new keyframe
only when needed. A size change alone does not disable vendor decoding or
zero-copy, and does not consume the decoder-error retry limit. Live validation
with remote hosts and the affected GPU paths is still required.

Full-chroma requests are retained on BUD per-stream encoder updates as well as
initialization and native keyframe requests. Capability logs distinguish
`chroma_444_requested` from the effective `chroma_444` declaration. When a
requested AYUV path is unavailable, `video_chroma_444_unavailable` records the
bounded GPU probe stage/result. Decoder state logs include the actual adapter,
decoded chroma and bit depth; no media data or continuous input is recorded.

The common video processor accepts AYUV decoder textures directly, alongside
NV12/P010. This fixes an initial 4:4:4 integration bug where the renderer's old
texture guard rejected otherwise accepted packed 4:4:4 input. No CPU repacking,
intermediate decoded texture or additional color conversion pass is introduced.
The regression test validates the format/array/geometry boundary without
claiming successful hardware 4:4:4 decoding. Actual GPU/driver output still needs
live confirmation. Failures record the vendor fallback reason for diagnosis.


## Host display selection

The Session menu queries the stock host's output list (application sub-ID 10,
reply 12) and current owner configuration (9, reply 11). Selecting a listed
physical or virtual output fetches both the current output list and fresh settings,
waits for both replies, revalidates the selected ID, changes `video[0].output`,
releases the previous output's forced size (`resolutionX/Y: 0`), and sends the
complete preserved configuration (11). The write is sent once;
read-only queries confirm the selected identity for up to five seconds. Other
video slots and unknown host settings are preserved. Refresh handles output
hotplug; a missing/invalid list leaves the stream running. Output IDs are neither
persisted as client preferences nor written to the event log.

This selects one existing stream, not simultaneous multiple stream windows or
creation of virtual displays. BUD and WebRTC reuse their existing reliable
control path and unchanged GPU decoding/presentation. No new dependency, image
copy, frame readback or per-frame display query is introduced. Live physical and
virtual multi-output switching still needs validation against a stock host.

HID mouse mode disables the selector in the UI and rejects requests in the
session controller. It keeps the existing primary-monitor-normalized absolute
reports, clamped to 0..32767; camera-relative reports retain their existing path.
HID keyboard alone does not disable switching. On the examined stock Windows
host, the list preserves DXGI adapter/output enumeration order. Both Microsoft
APIs document primary-first enumeration; the host serializes `hostOS: 1`.
The client uses this Windows-specific contract to request the first output when
necessary, preserving all other settings, and pauses HID mouse forwarding until
the reported output agrees. Unknown OS, missing/invalid list or failed
confirmation leaves the mouse paused with a visible explanation; refresh can
retry. Releases and keyboard forwarding remain available. Primary selection is
not inferred from display names, ID syntax or the client's own monitor layout.
See [reference evidence](NATIVE_REFERENCES.md#stock-windows-host-display-order).

This has synthetic/CI coverage; live multi-monitor validation is still needed.
Changing the host's primary monitor or capture configuration outside the client
requires refreshing the list or reconnecting: there is no host topology-change
subscription in the examined contract. The restriction governs this client's
absolute desktop reports and output selector, not physical input by someone at
the host or the operating system's cursor confinement during a game. No
relative-desktop or normal-injection fallback is introduced.


An announced BUD encoder rebuild (metadata or an encoder-generation change)
retires the old compressed queue, decoder and cached codec parameters before the
new keyframe. H.264/HEVC changes across this boundary can reuse the video worker
and presentation window; unannounced codec changes remain rejected. This does
not consume decoder-error recovery attempts or add decoded-frame copies. Initial
video requests wait for the complete host config even when the output-list reply
arrives first. Host configuration logs include display ordinals/counts rather
than private output identities to distinguish configuration confirmation from
successful media decoding.


Display selection returns the session's resolution preference to **Keep host**,
so an old fixed/client size is not automatically forced onto a different output.
The preference is updated in the session menu and saved profile; bandwidth, FPS,
pacing, orientation, immersive mode and other video slots are retained. A custom
size can be requested after selecting the display if that host output supports
it. Applying **Keep host** now explicitly sends the zero-size/no-override request
instead of silently retaining the last fixed request. This releases a client
constraint; live host behavior determines the resulting Windows display mode.
The client does not invent a display's original/maximum size from its name or
restore modes through a new host helper.

Live AirGPU testing identified a host-side limitation on its default 800x600
virtual output: NVENC rejected the capture device, the host returned to the
1920x1080 NVIDIA output on early attempts, and later selected H264 Encoder MFT
for the 800x600 output. A HEVC request could also fall back to H.264 on the host.
The client configuration reply confirms a requested output, not successful
capture from it. This VM result is not a general limitation of virtual displays;
switching between outputs backed by a compatible GPU still needs separate live
validation.
