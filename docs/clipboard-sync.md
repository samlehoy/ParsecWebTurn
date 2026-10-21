# Native clipboard synchronization (2026-10-21)

Scope: Unicode text only, using the original SHA-pinned 150-104a core and the
existing WebRTC/BUD control adapters. No core or host binary is patched, no new
dependency or clipboard setting is added, and clipboard contents are not logged.
Live confirmation of the repaired client-to-host direction is still pending.

## Evidence and missing native event

The unchanged WASM's `main_window_msg_func` handles Matoya event 16 (clipboard).
It checks `client_clipboard` and the stream state, calls `MTY_AppGetClipboard`,
and sends user-data ID 7 through `parsec_web_send_user_data`. Its outgoing text
is bounded to the core's 102400-byte buffer. The incoming clipboard path also
uses the existing local platform import. Synthetic keyboard/hotkey events are
not required to request clipboard synchronization.
The native notification read limits text to 102399 UTF-8 bytes at a character
boundary, avoiding a partial multibyte character in the core's byte truncation.

The native window previously forwarded focus, keys and pointer events, but
never delivered this clipboard-change event. The fix registers a documented
Win32 clipboard format listener, coalesces change notifications into the bounded
native input queue, and supplies the same zero-initialized 216-byte Matoya event
as the pinned core's window shims. The callback index at app+40 and its opaque
argument at app+884 match those shims. This version-bound bridge is covered by
the embedded core hash check; it neither changes the callback table nor patches
core data structures. The event uses guest allocation/free and runs only on the
existing WASM execution worker.

The installed stock host (150-105c, hash recorded in
[frame timing research](frame-timing-diagnostics.md)) also registers the Win32
clipboard listener. Its window procedure handles WM_CLIPBOARDUPDATE at RVA
0xE0830 and emits event 16 at 0xE084F. Its clipboard implementation uses
OpenClipboard/EmptyClipboard/SetClipboardData and sequence-number tracking.
These are static references, never runtime offsets or host modifications.

The lowlat host's `crates/daemon/src/app.rs` identifies clipboard application
messages as ID 7 in both directions and applies host-side permission checks.
The `feat/win-seams` client driver's `send_user_data` uses the same ordinary
application-message framing; the reviewed demo's focus handler only releases
input and does not itself implement clipboard synchronization. No additional
SDK, decoder, session layer or lowlat dependency is imported by this fix.

## Lifecycle and isolation

- Only an online, foreground, visible streaming window can notify the core.
  Native settings, background windows, detached streams and offline probes do
  not read the real clipboard through this path.
- Focus restoration and the first displayed video surface check for a local
  copy made outside the app. Ctrl+V and Shift+Insert notify the core synchronously
  before forwarding the paste key, including BUD USB HID keyboard mode.
- Incoming host copies are written with the native window as clipboard owner;
  their change notification is ignored to avoid an automatic echo loop.
- A failed read during a clipboard-change event returns null to the core, so a
  locked/unavailable clipboard does not turn into an empty clipboard update.
  An actual copied empty string remains valid. Local UI clipboard semantics are
  unchanged outside this event.
- Registration/dispatch failures log only a named event and, where available,
  an OS error code. Text is never logged. Listener removal follows window
  destruction, and notification storage stays bounded.

Automated coverage uses the unchanged core's callback/allocation ABI, a synthetic
desktop for Unicode/empty/unavailable reads, and queue-full/shutdown assertions.
No test opens or replaces a signed-in user's real clipboard. Actual host
permissions and end-to-end pasting remain live-test responsibilities.
