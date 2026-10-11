ParsecWebTurn v0.9.2-dev.1 — H.265 / HEVC video

This native Windows development release adds H.265 / HEVC video playback.
The changes below are compared with v0.9.1-dev.1.

## What's new

- **Choose HEVC video:** turn on **Prefer H.265 / HEVC** in Settings, save and
  reconnect. The client checks support on your selected graphics adapter before
  requesting HEVC. H.264 remains the default; the host can still choose H.264.
- **Native playback:** 8-bit HEVC uses the Windows decoder and the existing
  D3D11 video path. Video playback has been verified with a Parsec host.
- **See what you are receiving:** Connection stats shows the actual video codec
  and profile, decoder, resolution and graphics adapter.

## Updating

Close the app and replace `ParsecWebTurn.exe` in its existing folder. Keep your
`settings.json`; saved native sign-in and preferences remain in place.

This is a Windows x64 development release. HEVC availability depends on the
host, graphics driver and installed Windows decoder. If an HEVC connection has
video problems, disable the preference and reconnect using H.264.
