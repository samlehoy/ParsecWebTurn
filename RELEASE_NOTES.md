ParsecWebTurn v0.9.4-dev.3 - BUD TCP/TLS relays and audio controls

This native Windows development release adds TCP/TLS TURN carriers to experimental
BUD, optional audio buffering and raw PCM controls, and connection diagnostics.
Changes below are compared with v0.9.4-dev.2.

**BUD, vendor-specific HEVC decoding, Zero copy, 10-bit HEVC and the new Audio
options are experimental.** Compatibility and stability depend on the GPU,
driver, host and network. For stability-first use, choose **WebRTC with H.264**,
leave experimental options disabled, or stay on the
[stable v0.9.3 release](https://github.com/vampywiz17/ParsecWebTurn/releases/tag/v0.9.3).
H.264 remains the default codec and uses Media Foundation on all GPU vendors.

## What's new and improved

- **Experimental BUD over TURN TCP/TLS:** BUD can use the same configured relay URLs as WebRTC, without an additional transport selector. It prefers UDP, then TLS and plain TCP (port 80 preferred among TCP URLs). Failed startup attempts can try the next configured relay. Direct BUD remains UDP; Force TURN keeps using a relay. BUD TURN/TLS playback has been confirmed with a stock host using AMD AMF HEVC and Direct D3D11 HEVC.
- **More reliable TCP/TLS startup:** correctly handle pending nonblocking TCP connections. The event-driven carrier disables Nagle before TLS/application data, retains partial writes under backpressure and uses bounded reusable buffers without a batching delay. TLS validates the relay's certificate and hostname. Existing WebRTC routing and GPU decoding/presentation are retained.
- **Experimental Audio tab:** optionally customize minimum and maximum playback buffering and try the device-supported low-period WASAPI output mode, with fallback when unavailable. Automatic playback remains the default. The custom values start at 75/150 ms, matching the embedded core's current automatic limits; saved custom values are preserved. Lower limits can increase underruns on unstable paths.
- **Experimental raw PCM audio for BUD:** allow uncompressed stereo audio when the host supports and enables it. Opus remains the default and can still be selected by the host; WebRTC audio remains Opus. Raw PCM requires more bandwidth.
- **Audio statistics and transport diagnostics:** Connection stats shows the actual audio codec, local output buffer and limits, device capacity, decoded PCM backlog and underruns. Optional event logging identifies BUD setup/transport failures and audio playback errors without recording credentials or media payloads.
- **HEVC-dependent video options:** Zero copy is selectable only while Prefer H.265 / HEVC is enabled, matching the 10-bit option. The saved preference is retained.
- **BUD Main 10 startup fix:** remove an obsolete parser restriction that disconnected BUD when Main 10 video arrived. Advertised Main 10 uses the existing MF/P010 GPU path. Host and driver compatibility still needs testing; the request does not guarantee a 10-bit stream. RGB display output remains 8-bit, original-surface Zero copy remains 8-bit only, and 4:4:4 remains unsupported.

## Updating

Close the app and extract `ParsecWebTurn-v0.9.4-dev.3-win64.zip`, or replace
`ParsecWebTurn.exe` in its existing folder. Keep your `settings.json`; saved
native sign-in, Parsec preferences and Windows-protected server credentials
remain in place under the same Windows account.

Use **Settings > Network** for BUD, relay URLs, Force TURN and optional event
logging. The selected TURN carrier appears in Connection stats after connection.
TCP/TLS is a client-to-relay tunnel; the relay-to-host leg remains UDP. TCP can
still incur head-of-line blocking during packet loss, so UDP remains preferred.

Use **Settings > Audio** for optional custom buffering or BUD raw PCM permission.
Save and reconnect after changing audio or transport settings. Raw PCM also
requires `network_raw_audio=1` on a compatible host.

This is a **Windows x64 prerelease**, not a replacement for the stable release.
WebRTC remains the default. If experimental video has problems, disable 10-bit
HEVC, Prefer H.265 / HEVC and Zero copy, then reconnect using H.264.
