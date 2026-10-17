ParsecWebTurn v0.9.4-dev.4 - Relay IO improvements and 10-bit SDR output

This native Windows development release improves BUD and WebRTC TURN stream
handling and adds 10-bit RGB SDR output for compatible Main 10 streams.
Changes below are compared with v0.9.4-dev.3.

**BUD, vendor-specific HEVC decoding, Zero copy, 10-bit HEVC and the Audio
options remain experimental.** Compatibility and stability depend on the GPU,
driver, host and network. For stability-first use, choose **WebRTC with H.264**,
leave experimental options disabled, or stay on the
[stable v0.9.3 release](https://github.com/vampywiz17/ParsecWebTurn/releases/tag/v0.9.3).
H.264 remains the default codec and uses Media Foundation on all GPU vendors.

## What's new and improved

- **Native BUD TCP/TLS IO:** relay sockets now use the existing platform wake and native socket wait, including overlapped IOCP on Windows. Bounded reusable buffers handle partial sends, cancellation and queued TURN frames without a batching delay; incoming frames are parsed in place. TLS remains rustls with certificate and hostname verification. Direct BUD remains UDP.
- **More reliable BUD TLS backpressure:** a frame stays pending until its TURN framing and encrypted TLS output have drained to the socket. New protocol output cannot accumulate behind an earlier TLS record, and slow sends retain their original write deadline.
- **More efficient WebRTC TURN/TCP and TLS:** plain TCP receives use a bounded reusable buffer, and ChannelData plus padding is sent as one framed write on TCP/TLS. TLS keeps its existing receive path. No batching delay is introduced; UDP routing, ICE selection and SCTP reliability/ordering are unchanged.
- **Experimental 10-bit RGB SDR output:** when the host actually sends Main 10 and the decoder produces P010, compatible GPUs can present through a 10-bit RGB swap chain. The existing GPU conversion writes directly into the output buffer without another decoded-frame copy or conversion pass. Unsupported conversion or presentation falls back to 8-bit RGB. This is SDR, not HDR; application output depth does not prove physical display or scanout depth.
- **Clearer decoded and output depth:** Connection stats and optional event logging separately report decoded color depth, application RGB output depth and output fallback. Live Main 10 with 10-bit RGB output is currently verified with NVIDIA host encoding and AMD Radeon 780M client decoding. The tested AMD host path sent 8-bit HEVC; other host configurations remain unverified. Requesting 10-bit does not guarantee that the host sends it. Original-surface Zero copy remains 8-bit only, and 4:4:4 remains unsupported.
- **Leaner native components and clearer credits:** remove unused direct H.264 decoder code and an unused GPU protocol-core dependency while retaining Media Foundation H.264 playback on all GPU vendors. The README explicitly acknowledges lowlat inspiration and reused components and clarifies that ParsecWebTurn is an independent project.

## Updating

Close the app and extract `ParsecWebTurn-v0.9.4-dev.4-win64.zip`, or replace
`ParsecWebTurn.exe` in its existing folder. Keep your `settings.json`; saved
native sign-in, Parsec preferences and Windows-protected server credentials
remain in place under the same Windows account.

Use **Settings > Network** for BUD, relay URLs and Force TURN. UDP remains
preferred, followed by TLS and plain TCP. TCP/TLS tunnels only the client-to-relay
leg; the relay-to-host leg remains UDP. TCP can still incur head-of-line blocking
during packet loss.

Use **Settings > Video** for experimental HEVC, 10-bit HEVC and Zero copy.
The actual decoded depth and RGB output appear in Connection stats. A 10-bit
request cannot override missing host encoder support or firmware restrictions.

This is a **Windows x64 prerelease**, not a replacement for the stable release.
WebRTC remains the default. If experimental video has problems, disable 10-bit
HEVC, Prefer H.265 / HEVC and Zero copy, then reconnect using H.264.
