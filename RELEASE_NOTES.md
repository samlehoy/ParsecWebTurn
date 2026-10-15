ParsecWebTurn v0.9.4-dev.1 - Experimental BUD and GPU video paths

This native Windows development release adds an alternative BUD transport,
vendor-specific HEVC decoding and optional original-surface presentation.
Changes below are compared with the stable v0.9.3 release.

**BUD, vendor-specific HEVC decoding and Zero copy are experimental.** They have
worked on the tested systems, but compatibility and stability depend on the GPU,
driver, host and network. For stability-first use, choose **WebRTC with H.264**
and leave the experimental options disabled, or stay on the
[stable v0.9.3 release](https://github.com/vampywiz17/ParsecWebTurn/releases/tag/v0.9.3).
H.264 remains the default codec and uses Media Foundation on all GPU vendors.

## What's new and improved

- **Experimental BUD transport:** enable Experimental BUD in Settings to use the native UDP protocol with a regular Parsec host. Direct UDP, automatic fallback to a configured UDP TURN relay and Force TURN are supported. WebRTC remains the default and retains TURN over UDP, TCP and TLS; BUD supports UDP only and does not silently switch to WebRTC.
- **Experimental vendor-specific HEVC decoding:** when Prefer H.265 / HEVC is enabled, normal 8-bit HEVC playback uses Direct D3D11 on NVIDIA, AMF on AMD and VPL on Intel. These paths keep decoder output on the GPU and feed the existing presentation path directly. The existing GPU Media Foundation fallback/recovery remains available when a vendor path cannot be used.
- **Experimental Zero copy presentation:** the Video tab can select original NV12 decoder-surface presentation through Direct D3D11 and Windows Presentation API on supported NVIDIA, AMD and Intel GPUs. This removes the application's conversion/output pass and introduces no intermediate decoded-frame copy or CPU pixel readback. Windows 11 and compatible hardware/drivers are required; internal Windows/driver copies and direct scanout are not guaranteed.
- **More reliable HEVC startup and recovery:** handle short arrival bursts without exhausting presentation surfaces or unnecessarily abandoning the working Intel decoder. Original-surface presentation preserves the picture's aspect ratio and can recover to normal GPU rendering when unavailable.
- **BUD connection statistics:** show native RTT, decoded FPS, incoming/outgoing traffic, direct or relay route, host encode time when available and BUD delivery counters. Irrelevant WebRTC ICE/DTLS fields are hidden in BUD mode, and the last valid host encode time no longer flickers between reports.
- **Network and Video settings tabs:** connection servers, credentials, routing and transport settings are separated from GPU, codec and presentation choices. Both tabs share Save settings and retain unsaved edits when switching tabs.
- **Clearer playback status:** Connection stats uses shorter Zero copy, decoder fallback and playback-error descriptions while retaining detailed diagnostic information separately.
- **Safer host launch selection:** choosing the unsupported native-client launch option now explains that the web launch option is required instead of crashing the application. The web launch option is also used when BUD is selected in Settings.
- **Less input and shared-lock overhead:** reuse bounded input-event storage, pass native input arguments without temporary vectors and copy queued control messages outside the shared connection lock. Event ordering, immediate input wakeups and WebRTC reliability remain unchanged.

## Updating

Close the app and extract `ParsecWebTurn-v0.9.4-dev.1-win64.zip`, or replace
`ParsecWebTurn.exe` in its existing folder. Keep your `settings.json`; saved
native sign-in, Parsec preferences and Windows-protected server credentials
remain in place under the same Windows account.

Open **Settings > Network** to select Experimental BUD or configure TURN.
Open **Settings > Video** to select a GPU, Prefer H.265 / HEVC or Zero copy.
Save and reconnect after changing these options. The Zero copy checkbox applies
to HEVC; H.264 keeps the existing Media Foundation path.

This is a **Windows x64 prerelease**, not a replacement for the stable release.
If you need TURN over TCP/TLS, keep WebRTC selected. If experimental video has
problems, disable Prefer H.265 / HEVC and Zero copy and reconnect using H.264.
For the most conservative choice, use the published stable release instead.
