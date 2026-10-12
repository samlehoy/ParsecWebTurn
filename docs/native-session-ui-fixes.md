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
Live testing confirmed both the immersive-mode crash fix and fullscreen menu
hiding/restoration.

Disconnect explicitly retires the native video surface and input capture. Video
surface generations reject late ready/finished callbacks from previous sessions;
the decoder/presentation algorithm is unchanged. A disconnected old worker
cannot reveal a black video child above the computer-selection UI, and cannot
hide a newer session. The guest's existing disconnect event drives navigation;
there is no injected click or restart. Live testing showed that this surface-only
fix did not resolve the freeze.

Further inspection of the pinned core found a skipped guest-thread epilogue.
`main_audio_thread` calls `MTY_RunAndYield`, then destroys its audio object and
returns through `__wasi_thread_start_C`. That libc epilogue publishes pthread
completion and wakes joiners. During disconnect the main core clears the audio
loop's run flag and calls `MTY_ThreadDestroy` / `__pthread_join`.
Previously, our `web_run_and_yield` import trapped out of the worker's WASM call
stack, and Rust invoked callbacks separately. When the callback returned zero,
Rust marked its own worker finished, but the guest's audio cleanup and pthread
completion code never resumed. A guest join could therefore wait indefinitely.

Guest workers now execute the callback loop synchronously within the host import
using Wasmtime's supported reentrant calls. Normal callback completion returns
from the import into the original WASM stack so its cleanup and pthread exit
run naturally. The main UI retains its existing input-aware event loop. Worker
cadence, video decoder, presentation queue and mouse handling are unchanged.
The core binary is unchanged: no private pthread layout is written by Rust,
no completion flag is fabricated, and no thread is forcibly terminated.

An isolated shared-memory WASM regression exercises multiple callback ticks,
cleanup after the yield, atomic join notification and three successive workers.
It uses a hidden native window without OpenGL, audio hardware, network or real
profile data. Live disconnect/reconnect still requires testing on a Parsec host.

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
