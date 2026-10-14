ParsecWebTurn v0.9.3 - Native Windows client

This stable release replaces the v0.7.0 WebView2 application with the native Rust
Windows client. It retains Parsec's familiar interface and adds native decoding,
input and connection controls. The changes below cover everything delivered
since the previous stable release, v0.7.0.

## What's new and improved

- **Native Windows client:** the Rust client replaces Tauri/WebView2. Run a single portable EXE without a browser runtime. The embedded Parsec core is precompiled, so it needs no download or compilation at startup.
- **H.265 / HEVC playback:** enable Prefer H.265 / HEVC to request an 8-bit stream. H.264 remains the default and fallback. Playback uses Windows Media Foundation and D3D11; HEVC requires compatible host, GPU, driver and Windows decoder support.
- **Video GPU selection:** choose the adapter for stream decoding and rendering on multi-GPU systems, or keep Automatic. Virtual display-only adapters no longer appear as duplicate GPUs.
- **More responsive controls:** input wakes processing immediately. Native relative mouse input and cursor warp handle desktop drags correctly; immersive Mouse, Keyboard and Both modes capture the selected input while the app has focus. Custom Parsec hotkeys are respected.
- **Flexible connection routing:** use Cloudflare or custom STUN/TURN servers over UDP, TCP or TLS. STUN-only excludes relays; Force TURN relay keeps the connection on a relay even when a direct route is available, including over a VPN.
- **More reliable TURN connections:** forced relay setup signals readiness without waiting for every transport endpoint. TURN channel bindings renew before the five-minute boundary, addressing the reported ExpressTURN interruption. Multiple server addresses can be entered on separate lines.
- **Compatible overlay and fullscreen:** software drawing is the default for the interface, resolving the reported Intel overlay flicker while streamed video retains its GPU path. The native menu hides in fullscreen.
- **Reliable session controls:** overlay disconnect returns to the computer list. Settings opens and closes more reliably; coordinated control painting and scroll limits reduce redraw flicker.
- **Stream efficiency and recovery:** reuse media buffers and GPU video views, shorten shared locks, disable extra picture processing and request low-latency decoding. Bounded keyframe recovery helps restart video after temporary decoder errors or overload. Results depend on the driver and host.
- **Native sound and saved preferences:** Opus playback uses native Windows audio with device-supported low-latency output and fallback. Sign-in and Parsec preferences persist in an encrypted Windows-user profile; existing server settings remain compatible.
- **Connection stats and About:** separate native statistics show actual direct/TURN routing, relay details, traffic, RTT, decoded video format, decoder, GPU, audio format and app resource usage. About compares the embedded core version with Parsec's public version without installing a new core.
- **Stable update notifications:** background checks and Help > Check for updates show stable releases and their notes. Download the ZIP through your browser and replace the EXE manually while keeping your settings.

## Requirements and upgrading

Windows x64 with compatible D3D11 and OpenGL graphics drivers is required.
WebView2 is no longer needed. H.264 is the default; optional HEVC playback needs
a compatible GPU and installed Windows HEVC decoder.

Close the app, extract `ParsecWebTurn-v0.9.3-win64.zip`, and keep your existing
`settings.json` beside the new EXE. Server settings and Windows-protected
credentials remain compatible under the same Windows account. When upgrading
from v0.7.0, sign in to Parsec once in the native client. Its encrypted profile is
separate from the old WebView2 profile, which remains untouched. Existing native
users keep their saved sign-in and preferences when replacing the EXE.

The earlier WebView2 source is preserved on the
[legacy branch](https://github.com/vampywiz17/ParsecWebTurn/tree/legacy), and the
v0.7.0 download remains available.
