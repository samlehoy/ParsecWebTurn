ParsecWebTurn v0.9.4-dev.5 - Experimental USB HID input

This native Windows development release adds optional USB HID mouse and
keyboard forwarding through the unmodified stock host's USB/Yubikey path.
Changes below are compared with v0.9.4-dev.4.

**USB HID input, BUD, vendor-specific HEVC decoding, Zero copy, 10-bit HEVC
and the Audio options remain experimental.** Compatibility and stability depend
on the host, application, GPU, driver and network. For stability-first use,
choose **WebRTC with H.264**, leave experimental options disabled, or stay on
the [stable v0.9.3 release](https://github.com/vampywiz17/ParsecWebTurn/releases/tag/v0.9.3).
H.264 remains the default codec and uses Media Foundation on all GPU vendors.

## What's new and improved

- **Experimental USB HID mouse:** use the stock host's USB/Yubikey passthrough with BUD for absolute desktop positioning, native relative camera movement, five buttons, dragging and vertical/horizontal scrolling. Relative capture follows the host cursor state and hides the local pointer. One physical or virtual display is supported.
- **Experimental USB HID keyboard:** forward physical key presses, releases and modifiers through the same host USB path. Matching active layouts, including Hungarian on both computers, are supported; the host layout is not changed automatically. Simultaneous keys use a bitmap without a six-key array limit.
- **Simultaneous HID input:** mouse and keyboard share one USB device lifecycle and bounded transport queues, with separate reports and fair scheduling. Buttons keep the same owner across desktop/camera mode changes. Ordinary Parsec forwarding is disabled independently for each selected HID input, preventing duplicate input.
- **New Input settings tab:** select HID mouse and keyboard independently, save the preferences and reconnect. Both options require BUD and are disabled by default. Local menus remain usable; focus loss and disconnect release held input and capture. Only the usual EXE and settings.json are needed, without a host modification or additional launcher.
- **HID operational logging:** the existing optional client-events.jsonl records activation, state changes and setup/transfer/queue failures, without continuous mouse movement, coordinates, clicks, key history, typed text or USB payloads.

## Updating

Close the app and extract `ParsecWebTurn-v0.9.4-dev.5-win64.zip`, or replace
`ParsecWebTurn.exe` in its existing folder. Keep your `settings.json`; saved
native sign-in, Parsec preferences and Windows-protected server credentials
remain in place under the same Windows account.

To use HID input, enable **USB/Yubikey passthrough on the host**, select BUD
in **Settings > Network**, then enable either or both options in
**Settings > Input**. Save and reconnect. Select the same active keyboard
layout on host and client. Single physical and virtual displays are supported;
multi-monitor mapping and automatic layout synchronization are not provided.
Application/game compatibility is not guaranteed.

WebRTC remains the default and uses ordinary Parsec input. The original WASM
core, host binary and GPU decoding/presentation paths are unchanged by this
input integration.

This is a **Windows x64 prerelease**, not a replacement for the stable release.
