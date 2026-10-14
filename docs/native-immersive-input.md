# Native immersive input and cursor warp

The 150-104a core implements `MTY_AppGrabKeyboard` through
`env::web_set_kb_grab`, but its `MTY_AppGrabMouse` and
`MTY_WindowWarpCursor` functions are empty. Accepting the keyboard import alone
fixed the earlier crash without implementing immersive Mouse.

## Core binding

The original overlay and client settings still own the preference. Disassembly
of `main_set_immersive_mode` and the connection-start path establishes the
`client_immersive` values: 0 = Off, 1 = Both, 2 = Keyboard, 3 = Mouse.
Every transition calls the keyboard import, including Mouse and Off with zero.
That boolean cannot distinguish Mouse from Off, and saved configuration alone
cannot represent the overlay's temporary override.

`core_input.rs` reads the effective setting at that boundary, including the
core's configuration-layer precedence. This is an isolated, reverse-engineered
ABI for the exact pinned core, **not a public Parsec SDK API**. The configuration
hash pointer is at 2954756; the `client_immersive` literal is at 2123041.
`MTY_HashGet` establishes the bounded DJB2 bucket lookup. The setting's kind is
1 (integer), and its seven layers have presence bytes at 84 + 24*n and integer
values at 88 + 24*n. `main_set_immersive_mode` selects the highest present layer.
The reader validates the key/type, limits bucket traversal, checks all guest
addresses, and rejects a changing record. It never changes guest memory.

The existing Matoya app pointer supplies detach state at +48. `MTY_AppCreate`
stores its main-app opaque argument at +884; the main-app backlink at +9376 is
validated before reading the overlay's Disable Cursor Warp byte at +9464.
These small state checks run after regular input callbacks, so the core's detach
and warp actions reach Win32 even though their original setters have no import.
Effective configuration is read after the regular main input callback, alongside
detach/warp state, and on pointer-lock transitions. It is not cached in a WASM
host instance: the GUI worker has its own instance and may execute the keyboard
import without any app pointer. The live capture confirmed that a GUI update
previously left the main instance's cached mode at Off while shared configuration
was Both. Mouse input exports keep their immediate path; UI-thread messages are
posted only when policy changes.

`build.rs` already rejects any core with a different SHA-256. An offline test
initializes the actual pinned core and verifies this configuration schema.
Updating the core requires re-auditing this adapter; unavailable or invalid
state releases capture rather than guessing. No WASM code, imports, protocol
bits or configuration values are patched.

## Windows behavior

- Mouse confines the visible absolute cursor with `ClipCursor`. It does not
  enable relative movement or hide the cursor.
- Keyboard uses a focus-scoped low-level keyboard hook for reserved Windows
  shortcuts. Alt+Tab, Alt+Escape, Alt+F4, Ctrl+Escape / Ctrl+Shift+Escape and the
  Windows keys go through the existing native input queue. Event modifiers and
  intercepted key releases are preserved. Ordinary text retains the normal
  Windows keyboard-layout path. Secure OS shortcuts remain OS-controlled.
- Both combines these policies. Off releases them; an independent host request
  for relative input (including desktop drags) can still require pointer lock.
- Native relative intent is `cursor.relative || cursor.hidden`. The browser
  core omits the hidden branch because `MTY_AppCanWarpCursor` is false. The Rust
  adapter reads the already-decoded host cursor event and applies the native
  rule without changing the WASM. A later browser pointer-lock callback cannot
  cancel a known hidden-only native request. Clearing both host flags releases
  it, including while unfocused. Button state alone never selects relative mode.
- Relative mouse uses Raw Input deltas with one-pixel cursor confinement,
  matching native Matoya's capture strategy; absolute WM_MOUSEMOVE events are not
  also forwarded. Requested and acquired states are separate, so temporary
  focus/visibility suspension does not lose the request. The existing exported
  Matoya update reports actual acquisition.
- Disable Cursor Warp suppresses return-position warps only. It does not disable
  relative capture or independently selected immersive Mouse capture. The pinned
  main cursor-event handler reads this option only before its return-position
  warp, not before acquiring relative input.
- When the host leaves relative mode, its absolute cursor position is mapped
  through the video letterbox and restored with `SetCursorPos`. Ordinary remote
  cursor updates do not chase the local pointer. The matching synthetic move
  is not sent back to the host.
- Focus loss, minimize, native menus/settings, detach, hiding video with F8,
  disconnect and shutdown release capture. Returning to an active video window
  restores an applicable request. F8/F11 remain the existing local controls.

The core's configurable immersive toggle (default Ctrl+Shift+I) remains on its
normal input path. See [Parsec's immersive-mode documentation](https://support.parsec.app/hc/en-us/articles/32361385571860-Immersive-Mode-Setting).
The Windows implementation follows [ClipCursor ownership/release rules](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-clipcursor)
and [low-level hook requirements](https://learn.microsoft.com/en-us/windows/win32/winmsg/lowlevelkeyboardproc).

An isolated foreground-window test exercises Mouse/Keyboard/Both/Off, the real
hook-to-window key path, the cursor wire decoder and all native host-flag
combinations, one-pixel confinement, immediate opposite Raw Input deltas and
their relative control packets, enabled/disabled return-position warping, detach,
minimize/restore and disconnect. It runs separately from other window tests and
restores the previous desktop cursor confinement/position. No account, real
profile, network, decoder or administrator access is involved. A live host test
is still needed to confirm the complete remote interaction on the user's GPUs.
Video decoding, rendering, media queues and mouse coalescing are unchanged.

## Source evidence and limits (2026-10-12)

- [Official immersive documentation](https://support.parsec.app/hc/en-us/articles/32361385571860-Immersive-Mode-Setting)
  and [configuration values](https://support.parsec.app/hc/en-us/articles/32381443626516-All-Advanced-Configuration-Options)
  distinguish continuous Mouse/Both confinement from Keyboard and Off.
- [Archived SDK client example, `cursor_func`](https://github.com/mbroemme/parsec-sdk/blob/14222ac25be3bc28c49b4858213a737642031513/examples/client/main.c#L58)
  combines the two flags and restores the cursor on leaving relative mode.
  The [SDK header](https://github.com/mbroemme/parsec-sdk/blob/14222ac25be3bc28c49b4858213a737642031513/sdk/parsec.h)
  documents return coordinates and signed relative motion. This is an archived
  mirror, not proof of every detail in the current closed-source application.
- [lowlat host cursor source](https://github.com/nomi-san/lowlat/blob/7b4dc4ae79990ea8c59a108c80b3c89b695c3cb4/crates/host/src/cursor.rs)
  corroborates the hidden-only capture requirement. Its
  [cursor flags](https://github.com/nomi-san/lowlat/blob/7b4dc4ae79990ea8c59a108c80b3c89b695c3cb4/crates/core/src/cursor.rs)
  distinguish relative, hidden and touch suppression; suppression must not be
  treated as relative input. lowlat is a host, not a replacement client.
- [Matoya's Windows implementation](https://github.com/snowcone-ltd/libmatoya/blob/952f2a22bdb4cf832db110c00aa3e57952358eae/src/windows/appw.c)
  separates immersive grab from relative state, confines relative input to one
  pixel and consumes Raw Input. Our retained legacy button-message path differs
  from its NOLEGACY/raw-button path, but absolute motion is suppressed while
  relative capture is acquired. No source implementation was copied.

The earlier button-driven drag-confinement experiment was removed: the audited
native policy uses host cursor intent, not a guess based on any button press.
Automated tests establish the adapter's behavior for the supplied events; they
cannot prove what a particular live host emits or how input feels on each GPU.
