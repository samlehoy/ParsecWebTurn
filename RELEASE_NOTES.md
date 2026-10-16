ParsecWebTurn v0.9.4-dev.2 - Color range fixes and event logging

This native Windows development release corrects video color range, adds optional
operational logging and introduces an opt-in Main 10 HEVC request and GPU playback
path. Changes below are compared with v0.9.4-dev.1.

**BUD, vendor-specific HEVC decoding, Zero copy and 10-bit HEVC are experimental.**
Compatibility and stability depend on the GPU, driver, host and network. For
stability-first use, choose **WebRTC with H.264** and leave experimental options
disabled, or stay on the
[stable v0.9.3 release](https://github.com/vampywiz17/ParsecWebTurn/releases/tag/v0.9.3).
H.264 remains the default codec and uses Media Foundation on all GPU vendors.

## What's new and improved

- **Correct video black and white levels:** translate limited/full-range metadata correctly between Media Foundation and D3D11, including vendor HEVC output. This fixes lifted blacks and grey-looking whites in normal GPU rendering. Full-range support is declared consistently at startup and in later encoder requests.
- **Experimental 10-bit HEVC:** an optional Video setting requests Main 10 with WebRTC or BUD when the selected GPU exposes the required P010 decode/conversion capabilities. Later encoder and keyframe requests preserve the selected flags. Main 10 uses the existing GPU Media Foundation path and conversion pass; RGB display output remains 8-bit, and original-surface Zero copy remains 8-bit only. Host and driver support are required: the request does not guarantee a 10-bit stream. Connection stats shows the actual decoded color depth. 4:4:4 remains unsupported.
- **Optional operational event log:** enable event logging in the Network tab to record connection and route changes, transport failures, decoder recovery/errors and profile-save failures beside settings.json. UTF-8 JSON Lines records have UTC timestamps; background file writes use a bounded queue and approximately 5 MiB rotation with one previous file. Credentials and media payloads are excluded. Logging is disabled by default and applies immediately after saving.
- **Readable settings and statistics:** fix clipped event-log help in Settings and size the Connection stats explanation to its wrapped text, including the longer BUD description and narrower windows.
- **Clearer core provenance:** README documentation explains that the embedded proprietary Parsec WASM core remains unmodified and that Rust supplies the native framework rather than a rewrite of that core.

## Updating

Close the app and extract `ParsecWebTurn-v0.9.4-dev.2-win64.zip`, or replace
`ParsecWebTurn.exe` in its existing folder. Keep your `settings.json`; saved
native sign-in, Parsec preferences and Windows-protected server credentials
remain in place under the same Windows account.

Open **Settings > Network > Enable event logging** to create `client-events.jsonl`
beside `settings.json`. Saving applies this option immediately. The previous
rotated file is `client-events.previous.jsonl`.

Open **Settings > Video** to enable Prefer H.265 / HEVC and, optionally,
10-bit HEVC. Save and reconnect after changing video settings. Check
**Connection stats > Decoded color depth** for the received output rather than
assuming the host accepted the request. Main 10 is not an HDR or end-to-end
10-bit display mode; Zero copy uses normal GPU rendering when the received
stream is Main 10.

This is a **Windows x64 prerelease**, not a replacement for the stable release.
WebRTC remains the default and supports TURN over UDP, TCP and TLS; experimental
BUD supports UDP only. If experimental video has problems, disable 10-bit HEVC,
Prefer H.265 / HEVC and Zero copy, then reconnect using H.264.
