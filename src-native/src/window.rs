//! Native Win32 window. Its UI thread blocks in GetMessage; WASM runs separately.
use anyhow::{bail, Result};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    Arc, Mutex,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{Gdi::*, OpenGL::*},
    System::LibraryLoader::*,
    UI::Input::{KeyboardAndMouse::*, *},
    UI::WindowsAndMessaging::*,
};

#[derive(Clone, Debug)]
pub enum Event {
    Size(i32, i32),
    Focus(bool),
    Motion(i32, i32),
    RelativeMotion(i32, i32),
    RelativeMode(bool),
    Button(bool, i32, i32, i32),
    Text(u32),
    Key(bool, &'static str, i32),
    Scroll(i32, i32),
    Fullscreen(bool),
    #[cfg(feature = "usb-mouse-probe")]
    HidMouseRelease,
}

pub struct Window {
    pub stats: Arc<crate::stats::Shared>,
    stats_page: AtomicUsize,
    pub updates: Arc<crate::updater::State>,
    updates_page: AtomicUsize,
    core_version: Arc<crate::core_version::State>,
    about_page: AtomicUsize,
    connection_settings: Mutex<Option<Arc<crate::connection_settings::Manager>>>,
    settings_page: AtomicUsize,
    // Covers construction and destruction as well as the visible page. Its HWND
    // is not available until open() has already painted the native controls.
    settings_active: AtomicBool,
    pub gui_visibility: crate::gui_visibility::State,
    pub hwnd: AtomicUsize,
    pub closing: AtomicBool,
    pub active_contexts: AtomicUsize,
    pub events: crate::event_queue::Queue<Event>,
    pub dimensions: Mutex<(i32, i32)>,
    pub overlay: Mutex<crate::overlay::Shared>,
    pub software_overlay_enabled: AtomicBool,
    software_pending: Mutex<crate::overlay_software::Pending>,
    software_presenter: Mutex<crate::overlay_software::Presenter>,
    pub graphics: Mutex<Option<crate::graphics::GraphicsReport>>,
    pub audio_registry: Mutex<crate::audio_windows::Registry>,
    pub video_report: Mutex<Option<crate::video_output::Snapshot>>,
    video_hwnd: AtomicUsize,
    video_create_failed: AtomicBool,
    video_ready: AtomicBool,
    video_generation: Mutex<u64>,
    video_visible: AtomicBool,
    #[cfg(any(test, feature = "diagnostics"))]
    pub capture: Mutex<Option<std::path::PathBuf>>,
    #[cfg(any(test, feature = "diagnostics"))]
    pub synthetic_login: bool,
    #[cfg(any(test, feature = "diagnostics"))]
    pub script_steps: AtomicUsize,
    pub run_seconds: u64,
    pub live: bool,
    pub online: bool,
    #[cfg(any(test, feature = "diagnostics"))]
    pub network_origin_audit: bool,
    pub stop: Arc<crate::lifecycle::StopSignal>,
    relative_mouse: AtomicBool,
    #[cfg(feature = "usb-mouse-probe")]
    hid_cursor_active: AtomicBool,
    #[cfg(feature = "usb-mouse-probe")]
    hid_raw_mouse: AtomicBool,
    #[cfg(feature = "usb-mouse-probe")]
    hid_cursor_posted: AtomicBool,
    relative_pending: AtomicUsize,
    relative_requested: AtomicBool,
    relative_anchor: Mutex<Option<(i32, i32)>>,
    has_focus: AtomicBool,
    input_state: Mutex<crate::core_input::State>,
    core_hotkeys: Mutex<Option<crate::core_hotkeys::Binding>>,
    mouse_clipped: AtomicBool,
    input_suspended: AtomicBool,
    host_cursor: Mutex<Option<(i32, i32, bool)>>,
    warp_pending: AtomicBool,
    warp_echo: AtomicU64,
    keyboard_requested: AtomicBool,
    keyboard_grab: Mutex<crate::keyboard_grab::State>,
    pressed_buttons: AtomicUsize,
    pressed_keys: Mutex<std::collections::BTreeSet<&'static str>>,
    text_decoder: Mutex<crate::input::TextDecoder>,
    cursor_pending: Mutex<crate::cursor::Pending>,
    cursor_posted: AtomicBool,
    cursor: Mutex<crate::cursor::State>,
    fullscreen: Mutex<crate::fullscreen::State>,
    fullscreen_active: AtomicBool,
    fullscreen_pending: AtomicUsize,
    wake_pending: AtomicUsize,
    wake_lock: Mutex<crate::wake_lock::State>,
}

impl Window {
    unsafe fn open_about(&self) {
        self.core_version.check();
        let existing = self.about_page.load(Ordering::Acquire) as HWND;
        if !existing.is_null() {
            ShowWindow(existing, SW_RESTORE);
            SetForegroundWindow(existing);
            return;
        }
        let page = crate::updater_ui::open_about(self.handle(), self.core_version.clone());
        self.about_page.store(page as usize, Ordering::Release);
    }
    unsafe fn open_updates(&self, check: bool, activate: bool) {
        if check {
            self.updates.check(false);
        }
        let existing = self.updates_page.load(Ordering::Acquire) as HWND;
        if !existing.is_null() {
            if activate {
                ShowWindow(existing, SW_RESTORE);
                SetForegroundWindow(existing);
            }
            return;
        }
        let page = crate::updater_ui::open(self.handle(), self.updates.clone(), activate);
        self.updates_page.store(page as usize, Ordering::Release);
    }
    unsafe fn open_stats(&self) {
        let existing = self.stats_page.load(Ordering::Acquire) as HWND;
        if !existing.is_null() {
            ShowWindow(existing, SW_RESTORE);
            SetForegroundWindow(existing);
            return;
        }
        let page = crate::stats_ui::open(self.handle(), self.stats.clone());
        self.stats_page.store(page as usize, Ordering::Release);
    }
    pub fn create(
        synthetic_login: bool,
        live: bool,
        online: bool,
        network_origin_audit: bool,
    ) -> Result<Arc<Self>> {
        Self::create_mode(synthetic_login, live, online, network_origin_audit, true)
    }

    // The standalone D3D11 test has no OpenGL UI and must not depend on WGL.
    #[cfg(feature = "diagnostics")]
    pub fn create_video_probe() -> Result<Arc<Self>> {
        Self::create_mode(false, true, false, false, false)
    }

    fn create_mode(
        synthetic_login: bool,
        live: bool,
        online: bool,
        _network_origin_audit: bool,
        opengl_required: bool,
    ) -> Result<Arc<Self>> {
        let state = Arc::new(Self {
            stats: Default::default(),
            stats_page: AtomicUsize::new(0),
            updates: Default::default(),
            updates_page: AtomicUsize::new(0),
            core_version: Default::default(),
            about_page: AtomicUsize::new(0),
            connection_settings: Default::default(),
            settings_page: AtomicUsize::new(0),
            settings_active: AtomicBool::new(false),
            gui_visibility: Default::default(),
            hwnd: AtomicUsize::new(0),
            closing: AtomicBool::new(false),
            active_contexts: AtomicUsize::new(0),
            events: Default::default(),
            dimensions: Mutex::new((1024, 720)),
            graphics: Default::default(),
            overlay: Default::default(),
            software_overlay_enabled: AtomicBool::new(true),
            software_pending: Default::default(),
            software_presenter: Default::default(),
            video_report: Default::default(),
            audio_registry: Default::default(),
            video_hwnd: AtomicUsize::new(0),
            video_create_failed: AtomicBool::new(false),
            video_ready: AtomicBool::new(false),
            video_generation: Mutex::new(0),
            video_visible: AtomicBool::new(true),
            #[cfg(any(test, feature = "diagnostics"))]
            capture: Default::default(),
            #[cfg(any(test, feature = "diagnostics"))]
            synthetic_login,
            #[cfg(any(test, feature = "diagnostics"))]
            script_steps: AtomicUsize::new(0),
            run_seconds: if synthetic_login { 20 } else { 8 },
            live,
            online,
            #[cfg(any(test, feature = "diagnostics"))]
            network_origin_audit: _network_origin_audit,
            stop: Default::default(),
            relative_mouse: AtomicBool::new(false),
            #[cfg(feature = "usb-mouse-probe")]
            hid_cursor_active: AtomicBool::new(false),
            #[cfg(feature = "usb-mouse-probe")]
            hid_raw_mouse: AtomicBool::new(false),
            #[cfg(feature = "usb-mouse-probe")]
            hid_cursor_posted: AtomicBool::new(false),
            relative_pending: AtomicUsize::new(0),
            relative_requested: AtomicBool::new(false),
            relative_anchor: Mutex::new(None),
            has_focus: AtomicBool::new(false),
            input_state: Default::default(),
            core_hotkeys: Default::default(),
            mouse_clipped: AtomicBool::new(false),
            input_suspended: AtomicBool::new(false),
            host_cursor: Default::default(),
            warp_pending: AtomicBool::new(false),
            warp_echo: Default::default(),
            keyboard_requested: AtomicBool::new(false),
            keyboard_grab: Default::default(),
            pressed_buttons: AtomicUsize::new(0),
            pressed_keys: Default::default(),
            text_decoder: Default::default(),
            cursor_pending: Default::default(),
            cursor_posted: AtomicBool::new(false),
            cursor: Default::default(),
            fullscreen: Default::default(),
            fullscreen_active: AtomicBool::new(false),
            fullscreen_pending: AtomicUsize::new(0),
            wake_pending: AtomicUsize::new(0),
            wake_lock: Default::default(),
        });
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let ui = state.clone();
        std::thread::spawn(move || {
            // SAFETY: all window creation/destruction and message dispatch run
            // on this thread. Arc keeps the userdata alive until dispatch ends.
            let outcome = unsafe { create_window(&ui, opengl_required) };
            match outcome {
                Ok(hwnd) => {
                    ui.hwnd.store(hwnd as usize, Ordering::Release);
                    let _ = tx.send(Ok(()));
                    unsafe {
                        let mut msg: MSG = std::mem::zeroed();
                        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                            if msg.message == WM_KEYDOWN
                                && msg.wParam == b'S' as usize
                                && GetKeyState(VK_CONTROL as i32) < 0
                                && GetKeyState(VK_SHIFT as i32) < 0
                                && !ui.core_owns_key("KeyS", modifiers())
                            {
                                if msg.lParam & (1 << 30) == 0 {
                                    ui.open_stats();
                                }
                                continue;
                            }
                            let page = ui.settings_page.load(Ordering::Acquire) as HWND;
                            if !page.is_null() && (msg.hwnd == page || IsChild(page, msg.hwnd) != 0)
                            {
                                if msg.message == WM_KEYDOWN && msg.wParam == VK_ESCAPE as usize {
                                    PostMessageW(hwnd, WM_APP + 9, 0, 0);
                                    continue;
                                }
                                if IsDialogMessageW(page, &msg) != 0 {
                                    continue;
                                }
                            }
                            let updates = [
                                ui.updates_page.load(Ordering::Acquire),
                                ui.about_page.load(Ordering::Acquire),
                            ]
                            .into_iter()
                            .map(|h| h as HWND)
                            .find(|h| {
                                !h.is_null() && (msg.hwnd == *h || IsChild(*h, msg.hwnd) != 0)
                            })
                            .unwrap_or(std::ptr::null_mut());
                            if !updates.is_null()
                                && (msg.hwnd == updates || IsChild(updates, msg.hwnd) != 0)
                            {
                                if msg.message == WM_KEYDOWN && msg.wParam == VK_ESCAPE as usize {
                                    PostMessageW(updates, WM_CLOSE, 0, 0);
                                    continue;
                                }
                                if IsDialogMessageW(updates, &msg) != 0 {
                                    continue;
                                }
                            }
                            TranslateMessage(&msg);
                            DispatchMessageW(&msg);
                        }
                    }
                    ui.apply_wake_lock(false, false);
                    ui.hwnd.store(0, Ordering::Release);
                }
                Err(error) => {
                    let _ = tx.send(Err(error));
                }
            }
        });
        rx.recv()??;
        Ok(state)
    }

    pub fn handle(&self) -> HWND {
        self.hwnd.load(Ordering::Acquire) as HWND
    }
    pub fn bind_core_hotkeys(&self, binding: crate::core_hotkeys::Binding) {
        *self.core_hotkeys.lock().unwrap_or_else(|e| e.into_inner()) = Some(binding);
    }
    fn core_owns_key(&self, code: &str, mods: i32) -> bool {
        self.core_hotkeys
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|binding| binding.owns(code, mods).unwrap_or(true))
    }
    pub fn settings_open(&self) -> bool {
        self.settings_active.load(Ordering::Acquire)
    }
    pub fn repaint_settings_after_gl_pause(&self) {
        // The GL worker has passed its last swap. Queue one final native paint
        // on the UI thread in case that in-flight swap covered initial paint.
        unsafe {
            PostMessageW(self.handle(), WM_APP + 15, 0, 0);
        }
    }
    #[cfg(feature = "vendor-video")]
    pub fn direct_presentation_allowed(&self) -> bool {
        self.video_visible.load(Ordering::Acquire)
            && !self.settings_open()
            && !self.closing.load(Ordering::Acquire)
    }
    #[cfg(feature = "vendor-video")]
    pub fn experimental_zero_copy(&self) -> bool {
        self.connection_settings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|manager| manager.view().0.experimental_zero_copy)
    }
    pub fn video_gpu(&self) -> Option<crate::connection_settings::GpuPreference> {
        self.connection_settings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|manager| manager.view().0.video_gpu)
    }
    pub fn audio_buffer(&self) -> Option<(u32, u32)> {
        let settings = self
            .connection_settings
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        settings.as_ref().and_then(|m| {
            let s = m.view().0;
            (s.experimental_audio_buffer
                && s.audio_max_buffer_ms > 0
                && s.audio_max_buffer_ms <= 500
                && s.audio_min_buffer_ms <= s.audio_max_buffer_ms)
                .then_some((s.audio_min_buffer_ms, s.audio_max_buffer_ms))
        })
    }
    pub fn install_connection_settings(&self, manager: Arc<crate::connection_settings::Manager>) {
        *self
            .connection_settings
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(manager);
        unsafe {
            PostMessageW(self.handle(), WM_APP + 8, 0, 0);
        }
    }
    unsafe fn open_connection_settings(&self) {
        let page = self.settings_page.load(Ordering::Acquire) as HWND;
        if !page.is_null() {
            SetFocus(page);
            return;
        }
        let manager = self
            .connection_settings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(manager) = manager {
            // Suspend GL swaps before open() creates/shows/paints the GDI child,
            // not after it returns. The video surface must also be hidden first.
            self.settings_active.store(true, Ordering::Release);
            self.gui_visibility.cover(true);
            self.input_suspended.store(true, Ordering::Release);
            self.apply_input_capture();
            self.apply_relative_mouse(false);
            self.show_video_surface();
            let page = crate::connection_settings_ui::open(self.handle(), manager);
            self.settings_page.store(page as usize, Ordering::Release);
            if page.is_null() {
                self.settings_active.store(false, Ordering::Release);
                self.gui_visibility.cover(false);
                self.redraw_client();
            }
            self.input_suspended.store(false, Ordering::Release);
            self.show_video_surface();
        }
    }
    unsafe fn redraw_client(&self) {
        // Core GUI repaint comes from its visibility transition on the render
        // worker, not a resize notification with unchanged dimensions.
        RedrawWindow(
            self.handle(),
            std::ptr::null(),
            std::ptr::null_mut(),
            RDW_INVALIDATE | RDW_ALLCHILDREN | RDW_UPDATENOW,
        );
    }
    // HWND creation and visibility remain on the existing UI thread.
    pub fn ensure_video_surface(&self) -> Result<usize> {
        unsafe {
            PostMessageW(self.handle(), WM_APP + 5, 0, 0);
        }
        for _ in 0..150 {
            let hwnd = self.video_hwnd.load(Ordering::Acquire);
            if hwnd != 0 {
                return Ok(hwnd);
            }
            if self.closing.load(Ordering::Acquire)
                || self.video_create_failed.load(Ordering::Acquire)
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        bail!("native video surface unavailable")
    }
    fn set_video_ready(&self, ready: bool) {
        if self.video_ready.swap(ready, Ordering::AcqRel) != ready {
            unsafe {
                PostMessageW(self.handle(), WM_APP + 6, 0, 0);
            }
        }
    }
    pub fn begin_video(&self) -> u64 {
        let mut generation = self
            .video_generation
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *generation = generation.wrapping_add(1);
        #[cfg(feature = "usb-mouse-probe")]
        self.hid_cursor_active.store(false, Ordering::Release);
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
    pub fn end_session(&self) {
        self.begin_video(); // Retire the old surface before a reconnect can own it.
        *self.host_cursor.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.relative_requested.store(false, Ordering::Release);
        self.set_input_state(Default::default());
        self.set_relative_mouse(false);
        self.set_keyboard_grab(false);
    }
    pub fn publish_software_overlay(&self, frame: &crate::overlay::Frame) {
        if !self.software_overlay_enabled.load(Ordering::Acquire)
            || !self.video_ready.load(Ordering::Acquire)
        {
            return;
        }
        // Rasterize on the GUI render worker, never on the input/UI/video worker.
        let surface = crate::overlay_software::rasterize(frame);
        let mut pending = self
            .software_pending
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        pending.surface = surface;
        if !pending.posted {
            pending.posted = unsafe { PostMessageW(self.handle(), WM_APP + 14, 0, 0) != 0 };
        }
    }
    unsafe fn show_video_surface(&self) {
        let hwnd = self.video_hwnd.load(Ordering::Acquire) as HWND;
        if !hwnd.is_null() {
            let show = self.video_ready.load(Ordering::Acquire)
                && !self.settings_open()
                && self.video_visible.load(Ordering::Acquire)
                && !self.closing.load(Ordering::Acquire);
            ShowWindow(hwnd, if show { SW_SHOWNA } else { SW_HIDE });
            if !show {
                self.software_presenter
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .hide();
            }
        }
        self.apply_input_capture();
    }

    pub fn presented_viewport(&self) -> Option<crate::viewport::Viewport> {
        if !self.video_ready.load(Ordering::Acquire) || !self.video_visible.load(Ordering::Acquire)
        {
            return None;
        }
        let report = self.video_report.lock().unwrap_or_else(|e| e.into_inner());
        let v = report
            .as_ref()
            .filter(|v| v.frames_presented > 0 && v.failure_stage.is_none())?;
        let size = *self.dimensions.lock().unwrap_or_else(|e| e.into_inner());
        let viewport = crate::viewport::Viewport {
            source: (v.width?, v.height?),
            client: (u32::try_from(size.0).ok()?, u32::try_from(size.1).ok()?),
        };
        viewport.rect()?;
        Some(viewport)
    }

    unsafe fn mouse_button(&self, down: bool, button: i32, x: i32, y: i32) {
        let bit = 1usize << button;
        if down {
            self.pressed_buttons.fetch_or(bit, Ordering::AcqRel);
            SetCapture(self.handle());
        } else if self.pressed_buttons.fetch_and(!bit, Ordering::AcqRel) & !bit == 0
            && GetCapture() == self.handle()
        {
            ReleaseCapture();
        }
        self.push(Event::Button(down, button, x, y));
    }
    fn release_buttons(&self) {
        let pressed = self.pressed_buttons.swap(0, Ordering::AcqRel);
        for button in 0..5 {
            if pressed & (1 << button) != 0 {
                self.push(Event::Button(false, button, 0, 0));
            }
        }
    }

    pub fn set_relative_mouse(&self, enabled: bool) {
        // The browser build omits the native `relative || hidden` rule because
        // MTY_AppCanWarpCursor returns false. Known native host intent takes
        // precedence over that browser-specific pointer-lock callback.
        let enabled = self
            .host_cursor
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .map(|(_, _, relative)| relative)
            .unwrap_or(enabled);
        self.request_relative_mouse(enabled, false);
    }
    fn request_relative_mouse(&self, enabled: bool, authoritative: bool) {
        let state = *self.input_state.lock().unwrap_or_else(|e| e.into_inner());
        let suspended = !self.has_focus.load(Ordering::Acquire)
            || unsafe { IsIconic(self.handle()) != 0 || GetForegroundWindow() != self.handle() }
            || !self.video_ready.load(Ordering::Acquire)
            || !self.video_visible.load(Ordering::Acquire)
            || self.input_suspended.load(Ordering::Acquire)
            || self.settings_open()
            || state.detached;
        let previous = self.relative_requested.load(Ordering::Acquire);
        // The core may issue an unlock while focus/detach temporarily suspends
        // acquisition. Keep the host's relative request for the next activation.
        if authoritative || enabled || !suspended {
            self.relative_requested.store(enabled, Ordering::Release);
        }
        if enabled {
            self.warp_pending.store(false, Ordering::Release);
        } else if previous && !suspended {
            self.warp_pending.store(true, Ordering::Release);
        }
        self.relative_pending
            .store(if enabled { 2 } else { 1 }, Ordering::Release);
        unsafe {
            PostMessageW(self.handle(), WM_APP + 7, 0, 0);
        }
    }
    pub fn host_cursor_event(&self, event: &serde_json::Value) {
        if event["type"].as_i64() != Some(1) {
            return;
        }
        let cursor = &event["cursor"];
        if let (Some(x), Some(y), Some(relative), Some(hidden)) = (
            cursor["positionX"]
                .as_i64()
                .and_then(|v| i32::try_from(v).ok()),
            cursor["positionY"]
                .as_i64()
                .and_then(|v| i32::try_from(v).ok()),
            cursor["relative"].as_bool(),
            cursor["hidden"].as_bool(),
        ) {
            self.set_host_cursor(x, y, relative, hidden);
        }
    }
    fn set_host_cursor(&self, x: i32, y: i32, relative: bool, hidden: bool) {
        // Parsec SDK cursor_func and lowlat both use OR. A hidden-only event
        // must acquire native relative input too; button state is not evidence.
        let relative = relative || hidden;
        *self.host_cursor.lock().unwrap_or_else(|e| e.into_inner()) = Some((x, y, relative));
        #[cfg(feature = "usb-mouse-probe")]
        if self.hid_cursor_active.load(Ordering::Acquire) {
            self.refresh_hid_cursor();
        }
        // When capture is suspended, the guest already sees relative=false and
        // may omit its redundant unlock. Do not retain a stale relative request.
        if relative != self.relative_requested.load(Ordering::Acquire) {
            // A host transition is permanent, including while unfocused.
            self.request_relative_mouse(relative, true);
        }
    }
    unsafe fn restore_host_cursor(&self) {
        if !self.warp_pending.swap(false, Ordering::AcqRel)
            || !self.input_allowed()
            || self
                .input_state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .warp_disabled
        {
            return;
        }
        let cursor = *self.host_cursor.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((x, y, false)) = cursor {
            if let Some((cx, cy)) = self.presented_viewport().and_then(|v| v.unmap(x, y)) {
                let mut point = POINT { x: cx, y: cy };
                if ClientToScreen(self.handle(), &mut point) != 0
                    && SetCursorPos(point.x, point.y) != 0
                {
                    // Zero means no synthetic move. Keep the ordinary mouse
                    // path free of an additional mutex for this rare transition.
                    self.warp_echo
                        .store(((cx as u64) << 32 | cy as u64) + 1, Ordering::Release);
                }
            }
        }
    }
    pub fn set_keyboard_grab(&self, enabled: bool) {
        self.keyboard_requested.store(enabled, Ordering::Release);
        unsafe {
            PostMessageW(self.handle(), WM_APP + 12, 0, 0);
        }
    }
    pub fn set_input_state(&self, state: crate::core_input::State) {
        let mut current = self.input_state.lock().unwrap_or_else(|e| e.into_inner());
        let keyboard_changed = self
            .keyboard_requested
            .swap(state.mode.keyboard, Ordering::AcqRel)
            != state.mode.keyboard;
        if *current == state && !keyboard_changed {
            return;
        }
        *current = state;
        unsafe {
            PostMessageW(self.handle(), WM_APP + 12, 0, 0);
        }
    }
    unsafe fn input_allowed(&self) -> bool {
        GetForegroundWindow() == self.handle()
            && GetFocus() == self.handle()
            && IsIconic(self.handle()) == 0
            && self.video_ready.load(Ordering::Acquire)
            && self.video_visible.load(Ordering::Acquire)
            && !self.settings_open()
            && !self.input_suspended.load(Ordering::Acquire)
            && !self.closing.load(Ordering::Acquire)
            && !self
                .input_state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .detached
    }
    unsafe fn apply_input_capture(&self) {
        let allowed = self.input_allowed();
        #[cfg(feature = "usb-mouse-probe")]
        self.apply_hid_capture(allowed);
        self.keyboard_grab
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .set(
                self.handle(),
                allowed && self.keyboard_requested.load(Ordering::Acquire),
            );
        self.apply_relative_mouse(allowed && self.relative_requested.load(Ordering::Acquire));
        #[cfg(feature = "usb-mouse-probe")]
        self.select_mouse_cursor();
    }
    fn raw_mouse_active(&self) -> bool {
        #[cfg(feature = "usb-mouse-probe")]
        if self.hid_raw_mouse.load(Ordering::Acquire) {
            return true;
        }
        self.relative_mouse.load(Ordering::Acquire)
    }
    unsafe fn select_mouse_cursor(&self) {
        if self.relative_mouse.load(Ordering::Acquire) {
            SetCursor(std::ptr::null_mut());
            return;
        }
        let cursor = self.cursor.lock().unwrap_or_else(|e| e.into_inner());
        #[cfg(feature = "usb-mouse-probe")]
        if self.hid_raw_mouse.load(Ordering::Acquire) {
            if crate::hid_cursor::visible_position(
                *self.host_cursor.lock().unwrap_or_else(|e| e.into_inner()),
            )
            .is_some()
            {
                cursor.select();
            } else {
                SetCursor(std::ptr::null_mut());
            }
            return;
        }
        cursor.select();
    }
    #[cfg(feature = "usb-mouse-probe")]
    pub fn usb_cursor_generation(&self) -> u64 {
        *self
            .video_generation
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }
    #[cfg(feature = "usb-mouse-probe")]
    pub fn set_usb_cursor_active(&self, expected: u64, active: bool) {
        let generation = self
            .video_generation
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if *generation == expected
            && self.hid_cursor_active.swap(active, Ordering::AcqRel) != active
        {
            self.refresh_hid_cursor();
        }
    }
    #[cfg(feature = "usb-mouse-probe")]
    fn refresh_hid_cursor(&self) {
        if !self.hid_cursor_posted.swap(true, Ordering::AcqRel) {
            unsafe {
                if PostMessageW(self.handle(), crate::hid_cursor::REFRESH, 0, 0) == 0 {
                    self.hid_cursor_posted.store(false, Ordering::Release);
                }
            }
        }
    }
    #[cfg(feature = "usb-mouse-probe")]
    unsafe fn apply_hid_capture(&self, allowed: bool) {
        let wanted = allowed
            && self.hid_cursor_active.load(Ordering::Acquire)
            && self
                .host_cursor
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_some();
        let previous = self.hid_raw_mouse.load(Ordering::Acquire);
        if previous == wanted {
            return;
        }
        let registered = wanted || self.relative_mouse.load(Ordering::Acquire);
        let device = RAWINPUTDEVICE {
            usUsagePage: 1,
            usUsage: 2,
            dwFlags: if registered { 0 } else { RIDEV_REMOVE },
            hwndTarget: if registered {
                self.handle()
            } else {
                std::ptr::null_mut()
            },
        };
        let acquired =
            RegisterRawInputDevices(&device, 1, std::mem::size_of::<RAWINPUTDEVICE>() as u32) != 0
                && wanted;
        self.hid_raw_mouse.store(acquired, Ordering::Release);
        if previous && !acquired {
            self.push(Event::HidMouseRelease);
        }
        crate::event_log::record(
            "INFO",
            "hid_cursor_capture",
            serde_json::json!({"requested":wanted,"active":acquired}),
        );
    }
    unsafe fn apply_mouse_clip(&self) -> bool {
        let state = *self.input_state.lock().unwrap_or_else(|e| e.into_inner());
        let enabled = self.input_allowed() && (state.mode.mouse || self.raw_mouse_active());
        if enabled {
            let mut rect = RECT::default();
            let mut origin = POINT::default();
            if GetClientRect(self.handle(), &mut rect) != 0
                && ClientToScreen(self.handle(), &mut origin) != 0
                && rect.right > rect.left
                && rect.bottom > rect.top
            {
                rect.left += origin.x;
                rect.right += origin.x;
                rect.top += origin.y;
                rect.bottom += origin.y;
                #[cfg(feature = "usb-mouse-probe")]
                if self.hid_raw_mouse.load(Ordering::Acquire) {
                    let cursor = *self.host_cursor.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some((x, y)) = crate::hid_cursor::visible_position(cursor)
                        .and_then(|(x, y)| self.presented_viewport().and_then(|v| v.unmap(x, y)))
                    {
                        *self
                            .relative_anchor
                            .lock()
                            .unwrap_or_else(|e| e.into_inner()) =
                            Some((origin.x + x, origin.y + y));
                    }
                }
                if self.raw_mouse_active() {
                    if let Some((x, y)) = *self
                        .relative_anchor
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                    {
                        // Native Matoya holds relative capture at one screen
                        // pixel; movement comes from Raw Input, not OS position.
                        let x = x.clamp(rect.left, rect.right - 1);
                        let y = y.clamp(rect.top, rect.bottom - 1);
                        rect = RECT {
                            left: x,
                            top: y,
                            right: x + 1,
                            bottom: y + 1,
                        };
                    }
                }
                if ClipCursor(&rect) != 0 {
                    self.mouse_clipped.store(true, Ordering::Release);
                    return true;
                }
            }
        }
        if self.mouse_clipped.swap(false, Ordering::AcqRel) {
            ClipCursor(std::ptr::null());
        }
        false
    }
    unsafe fn apply_relative_mouse(&self, enabled: bool) {
        let enabled = enabled && self.input_allowed();
        let previous = self.relative_mouse.load(Ordering::Acquire);
        if enabled == previous {
            self.apply_mouse_clip();
            return;
        }
        let registered_input = enabled;
        #[cfg(feature = "usb-mouse-probe")]
        let registered_input = registered_input || self.hid_raw_mouse.load(Ordering::Acquire);
        let device = RAWINPUTDEVICE {
            usUsagePage: 1,
            usUsage: 2,
            dwFlags: if registered_input { 0 } else { RIDEV_REMOVE },
            hwndTarget: if registered_input {
                self.handle()
            } else {
                std::ptr::null_mut()
            },
        };
        let registered =
            RegisterRawInputDevices(&device, 1, std::mem::size_of::<RAWINPUTDEVICE>() as u32) != 0;
        let acquired = registered && enabled;
        *self
            .relative_anchor
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = if acquired {
            let mut point = POINT::default();
            if GetCursorPos(&mut point) != 0 {
                Some((point.x, point.y))
            } else {
                None
            }
        } else {
            None
        };
        self.relative_mouse.store(acquired, Ordering::Release);
        let clipped = self.apply_mouse_clip();
        if acquired && !clipped {
            self.apply_relative_mouse(false);
            return;
        }
        if acquired {
            SetCursor(std::ptr::null_mut());
        } else {
            self.cursor
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .select();
        }
        // A rejected request must also report the actual state to the guest.
        self.relative_mouse.store(acquired, Ordering::Release);
        self.push(Event::RelativeMode(acquired));
    }

    pub fn initial_geometry(&self) -> (i32, i32, i32, i32, bool) {
        unsafe {
            let mut origin = POINT { x: 0, y: 0 };
            ClientToScreen(self.handle(), &mut origin);
            (
                origin.x,
                origin.y,
                GetSystemMetrics(SM_CXSCREEN),
                GetSystemMetrics(SM_CYSCREEN),
                GetForegroundWindow() == self.handle(),
            )
        }
    }
    pub fn close(&self) {
        self.request_stop();
        unsafe {
            PostMessageW(self.handle(), WM_APP + 1, 0, 0);
        }
    }
    pub fn request_stop(&self) {
        self.closing.store(true, Ordering::Release);
        self.relative_requested.store(false, Ordering::Release);
        self.request_relative_mouse(false, true);
        self.events.stop();
        self.stop.stop();
    }
    pub fn set_cursor(&self, request: crate::cursor::Request) {
        if self.closing.load(Ordering::Acquire) {
            return;
        }
        self.cursor_pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .submit(request);
        if !self.cursor_posted.swap(true, Ordering::AcqRel) {
            unsafe {
                if PostMessageW(self.handle(), WM_APP + 2, 0, 0) == 0 {
                    self.cursor_posted.store(false, Ordering::Release);
                }
            }
        }
    }
    pub fn set_fullscreen(&self, enable: bool) {
        if self.closing.load(Ordering::Acquire) {
            return;
        }
        // One outstanding UI message, with the latest requested state.
        if self
            .fullscreen_pending
            .swap(if enable { 2 } else { 1 }, Ordering::AcqRel)
            == 0
        {
            unsafe {
                if PostMessageW(self.handle(), WM_APP + 3, 0, 0) == 0 {
                    self.fullscreen_pending.store(0, Ordering::Release);
                }
            }
        }
    }
    pub fn wake_lock_snapshot(&self) -> crate::wake_lock::State {
        self.wake_lock
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    // Only window_proc and the UI thread exit path call this method.
    fn apply_wake_lock(&self, requested: bool, visible: bool) {
        self.wake_lock
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .update(requested, visible);
    }
    pub fn set_wake_lock(&self, enable: bool) {
        if self.closing.load(Ordering::Acquire) || !self.online {
            return;
        }
        if self
            .wake_pending
            .swap(if enable { 2 } else { 1 }, Ordering::AcqRel)
            == 0
        {
            unsafe {
                if PostMessageW(self.handle(), WM_APP + 4, 0, 0) == 0 {
                    self.wake_pending.store(0, Ordering::Release);
                }
            }
        }
    }
    fn push(&self, event: Event) {
        self.events.push(event);
    }
    fn raw_mouse_motion(&self, mouse: &RAWMOUSE) {
        if self.raw_mouse_active() && mouse.usFlags & MOUSE_MOVE_ABSOLUTE == 0 {
            self.push(Event::RelativeMotion(mouse.lLastX, mouse.lLastY));
        }
    }
}

unsafe fn create_window(state: &Arc<Window>, opengl_required: bool) -> Result<HWND> {
    let instance = GetModuleHandleW(std::ptr::null());
    let name: Vec<u16> = "ParsecWebTurnNative\0".encode_utf16().collect();
    let class = WNDCLASSW {
        style: CS_OWNDC | CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
        hIcon: LoadIconW(instance, std::ptr::without_provenance(1)),
        lpszClassName: name.as_ptr(),
        ..std::mem::zeroed()
    };
    if RegisterClassW(&class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
        bail!("RegisterClassW failed: {}", GetLastError());
    }
    let title: Vec<u16> = crate::APP_TITLE.encode_utf16().chain([0]).collect();
    let hwnd = CreateWindowExW(
        0,
        name.as_ptr(),
        title.as_ptr(),
        WS_OVERLAPPEDWINDOW | WS_VISIBLE | WS_CLIPCHILDREN,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        1040,
        760,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        instance,
        Arc::as_ptr(state).cast(),
    );
    if hwnd.is_null() {
        bail!("CreateWindowExW failed: {}", GetLastError());
    }
    if opengl_required {
        let dc = GetDC(hwnd);
        let descriptor = PIXELFORMATDESCRIPTOR {
            nSize: std::mem::size_of::<PIXELFORMATDESCRIPTOR>() as u16,
            nVersion: 1,
            dwFlags: PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER,
            iPixelType: PFD_TYPE_RGBA,
            cColorBits: 32,
            cAlphaBits: 8,
            ..std::mem::zeroed()
        };
        let format = ChoosePixelFormat(dc, &descriptor);
        let mut actual: PIXELFORMATDESCRIPTOR = std::mem::zeroed();
        let described = DescribePixelFormat(
            dc,
            format,
            std::mem::size_of_val(&actual) as u32,
            &mut actual,
        );
        let configured =
            format != 0 && described != 0 && SetPixelFormat(dc, format, &descriptor) != 0;
        ReleaseDC(hwnd, dc);
        if !configured || actual.dwFlags & PFD_GENERIC_FORMAT != 0 {
            DestroyWindow(hwnd);
            bail!("A native accelerated OpenGL pixel format is required; no software fallback");
        }
    }
    let mut rect: RECT = std::mem::zeroed();
    GetClientRect(hwnd, &mut rect);
    *state.dimensions.lock().unwrap_or_else(|e| e.into_inner()) = (rect.right, rect.bottom);
    Ok(hwnd)
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if message == WM_NCCREATE {
        let create = &*(lp as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
    }
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Window;
    if !ptr.is_null() {
        let s = &*ptr;
        let x = lp as i16 as i32;
        let y = (lp >> 16) as i16 as i32;
        match message {
            crate::updater_ui::CLOSED => {
                let _ = s
                    .about_page
                    .compare_exchange(wp, 0, Ordering::AcqRel, Ordering::Acquire);
                let _ = s
                    .updates_page
                    .compare_exchange(wp, 0, Ordering::AcqRel, Ordering::Acquire);
                return 0;
            }
            crate::updater_ui::START => {
                SetTimer(hwnd, crate::updater_ui::NOTIFY_TIMER, 1000, None);
                return 0;
            }
            WM_TIMER if wp == crate::updater_ui::NOTIFY_TIMER => {
                if !s.closing.load(Ordering::Acquire) && s.updates.take_notification() {
                    s.open_updates(false, false);
                }
                if s.updates.view().phase != crate::updater::Phase::Checking {
                    KillTimer(hwnd, crate::updater_ui::NOTIFY_TIMER);
                }
                return 0;
            }
            WM_COMMAND if wp & 0xffff == crate::updater_ui::OPEN => {
                s.open_updates(true, true);
                return 0;
            }
            m if m == WM_APP + 10 => {
                let _ = s
                    .stats_page
                    .compare_exchange(wp, 0, Ordering::AcqRel, Ordering::Acquire);
                return 0;
            }
            WM_COMMAND if wp & 0xffff == crate::stats_ui::OPEN => {
                s.open_stats();
                return 0;
            }
            m if m == WM_APP + 8 => {
                crate::connection_settings_ui::install_menu(hwnd);
                return 0;
            }
            m if m == WM_APP + 9 => {
                let page = s.settings_page.swap(0, Ordering::AcqRel) as HWND;
                if !page.is_null() {
                    // Keep GL presentation suspended until the GDI child and
                    // all its controls have actually finished destruction.
                    DestroyWindow(page);
                    s.settings_active.store(false, Ordering::Release);
                    s.gui_visibility.cover(false);
                    SetFocus(hwnd);
                    s.show_video_surface();
                    s.redraw_client();
                }
                return 0;
            }
            m if m == WM_APP + 15 => {
                let page = s.settings_page.load(Ordering::Acquire) as HWND;
                if !page.is_null() && s.settings_open() {
                    RedrawWindow(
                        page,
                        std::ptr::null(),
                        std::ptr::null_mut(),
                        RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN | RDW_UPDATENOW,
                    );
                }
                return 0;
            }
            WM_COMMAND if wp & 0xffff == crate::connection_settings_ui::OPEN => {
                s.open_connection_settings();
                return 0;
            }
            WM_COMMAND if wp & 0xffff == crate::connection_settings_ui::ABOUT => {
                s.open_about();
                return 0;
            }
            WM_KEYDOWN
                if wp as u32 == VK_OEM_COMMA as u32
                    && GetKeyState(VK_CONTROL as i32) < 0
                    && !s.core_owns_key("Comma", modifiers()) =>
            {
                if lp & (1 << 30) == 0 {
                    s.open_connection_settings();
                }
                return 0;
            }
            WM_GETMINMAXINFO if s.settings_page.load(Ordering::Acquire) != 0 => {
                let limits = &mut *(lp as *mut MINMAXINFO);
                limits.ptMinTrackSize.x = 900;
                limits.ptMinTrackSize.y = 600;
                return 0;
            }
            WM_CLOSE => {
                s.input_suspended.store(true, Ordering::Release);
                #[cfg(feature = "usb-mouse-probe")]
                s.apply_hid_capture(false);
                s.keyboard_grab
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .set(hwnd, false);
                s.apply_relative_mouse(false);
                s.apply_wake_lock(false, false);
                s.request_stop();
                return 0;
            }
            m if m == WM_APP + 1 => {
                DestroyWindow(hwnd);
                return 0;
            }
            m if m == WM_APP + 2 => {
                s.cursor_posted.store(false, Ordering::Release);
                let pending = std::mem::take(
                    &mut *s.cursor_pending.lock().unwrap_or_else(|e| e.into_inner()),
                );
                s.cursor
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .apply(pending);
                // Refresh stationary pointers too, without changing another
                // window's cursor or the system resize/titlebar cursors.
                let mut point: POINT = std::mem::zeroed();
                if GetCursorPos(&mut point) != 0 && WindowFromPoint(point) == hwnd {
                    ScreenToClient(hwnd, &mut point);
                    let mut rect: RECT = std::mem::zeroed();
                    GetClientRect(hwnd, &mut rect);
                    if point.x >= rect.left
                        && point.x < rect.right
                        && point.y >= rect.top
                        && point.y < rect.bottom
                    {
                        s.select_mouse_cursor();
                    }
                }
                return 0;
            }
            WM_SETCURSOR if lp as u16 as u32 == HTCLIENT => {
                s.select_mouse_cursor();
                return 1;
            }
            #[cfg(feature = "usb-mouse-probe")]
            m if m == crate::hid_cursor::REFRESH => {
                s.hid_cursor_posted.store(false, Ordering::Release);
                s.apply_input_capture();
                return 0;
            }
            m if m == WM_APP + 3 => {
                let pending = s.fullscreen_pending.swap(0, Ordering::AcqRel);
                if pending != 0 {
                    let mut fullscreen = s.fullscreen.lock().unwrap_or_else(|e| e.into_inner());
                    // Like a rejected browser fullscreen request, an OS failure
                    // leaves the guest running and reports the retained state.
                    if fullscreen.set(hwnd, pending == 2).is_err() {
                        crate::event_log::record(
                            "WARN",
                            "fullscreen_change_failed",
                            serde_json::json!({"requested":pending == 2}),
                        );
                    }
                    let active = fullscreen.active();
                    crate::event_log::record(
                        "INFO",
                        "fullscreen_state",
                        serde_json::json!({"active":active}),
                    );
                    s.fullscreen_active.store(active, Ordering::Release);
                    s.push(Event::Fullscreen(active));
                }
                return 0;
            }
            m if m == WM_APP + 4 => {
                let pending = s.wake_pending.swap(0, Ordering::AcqRel);
                if pending != 0 {
                    s.apply_wake_lock(
                        pending == 2 && !s.closing.load(Ordering::Acquire),
                        IsIconic(hwnd) == 0,
                    );
                }
                return 0;
            }
            m if m == WM_APP + 5 => {
                if s.video_hwnd.load(Ordering::Acquire) == 0 && !s.closing.load(Ordering::Acquire) {
                    let class: Vec<u16> = "ParsecNativeVideoSurface\0".encode_utf16().collect();
                    let (width, height) = *s.dimensions.lock().unwrap_or_else(|e| e.into_inner());
                    // Disabled child receives no input: parent retains its
                    // documented keyboard/mouse handling and window shortcuts.
                    let video_class = WNDCLASSW {
                        lpfnWndProc: Some(video_surface_proc),
                        hInstance: GetModuleHandleW(std::ptr::null()),
                        lpszClassName: class.as_ptr(),
                        ..std::mem::zeroed()
                    };
                    RegisterClassW(&video_class);
                    let child = CreateWindowExW(
                        0,
                        class.as_ptr(),
                        std::ptr::null(),
                        WS_CHILD | WS_DISABLED,
                        0,
                        0,
                        width.max(1),
                        height.max(1),
                        hwnd,
                        std::ptr::null_mut(),
                        GetModuleHandleW(std::ptr::null()),
                        std::ptr::null(),
                    );
                    s.video_create_failed
                        .store(child.is_null(), Ordering::Release);
                    s.video_hwnd.store(child as usize, Ordering::Release);
                }
                return 0;
            }
            m if m == WM_APP + 6 => {
                s.show_video_surface();
                return 0;
            }
            m if m == WM_APP + 7 => {
                let pending = s.relative_pending.swap(0, Ordering::AcqRel);
                if pending != 0 {
                    s.apply_input_capture();
                    s.restore_host_cursor();
                }
                return 0;
            }
            m if m == WM_APP + 12 => {
                s.apply_input_capture();
                return 0;
            }
            m if m == WM_APP + 14 => {
                let surface = {
                    let mut pending = s.software_pending.lock().unwrap_or_else(|e| e.into_inner());
                    pending.posted = false;
                    pending.surface.take()
                };
                let mut presenter = s
                    .software_presenter
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                let size = *s.dimensions.lock().unwrap_or_else(|e| e.into_inner());
                if s.video_ready.load(Ordering::Acquire)
                    && s.video_visible.load(Ordering::Acquire)
                    && IsIconic(hwnd) == 0
                    && !s.settings_open()
                    && !s.closing.load(Ordering::Acquire)
                {
                    if let Some(surface) =
                        surface.filter(|surface| surface.size == (size.0 as u32, size.1 as u32))
                    {
                        if !presenter.show(hwnd, &surface) {
                            s.overlay
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .report
                                .failure = Some("software-gui-composition-failed");
                        }
                    } else {
                        presenter.hide();
                    }
                } else {
                    presenter.hide();
                }
                return 0;
            }
            m if m == WM_APP + 13 => {
                if let Some(code) =
                    crate::input::code(wp as u32, ((lp >> 16) & 255) as u8, lp & (1 << 24) != 0)
                {
                    let down = lp & (1 << 31) == 0;
                    let mut pressed = s.pressed_keys.lock().unwrap_or_else(|e| e.into_inner());
                    if down {
                        pressed.insert(code);
                    } else {
                        pressed.remove(code);
                    }
                    let mods = (lp & 15) as i32;
                    s.push(Event::Key(down, code, mods));
                }
                return 0;
            }
            WM_INPUT if s.raw_mouse_active() => {
                let mut raw: RAWINPUT = std::mem::zeroed();
                let mut size = std::mem::size_of::<RAWINPUT>() as u32;
                let read = GetRawInputData(
                    lp as HRAWINPUT,
                    RID_INPUT,
                    (&mut raw as *mut RAWINPUT).cast(),
                    &mut size,
                    std::mem::size_of::<RAWINPUTHEADER>() as u32,
                );
                if read != u32::MAX
                    && raw.header.dwType == RIM_TYPEMOUSE
                    && read
                        >= std::mem::size_of::<RAWINPUTHEADER>() as u32
                            + std::mem::size_of::<RAWMOUSE>() as u32
                {
                    s.raw_mouse_motion(&raw.data.mouse);
                }
                // DefWindowProc releases the foreground WM_INPUT bookkeeping.
            }
            WM_KEYDOWN
                if wp as u32 == VK_F8 as u32
                    && s.video_hwnd.load(Ordering::Acquire) != 0
                    && !s.core_owns_key("F8", modifiers()) =>
            {
                if lp & (1 << 30) == 0 {
                    s.apply_relative_mouse(false);
                    s.video_visible.fetch_xor(true, Ordering::AcqRel);
                    s.show_video_surface();
                }
                return 0;
            }
            WM_KEYUP
                if wp as u32 == VK_F8 as u32
                    && s.video_hwnd.load(Ordering::Acquire) != 0
                    && !s.core_owns_key("F8", modifiers()) =>
            {
                return 0
            }
            WM_KEYDOWN if wp as u32 == VK_F11 as u32 && !s.core_owns_key("F11", modifiers()) => {
                if lp & (1 << 30) == 0 {
                    s.set_fullscreen(!s.fullscreen_active.load(Ordering::Acquire));
                }
                return 0;
            }
            WM_KEYUP if wp as u32 == VK_F11 as u32 && !s.core_owns_key("F11", modifiers()) => {
                return 0
            }
            WM_SIZE => {
                s.software_presenter
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .hide();
                let requested =
                    s.wake_lock_snapshot().requested && !s.closing.load(Ordering::Acquire);
                s.apply_wake_lock(requested, wp as u32 != SIZE_MINIMIZED);
                let w = (lp as u32 & 0xffff) as i32;
                let h = ((lp as u32 >> 16) & 0xffff) as i32;
                *s.dimensions.lock().unwrap_or_else(|e| e.into_inner()) = (w, h);
                let page = s.settings_page.load(Ordering::Acquire) as HWND;
                if !page.is_null() {
                    SetWindowPos(
                        page,
                        std::ptr::null_mut(),
                        0,
                        0,
                        w.max(1),
                        h.max(1),
                        SWP_NOACTIVATE | SWP_NOZORDER,
                    );
                }
                let child = s.video_hwnd.load(Ordering::Acquire) as HWND;
                if !child.is_null() {
                    SetWindowPos(
                        child,
                        std::ptr::null_mut(),
                        0,
                        0,
                        w.max(1),
                        h.max(1),
                        SWP_NOACTIVATE | SWP_NOZORDER,
                    );
                }
                s.push(Event::Size(w, h));
                s.apply_input_capture();
            }
            WM_MOVE => {
                s.software_presenter
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .hide();
                s.apply_input_capture();
            }
            WM_SETFOCUS => {
                s.has_focus.store(true, Ordering::Release);
                // Foreground activation can still be in progress while this
                // synchronous focus notification is being dispatched.
                PostMessageW(hwnd, WM_APP + 12, 0, 0);
                s.push(Event::Focus(true));
            }
            WM_ACTIVATE if wp & 0xffff != WA_INACTIVE as usize => {
                PostMessageW(hwnd, WM_APP + 12, 0, 0);
            }
            WM_ENTERMENULOOP | WM_ENTERSIZEMOVE => {
                s.input_suspended.store(true, Ordering::Release);
                s.apply_input_capture();
            }
            WM_EXITMENULOOP | WM_EXITSIZEMOVE => {
                s.input_suspended.store(false, Ordering::Release);
                s.apply_input_capture();
            }
            WM_KILLFOCUS => {
                s.has_focus.store(false, Ordering::Release);
                #[cfg(feature = "usb-mouse-probe")]
                s.apply_hid_capture(false);
                s.keyboard_grab
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .set(hwnd, false);
                s.apply_relative_mouse(false);
                s.release_buttons();
                let mut pressed = s.pressed_keys.lock().unwrap_or_else(|e| e.into_inner());
                for code in std::mem::take(&mut *pressed) {
                    s.push(Event::Key(false, code, 0));
                }
                s.push(Event::Focus(false));
                *s.text_decoder.lock().unwrap_or_else(|e| e.into_inner()) = Default::default();
            }
            WM_CAPTURECHANGED => s.release_buttons(),
            WM_MOUSEMOVE if !s.raw_mouse_active() => {
                let echo = s.warp_echo.swap(0, Ordering::AcqRel);
                if echo == 0 || echo - 1 != (u64::from(x as u32) << 32 | u64::from(y as u32)) {
                    s.push(Event::Motion(x, y));
                }
            }
            WM_LBUTTONDOWN | WM_LBUTTONUP => s.mouse_button(message == WM_LBUTTONDOWN, 0, x, y),
            WM_MBUTTONDOWN | WM_MBUTTONUP => {
                s.mouse_button(message == WM_MBUTTONDOWN, 1, x, y);
            }
            WM_XBUTTONDOWN | WM_XBUTTONUP => {
                s.mouse_button(
                    message == WM_XBUTTONDOWN,
                    if ((wp >> 16) & 0xffff) == 1 { 3 } else { 4 },
                    x,
                    y,
                );
                return 1;
            }
            WM_RBUTTONDOWN | WM_RBUTTONUP => s.mouse_button(message == WM_RBUTTONDOWN, 2, x, y),
            WM_KEYDOWN | WM_KEYUP | WM_SYSKEYDOWN | WM_SYSKEYUP => {
                if let Some(code) =
                    crate::input::code(wp as u32, ((lp >> 16) & 0xFF) as u8, lp & (1 << 24) != 0)
                {
                    let down = matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN);
                    let mut pressed = s.pressed_keys.lock().unwrap_or_else(|e| e.into_inner());
                    if down {
                        pressed.insert(code);
                    } else {
                        pressed.remove(code);
                    }
                    let mods = modifiers()
                        | if pressed.contains("MetaLeft") || pressed.contains("MetaRight") {
                            8
                        } else {
                            0
                        };
                    s.push(Event::Key(down, code, mods));
                }
                // Keep system handling (notably Alt+F4) in DefWindowProc.
                if matches!(message, WM_KEYDOWN | WM_KEYUP)
                    || s.keyboard_grab
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .active()
                {
                    return 0;
                }
            }
            WM_CHAR => {
                if let Some(ch) = s
                    .text_decoder
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(wp as u16)
                {
                    s.push(Event::Text(ch as u32));
                }
                return 0;
            }
            WM_MOUSEWHEEL => {
                s.push(Event::Scroll(0, -((wp >> 16) as i16 as i32)));
                return 0;
            }
            WM_MOUSEHWHEEL => {
                s.push(Event::Scroll((wp >> 16) as i16 as i32, 0));
                return 0;
            }
            WM_DESTROY => {
                KillTimer(hwnd, crate::updater_ui::NOTIFY_TIMER);
                s.input_suspended.store(true, Ordering::Release);
                #[cfg(feature = "usb-mouse-probe")]
                s.apply_hid_capture(false);
                s.software_presenter
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .destroy();
                s.keyboard_grab
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .set(hwnd, false);
                s.apply_relative_mouse(false);
                s.apply_wake_lock(false, false);
                // Cursor resources are created, replaced and freed on this UI
                // thread. The remaining Arc state owns no native cursor handle.
                *s.cursor.lock().unwrap_or_else(|e| e.into_inner()) = Default::default();
                *s.cursor_pending.lock().unwrap_or_else(|e| e.into_inner()) = Default::default();
                PostQuitMessage(0);
                return 0;
            }
            _ => {}
        }
    }
    DefWindowProcW(hwnd, message, wp, lp)
}

unsafe fn modifiers() -> i32 {
    let mut mods = 0;
    for (vk, bit) in [(VK_SHIFT, 1), (VK_CONTROL, 2), (VK_MENU, 4)] {
        if GetKeyState(vk as i32) < 0 {
            mods |= bit;
        }
    }
    if GetKeyState(VK_LWIN as i32) < 0 || GetKeyState(VK_RWIN as i32) < 0 {
        mods |= 8;
    }
    if GetKeyState(VK_CAPITAL as i32) & 1 != 0 {
        mods |= 16;
    }
    if GetKeyState(VK_NUMLOCK as i32) & 1 != 0 {
        mods |= 32;
    }
    mods
}

// DXGI owns video pixels; do not repaint the child with GDI/static-control text.
unsafe extern "system" fn video_surface_proc(
    hwnd: HWND,
    message: u32,
    wp: WPARAM,
    lp: LPARAM,
) -> LRESULT {
    match message {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let mut paint: PAINTSTRUCT = std::mem::zeroed();
            BeginPaint(hwnd, &mut paint);
            EndPaint(hwnd, &paint);
            0
        }
        _ => DefWindowProcW(hwnd, message, wp, lp),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "usb-mouse-probe")]
    #[test]
    fn hid_cursor_follows_host_without_feedback_and_restores_local_menus() {
        struct Restore {
            clip: RECT,
            point: POINT,
            foreground: usize,
        }
        impl Drop for Restore {
            fn drop(&mut self) {
                unsafe {
                    ClipCursor(&self.clip);
                    SetCursorPos(self.point.x, self.point.y);
                    SetForegroundWindow(self.foreground as HWND);
                }
            }
        }
        let _restore = unsafe {
            let mut clip = RECT::default();
            let mut point = POINT::default();
            assert_ne!(GetClipCursor(&mut clip), 0);
            assert_ne!(GetCursorPos(&mut point), 0);
            Restore {
                clip,
                point,
                foreground: GetForegroundWindow() as usize,
            }
        };
        let window = Window::create_mode(false, false, false, false, false).unwrap();
        #[track_caller]
        fn wait(mut ready: impl FnMut() -> bool) {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !ready() {
                assert!(
                    std::time::Instant::now() < until,
                    "HID cursor UI did not settle"
                );
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            unsafe {
                SetWindowPos(
                    window.handle(),
                    std::ptr::null_mut(),
                    0,
                    0,
                    900,
                    600,
                    SWP_NOZORDER,
                );
                SetForegroundWindow(window.handle());
            }
            wait(|| unsafe { GetForegroundWindow() == window.handle() });
            let generation = window.begin_video();
            *window.video_report.lock().unwrap() = Some(crate::video_output::Snapshot {
                width: Some(1920),
                height: Some(1080),
                frames_presented: 1,
                ..Default::default()
            });
            window.set_video_ready_for(generation, true);
            window.set_host_cursor(960, 540, false, false);
            window.set_usb_cursor_active(generation, true);
            wait(|| window.hid_raw_mouse.load(Ordering::Acquire));
            assert!(
                !window.relative_mouse.load(Ordering::Acquire),
                "HID capture must not change core relative mode"
            );
            let at_host = |x, y| unsafe {
                let (x, y) = window.presented_viewport().unwrap().unmap(x, y).unwrap();
                let mut expected = POINT { x, y };
                ClientToScreen(window.handle(), &mut expected);
                let mut actual = POINT::default();
                GetCursorPos(&mut actual);
                (actual.x, actual.y) == (expected.x, expected.y)
            };
            wait(|| at_host(960, 540));
            std::thread::sleep(std::time::Duration::from_millis(50));
            window.events.drain();
            for (x, y) in [(1000, 550), (1100, 520), (960, 540)] {
                window.set_host_cursor(x, y, false, false);
                wait(|| at_host(x, y));
                std::thread::sleep(std::time::Duration::from_millis(40));
                assert!(
                    !window
                        .events
                        .drain()
                        .iter()
                        .any(|e| matches!(e, Event::Motion(..) | Event::RelativeMotion(..))),
                    "host cursor positioning generated outgoing motion"
                );
            }
            window.raw_mouse_motion(&RAWMOUSE {
                lLastX: 7,
                lLastY: -4,
                ..Default::default()
            });
            assert_eq!(
                window
                    .events
                    .drain()
                    .iter()
                    .filter(|e| matches!(e, Event::RelativeMotion(7, -4)))
                    .count(),
                1
            );
            unsafe {
                SendMessageW(window.handle(), WM_MOUSEMOVE, 0, (200 << 16) | 200);
            }
            assert!(!window
                .events
                .drain()
                .iter()
                .any(|e| matches!(e, Event::Motion(..))));
            window.set_input_state(crate::core_input::State {
                detached: true,
                ..Default::default()
            });
            wait(|| {
                !window.hid_raw_mouse.load(Ordering::Acquire)
                    && !window.mouse_clipped.load(Ordering::Acquire)
            });
            assert!(window
                .events
                .drain()
                .iter()
                .any(|e| matches!(e, Event::HidMouseRelease)));
            unsafe {
                SendMessageW(window.handle(), WM_MOUSEMOVE, 0, (200 << 16) | 200);
            }
            assert!(
                window
                    .events
                    .drain()
                    .iter()
                    .any(|e| matches!(e, Event::Motion(200, 200))),
                "local menu input was lost"
            );
            window.set_input_state(Default::default());
            wait(|| window.hid_raw_mouse.load(Ordering::Acquire));
            unsafe {
                SendMessageW(window.handle(), WM_ENTERMENULOOP, 0, 0);
            }
            assert!(!window.hid_raw_mouse.load(Ordering::Acquire));
            unsafe {
                SendMessageW(window.handle(), WM_EXITMENULOOP, 0, 0);
            }
            wait(|| window.hid_raw_mouse.load(Ordering::Acquire));
            window.set_usb_cursor_active(generation, false);
            wait(|| {
                !window.hid_raw_mouse.load(Ordering::Acquire)
                    && !window.mouse_clipped.load(Ordering::Acquire)
            });
            let new = window.begin_video();
            window.set_usb_cursor_active(generation, true);
            assert!(!window.hid_cursor_active.load(Ordering::Acquire));
            window.set_usb_cursor_active(new, true);
            window.set_usb_cursor_active(generation, false);
            assert!(window.hid_cursor_active.load(Ordering::Acquire));
        }));
        if result.is_err() {
            unsafe {
                let mut point = POINT::default();
                GetCursorPos(&mut point);
                let mut clip = RECT::default();
                GetClipCursor(&mut clip);
                eprintln!("HID cursor test state: foreground={}, ready={}, visible={}, hid={}, raw={}, suspended={}, detached={}, position=({},{}), clip=({},{},{},{}), dimensions={:?}, host={:?}",
                GetForegroundWindow()==window.handle(), window.video_ready.load(Ordering::Acquire),
                window.video_visible.load(Ordering::Acquire),window.hid_cursor_active.load(Ordering::Acquire),
                window.hid_raw_mouse.load(Ordering::Acquire),window.input_suspended.load(Ordering::Acquire),
                window.input_state.lock().unwrap().detached,point.x,point.y,clip.left,clip.top,clip.right,clip.bottom,
                *window.dimensions.lock().unwrap(),*window.host_cursor.lock().unwrap());
            }
        }
        window.close();
        result.unwrap();
    }
    #[test]
    #[ignore = "requires an accelerated OpenGL desktop; run the isolated GUI harness explicitly"]
    fn idle_core_gui_presents_after_back_without_extra_input() {
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce).unwrap();
        let suffix: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
        let path = std::env::temp_dir().join(format!("parsec-settings-redraw-{suffix}.json"));
        assert!(!path.exists());
        // Original embedded core, anonymous in-memory filesystem, networking
        // disabled. No production profile, credentials or host are involved.
        let window = Window::create_mode(false, true, false, false, true).unwrap();
        window.install_connection_settings(crate::connection_settings::Manager::open(path.clone()));
        let worker_window = window.clone();
        let runtime = std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(move || -> anyhow::Result<()> {
                let engine = wasmtime::Engine::new(&crate::core_config::config())?;
                let module = crate::core_runtime::load(&engine)?;
                let (mut store, instance) =
                    crate::instantiate_mode(&engine, &module, Some(worker_window.clone()))?;
                let startup = instance
                    .get_typed_func::<(), ()>(&mut store, "_start")?
                    .call(&mut store, ());
                let outcome = if store.data().event_loop.is_some() {
                    crate::desktop::run(&mut store, &instance)
                } else {
                    startup
                };
                store
                    .data()
                    .backend
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .destroy();
                worker_window.request_stop();
                engine.increment_epoch();
                if let Some(runtime) = &store.data().threads {
                    runtime.http.shutdown();
                    runtime.websocket.shutdown();
                }
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
                while worker_window.active_contexts.load(Ordering::Acquire) != 0
                    && std::time::Instant::now() < deadline
                {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                anyhow::ensure!(
                    worker_window.active_contexts.load(Ordering::Acquire) == 0,
                    "GUI context did not stop"
                );
                worker_window.close();
                match outcome {
                    Err(error) if crate::lifecycle::is_cancellation(&error) => Ok(()),
                    other => other,
                }
            })
            .unwrap();
        let frames = || {
            window
                .graphics
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .map_or(0, |g| g.frames_presented)
        };
        let wait = |ready: &dyn Fn() -> bool, seconds: u64| -> anyhow::Result<()> {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
            while !ready() {
                anyhow::ensure!(!runtime.is_finished(), "Core GUI stopped unexpectedly");
                anyhow::ensure!(
                    std::time::Instant::now() < deadline,
                    "Core GUI transition timed out"
                );
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Ok(())
        };
        let result = (|| -> anyhow::Result<()> {
            wait(&|| frames() > 0, 10)?;
            for _ in 0..3 {
                unsafe {
                    anyhow::ensure!(
                        PostMessageW(
                            window.handle(),
                            WM_COMMAND,
                            crate::connection_settings_ui::OPEN,
                            0
                        ) != 0
                    );
                }
                wait(
                    &|| {
                        window.settings_page.load(Ordering::Acquire) != 0
                            && !window.input_suspended.load(Ordering::Acquire)
                    },
                    5,
                )?;
                let page = window.settings_page.load(Ordering::Acquire) as HWND;
                let size = *window.dimensions.lock().unwrap_or_else(|e| e.into_inner());
                // Let an idle GUI observe coverage; Back must produce a real
                // SwapBuffers afterward, not merely enqueue a notification.
                std::thread::sleep(std::time::Duration::from_millis(250));
                let before = frames();
                unsafe {
                    let class: Vec<u16> = "BUTTON\0".encode_utf16().collect();
                    let label: Vec<u16> = "Back\0".encode_utf16().collect();
                    let back =
                        FindWindowExW(page, std::ptr::null_mut(), class.as_ptr(), label.as_ptr());
                    anyhow::ensure!(!back.is_null());
                    SendMessageW(page, WM_COMMAND, GetDlgCtrlID(back) as usize, back as isize);
                }
                // No mouse, keyboard, activation or size event is injected.
                wait(&|| !window.settings_open() && frames() > before, 3)?;
                anyhow::ensure!(unsafe { IsWindow(page) } == 0);
                anyhow::ensure!(
                    *window.dimensions.lock().unwrap_or_else(|e| e.into_inner()) == size
                );
            }
            anyhow::ensure!(!path.exists());
            Ok(())
        })();
        window.request_stop();
        let finished = runtime.join().unwrap();
        if window.active_contexts.load(Ordering::Acquire) == 0 {
            window.close();
        }
        result.unwrap();
        finished.unwrap();
    }

    #[test]
    #[ignore = "foreground desktop test; run separately so other window tests cannot steal focus"]
    fn immersive_native_capture_lifecycle() {
        struct Restore {
            foreground: usize,
            clip: RECT,
            cursor: POINT,
        }
        impl Drop for Restore {
            fn drop(&mut self) {
                unsafe {
                    ClipCursor(&self.clip);
                    SetCursorPos(self.cursor.x, self.cursor.y);
                    SetForegroundWindow(self.foreground as HWND);
                }
            }
        }
        let _restore = unsafe {
            let mut clip = RECT::default();
            let mut cursor = POINT::default();
            assert_ne!(GetClipCursor(&mut clip), 0);
            assert_ne!(GetCursorPos(&mut cursor), 0);
            Restore {
                foreground: GetForegroundWindow() as usize,
                clip,
                cursor,
            }
        };
        let window = Window::create_mode(false, false, false, false, false).unwrap();
        #[track_caller]
        fn wait(mut check: impl FnMut() -> bool) {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while !check() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "native capture did not settle"
                );
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            unsafe {
                SetForegroundWindow(window.handle());
            }
            wait(|| unsafe { GetForegroundWindow() == window.handle() });
            let mode = |mouse, keyboard| crate::core_input::State {
                mode: crate::core_input::Mode { mouse, keyboard },
                ..Default::default()
            };
            let clipped = || window.mouse_clipped.load(Ordering::Acquire);
            let keyboard = || window.keyboard_grab.lock().unwrap().active();
            let relative = || window.relative_mouse.load(Ordering::Acquire);
            window.set_video_ready(true);
            window.set_input_state(mode(true, false));
            wait(|| clipped() && !keyboard() && !relative());
            unsafe {
                let mut actual = RECT::default();
                GetClipCursor(&mut actual);
                let mut origin = POINT::default();
                ClientToScreen(window.handle(), &mut origin);
                let mut client = RECT::default();
                GetClientRect(window.handle(), &mut client);
                // ClipCursor intersects with the virtual desktop. The CI
                // display can be narrower than the default client window.
                let sx = GetSystemMetrics(SM_XVIRTUALSCREEN);
                let sy = GetSystemMetrics(SM_YVIRTUALSCREEN);
                assert_eq!(
                    (actual.left, actual.top, actual.right, actual.bottom),
                    (
                        origin.x.max(sx),
                        origin.y.max(sy),
                        (origin.x + client.right).min(sx + GetSystemMetrics(SM_CXVIRTUALSCREEN)),
                        (origin.y + client.bottom).min(sy + GetSystemMetrics(SM_CYVIRTUALSCREEN))
                    )
                );
            }
            window.set_input_state(mode(false, true));
            wait(|| !clipped() && keyboard());
            // Exercise the actual hook-to-window path without synthesizing global
            // OS input. Key-up must still arrive after Alt has been released.
            crate::keyboard_grab::test_reserved_pair(window.handle());
            let mut received = Vec::new();
            wait(|| {
                received.extend(window.events.drain());
                received
                    .iter()
                    .any(|e| matches!(e, Event::Key(false, "Tab", 0)))
            });
            assert!(received
                .iter()
                .any(|e| matches!(e, Event::Key(true, "Tab", 4))));
            window.set_input_state(mode(true, true));
            wait(|| clipped() && keyboard());
            window.set_relative_mouse(true);
            wait(relative);
            window.set_relative_mouse(false);
            wait(|| !relative() && clipped()); // Immersive Mouse survives raw unlock.
            window.set_input_state(mode(false, false));
            wait(|| !clipped() && !keyboard());
            // Native relative intent comes from host flags, not button state.
            unsafe {
                SendMessageW(window.handle(), WM_LBUTTONDOWN, 1, (100 << 16) | 100);
            }
            assert!(!relative() && !clipped());
            unsafe {
                SendMessageW(window.handle(), WM_LBUTTONUP, 0, (100 << 16) | 100);
            }
            wait(|| !clipped());
            window.set_relative_mouse(true);
            wait(|| relative() && clipped());
            window.set_input_state(crate::core_input::State {
                warp_disabled: true,
                ..mode(false, false)
            });
            wait(|| relative() && clipped()); // Warp controls return position only.
            window.set_relative_mouse(false);
            wait(|| !relative() && !clipped());
            *window.video_report.lock().unwrap() = Some(crate::video_output::Snapshot {
                width: Some(1920),
                height: Some(1080),
                frames_presented: 1,
                ..Default::default()
            });
            // SDK/lowlat truth table: either flag locks, both clear unlock.
            for (rel, hidden) in [(true, false), (false, true), (true, true)] {
                let mut wire = vec![0; 34];
                wire[12] = 9;
                wire[24..26].copy_from_slice(&960i16.to_be_bytes());
                wire[26..28].copy_from_slice(&540i16.to_be_bytes());
                let flags: u16 = if rel { 0x100 } else { 0 } | if hidden { 0x200 } else { 0 };
                wire[32..34].copy_from_slice(&flags.to_be_bytes());
                let crate::control::Message::Buffer { event, .. } =
                    crate::control::decode(&wire).unwrap()
                else {
                    panic!("not a cursor event");
                };
                window.host_cursor_event(&event);
                wait(|| relative() && clipped());
                unsafe {
                    let mut rect = RECT::default();
                    assert_ne!(GetClipCursor(&mut rect), 0);
                    assert_eq!((rect.right - rect.left, rect.bottom - rect.top), (1, 1));
                }
                // Browser callback must not cancel a hidden-only native request.
                window.set_relative_mouse(false);
                assert!(window.relative_requested.load(Ordering::Acquire));
                window.events.drain();
                for dx in [-400, 1] {
                    window.raw_mouse_motion(&RAWMOUSE {
                        lLastX: dx,
                        ..Default::default()
                    });
                }
                let motion: Vec<_> = window
                    .events
                    .drain()
                    .into_iter()
                    .filter_map(|e| {
                        if let Event::RelativeMotion(x, y) = e {
                            Some((x, y))
                        } else {
                            None
                        }
                    })
                    .collect();
                assert_eq!(motion, [(-400, 0), (1, 0)]); // Immediate reversal, no absolute dead zone.
                for (x, y) in motion {
                    let packet = crate::control::input(
                        &serde_json::json!({"type":4,"relative":true,"x":x,"y":y}),
                    )
                    .unwrap();
                    assert_eq!(packet, crate::control::header(3, 1, x, y));
                }
                let mut held = POINT::default();
                unsafe {
                    GetCursorPos(&mut held);
                }
                window.set_host_cursor(960, 540, false, false);
                wait(|| !relative() && !clipped());
                unsafe {
                    SendMessageW(window.handle(), WM_APP + 7, 0, 0);
                    let mut after = POINT::default();
                    GetCursorPos(&mut after);
                    assert_eq!((after.x, after.y), (held.x, held.y)); // Disabled warp leaves position alone.
                }
            }
            window.set_input_state(mode(false, false));
            window.set_host_cursor(960, 540, false, true);
            wait(relative);
            window.set_host_cursor(960, 540, false, false);
            wait(|| !relative());
            // Serialize with WM_APP+7: it applies unlock then warps through
            // the letterboxed viewport. No global input injection is needed.
            unsafe {
                SendMessageW(window.handle(), WM_APP + 7, 0, 0);
                let (x, y) = window
                    .presented_viewport()
                    .unwrap()
                    .unmap(960, 540)
                    .unwrap();
                let mut expected = POINT { x, y };
                ClientToScreen(window.handle(), &mut expected);
                let mut actual = POINT::default();
                GetCursorPos(&mut actual);
                assert_eq!((actual.x, actual.y), (expected.x, expected.y));
            }
            window.set_host_cursor(960, 540, false, true);
            wait(relative);
            window.set_input_state(mode(true, true));
            wait(|| relative() && clipped() && keyboard());
            window.set_input_state(crate::core_input::State {
                detached: true,
                ..mode(true, true)
            });
            wait(|| !relative() && !clipped() && !keyboard());
            window.set_relative_mouse(false); // Core detach cleanup must not lose the request.
            window.set_input_state(mode(true, true));
            wait(|| relative() && clipped() && keyboard());
            unsafe {
                ShowWindow(window.handle(), SW_MINIMIZE);
            }
            wait(|| !relative() && !clipped() && !keyboard());
            window.set_relative_mouse(false); // Same for focus-loss cleanup.
            unsafe {
                ShowWindow(window.handle(), SW_RESTORE);
                SetForegroundWindow(window.handle());
            }
            wait(|| relative() && clipped() && keyboard());
            unsafe {
                ShowWindow(window.handle(), SW_MINIMIZE);
            }
            wait(|| !relative() && !clipped() && !keyboard());
            window.set_host_cursor(100, 100, false, false);
            unsafe {
                ShowWindow(window.handle(), SW_RESTORE);
                SetForegroundWindow(window.handle());
            }
            wait(|| !relative() && clipped() && keyboard());
            window.end_session();
            wait(|| !relative() && !clipped() && !keyboard());
        }));
        if result.is_err() {
            eprintln!("capture state: requested={} relative={} clipped={} keyboard={} focused={} suspended={} iconic={} foreground={} input={:?}",
                window.relative_requested.load(Ordering::Acquire),window.relative_mouse.load(Ordering::Acquire),
                window.mouse_clipped.load(Ordering::Acquire),window.keyboard_grab.lock().unwrap().active(),
                window.has_focus.load(Ordering::Acquire),window.input_suspended.load(Ordering::Acquire),
                unsafe {IsIconic(window.handle())},unsafe {GetForegroundWindow()==window.handle()},
                *window.input_state.lock().unwrap());
        }
        window.close();
        wait(|| window.hwnd.load(Ordering::Acquire) == 0);
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    }
    #[test]
    fn worker_yield_returns_to_guest_cleanup_and_notifies_join_on_repeated_sessions() {
        let window = Window::create_mode(false, false, false, false, false).unwrap();
        unsafe {
            ShowWindow(window.handle(), SW_HIDE);
        }
        let result = (|| -> anyhow::Result<()> {
            let engine = wasmtime::Engine::new(&crate::core_config::config())?;
            // Models the relevant guest contract without an account, a codec,
            // audio hardware or private pthread-layout writes from the host.
            let module = wasmtime::Module::new(
                &engine,
                r#"(module
                (import "env" "memory" (memory 1 1 shared))
                (import "env" "web_run_and_yield" (func $yield (param i32 i32)))
                (table (export "__indirect_function_table") 1 funcref)
                (elem (i32.const 0) $tick)
                (func $tick (param i32) (result i32)
                    i32.const 12 i32.const 1 i32.atomic.rmw.add drop
                    i32.const 0 i32.atomic.load i32.eqz)
                (func (export "wasi_thread_start") (param i32 i32)
                    i32.const 0 i32.const 0 call $yield
                    ;; Real WASM audio/thread cleanup also follows the yield.
                    i32.const 4 i32.const 1 i32.atomic.store
                    i32.const 8 i32.const 0 i32.atomic.store
                    i32.const 8 i32.const 1 memory.atomic.notify drop))"#,
            )?;
            let (store, _) = crate::instantiate_mode(&engine, &module, Some(window.clone()))?;
            let runtime = store.data().threads.as_ref().unwrap();
            let memory = &runtime.memory;
            for _ in 0..3 {
                for (ptr, value) in [(0, 0), (4, 0), (8, 1), (12, 0)] {
                    memory.sync_word(ptr)?.store(value, Ordering::SeqCst);
                }
                let id = runtime.spawn(0);
                anyhow::ensure!(id > 0, "worker spawn failed");
                let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
                while memory.sync_word(12)?.load(Ordering::SeqCst) < 2 {
                    anyhow::ensure!(std::time::Instant::now() < until, "callback did not run");
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                memory.sync_word(0)?.store(1, Ordering::SeqCst);
                memory
                    .0
                    .atomic_wait32(8, 1, Some(std::time::Duration::from_secs(2)))?;
                anyhow::ensure!(
                    memory.sync_word(8)?.load(Ordering::SeqCst) == 0,
                    "guest join remained blocked after callback completion"
                );
                anyhow::ensure!(
                    memory.sync_word(4)?.load(Ordering::SeqCst) == 1,
                    "guest cleanup following yield was skipped"
                );
                loop {
                    let records = runtime.snapshot();
                    if let Some(record) = records.iter().find(|r| r.id == id && r.finished) {
                        anyhow::ensure!(record.error.is_none(), "guest worker failed");
                        break;
                    }
                    anyhow::ensure!(std::time::Instant::now() < until, "worker did not finish");
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
            }
            Ok(())
        })();
        window.close();
        result.unwrap();
    }
    #[test]
    fn disconnect_retires_video_and_old_workers_cannot_reveal_or_hide_new_session() {
        let window = Window::create_mode(false, false, false, false, false).unwrap();
        unsafe {
            ShowWindow(window.handle(), SW_HIDE);
        }
        let old = window.begin_video();
        window.set_video_ready_for(old, true);
        assert!(window.video_ready.load(Ordering::Acquire));
        window.end_session();
        assert!(!window.video_ready.load(Ordering::Acquire));
        window.set_video_ready_for(old, true);
        assert!(!window.video_ready.load(Ordering::Acquire));
        let new = window.begin_video();
        window.set_video_ready_for(new, true);
        window.finish_video(old);
        assert!(window.video_ready.load(Ordering::Acquire));
        window.finish_video(new);
        assert!(!window.video_ready.load(Ordering::Acquire));
        window.close();
    }
}
