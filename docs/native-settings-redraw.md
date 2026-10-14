# Native Settings transitions and scrolling

Settings is a GDI child of the main window; the account/computer picker uses the
original core's OpenGL GUI on its own worker. `settings_active` covers the entire
native page lifetime, including creation and destruction. The existing GL swap
gate, video-child visibility and software overlay use this state. The GL worker
also posts one native repaint when it observes coverage, after its previous
GL callback has returned. This covers overlap with an earlier in-flight swap
even when the idle core skips further presentation. This is not a GPU completion fence.

## Idle core GUI exposure

Inspection of the pinned, unchanged core's `main_render_loop` shows that the
disconnected GUI can skip `MTY_WindowDrawUI`/`MTY_WindowPresent` when its content
has not changed. A window-size event with unchanged dimensions does not reliably
force presentation. The earlier native fixture tested the notification only;
the user confirmed that Back still needed further interaction on the idle picker.

Settings coverage is now forwarded through the existing exported
`mty_window_update_visibility`. Only the graphics-owning guest worker consumes
these notifications before its render callback; audio and other worker loops
do not consume them. The app handle is shared from the existing `web_set_app`
import, rather than discovered through a private memory offset.

The bounded, coalesced state preserves a hidden callback followed by a visible
callback even for a quick open/Back. This matters because the core tracks its
previous visibility and redraws on exposure. Ordinary idle behavior resumes
afterward; no permanent forced-render loop is added. The guest's existing clock
waits and WASI deadline semantics remain unchanged, so a render callback already
in an idle wait may finish that wait before observing the transition.

No resize or artificial mouse/keyboard event is injected for exposure. Native
`RedrawWindow` invalidation remains for exposed native regions. The video decoder,
GPU presentation path and TURN configuration are unchanged.

## Native page painting

`WS_EX_COMPOSITED` buffers the Settings page and its descendants together. It is
applied only to the GDI page class, which has none of `CS_OWNDC`, `CS_CLASSDC` or
`CS_PARENTDC`; it is not applied to the OpenGL parent or video surface. This
avoids showing separate background erasure and child-label painting during scroll.

Wheel, scrollbar and focus scrolling clamp the requested position before layout.
An unchanged position performs no layout, control movement or forced repaint.
Resizing and provider changes still perform their required complete reflow.

## Verification

- Coalesced visibility tests cover fast repeated open/Back, initialization and
  both visibility phases without an unbounded event history.
- A native page subclass counts actual `WM_PAINT` messages: repeated wheel,
  line/page and end-scroll commands at both boundaries must produce none, while
  a real position change must repaint. Existing incremental-paint pixel checks
  still compare scrolling/reflow against a clean repaint.
- An original-core integration test runs the embedded AOT module with networking
  disabled and an anonymous in-memory filesystem. Three Settings/Back cycles must
  produce actual successful GL presentations, without additional input or a
  window-size change. The fixture never reads a production profile or credentials.

These fixtures do not replace a visual retest on the reported Intel notebook.
The original-core GUI test is explicitly ignored on a headless runner without
accelerated OpenGL. Manually dispatch native validation with `gui_harness=true`
to download its separate test EXE, then run the exact command in its README on
a suitable desktop. This harness is never included in the normal client package.

References: [child-window update regions](https://learn.microsoft.com/en-us/windows/win32/gdi/child-window-update-region),
[RedrawWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-redrawwindow),
[documented composited style and class restrictions](https://learn.microsoft.com/en-us/windows/win32/winmsg/extended-window-styles).
