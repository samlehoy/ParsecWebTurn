ParsecWebTurn v0.9.4 - BUD relays, GPU video and HID input

This stable Windows release brings together the changes from v0.9.4-dev.1 through
v0.9.4-dev.7, following the maintainer's Intel, AMD and NVIDIA video testing.
The changes below cover everything delivered since the previous stable release,
**v0.9.3**. This remains the ParsecWebTurn client built around the original,
unmodified Parsec WASM core.

## What's new and improved

- **BUD transport:** connect to unmodified Parsec hosts directly over UDP or through configured TURN relays over UDP, TLS or TCP. Use the same Network settings as WebRTC, including Force TURN; startup retries prefer UDP, then TLS, then TCP, with port 80 first among TCP URLs. An established connection is not migrated and BUD never silently switches to WebRTC.
- **Responsive relay IO:** native event-driven BUD TCP/TLS sockets retain partial writes, bounded reusable buffers, TLS backpressure and write deadlines. TLS verifies the server certificate and hostname. WebRTC TURN/TCP reuses receive storage, and TCP/TLS ChannelData is submitted with its padding in one framed write. Newly queued BUD HID reports and control replies drain before the next network wait, without a batching delay or changes to protocol retransmission timers.
- **Vendor HEVC decoding (experimental):** normal 8-bit HEVC uses Direct D3D11 on NVIDIA, AMF on AMD and VPL on Intel, with the existing Media Foundation recovery/fallback. H.264 continues to use Media Foundation on every vendor. Burst handling and surface ownership fixes improve HEVC startup and avoid unnecessary Intel decoder fallback.
- **Zero copy presentation (experimental):** supported NVIDIA, AMD and Intel GPUs can present original 8-bit NV12 decoder surfaces through Windows Presentation API, preserving aspect ratio and falling back to normal GPU rendering when needed. This removes the application's conversion/output pass without decoded-frame CPU readback; internal Windows/driver copies and direct scanout are not guaranteed. Windows 11 and compatible drivers are required.
- **Correct color range:** translate full/limited-range metadata correctly between Media Foundation and D3D11, including vendor output. This fixes grey-looking whites and lifted blacks. Startup, later encoder requests and keyframe requests consistently retain the selected range and HEVC flags.
- **10-bit HEVC and RGB SDR output (experimental):** request Main 10 over WebRTC or BUD when supported, including the BUD startup fix for negotiated 10-bit video. Actual P010 decoder output can feed a 10-bit RGB swap chain through the existing GPU conversion pass, with compatible 8-bit RGB output fallback. Stats distinguish decoded depth from application output. Host/driver support determines the received stream; this is SDR, not HDR, and original-surface Zero copy stays 8-bit only.
- **Audio controls:** add a dedicated Audio tab with optional minimum/maximum playback buffering, supported low-period shared WASAPI output with fallback, bounded fresh PCM queues and BUD-only raw PCM permission. Keep automatic buffering and Opus as defaults, align the menu's initial custom limits with the pinned core, and handle PCM/Opus transitions safely. Raw PCM also requires host support and can fall back to Opus; WebRTC remains stereo Opus.
- **Multichannel audio (experimental, BUD):** optionally request the Windows output's compatible 5.1/7.1 channel layout, decode Opus/PCM with per-packet format information and report the actual layout. Stereo remains the default and fallback. This feature has synthetic tests and silent WASAPI validation only; live surround playback and channel ordering are not confirmed.
- **USB HID mouse and keyboard (experimental, BUD):** use the stock host's USB/Yubikey passthrough for absolute desktop positioning, native relative camera control, five mouse buttons, dragging, scrolling and physical keyboard input. Mouse and keyboard share one device lifecycle and bounded queues; selected HID devices replace ordinary Parsec forwarding independently. Local menus remain accessible and focus loss/disconnect releases held input. No helper executable or host patch is needed; one physical or virtual display and matching keyboard layouts are supported.
- **Dragging and cursor transitions:** restore the host-reported endpoint when leaving relative/hidden capture so the next HID click does not return to the old drag anchor. Keep the cursor hidden until position restoration and replace its image before retiring the old handle. Prompt BUD capture-state wakeups reduce transition flicker while preserving camera input, event order, focus handling and local controls.
- **Two-way text clipboard:** forward native clipboard changes, focus restoration and paste-key requests through the original core while respecting core/host permissions. Preserve UTF-8 boundaries, avoid echoes and distinguish unavailable reads from copied empty text. Client-to-host pasting has been verified live; file/image transfer is not added.
- **Display timing and burst catch-up:** add experimental Automatic, Low latency and VSync choices. Automatic preserves previous behavior; explicit DXGI modes request a one-presentation queue. Interruptible display/input readiness waits fix the reported Low latency stutter while preferring fresh decoded input over a pending presentation. BUD can skip obsolete display work without dropping reference decoding. No extra decoded-frame copy, RGB target or rendering thread is introduced; original-surface timing stays Windows-managed.
- **More useful diagnostics:** Connection stats adds native BUD route/RTT/traffic/delivery counters, actual audio codec and queue/buffer details, decoded/RGB precision, automatic host video/audio timing and bounded local video-stage measurements. Correct encode/frame-interval labels and add Present submission interval. The embedded overlay receives native Decode and Network values. Timings describe their reported host durations or CPU-observed stages, not GPU completion, scanout or end-to-end latency; unavailable values remain unknown.
- **Optional event logging:** Network settings can create a bounded asynchronous UTF-8 JSON Lines log beside settings.json, with UTC connection/route transitions, transport/decoder/HID failures and audio/profile state. Rotation retains one previous file. Logging is off by default and excludes credentials, media, clipboard content and continuous mouse/key activity; temporary input trace capture has been removed.
- **Settings, recovery and maintenance:** separate Network, Video, Audio and Input with consistent tab spacing, shared saving and preserved unsaved edits. Disable Zero copy and 10-bit controls while HEVC is off, fix wrapped settings/stats text and guide unsupported host launch choices. Reuse bounded media/input storage, shorten shared locks and release retired video reuse pools. Add concise bug/feature/question issue forms, consolidate active native tests, remove retired experiments, shorten the README, clarify the unchanged-core provenance and credit lowlat's reused open-source components.

## Compatibility and validation

**Vendor HEVC decoding, Zero copy, 10-bit HEVC, display timing, multichannel
audio and USB HID input remain experimental options in this stable release.**
WebRTC, H.264, stereo Opus and Automatic display timing remain the defaults.
BUD, custom audio buffering, raw PCM permission and host timing no longer carry
an experimental label. For the most conservative setup, keep the defaults and
leave experimental options disabled.

Successful testing applies to the supported configurations tested, not every
GPU/driver combination. Confirmed Main 10 host encoding is documented for the
tested NVIDIA setup; other encoders may send 8-bit. Stats show the stream and
output actually received. Multichannel audio remains supported by synthetic
tests only.

## Requirements and upgrading

Windows x64 with compatible D3D11/OpenGL graphics drivers is required. No browser,
WebView2 runtime or additional client helper is needed. Optional original-surface
Zero copy requires Windows 11 and driver support. Vendor video, Main 10 and
multichannel options depend on compatible host and client capabilities.

Close the app and extract `ParsecWebTurn-v0.9.4-win64.zip`, or replace
`ParsecWebTurn.exe` in its existing folder. Keep your `settings.json`; saved native
sign-in, embedded Parsec preferences and Windows-protected server credentials
remain compatible under the same Windows account. Use the host's **web** launch
option, including when BUD is selected in Settings.

USB HID input requires BUD and USB/Yubikey passthrough enabled on the stock host,
including its supported USB driver. BUD raw PCM requires host permission
(`network_raw_audio=1`). Save and reconnect after changing connection options.
The original WASM core and stock host remain unmodified.
