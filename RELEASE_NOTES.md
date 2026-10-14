ParsecWebTurn v0.9.3-dev.1 - Connection and input improvements

This native Windows development release improves connection routing, mouse and
keyboard capture, overlay compatibility and streaming stability. Changes below
are compared with v0.9.2-dev.1.

## What's new and improved

- **Force TURN relay:** a new Settings checkbox keeps the next connection on a TURN relay even when a direct route is available, including over a VPN. Conflicting STUN-only settings and missing custom TURN servers are checked before connecting.
- **TURN transport preference:** relay connections prefer UDP, with TLS and then plain TCP available as timed fallbacks when the preferred route has not connected.
- **Working immersive input:** Mouse confines the pointer to the client window, Keyboard forwards reserved shortcuts while the client has focus, and Both combines them. Capture is released on focus loss and disconnect; Windows security shortcuts remain controlled by Windows.
- **Smoother relative mouse control:** host cursor signals now activate native Raw Input correctly, including during desktop drags. Changing direction no longer requires moving back from outside the client. Cursor Warp restores the host cursor position when leaving relative mode and respects its overlay setting.
- **More compatible overlay:** software drawing is now the default for the interface, resolving the reported Intel overlay flicker. Video decoding and presentation retain their GPU path without adding decoded-frame CPU readback.
- **Reliable disconnect:** disconnecting from the overlay returns to the computer list instead of freezing on a black screen. Callbacks from an ended stream cannot reveal its old video surface.
- **Cleaner fullscreen:** the Settings, Connection stats and Help menu disappears in fullscreen and returns in windowed mode.
- **Custom hotkeys:** Parsec hotkey assignments take priority when they conflict with native F8/F11, Settings or Connection stats shortcuts. Hotkey recording and saving remain handled by the original core.
- **Less allocation and lock contention:** audio and video reuse bounded input buffers, and media hand-off holds the shared connection lock for less time.
- **Video recovery:** after a temporary decoder error or input overload, the client can request a fresh keyframe and restart decoding. Recovery has bounded retries and depends on the host supplying the requested keyframe.
- **Low-latency HEVC:** the 8-bit HEVC decoder also requests low-latency operation through Windows transform attributes when its codec API does not accept the existing request. The benefit depends on the decoder and driver.

## Updating

Close the app and replace `ParsecWebTurn.exe` in its existing folder. Keep your
`settings.json`; saved native sign-in and preferences remain in place.

To use a relay over a VPN, configure a TURN provider, enable **Force TURN relay**
in Settings, save and reconnect. Connection settings apply to the next connection.

This is a Windows x64 development release. H.264 remains the default video codec;
8-bit HEVC is available through **Prefer H.265 / HEVC** when supported by the host,
graphics driver and installed Windows decoder.
