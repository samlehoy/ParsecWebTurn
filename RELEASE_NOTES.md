ParsecWebTurn v0.9.4-dev.7 - Input and low-latency presentation fixes

This native Windows development release improves HID dragging and cursor
transitions, removes avoidable BUD output waits, and fixes reported stutter in
the normal DXGI Low latency path. Changes below are compared with v0.9.4-dev.6.

**Vendor-specific HEVC decoding, Zero copy, 10-bit HEVC, display timing,
multichannel audio and USB HID input remain experimental.** Defaults remain
WebRTC, H.264 and Automatic display timing. BUD, custom audio buffering, raw PCM
permission and host timing keep their existing status. For stability-first use,
leave experimental options disabled or stay on the
[stable v0.9.3 release](https://github.com/vampywiz17/ParsecWebTurn/releases/tag/v0.9.3).
H.264 continues to use Media Foundation on all GPU vendors.

## What's new and improved

- **HID dragging and cursor restoration:** restore the local pointer to the host-reported endpoint when leaving relative or hidden-cursor capture. This prevents the old capture anchor from being sent back as the next absolute HID position after a drag. Native relative camera input, focus handling and local menu controls retain their existing behavior.
- **Cursor transition visibility:** keep the pointer hidden until its return position is restored, and replace an active cursor image before releasing its old handle. For BUD, validated host capture-state changes now wake the existing input callback promptly, preserving control-message order and avoiding a redundant follow-up UI tick. Live testing reported substantially reduced cursor flicker; occasional delay before the host cursor response arrives can remain.
- **BUD control and HID output:** submit newly queued HID reports and control replies through the existing carrier before the next network wait. Reuse UDP batching and TCP/TLS backpressure, preserve message ordering, and keep ACK/retransmission timers unchanged. This removes an avoidable local wait; it is not a claim of a fixed end-to-end latency reduction.
- **Low latency display timing:** use the documented DXGI readiness event together with an interruptible input-queue wake when a prepared picture is pending and no compressed input is available. Feed fresh input before retrying a remaining picture, preserve BUD burst catch-up, and retain the one-presentation limit and nonblocking Present behavior. No extra decoded-frame copy, RGB surface or rendering thread is added. Live testing confirmed that the reported stutter disappeared on the tested 60 FPS / 59.95 Hz DXGI setup. Automatic, VSync and original-surface Zero copy retain their presentation policies; universal smoothness or a measured scanout-latency gain is not established.
- **Presentation cadence diagnostics:** add **Present submission interval** to Connection stats, showing latest/average/peak time between accepted DXGI Present calls. This measures CPU-observed submission cadence, includes idle periods, and is not a GPU execution or physical display timing measurement.
- **Settings and documentation:** give Network, Video, Audio and Input equal-width tabs with consistent spacing, and separate them more clearly from Back and Save settings. Shorten the README and bring it up to date with the current native client and validation limits.
- **Maintenance:** remove the temporary mouse/cursor trace capture and its build switches from the merged client. Keep targeted regression tests for native capture, callback scheduling, interruptible display waits, output ordering and send accounting. No new runtime dependency or continuous input logging is introduced.

## Validation and next stable release

Extensive **Intel, AMD and NVIDIA** testing starts now, covering normal GPU
rendering, **Zero copy**, and **H.265/HEVC, including 10-bit**, across supported
combinations. These are the next validation targets, not a claim that every GPU
and mode has already passed. Confirmed 10-bit host encoding currently remains
limited to the tested NVIDIA setup; host capability can still cause an 8-bit
fallback. Multichannel audio remains supported by synthetic tests only.

The next stable release is currently targeted for **the second half of next
week (October 29-November 1, 2026)**. It will be published after the supported
combinations prove stable and any blocking issues are resolved; the schedule
depends on those results.

## Updating

Close the app and extract `ParsecWebTurn-v0.9.4-dev.7-win64.zip`, or replace
`ParsecWebTurn.exe` in its existing folder. Keep your `settings.json`; native
sign-in, Parsec preferences and Windows-protected server credentials remain in
place under the same Windows account. No helper executable or tracing launcher
is needed.

Display timing is under **Settings > Video**. Save and reconnect to apply;
Automatic remains the default. USB HID input remains under **Settings > Input**,
requires BUD and the stock host's USB/Yubikey passthrough option.

The original WASM core and stock host remain unmodified. This is a
**Windows x64 prerelease**, not a replacement for the stable release.
