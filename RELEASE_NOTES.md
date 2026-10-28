ParsecWebTurn v0.9.5-dev.1 - macOS Apple Silicon preview

This development release adds the first native macOS (Apple Silicon) build of
ParsecWebTurn, built around the original, unmodified Parsec WASM core. Changes
are listed since **v0.9.4**. No Windows package is published for this release;
Windows users should stay on v0.9.4.

## What's new

- **Native macOS client (experimental):** AppKit window, menu and Settings window with Network, TURN, Video, Audio and Input tabs. TURN uses the same STUN URL, Custom/Cloudflare provider and credential validation as Windows. WebRTC remains the default transport.
- **macOS video:** H.264 and HEVC are decoded by VideoToolbox and presented through `AVSampleBufferDisplayLayer`. HEVC is offered only when `VTIsHardwareDecodeSupported` reports support; Main 10 is not offered yet. The Parsec UI overlay is not shown above macOS video yet.
- **macOS audio, profile and input:** CoreAudio output, ChaCha20-Poly1305 profile protection with a locally created key, and a profile lock. Key presses do not beep or type into local responders, Command shortcuts stay local, and settings/alert windows do not feed the remote host.
- **BUD on macOS (experimental):** the BUD transport and USB HID mouse/keyboard options build for macOS. BUD is selected only through its explicit Settings checkbox and never silently replaces WebRTC.
- **App bundle:** an ad-hoc signed `ParsecWebTurn.app`. Settings and the optional event log are stored in `~/Library/Application Support/ParsecWebTurn/` instead of inside the bundle.

## Validation

The package has been built, signature-verified and launched on macOS 27 (Apple
Silicon). This is a development build: expect rough edges, and keep WebRTC, H.264
and the other defaults for the most conservative setup.

## Requirements and installing

Apple Silicon Mac with macOS 11 or later. Intel Macs are not supported by this
package.

Extract `ParsecWebTurn-v0.9.5-dev.1-macos-arm64.zip` and move `ParsecWebTurn.app`
to Applications. The app is not notarized, so the first launch is blocked: open
**System Settings → Privacy & Security** and choose **Open Anyway**, or run
`xattr -dr com.apple.quarantine /Applications/ParsecWebTurn.app`. Verify the
download against `SHA256SUMS-macos.txt` if you wish.

Use the host's **web** launch option, including when BUD is selected in Settings.
USB HID input requires BUD and USB/Yubikey passthrough enabled on the stock host.
