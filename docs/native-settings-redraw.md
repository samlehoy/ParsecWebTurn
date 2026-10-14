# Native Settings transitions

Settings is a GDI child of the main window; the account/computer picker uses the
original core's OpenGL GUI on its own worker. The parent clips its children with
`WS_CLIPCHILDREN`. Suspending `SwapBuffers` based only on the child's returned
HWND left construction/initial paint and destruction outside that suspension.

`settings_active` now covers the whole transition: it is set before creating
the native page and cleared after its controls finish destruction (or creation
fails). OpenGL swaps, software overlay visibility and video-child visibility
use this state. The flag is a presentation gate, not a GPU completion fence;
it does not claim that already submitted driver work has finished.
On its first skipped swap the GL worker posts one native repaint request back
to the UI thread, covering an earlier in-flight swap that could overlap the
initial Settings paint. A late request checks the current page before painting;
it does not draw controls from the GL worker or run every frame.

Back also requests a core GUI redraw using the existing window-size notification
with the unchanged client dimensions. Native invalidation by itself does not
request a new frame from the original GUI worker. The existing input queue wakes
that worker, while `RedrawWindow` invalidates and updates exposed native children.
No mouse event, fake focus change or physical window resize is injected. No new
periodic work, frame readback, decoder or video presentation change is introduced.

The isolated native regression opens and closes Settings three times using its
actual Back command, with a missing synthetic settings path and no account or
stream. It checks destruction, presentation-state restoration, unchanged window
dimensions and the redraw notification without additional user input. This is
not a reproduction of the reported Intel notebook/driver timing; that machine
still needs a live visual retest.

References: [child-window update regions](https://learn.microsoft.com/en-us/windows/win32/gdi/child-window-update-region),
[RedrawWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-redrawwindow).
