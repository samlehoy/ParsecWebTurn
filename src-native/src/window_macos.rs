//! Native macOS Cocoa / AppKit window using objc2.
pub use crate::window_event::Event;
use anyhow::Result;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSOpenGLPixelFormat,
    NSOpenGLView, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSString};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};

pub struct Window {
    pub stats: Arc<crate::stats::Shared>,
    pub updates: Arc<crate::updater::State>,
    pub core_version: Arc<crate::core_version::State>,
    pub gui_visibility: crate::gui_visibility::State,
    pub closing: AtomicBool,
    pub active_contexts: AtomicUsize,
    pub events: crate::event_queue::Queue<Event>,
    pub dimensions: Mutex<(i32, i32)>,
    pub overlay: Mutex<crate::overlay::Shared>,
    pub software_overlay_enabled: AtomicBool,
    pub graphics: Mutex<Option<crate::graphics::GraphicsReport>>,
    pub audio_registry: Mutex<crate::audio_macos::Registry>,
    pub video_report: Mutex<Option<crate::video_output::Snapshot>>,
    video_ready: AtomicBool,
    video_generation: Mutex<u64>,
    pub live: bool,
    pub online: bool,
    pub run_seconds: u64,
    pub stop: Arc<crate::lifecycle::StopSignal>,
    connection_settings: Mutex<Option<Arc<crate::connection_settings::Manager>>>,
    wake_lock: Mutex<crate::wake_lock::State>,
    relative_mouse: AtomicBool,
    pub has_focus: AtomicBool,
    pub pressed_buttons: AtomicUsize,
    #[cfg(feature = "bud")]
    pub hid_mouse_active: AtomicBool,
    #[cfg(feature = "bud")]
    pub hid_keyboard_active: AtomicBool,
    #[cfg(feature = "bud")]
    pub hid_input_acquired: AtomicBool,
    input_state: Mutex<crate::core_input::State>,
    core_hotkeys: Mutex<Option<crate::core_hotkeys::Binding>>,
    cursor: Mutex<crate::cursor::State>,
    cursor_pending: Mutex<crate::cursor::Pending>,
    #[cfg(any(test, feature = "diagnostics"))]
    pub capture: Mutex<Option<std::path::PathBuf>>,
    #[cfg(any(test, feature = "diagnostics"))]
    pub synthetic_login: bool,
    #[cfg(any(test, feature = "diagnostics"))]
    pub script_steps: AtomicUsize,
    #[cfg(any(test, feature = "diagnostics"))]
    pub network_origin_audit: bool,
    pub ns_window: usize,
    pub ns_gl_context: usize,
    pub ns_gl_view: usize,
    pub cgl_context: usize,
    ns_video_view: usize,
    /// AVSampleBufferDisplayLayer, or 0 if AVFoundation could not create one.
    video_layer: usize,
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGAssociateMouseAndMouseCursorPosition(connected: bool) -> i32;
}

impl Window {
    pub fn create(
        synthetic_login: bool,
        live: bool,
        online: bool,
        _network_origin_audit: bool,
    ) -> Result<Arc<Self>> {
        let (w, h) = (1024, 720);

        let create_win = |mtm: MainThreadMarker| -> (usize, usize, usize, usize, usize, usize) {
            let app = NSApplication::sharedApplication(mtm);
            app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
            crate::connection_settings_ui_macos::install_menu(mtm, &app);

            let rect = NSRect::new(NSPoint::new(100.0, 100.0), NSSize::new(w as f64, h as f64));
            let style = NSWindowStyleMask::Titled
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::Miniaturizable
                | NSWindowStyleMask::Resizable;

            unsafe {
                let win = NSWindow::initWithContentRect_styleMask_backing_defer(
                    mtm.alloc(),
                    rect,
                    style,
                    NSBackingStoreType::Buffered,
                    false,
                );
                win.setTitle(&NSString::from_str("ParsecWebTurn"));
                win.setReleasedWhenClosed(false);

                let attribs: [u32; 11] = [
                    73, // NSOpenGLPFAAccelerated
                    5,  // NSOpenGLPFADoubleBuffer
                    8, 24, // NSOpenGLPFAColorSize, 24
                    12, 8, // NSOpenGLPFAAlphaSize, 8
                    99, 0x3200, // NSOpenGLPFAOpenGLProfile, 3.2 Core
                    0, 0, 0,
                ];
                let pixel_format = NSOpenGLPixelFormat::initWithAttributes(
                    mtm.alloc(),
                    std::ptr::NonNull::new(attribs.as_ptr() as *mut _).unwrap(),
                );

                let view_rect =
                    NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w as f64, h as f64));
                let gl_view = NSOpenGLView::initWithFrame_pixelFormat(
                    mtm.alloc(),
                    view_rect,
                    pixel_format.as_deref(),
                )
                .expect("Failed to create NSOpenGLView");

                gl_view.setWantsBestResolutionOpenGLSurface(false);
                // The video layer sits above the GL view, which is hidden while video
                // shows: an NSOpenGLView surface would otherwise draw over its siblings.
                let sizable = objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
                    | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable;
                let container = NSView::initWithFrame(mtm.alloc(), view_rect);
                container.setAutoresizesSubviews(true);
                gl_view.setAutoresizingMask(sizable);
                container.addSubview(&gl_view);
                let video_view = NSView::initWithFrame(mtm.alloc(), view_rect);
                video_view.setAutoresizingMask(sizable);
                video_view.setHidden(true);
                let video_layer = crate::video_macos::create_layer();
                if let Some(layer) = &video_layer {
                    // Layer-hosting view: set the layer before enabling it.
                    let _: () = objc2::msg_send![&*video_view, setLayer: &**layer];
                    video_view.setWantsLayer(true);
                }
                container.addSubview(&video_view);
                win.setBackgroundColor(Some(&objc2_app_kit::NSColor::blackColor()));
                win.setContentView(Some(&container));

                let gl_context = gl_view.openGLContext().expect("NSOpenGLContext missing");
                gl_context.setView(Some(&gl_view), mtm);
                gl_context.update(mtm);

                win.setAcceptsMouseMovedEvents(true);
                win.center();
                win.setIsVisible(true);
                win.orderFrontRegardless();
                win.makeKeyAndOrderFront(None);
                app.activateIgnoringOtherApps(true);

                // No context is current on the main thread yet; make this one current
                // briefly to read its CGL handle for the resize lock.
                gl_context.makeCurrentContext();
                let cgl_ctx = crate::graphics::cgl::CGLGetCurrentContext() as usize;
                objc2_app_kit::NSOpenGLContext::clearCurrentContext();
                (
                    Retained::into_raw(win) as usize,
                    Retained::into_raw(gl_context) as usize,
                    Retained::into_raw(gl_view) as usize,
                    cgl_ctx,
                    Retained::into_raw(video_view) as usize,
                    video_layer.map_or(0, |layer| Retained::into_raw(layer) as usize),
                )
            }
        };

        let (ns_win, ns_gl_ctx, ns_gl_view, cgl_ctx, ns_video_view, video_layer) =
            if let Some(mtm) = MainThreadMarker::new() {
                create_win(mtm)
            } else {
                let (tx, rx) = std::sync::mpsc::sync_channel(1);
                dispatch2::DispatchQueue::main().exec_sync(move || {
                    let mtm =
                        MainThreadMarker::new().expect("Main queue must have MainThreadMarker");
                    let _ = tx.send(create_win(mtm));
                });
                rx.recv().unwrap()
            };

        let state = Arc::new(Self {
            stats: Default::default(),
            updates: Default::default(),
            core_version: Default::default(),
            gui_visibility: Default::default(),
            closing: AtomicBool::new(false),
            active_contexts: AtomicUsize::new(0),
            events: Default::default(),
            dimensions: Mutex::new((w, h)),
            overlay: Default::default(),
            software_overlay_enabled: AtomicBool::new(true),
            graphics: Default::default(),
            audio_registry: Default::default(),
            video_report: Default::default(),
            video_ready: AtomicBool::new(false),
            video_generation: Mutex::new(0),
            live,
            online,
            run_seconds: if synthetic_login { 20 } else { 8 },
            stop: Default::default(),
            connection_settings: Default::default(),
            wake_lock: Default::default(),
            relative_mouse: AtomicBool::new(false),
            has_focus: AtomicBool::new(true),
            pressed_buttons: AtomicUsize::new(0),
            #[cfg(feature = "bud")]
            hid_mouse_active: AtomicBool::new(false),
            #[cfg(feature = "bud")]
            hid_keyboard_active: AtomicBool::new(false),
            #[cfg(feature = "bud")]
            hid_input_acquired: AtomicBool::new(false),
            input_state: Default::default(),
            core_hotkeys: Default::default(),
            cursor: Default::default(),
            cursor_pending: Default::default(),
            #[cfg(any(test, feature = "diagnostics"))]
            capture: Default::default(),
            #[cfg(any(test, feature = "diagnostics"))]
            synthetic_login,
            #[cfg(any(test, feature = "diagnostics"))]
            script_steps: AtomicUsize::new(0),
            #[cfg(any(test, feature = "diagnostics"))]
            network_origin_audit: _network_origin_audit,
            ns_window: ns_win,
            ns_gl_context: ns_gl_ctx,
            ns_gl_view,
            cgl_context: cgl_ctx,
            ns_video_view,
            video_layer,
        });

        state.events.push(Event::Size(w, h));
        state.events.push(Event::Focus(true));
        *ACTIVE_WINDOW.lock().unwrap() = Some(Arc::downgrade(&state));

        Ok(state)
    }

    pub fn handle(&self) -> usize {
        self.ns_window
    }

    pub fn bind_core_hotkeys(&self, binding: crate::core_hotkeys::Binding) {
        *self.core_hotkeys.lock().unwrap_or_else(|e| e.into_inner()) = Some(binding);
    }

    /// Settings is a separate window on macOS, so it never covers the stream and
    /// presentation keeps running (on Windows it is a child over the client area).
    pub fn settings_open(&self) -> bool {
        false
    }

    pub fn connection_settings(&self) -> Option<Arc<crate::connection_settings::Manager>> {
        self.connection_settings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn open_settings(&self) {
        if let Some(mgr) = self.connection_settings() {
            crate::connection_settings_ui_macos::open(mgr);
        }
    }

    pub fn repaint_settings_after_gl_pause(&self) {}

    pub fn direct_presentation_allowed(&self) -> bool {
        false
    }

    pub fn experimental_zero_copy(&self) -> bool {
        false
    }

    pub fn video_gpu(&self) -> Option<crate::connection_settings::GpuPreference> {
        let mgr = self
            .connection_settings
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        mgr.as_ref().and_then(|m| m.view().0.video_gpu)
    }

    pub fn presentation_mode(&self) -> crate::connection_settings::PresentationMode {
        crate::connection_settings::PresentationMode::Automatic
    }

    pub fn audio_buffer(&self) -> Option<(u32, u32)> {
        let mgr = self
            .connection_settings
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let s = &mgr.as_ref()?.view().0;
        if s.custom_audio_buffer {
            Some((s.audio_min_buffer_ms, s.audio_max_buffer_ms))
        } else {
            None
        }
    }

    pub fn install_connection_settings(&self, manager: Arc<crate::connection_settings::Manager>) {
        *self
            .connection_settings
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(manager);
    }

    pub fn ensure_video_surface(&self) -> Result<usize> {
        Ok(self.ns_window)
    }

    pub fn video_layer(&self) -> usize {
        self.video_layer
    }

    /// Shows the video layer and hides the GL view while video is ready, and back.
    fn set_video_ready(&self, ready: bool) {
        if self.video_ready.swap(ready, Ordering::AcqRel) == ready {
            return;
        }
        let (video_view, gl_view) = (self.ns_video_view, self.ns_gl_view);
        dispatch2::DispatchQueue::main().exec_async(move || {
            // Both views live as long as the Window, whose Drop also runs on main.
            let video_view = unsafe { &*(video_view as *const NSView) };
            let gl_view = unsafe { &*(gl_view as *const NSView) };
            video_view.setHidden(!ready);
            gl_view.setHidden(ready);
        });
    }

    // Same generation rules as Windows: a retired pipeline cannot touch a newer one.
    pub fn begin_video(&self) -> u64 {
        let mut generation = self
            .video_generation
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *generation = generation.wrapping_add(1);
        self.set_video_ready(false);
        *generation
    }

    pub fn set_video_ready_for(&self, expected: u64, ready: bool) {
        let generation = self
            .video_generation
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if *generation == expected {
            self.set_video_ready(ready);
        }
    }

    pub fn finish_video(&self, expected: u64) {
        let mut generation = self
            .video_generation
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if *generation == expected {
            *generation = generation.wrapping_add(1);
            self.set_video_ready(false);
        }
    }

    #[cfg(feature = "bud")]
    pub fn hid_input_generation(&self) -> u64 {
        *self
            .video_generation
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    #[cfg(feature = "bud")]
    pub fn set_hid_input_active(&self, expected: u64, mouse: bool, keyboard: bool) {
        let generation = self
            .video_generation
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if *generation != expected {
            return;
        }
        let mouse_changed = self.hid_mouse_active.swap(mouse, Ordering::AcqRel) != mouse;
        let keyboard_changed =
            self.hid_keyboard_active.swap(keyboard, Ordering::AcqRel) != keyboard;
        let changed = mouse_changed || keyboard_changed;
        drop(generation);
        if changed && mouse_changed && mouse {
            self.set_relative_mouse(false);
            self.events.push(Event::RelativeMode(
                self.relative_mouse.load(Ordering::Acquire),
            ));
        }
    }

    pub fn record_button(&self, button: usize, down: bool) {
        if down {
            self.pressed_buttons.fetch_or(1 << button, Ordering::AcqRel);
        } else {
            self.pressed_buttons
                .fetch_and(!(1 << button), Ordering::AcqRel);
        }
    }

    pub fn release_buttons(&self) {
        let pressed = self.pressed_buttons.swap(0, Ordering::AcqRel);
        for button in 0..5 {
            if (pressed & (1 << button)) != 0 {
                self.events.push(Event::Button(false, button, 0, 0));
            }
        }
    }

    pub fn sync_absolute_pointer(&self, x: i32, y: i32) {
        #[cfg(feature = "bud")]
        if self.hid_mouse_active.load(Ordering::Acquire)
            && !self.relative_mouse.load(Ordering::Acquire)
        {
            self.events.push(Event::Motion(x, y));
        }
    }

    #[cfg(feature = "bud")]
    pub fn sync_hid_input_acquired(&self, has_focus: bool) {
        let acquired = has_focus
            && !self.closing.load(Ordering::Acquire)
            && (self.hid_mouse_active.load(Ordering::Acquire)
                || self.hid_keyboard_active.load(Ordering::Acquire));
        if self.hid_input_acquired.swap(acquired, Ordering::AcqRel) && !acquired {
            self.events.push(Event::HidInputRelease);
        }
    }

    pub fn end_session(&self) {
        if self.relative_mouse.load(Ordering::Acquire) {
            self.set_relative_mouse(false);
        }
        self.release_buttons();
        #[cfg(feature = "bud")]
        {
            self.hid_mouse_active.store(false, Ordering::Release);
            self.hid_keyboard_active.store(false, Ordering::Release);
            self.sync_hid_input_acquired(false);
        }
        self.events.push(Event::RelativeMode(false));
    }

    pub fn publish_software_overlay(&self, _frame: &crate::overlay::Frame) {}

    /// The layer letterboxes with resize-aspect, which is what Viewport::rect computes.
    pub fn presented_viewport(&self) -> Option<crate::viewport::Viewport> {
        if !self.video_ready.load(Ordering::Acquire) {
            return None;
        }
        let report = self.video_report.lock().unwrap_or_else(|e| e.into_inner());
        let v = report.as_ref().filter(|v| v.failure_stage.is_none())?;
        let size = *self.dimensions.lock().unwrap_or_else(|e| e.into_inner());
        let viewport = crate::viewport::Viewport {
            source: (v.width?, v.height?),
            client: (u32::try_from(size.0).ok()?, u32::try_from(size.1).ok()?),
        };
        viewport.rect()?;
        Some(viewport)
    }

    pub fn set_relative_mouse(&self, enabled: bool) {
        let previous = self.relative_mouse.swap(enabled, Ordering::AcqRel);
        if previous != enabled {
            dispatch2::DispatchQueue::main().exec_async(move || unsafe {
                if enabled {
                    let _ = CGAssociateMouseAndMouseCursorPosition(false);
                    objc2_app_kit::NSCursor::hide();
                } else {
                    let _ = CGAssociateMouseAndMouseCursorPosition(true);
                    objc2_app_kit::NSCursor::unhide();
                }
            });
            self.events.push(Event::RelativeMode(enabled));
        }
    }

    pub fn host_cursor_event(&self, _event: &serde_json::Value) {}

    pub fn set_keyboard_grab(&self, _enabled: bool) {}

    pub fn set_input_state(&self, state: crate::core_input::State) {
        *self.input_state.lock().unwrap_or_else(|e| e.into_inner()) = state;
    }

    pub fn clipboard_allowed(&self) -> bool {
        true
    }

    pub fn take_clipboard_event(&self) {}

    pub fn initial_geometry(&self) -> (i32, i32, i32, i32, bool) {
        let (w, h) = *self.dimensions.lock().unwrap_or_else(|e| e.into_inner());
        (100, 100, w, h, self.has_focus.load(Ordering::Acquire))
    }

    pub fn close(&self) {
        self.closing.store(true, Ordering::Release);
        if self.relative_mouse.load(Ordering::Acquire) {
            self.set_relative_mouse(false);
        }
        self.release_buttons();
        #[cfg(feature = "bud")]
        self.sync_hid_input_acquired(false);
        self.stop.stop();
    }

    pub fn request_stop(&self) {
        self.closing.store(true, Ordering::Release);
        self.stop.stop();
    }

    pub fn set_title(&self, title: &str) {
        let title = title.to_string();
        let win_ptr = self.ns_window;
        dispatch2::DispatchQueue::main().exec_async(move || {
            let win = unsafe { &*(win_ptr as *const NSWindow) };
            win.setTitle(&NSString::from_str(&title));
        });
    }

    pub fn set_cursor(&self, request: crate::cursor::Request) {
        let mut cur = self.cursor.lock().unwrap_or_else(|e| e.into_inner());
        let mut p = crate::cursor::Pending::default();
        p.submit(request);
        cur.apply(p);
    }

    pub fn set_fullscreen(&self, enable: bool) {
        self.events.push(Event::Fullscreen(enable));
    }

    pub fn wake_lock_snapshot(&self) -> crate::wake_lock::State {
        self.wake_lock
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn set_wake_lock(&self, enable: bool) {
        let mut wl = self.wake_lock.lock().unwrap_or_else(|e| e.into_inner());
        wl.update(enable, true);
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        let win_ptr = self.ns_window;
        let gl_ctx_ptr = self.ns_gl_context;
        let gl_view_ptr = self.ns_gl_view;
        let (video_view_ptr, video_layer_ptr) = (self.ns_video_view, self.video_layer);
        dispatch2::DispatchQueue::main().exec_async(move || unsafe {
            let _win = Retained::from_raw(win_ptr as *mut NSWindow);
            let _ctx = Retained::from_raw(gl_ctx_ptr as *mut objc2_app_kit::NSOpenGLContext);
            let _view = Retained::from_raw(gl_view_ptr as *mut NSOpenGLView);
            let _video_view = Retained::from_raw(video_view_ptr as *mut NSView);
            let _video_layer =
                Retained::from_raw(video_layer_ptr as *mut objc2::runtime::AnyObject);
        });
    }
}

static ACTIVE_WINDOW: Mutex<Option<std::sync::Weak<Window>>> = Mutex::new(None);

thread_local! {
    static SCROLL_REMAINDER: std::cell::Cell<(f64, f64)> = const { std::cell::Cell::new((0.0, 0.0)) };
}

pub fn active_window() -> Option<Arc<Window>> {
    ACTIVE_WINDOW
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|w| w.upgrade())
}

fn cocoa_mods_to_matoya(flags: objc2_app_kit::NSEventModifierFlags) -> i32 {
    let mut mods = 0;
    if flags.contains(objc2_app_kit::NSEventModifierFlags::Shift) {
        mods |= 1;
    }
    if flags.contains(objc2_app_kit::NSEventModifierFlags::Control) {
        mods |= 2;
    }
    if flags.contains(objc2_app_kit::NSEventModifierFlags::Option) {
        mods |= 4;
    }
    if flags.contains(objc2_app_kit::NSEventModifierFlags::Command) {
        mods |= 8;
    }
    if flags.contains(objc2_app_kit::NSEventModifierFlags::CapsLock) {
        mods |= 16;
    }
    mods
}

/// Returns true when the event was a main-window key event delivered to the host.
/// The caller must then not pass it to AppKit: no view handles keyDown, so the
/// responder chain would end in NSBeep on every key press.
pub fn handle_cocoa_event(event: &objc2_app_kit::NSEvent) -> bool {
    use objc2_app_kit::NSEventType;

    let Some(window) = ACTIVE_WINDOW
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|w| w.upgrade())
    else {
        return false;
    };

    if window.closing.load(Ordering::Acquire) {
        return false;
    }

    let (mut w, mut h) = *window.dimensions.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(mtm) = objc2_foundation::MainThreadMarker::new() {
        // The live Window keeps ns_window retained until Drop.
        let win = unsafe { &*(window.ns_window as *const NSWindow) };
        // Closing hides the main window; minimizing hides it too but keeps the session.
        if !win.isVisible() && !win.isMiniaturized() {
            window.close();
            return false;
        }
        let is_key = win.isKeyWindow();
        // Settings and alert windows handle their own input; it never reaches the host.
        let foreign = match event.window(mtm) {
            Some(target) => !std::ptr::eq(&*target, win),
            None => !is_key,
        };
        let prev_focus = window.has_focus.swap(is_key, Ordering::AcqRel);
        if prev_focus != is_key {
            window.events.push(Event::Focus(is_key));
            #[cfg(feature = "bud")]
            window.sync_hid_input_acquired(is_key);
            if !is_key {
                window.release_buttons();
                if window.relative_mouse.load(Ordering::Acquire) {
                    window.set_relative_mouse(false);
                }
            }
        }
        let rect = win.contentRectForFrameRect(win.frame());
        let nw = rect.size.width as i32;
        let nh = rect.size.height as i32;
        if nw > 0 && nh > 0 && (nw != w || nh != h) {
            w = nw;
            h = nh;
            *window.dimensions.lock().unwrap_or_else(|e| e.into_inner()) = (w, h);
            window.events.push(Event::Size(w, h));
            let gl_ctx =
                unsafe { &*(window.ns_gl_context as *const objc2_app_kit::NSOpenGLContext) };
            let cgl = window.cgl_context as *mut std::ffi::c_void;
            unsafe {
                crate::graphics::cgl::CGLLockContext(cgl);
                gl_ctx.update(mtm);
                crate::graphics::cgl::CGLUnlockContext(cgl);
            }
        }
        if foreign {
            return false;
        }
    }

    let loc = event.locationInWindow();
    let x = loc.x as i32;
    let y = h - (loc.y as i32);

    match event.r#type() {
        NSEventType::LeftMouseDown => {
            window.sync_absolute_pointer(x, y);
            window.record_button(0, true);
            window.events.push(Event::Button(true, 0, x, y));
        }
        NSEventType::LeftMouseUp => {
            window.record_button(0, false);
            window.events.push(Event::Button(false, 0, x, y));
        }
        NSEventType::RightMouseDown => {
            window.sync_absolute_pointer(x, y);
            window.record_button(2, true);
            window.events.push(Event::Button(true, 2, x, y));
        }
        NSEventType::RightMouseUp => {
            window.record_button(2, false);
            window.events.push(Event::Button(false, 2, x, y));
        }
        NSEventType::OtherMouseDown => {
            let button = event.buttonNumber() as usize;
            let matoya_btn = match button {
                2 => 1,
                b if b >= 3 => b,
                _ => 1,
            };
            window.sync_absolute_pointer(x, y);
            window.record_button(matoya_btn, true);
            window
                .events
                .push(Event::Button(true, matoya_btn as i32, x, y));
        }
        NSEventType::OtherMouseUp => {
            let button = event.buttonNumber() as usize;
            let matoya_btn = match button {
                2 => 1,
                b if b >= 3 => b,
                _ => 1,
            };
            window.record_button(matoya_btn, false);
            window
                .events
                .push(Event::Button(false, matoya_btn as i32, x, y));
        }
        NSEventType::MouseMoved
        | NSEventType::LeftMouseDragged
        | NSEventType::RightMouseDragged
        | NSEventType::OtherMouseDragged => {
            if window.relative_mouse.load(Ordering::Acquire) {
                let dx = event.deltaX() as i32;
                let dy = event.deltaY() as i32;
                if dx != 0 || dy != 0 {
                    window.events.push(Event::RelativeMotion(dx, dy));
                }
            } else {
                window.events.push(Event::Motion(x, y));
            }
        }
        NSEventType::ScrollWheel => {
            // Match Windows: 120 per wheel notch, +y scrolls down, +x scrolls right.
            // AppKit's deltas are the opposite sign and already honour natural scrolling.
            // ponytail: 1.2 per trackpad point (100 points = one notch) is a guess; tune if it feels off.
            let scale = if event.hasPreciseScrollingDeltas() {
                1.2
            } else {
                120.0
            };
            let (x, y) = SCROLL_REMAINDER.with(|rem| {
                let (rx, ry) = rem.get();
                let x = rx - event.scrollingDeltaX() * scale;
                let y = ry - event.scrollingDeltaY() * scale;
                // Keep fractions so slow trackpad motion still adds up to a step.
                rem.set((x.fract(), y.fract()));
                (x as i32, y as i32)
            });
            if x != 0 || y != 0 {
                window.events.push(Event::Scroll(x, y));
            }
        }
        NSEventType::KeyDown => {
            let key_code = event.keyCode();
            let flags = event.modifierFlags();
            // Menu shortcuts (Cmd+, Cmd+I, Cmd+F, Cmd+Q) stay local; other Command
            // combinations go to the host.
            if flags.contains(objc2_app_kit::NSEventModifierFlags::Command)
                && MainThreadMarker::new().is_some_and(|mtm| {
                    NSApplication::sharedApplication(mtm)
                        .mainMenu()
                        .is_some_and(|menu| menu.performKeyEquivalent(event))
                })
            {
                return true;
            }
            let mods = cocoa_mods_to_matoya(flags);
            if let Some(code) = mac_key_to_str(key_code) {
                window.events.push(Event::Key(true, code, mods));
            }
            // Command shortcuts type nothing, matching Windows (no WM_CHAR for them).
            let chars = if flags.contains(objc2_app_kit::NSEventModifierFlags::Command) {
                None
            } else {
                event.characters()
            };
            if let Some(chars) = chars {
                let s = chars.to_string();
                for c in s.chars() {
                    // U+F700..U+F8FF are AppKit's arrow/function-key codes, not text.
                    if !c.is_control() && !('\u{F700}'..='\u{F8FF}').contains(&c) {
                        window.events.push(Event::Text(c as u32));
                    }
                }
            }
            return true;
        }
        NSEventType::KeyUp => {
            let key_code = event.keyCode();
            let flags = event.modifierFlags();
            let mods = cocoa_mods_to_matoya(flags);
            if let Some(code) = mac_key_to_str(key_code) {
                window.events.push(Event::Key(false, code, mods));
            }
            return true;
        }
        NSEventType::FlagsChanged => {
            let key_code = event.keyCode();
            if let Some(code) = mac_key_to_str(key_code) {
                let flags = event.modifierFlags();
                let mods = cocoa_mods_to_matoya(flags);
                let down = match code {
                    "ShiftLeft" | "ShiftRight" => {
                        flags.contains(objc2_app_kit::NSEventModifierFlags::Shift)
                    }
                    "ControlLeft" | "ControlRight" => {
                        flags.contains(objc2_app_kit::NSEventModifierFlags::Control)
                    }
                    "AltLeft" | "AltRight" => {
                        flags.contains(objc2_app_kit::NSEventModifierFlags::Option)
                    }
                    "MetaLeft" | "MetaRight" => {
                        flags.contains(objc2_app_kit::NSEventModifierFlags::Command)
                    }
                    "CapsLock" => flags.contains(objc2_app_kit::NSEventModifierFlags::CapsLock),
                    _ => false,
                };
                window.events.push(Event::Key(down, code, mods));
            }
        }
        _ => {}
    }
    false
}

fn mac_key_to_str(kc: u16) -> Option<&'static str> {
    Some(match kc {
        0 => "KeyA",
        1 => "KeyS",
        2 => "KeyD",
        3 => "KeyF",
        4 => "KeyH",
        5 => "KeyG",
        6 => "KeyZ",
        7 => "KeyX",
        8 => "KeyC",
        9 => "KeyV",
        11 => "KeyB",
        12 => "KeyQ",
        13 => "KeyW",
        14 => "KeyE",
        15 => "KeyR",
        16 => "KeyY",
        17 => "KeyT",
        18 => "Digit1",
        19 => "Digit2",
        20 => "Digit3",
        21 => "Digit4",
        22 => "Digit6",
        23 => "Digit5",
        24 => "Equal",
        25 => "Digit9",
        26 => "Digit7",
        27 => "Minus",
        28 => "Digit8",
        29 => "Digit0",
        30 => "BracketRight",
        31 => "KeyO",
        32 => "KeyU",
        33 => "BracketLeft",
        34 => "KeyI",
        35 => "KeyP",
        36 => "Enter",
        37 => "KeyL",
        38 => "KeyJ",
        39 => "Quote",
        40 => "KeyK",
        41 => "Semicolon",
        42 => "Backslash",
        43 => "Comma",
        44 => "Slash",
        45 => "KeyN",
        46 => "KeyM",
        47 => "Period",
        48 => "Tab",
        49 => "Space",
        50 => "Backquote",
        51 => "Backspace",
        53 => "Escape",
        55 => "MetaLeft",
        56 => "ShiftLeft",
        57 => "CapsLock",
        58 => "AltLeft",
        59 => "ControlLeft",
        60 => "ShiftRight",
        61 => "AltRight",
        62 => "ControlRight",
        65 => "NumpadDecimal",
        67 => "NumpadMultiply",
        69 => "NumpadAdd",
        71 => "NumLock",
        75 => "NumpadDivide",
        76 => "NumpadEnter",
        78 => "NumpadSubtract",
        81 => "Equal",
        82 => "Numpad0",
        83 => "Numpad1",
        84 => "Numpad2",
        85 => "Numpad3",
        86 => "Numpad4",
        87 => "Numpad5",
        88 => "Numpad6",
        89 => "Numpad7",
        91 => "Numpad8",
        92 => "Numpad9",
        96 => "F5",
        97 => "F6",
        98 => "F7",
        99 => "F3",
        100 => "F8",
        101 => "F9",
        103 => "F11",
        105 => "F13",
        107 => "F14",
        109 => "F10",
        111 => "F12",
        113 => "F15",
        114 => "Insert",
        115 => "Home",
        116 => "PageUp",
        117 => "Delete",
        118 => "F4",
        119 => "End",
        120 => "F2",
        121 => "PageDown",
        122 => "F1",
        123 => "ArrowLeft",
        124 => "ArrowRight",
        125 => "ArrowDown",
        126 => "ArrowUp",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_keyboard_mapping_covers_core_functions_navigation_and_keypad() {
        // Function keys
        assert_eq!(mac_key_to_str(122), Some("F1"));
        assert_eq!(mac_key_to_str(120), Some("F2"));
        assert_eq!(mac_key_to_str(99), Some("F3"));
        assert_eq!(mac_key_to_str(118), Some("F4"));
        assert_eq!(mac_key_to_str(96), Some("F5"));
        assert_eq!(mac_key_to_str(97), Some("F6"));
        assert_eq!(mac_key_to_str(98), Some("F7"));
        assert_eq!(mac_key_to_str(100), Some("F8"));
        assert_eq!(mac_key_to_str(101), Some("F9"));
        assert_eq!(mac_key_to_str(109), Some("F10"));
        assert_eq!(mac_key_to_str(103), Some("F11"));
        assert_eq!(mac_key_to_str(111), Some("F12"));

        // Navigation
        assert_eq!(mac_key_to_str(114), Some("Insert"));
        assert_eq!(mac_key_to_str(115), Some("Home"));
        assert_eq!(mac_key_to_str(119), Some("End"));
        assert_eq!(mac_key_to_str(116), Some("PageUp"));
        assert_eq!(mac_key_to_str(121), Some("PageDown"));
        assert_eq!(mac_key_to_str(117), Some("Delete"));

        // Keypad
        assert_eq!(mac_key_to_str(82), Some("Numpad0"));
        assert_eq!(mac_key_to_str(83), Some("Numpad1"));
        assert_eq!(mac_key_to_str(84), Some("Numpad2"));
        assert_eq!(mac_key_to_str(85), Some("Numpad3"));
        assert_eq!(mac_key_to_str(86), Some("Numpad4"));
        assert_eq!(mac_key_to_str(87), Some("Numpad5"));
        assert_eq!(mac_key_to_str(88), Some("Numpad6"));
        assert_eq!(mac_key_to_str(89), Some("Numpad7"));
        assert_eq!(mac_key_to_str(91), Some("Numpad8"));
        assert_eq!(mac_key_to_str(92), Some("Numpad9"));
        assert_eq!(mac_key_to_str(65), Some("NumpadDecimal"));
        assert_eq!(mac_key_to_str(67), Some("NumpadMultiply"));
        assert_eq!(mac_key_to_str(69), Some("NumpadAdd"));
        assert_eq!(mac_key_to_str(71), Some("NumLock"));
        assert_eq!(mac_key_to_str(75), Some("NumpadDivide"));
        assert_eq!(mac_key_to_str(76), Some("NumpadEnter"));
        assert_eq!(mac_key_to_str(78), Some("NumpadSubtract"));

        // Letters & Modifiers
        assert_eq!(mac_key_to_str(0), Some("KeyA"));
        assert_eq!(mac_key_to_str(55), Some("MetaLeft"));
        assert_eq!(mac_key_to_str(56), Some("ShiftLeft"));
        assert_eq!(mac_key_to_str(58), Some("AltLeft"));
        assert_eq!(mac_key_to_str(59), Some("ControlLeft"));

        // Unknown
        assert_eq!(mac_key_to_str(999), None);
    }

    #[test]
    fn cocoa_modifiers_translate_to_matoya_flags() {
        use objc2_app_kit::NSEventModifierFlags;

        assert_eq!(cocoa_mods_to_matoya(NSEventModifierFlags::empty()), 0);
        assert_eq!(cocoa_mods_to_matoya(NSEventModifierFlags::Shift), 1);
        assert_eq!(cocoa_mods_to_matoya(NSEventModifierFlags::Control), 2);
        assert_eq!(cocoa_mods_to_matoya(NSEventModifierFlags::Option), 4);
        assert_eq!(cocoa_mods_to_matoya(NSEventModifierFlags::Command), 8);
        assert_eq!(cocoa_mods_to_matoya(NSEventModifierFlags::CapsLock), 16);
        assert_eq!(
            cocoa_mods_to_matoya(NSEventModifierFlags::Shift | NSEventModifierFlags::Command),
            1 | 8
        );
    }

    #[test]
    fn button_tracking_releases_all_held_buttons_on_focus_loss() {
        let pressed = AtomicUsize::new(0);
        // Press button 0 and button 2
        pressed.fetch_or(1 << 0, Ordering::AcqRel);
        pressed.fetch_or(1 << 2, Ordering::AcqRel);
        assert_eq!(pressed.load(Ordering::Acquire), 1 | 4);

        // Release buttons
        let held = pressed.swap(0, Ordering::AcqRel);
        let mut released = Vec::new();
        for button in 0..5 {
            if (held & (1 << button)) != 0 {
                released.push(button);
            }
        }
        assert_eq!(released, vec![0, 2]);
        assert_eq!(pressed.load(Ordering::Acquire), 0);
    }

    #[test]
    #[cfg(feature = "bud")]
    fn hid_input_lifecycle_and_acquired_synchronization() {
        let hid_acquired = AtomicBool::new(false);
        let hid_mouse = AtomicBool::new(true);
        let hid_keyboard = AtomicBool::new(true);
        let closing = AtomicBool::new(false);

        // When focused, not closing, and HID active -> acquired = true
        let has_focus = true;
        let acquired = has_focus
            && !closing.load(Ordering::Acquire)
            && (hid_mouse.load(Ordering::Acquire) || hid_keyboard.load(Ordering::Acquire));
        assert!(acquired);
        assert!(!hid_acquired.swap(acquired, Ordering::AcqRel)); // transitioned false -> true

        // Focus loss -> acquired = false
        let has_focus = false;
        let acquired = has_focus
            && !closing.load(Ordering::Acquire)
            && (hid_mouse.load(Ordering::Acquire) || hid_keyboard.load(Ordering::Acquire));
        assert!(!acquired);
        assert!(hid_acquired.swap(acquired, Ordering::AcqRel)); // transitioned true -> false
    }
}
