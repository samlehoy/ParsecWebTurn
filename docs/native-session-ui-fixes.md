# Session UI investigation (2026-10-12)

Only dev is changed. Mouse input is deliberately unchanged at the user's request.

Immersive mode reached `env::web_set_kb_grab(1)`, which previously trapped as
unsupported. The native import now posts a focus-scoped keyboard request. A
[documented low-level hook](https://learn.microsoft.com/en-us/windows/win32/winmsg/lowlevelkeyboardproc)
intercepts reserved Windows keys and Alt+Tab / Alt+Escape / Ctrl+Escape while the
main client is focused. Other keys use the existing window event path. The hook
only posts an event; it never executes WASM or waits. Focus loss, disconnect and
shutdown release it. Ctrl+Alt+Delete cannot be captured this way. A failed hook
installation leaves ordinary keyboard handling available rather than stopping
the client.

Fullscreen saves and detaches the Win32 menu as well as saving/restoring window
styles and placement. Its isolated probe checks that the same menu is restored.

Disconnect explicitly retires the native video surface and input capture. Video
surface generations reject late ready/finished callbacks from previous sessions;
the decoder/presentation algorithm is unchanged. A disconnected old worker
cannot reveal a black video child above the computer-selection UI, and cannot
hide a newer session. The guest's existing disconnect event drives navigation;
there is no injected click or restart. This needs live host verification.

The Constant FPS report was subsequently refined: with a stationary picture,
the measured rate reaches the requested value, and it drops during movement.
This is consistent with a working request and a throughput limitation under
load, but does not establish which stage is limiting throughput. Connection
stats displays measured decoder output, not the requested host frame rate.
The pinned core toggles the `fullFPS`
boolean in its per-stream configuration and serializes the array into user-data
sub-ID 11. The Rust adapter forwards that text as private control opcode 17,
with a NUL-terminated payload and sub-ID, matching the pinned JavaScript adapter.
The SDK header describes `fullFPS` as a desktop-host request to **attempt** to
stream at `encoderFPS`; a checked menu item is not confirmation of the received
frame rate. No invented protocol bit, artificial frame duplication, or decoder
queue pacing has been added. There is no confirmed Constant FPS control defect
to fix from this observation alone; its request path is unchanged.
