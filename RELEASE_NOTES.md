ParsecWebTurn v0.9.4-dev.6 - Playback diagnostics and display timing

This native Windows development release adds host/local playback diagnostics,
optional display timing controls, two-way clipboard improvements and experimental
multichannel audio. Changes below are compared with v0.9.4-dev.5.

**Vendor-specific HEVC decoding, Zero copy, 10-bit HEVC, display timing,
multichannel audio and USB HID input remain experimental.** BUD, custom audio
buffering, raw PCM permission and host timing no longer carry the experimental
label. Existing defaults are unchanged: WebRTC, H.264 and automatic playback.
For stability-first use, leave experimental options disabled or stay on the
[stable v0.9.3 release](https://github.com/vampywiz17/ParsecWebTurn/releases/tag/v0.9.3).
H.264 uses Media Foundation on all GPU vendors.

## What's new and improved

- **Two-way text clipboard:** forward local clipboard changes, stream focus restoration and paste-key requests through the original core, including BUD HID keyboard input. Preserve clipboard permissions, avoid echoing host copies, handle unavailable reads and UTF-8 boundaries safely, and keep clipboard contents out of logs. Client-to-host pasting has been confirmed in live use.
- **Host video/audio timing:** automatically request available reports while Connection stats is visible or event logging is enabled. Show latest/average/peak processing durations, video capture and encode segments, frame/block intervals, counts and freshness. Correct the previously swapped host encode and video frame interval labels; no decimal rescaling is applied. These are host durations, not end-to-end or GPU timings. Live report reception is confirmed with BUD; WebRTC live validation and unusual audio interval values remain open.
- **Local video timing:** show compressed-queue wait, decoder setup/API service, correlated input-to-output time, decoded-surface handoff, presentation preparation and Present CPU durations. Collection uses fixed-size metadata, without per-frame histories, file logging or live pixel readback. These overlapping measurements must not be added together as total latency.
- **Display timing (experimental):** add Automatic, Low latency and VSync to Video settings. Automatic remains the default and preserves previous behavior. The explicit modes request a one-presentation DXGI queue with nonblocking readiness checks and compatible fallback. VSync can add display delay. Original-surface Zero copy retains Windows-managed timing. Save and reconnect to apply; live latency gains are not established.
- **BUD burst handling:** continue decoding every compressed reference while omitting obsolete display updates when later input is queued. Normal rendering avoids unnecessary conversion/output writes for those updates; the newest RGB output can retry Present without repeating conversion or adding another surface. Synthetic tests demonstrate reduced display work, not a measured live end-to-end latency improvement. WebRTC transport behavior is unchanged.
- **Memory diagnostics and lifecycle:** distinguish total/private resident memory from private committed memory in Connection stats, and record a few startup snapshots through the existing optional event log. Release the retired video's compressed-buffer reuse pool even when final statistics remain referenced. There is no forced working-set trimming or claimed 40 MB memory target.
- **Multichannel audio (experimental, BUD; synthetic tests only):** optionally request the Windows output's 5.1/7.1 layout and process the actual Opus/PCM channel format with native multichannel playback. Stereo remains the default, and the host can fall back to stereo. Validation covers synthetic codec/layout tests and silent WASAPI output; live multichannel operation, channel ordering and audible surround playback remain unverified. WebRTC retains stereo.
- **Embedded overlay metrics:** supply native decoder input-to-output time and fresh BUD/WebRTC RTT through the original web import. No new GPU timing or pixel copy is introduced. The web interface has no bitrate output, so the embedded Bitrate field remains N/A; native Connection stats keeps its existing traffic counters. Live overlay validation of the new values is still needed.
- **Feature labels and saved settings:** remove the experimental labels from BUD, custom audio buffering, raw PCM permission and host timing in UI, code and current documentation. Existing settings keys are accepted when loading older profiles; other experimental options keep their labels.
- **UI and input polish:** size both statistics columns for wrapped text to prevent overlapping rows, shorten the display catch-up counter label, and hide the local cursor earlier when acquiring native relative input. The intermittent transition flash has not been proven eliminated.
- **Maintenance and reporting:** add concise bug, feature-request and question issue forms; move the active TURN integration fixture under native tests; retire the separate legacy BUD test-file logger and obsolete launcher instructions. Connection failure stages remain in the normal event log. Diagnostic fixtures remain separate from the normal executable.

## Updating

Close the app and extract `ParsecWebTurn-v0.9.4-dev.6-win64.zip`, or replace
`ParsecWebTurn.exe` in its existing folder. Keep your `settings.json`; saved
native sign-in, Parsec preferences and Windows-protected server credentials
remain in place under the same Windows account. Older BUD/audio preference
names are migrated when settings are saved.

Display timing is under **Settings > Video** and applies after reconnecting.
Automatic remains the default. Multichannel audio is under **Settings > Audio**,
disabled by default; select a matching Windows output/host speaker layout and
reconnect. Ordinary stereo playback needs no changes.

The original WASM core and stock host remain unmodified. This is a
**Windows x64 prerelease**, not a replacement for the stable release.
