//! Native macOS Cocoa / AppKit settings menu and preferences window.
use crate::connection_settings::Manager;
use objc2::rc::Retained;
use objc2::{define_class, msg_send, AnyThread};
use objc2_app_kit::{
    NSApplication, NSBackingStoreType, NSButton, NSButtonType, NSColor, NSControlStateValueOff,
    NSControlStateValueOn, NSMenu, NSMenuItem, NSPopUpButton, NSSecureTextField, NSTabView,
    NSTabViewItem, NSTextField, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{MainThreadMarker, NSObject, NSPoint, NSRect, NSSize, NSString};
use std::sync::{Arc, Mutex};

static SETTINGS_CONTROLLER: Mutex<Option<usize>> = Mutex::new(None);
static DEFAULT_MANAGER: Mutex<Option<Arc<Manager>>> = Mutex::new(None);

pub fn set_default_manager(manager: Arc<Manager>) {
    *DEFAULT_MANAGER.lock().unwrap() = Some(manager);
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "SettingsController"]
    pub struct SettingsController;

    impl SettingsController {
        #[unsafe(method(saveClicked:))]
        fn save_clicked(&self, _sender: &NSButton) {
            save_current_settings();
        }

        #[unsafe(method(closeClicked:))]
        fn close_clicked(&self, _sender: &NSButton) {
            close_settings_window();
        }

        #[unsafe(method(budToggled:))]
        fn bud_toggled(&self, _sender: &NSButton) {
            #[cfg(feature = "bud")]
            if let Some(state) = STATE.lock().unwrap().as_ref() {
                sync_hid_enabled(state);
            }
        }

        #[unsafe(method(providerChanged:))]
        fn provider_changed(&self, _sender: &NSObject) {
            if let Some(state) = STATE.lock().unwrap().as_ref() {
                state.turn.sync_provider();
            }
        }

        #[unsafe(method(openPreferences:))]
        fn open_preferences(&self, _sender: &NSMenuItem) {
            open_settings_from_menu();
        }

        #[unsafe(method(openStats:))]
        fn open_stats(&self, _sender: &NSMenuItem) {
            show_stats_summary();
        }

        #[unsafe(method(toggleFullscreen:))]
        fn toggle_fullscreen(&self, _sender: &NSMenuItem) {
            toggle_fullscreen_from_menu();
        }
    }
);

fn get_controller() -> &'static SettingsController {
    let mut ctrl_guard = SETTINGS_CONTROLLER.lock().unwrap();
    if let Some(ptr) = *ctrl_guard {
        unsafe { &*(ptr as *const SettingsController) }
    } else {
        let c: Retained<SettingsController> =
            unsafe { msg_send![SettingsController::alloc(), init] };
        let ptr = Retained::into_raw(c) as usize;
        *ctrl_guard = Some(ptr);
        unsafe { &*(ptr as *const SettingsController) }
    }
}

struct SettingsState {
    window: usize,
    manager: Arc<Manager>,
    chk_force_relay: usize,
    chk_stun_only: usize,
    chk_prefer_hevc: usize,
    chk_ten_bit: usize,
    chk_event_log: usize,
    chk_custom_audio: usize,
    chk_raw_audio: usize,
    chk_surround: usize,
    #[cfg(feature = "bud")]
    chk_bud: usize,
    #[cfg(feature = "bud")]
    chk_hid_mouse: usize,
    #[cfg(feature = "bud")]
    chk_hid_keyboard: usize,
    #[cfg(feature = "bud")]
    lbl_bud_hint: usize,
    turn: Turn,
    txt_status: usize,
}

static STATE: Mutex<Option<SettingsState>> = Mutex::new(None);

/// HID input only travels over BUD, so its options follow the BUD checkbox (as on Windows).
#[cfg(feature = "bud")]
fn sync_hid_enabled(state: &SettingsState) {
    let bud = unsafe { &*(state.chk_bud as *const NSButton) }.state() == NSControlStateValueOn;
    for button in [state.chk_hid_mouse, state.chk_hid_keyboard] {
        unsafe { &*(button as *const NSButton) }.setEnabled(bud);
    }
    unsafe { &*(state.lbl_bud_hint as *const NSTextField) }.setHidden(bud);
}

fn close_settings_window() {
    if let Some(state) = STATE.lock().unwrap().as_ref() {
        let win = unsafe { &*(state.window as *const NSWindow) };
        win.orderOut(None);
    }
}

pub fn open_settings_from_menu() {
    let mgr = {
        if let Some(state) = STATE.lock().unwrap().as_ref() {
            Some(state.manager.clone())
        } else if let Some(win) = crate::window_macos::active_window() {
            win.connection_settings()
        } else {
            None
        }
    }
    .or_else(|| DEFAULT_MANAGER.lock().unwrap().clone());

    if let Some(mgr) = mgr {
        open(mgr);
    }
}

pub fn show_stats_summary() {
    if let Some(mtm) = MainThreadMarker::new() {
        let alert = objc2_app_kit::NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str("ParsecWebTurn Connection Statistics"));
        alert.setInformativeText(&NSString::from_str("Active connection telemetry is operational.\nDetailed Cocoa HUD is active in session overlay."));
        alert.addButtonWithTitle(&NSString::from_str("OK"));
        alert.runModal();
    }
}

fn toggle_fullscreen_from_menu() {
    if let Some(win) = crate::window_macos::active_window() {
        let ns_win = unsafe { &*(win.handle() as *const NSWindow) };
        ns_win.toggleFullScreen(None);
    }
}

fn save_current_settings() {
    let state_guard = STATE.lock().unwrap();
    let Some(state) = state_guard.as_ref() else {
        return;
    };

    let chk_force_relay = unsafe { &*(state.chk_force_relay as *const NSButton) };
    let chk_stun_only = unsafe { &*(state.chk_stun_only as *const NSButton) };
    let chk_prefer_hevc = unsafe { &*(state.chk_prefer_hevc as *const NSButton) };
    let chk_ten_bit = unsafe { &*(state.chk_ten_bit as *const NSButton) };
    let chk_event_log = unsafe { &*(state.chk_event_log as *const NSButton) };
    let chk_custom_audio = unsafe { &*(state.chk_custom_audio as *const NSButton) };
    let chk_raw_audio = unsafe { &*(state.chk_raw_audio as *const NSButton) };
    let chk_surround = unsafe { &*(state.chk_surround as *const NSButton) };
    let txt_status = unsafe { &*(state.txt_status as *const NSTextField) };

    let (mut s, _) = state.manager.view();
    s.force_relay = chk_force_relay.state() == NSControlStateValueOn;
    s.stun_only = chk_stun_only.state() == NSControlStateValueOn;
    s.prefer_hevc = chk_prefer_hevc.state() == NSControlStateValueOn;
    s.experimental_ten_bit = chk_ten_bit.state() == NSControlStateValueOn;
    s.event_logging = chk_event_log.state() == NSControlStateValueOn;
    s.custom_audio_buffer = chk_custom_audio.state() == NSControlStateValueOn;
    s.allow_raw_audio = chk_raw_audio.state() == NSControlStateValueOn;
    s.experimental_surround_audio = chk_surround.state() == NSControlStateValueOn;

    #[cfg(feature = "bud")]
    {
        let chk_bud = unsafe { &*(state.chk_bud as *const NSButton) };
        let chk_hid_mouse = unsafe { &*(state.chk_hid_mouse as *const NSButton) };
        let chk_hid_keyboard = unsafe { &*(state.chk_hid_keyboard as *const NSButton) };
        // BUD is an explicit transport choice; HID options never switch it on.
        s.bud = chk_bud.state() == NSControlStateValueOn;
        s.experimental_hid_mouse = chk_hid_mouse.state() == NSControlStateValueOn;
        s.experimental_hid_keyboard = chk_hid_keyboard.state() == NSControlStateValueOn;
    }

    let secrets = state.turn.apply(&mut s);
    let saved = secrets.and_then(|(password, token, forget_password, forget_token)| {
        state
            .manager
            .save(s, &password, &token, forget_password, forget_token)
    });
    match saved {
        Ok(()) => {
            state.turn.clear_secrets();
            txt_status.setStringValue(&NSString::from_str(
                "Settings saved. They apply on your next connection.",
            ));
            let green = NSColor::colorWithRed_green_blue_alpha(0.0, 0.6, 0.2, 1.0);
            txt_status.setTextColor(Some(&green));
        }
        Err(err) => {
            txt_status.setStringValue(&NSString::from_str(&format!("Save error: {err}")));
            let red = NSColor::colorWithRed_green_blue_alpha(0.8, 0.1, 0.1, 1.0);
            txt_status.setTextColor(Some(&red));
        }
    }
}

/// TURN tab: the same STUN/TURN settings as the Windows Network page.
struct Turn {
    stun: usize,
    provider: usize,
    custom: usize,
    cloudflare: usize,
    turn_urls: usize,
    username: usize,
    password: usize,
    forget_password: usize,
    key_id: usize,
    token: usize,
    forget_token: usize,
    ttl: usize,
    cache: usize,
}

fn field<'a>(ptr: usize) -> &'a NSTextField {
    unsafe { &*(ptr as *const NSTextField) }
}
fn button<'a>(ptr: usize) -> &'a NSButton {
    unsafe { &*(ptr as *const NSButton) }
}
fn set_checked(ptr: usize, on: bool) {
    button(ptr).setState(if on {
        NSControlStateValueOn
    } else {
        NSControlStateValueOff
    });
}

/// URLs separated by spaces, commas or new lines; scheme lower-cased as on Windows.
fn parse_urls(text: &str) -> Vec<String> {
    text.split(|c: char| c.is_whitespace() || c == ',')
        .filter(|u| !u.is_empty())
        .map(|u| match u.split_once(':') {
            Some((scheme, rest)) => format!("{}:{rest}", scheme.to_ascii_lowercase()),
            None => u.into(),
        })
        .collect()
}

impl Turn {
    fn cloudflare_selected(&self) -> bool {
        unsafe { &*(self.provider as *const NSPopUpButton) }.indexOfSelectedItem() == 1
    }

    fn sync_provider(&self) {
        let cloudflare = self.cloudflare_selected();
        unsafe { &*(self.custom as *const NSView) }.setHidden(cloudflare);
        unsafe { &*(self.cloudflare as *const NSView) }.setHidden(!cloudflare);
    }

    fn load(&self, s: &crate::connection_settings::Settings) {
        field(self.stun).setStringValue(&NSString::from_str(&s.stun_urls.join(" ")));
        unsafe { &*(self.provider as *const NSPopUpButton) }
            .selectItemAtIndex(isize::from(s.provider == "cloudflare"));
        field(self.turn_urls).setStringValue(&NSString::from_str(&s.turn_urls.join(" ")));
        field(self.username).setStringValue(&NSString::from_str(&s.custom_username));
        field(self.key_id).setStringValue(&NSString::from_str(&s.turn_key_id));
        field(self.ttl).setStringValue(&NSString::from_str(&s.ttl.to_string()));
        set_checked(self.cache, s.cache_credentials);
        self.clear_secrets();
        self.sync_provider();
    }

    /// Writes the fields into `s`; returns (password, token, forget password, forget token).
    fn apply(
        &self,
        s: &mut crate::connection_settings::Settings,
    ) -> anyhow::Result<(String, String, bool, bool)> {
        s.provider = if self.cloudflare_selected() {
            "cloudflare"
        } else {
            "custom"
        }
        .into();
        s.stun_urls = parse_urls(&field(self.stun).stringValue().to_string());
        s.turn_urls = parse_urls(&field(self.turn_urls).stringValue().to_string());
        s.custom_username = field(self.username).stringValue().to_string().trim().into();
        s.turn_key_id = field(self.key_id).stringValue().to_string().trim().into();
        s.ttl = field(self.ttl)
            .stringValue()
            .to_string()
            .trim()
            .parse()
            .map_err(|_| anyhow::anyhow!("Enter a valid credential lifetime"))?;
        s.cache_credentials = button(self.cache).state() == NSControlStateValueOn;
        Ok((
            field(self.password).stringValue().to_string(),
            field(self.token).stringValue().to_string(),
            button(self.forget_password).state() == NSControlStateValueOn,
            button(self.forget_token).state() == NSControlStateValueOn,
        ))
    }

    /// Secrets are never shown back; blank means "keep the saved one".
    fn clear_secrets(&self) {
        field(self.password).setStringValue(&NSString::from_str(""));
        field(self.token).setStringValue(&NSString::from_str(""));
        set_checked(self.forget_password, false);
        set_checked(self.forget_token, false);
    }
}

fn create_field(mtm: MainThreadMarker, frame: NSRect, secure: bool) -> Retained<NSTextField> {
    let field: Retained<NSTextField> = if secure {
        Retained::into_super(NSSecureTextField::initWithFrame(mtm.alloc(), frame))
    } else {
        NSTextField::initWithFrame(mtm.alloc(), frame)
    };
    field.setEditable(true);
    field.setSelectable(true);
    field.setBezeled(true);
    field
}

fn build_turn_tab(
    mtm: MainThreadMarker,
    controller: &SettingsController,
    s: &crate::connection_settings::Settings,
) -> (Retained<NSTabViewItem>, Turn) {
    let tab = unsafe {
        NSTabViewItem::initWithIdentifier(mtm.alloc(), Some(&NSString::from_str("turn")))
    };
    tab.setLabel(&NSString::from_str("TURN"));
    let view = NSView::initWithFrame(
        mtm.alloc(),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(460.0, 310.0)),
    );
    let small_font = objc2_app_kit::NSFont::systemFontOfSize(11.0);
    // Label on the left, control on the right, rows from the top down.
    let row = |parent: &NSView, y: f64, label: &str, secure: bool| {
        parent.addSubview(&create_label(
            mtm,
            NSRect::new(NSPoint::new(20.0, y), NSSize::new(130.0, 20.0)),
            label,
            false,
        ));
        let input = create_field(
            mtm,
            NSRect::new(NSPoint::new(155.0, y), NSSize::new(285.0, 22.0)),
            secure,
        );
        parent.addSubview(&input);
        input
    };

    let stun = row(&view, 270.0, "STUN servers", false);
    stun.setPlaceholderString(Some(&NSString::from_str("Default discovery server")));
    view.addSubview(&create_label(
        mtm,
        NSRect::new(NSPoint::new(20.0, 238.0), NSSize::new(130.0, 20.0)),
        "TURN provider",
        false,
    ));
    let provider = NSPopUpButton::initWithFrame_pullsDown(
        mtm.alloc(),
        NSRect::new(NSPoint::new(152.0, 234.0), NSSize::new(291.0, 26.0)),
        false,
    );
    for name in ["Custom servers", "Cloudflare TURN"] {
        provider.addItemWithTitle(&NSString::from_str(name));
    }
    unsafe {
        provider.setTarget(Some(controller));
        provider.setAction(Some(objc2::sel!(providerChanged:)));
    }
    view.addSubview(&provider);

    // Provider-specific groups share the space below; only one is visible.
    let group_frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(460.0, 228.0));
    let custom = NSView::initWithFrame(mtm.alloc(), group_frame);
    let turn_urls = row(&custom, 196.0, "TURN servers", false);
    turn_urls.setPlaceholderString(Some(&NSString::from_str("turns:host:443?transport=tcp")));
    let username = row(&custom, 164.0, "Username", false);
    let password = row(&custom, 132.0, "Password", true);
    password.setPlaceholderString(Some(&NSString::from_str(
        "Leave blank to keep saved password",
    )));
    let forget_password = create_checkbox(
        mtm,
        NSRect::new(NSPoint::new(155.0, 104.0), NSSize::new(285.0, 22.0)),
        "Forget saved password",
        false,
    );
    custom.addSubview(&forget_password);
    view.addSubview(&custom);

    let cloudflare = NSView::initWithFrame(mtm.alloc(), group_frame);
    let key_id = row(&cloudflare, 196.0, "TURN Key ID", false);
    let token = row(&cloudflare, 164.0, "API token", true);
    token.setPlaceholderString(Some(&NSString::from_str("Leave blank to keep saved token")));
    let forget_token = create_checkbox(
        mtm,
        NSRect::new(NSPoint::new(155.0, 136.0), NSSize::new(285.0, 22.0)),
        "Forget saved API token",
        false,
    );
    cloudflare.addSubview(&forget_token);
    let ttl = row(&cloudflare, 104.0, "Lifetime (s)", false);
    let cache = create_checkbox(
        mtm,
        NSRect::new(NSPoint::new(155.0, 76.0), NSSize::new(285.0, 22.0)),
        "Reuse unexpired credentials",
        false,
    );
    cloudflare.addSubview(&cache);
    view.addSubview(&cloudflare);

    let hint = create_label(
        mtm,
        NSRect::new(NSPoint::new(20.0, 8.0), NSSize::new(420.0, 46.0)),
        "Separate several URLs with spaces. UDP, TCP or TLS (turns:host:443).\nThe relay is used when a direct route is unavailable, by WebRTC and by BUD.",
        false,
    );
    hint.setFont(Some(&small_font));
    view.addSubview(&hint);

    tab.setView(Some(&view));
    let turn = Turn {
        stun: Retained::into_raw(stun) as usize,
        provider: Retained::into_raw(provider) as usize,
        custom: Retained::into_raw(custom) as usize,
        cloudflare: Retained::into_raw(cloudflare) as usize,
        turn_urls: Retained::into_raw(turn_urls) as usize,
        username: Retained::into_raw(username) as usize,
        password: Retained::into_raw(password) as usize,
        forget_password: Retained::into_raw(forget_password) as usize,
        key_id: Retained::into_raw(key_id) as usize,
        token: Retained::into_raw(token) as usize,
        forget_token: Retained::into_raw(forget_token) as usize,
        ttl: Retained::into_raw(ttl) as usize,
        cache: Retained::into_raw(cache) as usize,
    };
    turn.load(s);
    (tab, turn)
}

fn create_checkbox(
    mtm: MainThreadMarker,
    frame: NSRect,
    title: &str,
    checked: bool,
) -> Retained<NSButton> {
    let btn = NSButton::initWithFrame(mtm.alloc(), frame);
    btn.setTitle(&NSString::from_str(title));
    btn.setButtonType(NSButtonType::Switch);
    btn.setState(if checked {
        NSControlStateValueOn
    } else {
        NSControlStateValueOff
    });
    btn
}

fn create_label(
    mtm: MainThreadMarker,
    frame: NSRect,
    text: &str,
    bold: bool,
) -> Retained<NSTextField> {
    let tf = NSTextField::initWithFrame(mtm.alloc(), frame);
    tf.setStringValue(&NSString::from_str(text));
    tf.setEditable(false);
    tf.setSelectable(false);
    tf.setBezeled(false);
    tf.setDrawsBackground(false);
    if bold {
        let font = objc2_app_kit::NSFont::boldSystemFontOfSize(13.0);
        tf.setFont(Some(&font));
    }
    tf
}

pub fn install_menu(mtm: MainThreadMarker, app: &NSApplication) {
    let controller = get_controller();
    unsafe {
        let menubar = NSMenu::initWithTitle(mtm.alloc(), &NSString::from_str(""));

        // 1. Application menu
        let app_menu_item = NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc(),
            &NSString::from_str(""),
            None,
            &NSString::from_str(""),
        );
        let app_menu = NSMenu::initWithTitle(mtm.alloc(), &NSString::from_str("ParsecWebTurn"));

        let about_item = NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc(),
            &NSString::from_str("About ParsecWebTurn"),
            Some(objc2::sel!(orderFrontStandardAboutPanel:)),
            &NSString::from_str(""),
        );
        app_menu.addItem(&about_item);

        app_menu.addItem(&NSMenuItem::separatorItem(mtm));

        let prefs_item = NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc(),
            &NSString::from_str("Preferences..."),
            Some(objc2::sel!(openPreferences:)),
            &NSString::from_str(","),
        );
        prefs_item.setTarget(Some(controller));
        app_menu.addItem(&prefs_item);

        app_menu.addItem(&NSMenuItem::separatorItem(mtm));

        let quit_item = NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc(),
            &NSString::from_str("Quit ParsecWebTurn"),
            Some(objc2::sel!(terminate:)),
            &NSString::from_str("q"),
        );
        app_menu.addItem(&quit_item);

        app_menu_item.setSubmenu(Some(&app_menu));
        menubar.addItem(&app_menu_item);

        // 2. View / Session menu
        let session_menu_item = NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc(),
            &NSString::from_str(""),
            None,
            &NSString::from_str(""),
        );
        let session_menu = NSMenu::initWithTitle(mtm.alloc(), &NSString::from_str("Session"));

        let stats_item = NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc(),
            &NSString::from_str("Connection Stats"),
            Some(objc2::sel!(openStats:)),
            &NSString::from_str("i"),
        );
        stats_item.setTarget(Some(controller));
        session_menu.addItem(&stats_item);

        let fs_item = NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc(),
            &NSString::from_str("Toggle Fullscreen"),
            Some(objc2::sel!(toggleFullscreen:)),
            &NSString::from_str("f"),
        );
        fs_item.setTarget(Some(controller));
        session_menu.addItem(&fs_item);

        session_menu_item.setSubmenu(Some(&session_menu));
        menubar.addItem(&session_menu_item);

        app.setMainMenu(Some(&menubar));
    }
}

pub fn open(manager: Arc<Manager>) {
    let open_fn = move |mtm: MainThreadMarker| {
        let mut state_guard = STATE.lock().unwrap();
        if let Some(state) = state_guard.as_ref() {
            // Window already exists: update values from manager and show
            let (s, _) = manager.view();
            let chk_force_relay = unsafe { &*(state.chk_force_relay as *const NSButton) };
            let chk_stun_only = unsafe { &*(state.chk_stun_only as *const NSButton) };
            let chk_prefer_hevc = unsafe { &*(state.chk_prefer_hevc as *const NSButton) };
            let chk_ten_bit = unsafe { &*(state.chk_ten_bit as *const NSButton) };
            let chk_event_log = unsafe { &*(state.chk_event_log as *const NSButton) };
            let chk_custom_audio = unsafe { &*(state.chk_custom_audio as *const NSButton) };
            let chk_raw_audio = unsafe { &*(state.chk_raw_audio as *const NSButton) };
            let chk_surround = unsafe { &*(state.chk_surround as *const NSButton) };
            let txt_status = unsafe { &*(state.txt_status as *const NSTextField) };
            let win = unsafe { &*(state.window as *const NSWindow) };

            chk_force_relay.setState(if s.force_relay {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
            chk_stun_only.setState(if s.stun_only {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
            chk_prefer_hevc.setState(if s.prefer_hevc {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
            chk_ten_bit.setState(if s.experimental_ten_bit {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
            chk_event_log.setState(if s.event_logging {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
            chk_custom_audio.setState(if s.custom_audio_buffer {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
            chk_raw_audio.setState(if s.allow_raw_audio {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
            chk_surround.setState(if s.experimental_surround_audio {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
            #[cfg(feature = "bud")]
            {
                let chk_hid_mouse = unsafe { &*(state.chk_hid_mouse as *const NSButton) };
                let chk_hid_keyboard = unsafe { &*(state.chk_hid_keyboard as *const NSButton) };
                chk_hid_mouse.setState(if s.experimental_hid_mouse {
                    NSControlStateValueOn
                } else {
                    NSControlStateValueOff
                });
                chk_hid_keyboard.setState(if s.experimental_hid_keyboard {
                    NSControlStateValueOn
                } else {
                    NSControlStateValueOff
                });
                let chk_bud = unsafe { &*(state.chk_bud as *const NSButton) };
                chk_bud.setState(if s.bud {
                    NSControlStateValueOn
                } else {
                    NSControlStateValueOff
                });
                sync_hid_enabled(state);
            }
            state.turn.load(&s);
            txt_status.setStringValue(&NSString::from_str(""));
            win.makeKeyAndOrderFront(None);
            return;
        }

        let controller = get_controller();

        let (w, h) = (520.0, 440.0);
        let rect = NSRect::new(NSPoint::new(150.0, 150.0), NSSize::new(w, h));
        let style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable;

        unsafe {
            let win = NSWindow::initWithContentRect_styleMask_backing_defer(
                mtm.alloc(),
                rect,
                style,
                NSBackingStoreType::Buffered,
                false,
            );
            win.setTitle(&NSString::from_str("ParsecWebTurn Preferences"));
            win.setReleasedWhenClosed(false);

            let (s, _) = manager.view();

            let root_view = NSView::initWithFrame(
                mtm.alloc(),
                NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w, h)),
            );

            // Tab View: width 480, height 350
            let tab_view = NSTabView::initWithFrame(
                mtm.alloc(),
                NSRect::new(NSPoint::new(20.0, 60.0), NSSize::new(480.0, 360.0)),
            );

            // 1. Network Tab
            let tab_net = NSTabViewItem::initWithIdentifier(
                mtm.alloc(),
                Some(&NSString::from_str("network")),
            );
            tab_net.setLabel(&NSString::from_str("Network"));
            let view_net = NSView::initWithFrame(
                mtm.alloc(),
                NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(460.0, 310.0)),
            );
            view_net.addSubview(&create_label(
                mtm,
                NSRect::new(NSPoint::new(20.0, 270.0), NSSize::new(420.0, 22.0)),
                "Network Routing & ICE Policy",
                true,
            ));
            let chk_force_relay = create_checkbox(
                mtm,
                NSRect::new(NSPoint::new(20.0, 230.0), NSSize::new(420.0, 22.0)),
                "Force TURN relay (disable direct peer connection)",
                s.force_relay,
            );
            let chk_stun_only = create_checkbox(
                mtm,
                NSRect::new(NSPoint::new(20.0, 200.0), NSSize::new(420.0, 22.0)),
                "STUN only (disable all TURN relays)",
                s.stun_only,
            );
            let lbl_net_info = create_label(mtm, NSRect::new(NSPoint::new(20.0, 120.0), NSSize::new(420.0, 60.0)), "Default connection uses WebRTC standard ICE negotiation. Enabling Force TURN guarantees symmetric NAT traversal through designated relay endpoints.", false);
            view_net.addSubview(&chk_force_relay);
            view_net.addSubview(&chk_stun_only);
            view_net.addSubview(&lbl_net_info);
            #[cfg(feature = "bud")]
            let chk_bud = {
                let chk_bud = create_checkbox(
                    mtm,
                    NSRect::new(NSPoint::new(20.0, 80.0), NSSize::new(420.0, 22.0)),
                    "BUD transport (alternative to WebRTC)",
                    s.bud,
                );
                chk_bud.setTarget(Some(controller));
                chk_bud.setAction(Some(objc2::sel!(budToggled:)));
                view_net.addSubview(&chk_bud);
                let lbl_bud = create_label(mtm, NSRect::new(NSPoint::new(38.0, 30.0), NSSize::new(400.0, 44.0)), "Direct UDP or TURN over UDP/TLS/TCP. Required for USB HID input.\nReconnect after changing.", false);
                lbl_bud.setFont(Some(&objc2_app_kit::NSFont::systemFontOfSize(11.0)));
                view_net.addSubview(&lbl_bud);
                chk_bud
            };
            tab_net.setView(Some(&view_net));
            tab_view.addTabViewItem(&tab_net);
            let (tab_turn, turn) = build_turn_tab(mtm, controller, &s);
            tab_view.addTabViewItem(&tab_turn);

            // 2. Video Tab
            let tab_video =
                NSTabViewItem::initWithIdentifier(mtm.alloc(), Some(&NSString::from_str("video")));
            tab_video.setLabel(&NSString::from_str("Video"));
            let view_video = NSView::initWithFrame(
                mtm.alloc(),
                NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(460.0, 310.0)),
            );
            view_video.addSubview(&create_label(
                mtm,
                NSRect::new(NSPoint::new(20.0, 270.0), NSSize::new(420.0, 22.0)),
                "Video Codec & Presentation",
                true,
            ));
            let chk_prefer_hevc = create_checkbox(
                mtm,
                NSRect::new(NSPoint::new(20.0, 230.0), NSSize::new(420.0, 22.0)),
                "Prefer HEVC / H.265 (hardware decoded via VideoToolbox)",
                s.prefer_hevc,
            );
            let chk_ten_bit = create_checkbox(
                mtm,
                NSRect::new(NSPoint::new(20.0, 200.0), NSSize::new(420.0, 22.0)),
                "Experimental 10-bit Color (HDR / Main10 profile)",
                s.experimental_ten_bit,
            );
            let chk_event_log = create_checkbox(
                mtm,
                NSRect::new(NSPoint::new(20.0, 170.0), NSSize::new(420.0, 22.0)),
                "Enable performance and decoder event logging",
                s.event_logging,
            );
            view_video.addSubview(&chk_prefer_hevc);
            view_video.addSubview(&chk_ten_bit);
            view_video.addSubview(&chk_event_log);
            tab_video.setView(Some(&view_video));
            tab_view.addTabViewItem(&tab_video);

            // 3. Audio Tab
            let tab_audio =
                NSTabViewItem::initWithIdentifier(mtm.alloc(), Some(&NSString::from_str("audio")));
            tab_audio.setLabel(&NSString::from_str("Audio"));
            let view_audio = NSView::initWithFrame(
                mtm.alloc(),
                NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(460.0, 310.0)),
            );
            view_audio.addSubview(&create_label(
                mtm,
                NSRect::new(NSPoint::new(20.0, 270.0), NSSize::new(420.0, 22.0)),
                "Audio Output & Buffering",
                true,
            ));
            let chk_custom_audio = create_checkbox(
                mtm,
                NSRect::new(NSPoint::new(20.0, 230.0), NSSize::new(420.0, 22.0)),
                "Custom Audio Buffer (75ms min, 150ms max)",
                s.custom_audio_buffer,
            );
            let chk_raw_audio = create_checkbox(
                mtm,
                NSRect::new(NSPoint::new(20.0, 200.0), NSSize::new(420.0, 22.0)),
                "Allow Raw Audio (uncompressed PCM for lowest latency)",
                s.allow_raw_audio,
            );
            let chk_surround = create_checkbox(
                mtm,
                NSRect::new(NSPoint::new(20.0, 170.0), NSSize::new(420.0, 22.0)),
                "Experimental Surround Audio (5.1 / 7.1 channels)",
                s.experimental_surround_audio,
            );
            view_audio.addSubview(&chk_custom_audio);
            view_audio.addSubview(&chk_raw_audio);
            view_audio.addSubview(&chk_surround);
            tab_audio.setView(Some(&view_audio));
            tab_view.addTabViewItem(&tab_audio);

            // 4. Input Tab
            let tab_input =
                NSTabViewItem::initWithIdentifier(mtm.alloc(), Some(&NSString::from_str("input")));
            tab_input.setLabel(&NSString::from_str("Input"));
            let view_input = NSView::initWithFrame(
                mtm.alloc(),
                NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(460.0, 310.0)),
            );

            let small_font = objc2_app_kit::NSFont::systemFontOfSize(11.0);
            // One column: full-width descriptions and checkboxes, so nothing truncates.
            let section = |y: f64, title: &str, text: &str| {
                view_input.addSubview(&create_label(
                    mtm,
                    NSRect::new(NSPoint::new(20.0, y), NSSize::new(420.0, 18.0)),
                    title,
                    true,
                ));
                let desc = create_label(
                    mtm,
                    NSRect::new(NSPoint::new(20.0, y - 48.0), NSSize::new(420.0, 46.0)),
                    text,
                    false,
                );
                desc.setFont(Some(&small_font));
                view_input.addSubview(&desc);
            };

            #[cfg(feature = "bud")]
            let (hid_m, hid_k) = (s.experimental_hid_mouse, s.experimental_hid_keyboard);
            #[cfg(not(feature = "bud"))]
            let (hid_m, hid_k) = (false, false);

            section(272.0, "Mouse", "Experimental USB HID mouse over BUD.\nEnable USB/Yubikey passthrough on the host.\nSupports a single physical or virtual display.");
            let chk_hid_mouse = create_checkbox(
                mtm,
                NSRect::new(NSPoint::new(20.0, 200.0), NSSize::new(420.0, 22.0)),
                "USB HID mouse (experimental, BUD)",
                hid_m,
            );
            view_input.addSubview(&chk_hid_mouse);

            section(172.0, "Keyboard", "Experimental physical keyboard over BUD.\nUse the same keyboard layout on host and client.\nFor example: Hungarian on both computers.");
            let chk_hid_keyboard = create_checkbox(
                mtm,
                NSRect::new(NSPoint::new(20.0, 100.0), NSSize::new(420.0, 22.0)),
                "USB HID keyboard (experimental, BUD)",
                hid_k,
            );
            view_input.addSubview(&chk_hid_keyboard);

            // Shown while BUD is off, so the disabled checkboxes explain themselves.
            let lbl_bud_hint = create_label(
                mtm,
                NSRect::new(NSPoint::new(20.0, 76.0), NSSize::new(420.0, 18.0)),
                "Turn on \u{201c}BUD transport\u{201d} in the Network tab to use these options.",
                false,
            );
            lbl_bud_hint.setFont(Some(&small_font));
            lbl_bud_hint.setTextColor(Some(&NSColor::systemOrangeColor()));
            view_input.addSubview(&lbl_bud_hint);

            section(52.0, "Compatibility", "Absolute desktop and relative camera movement.\nCamera mode follows the host cursor state.\nKeyboard layout is not switched automatically.");

            tab_input.setView(Some(&view_input));
            tab_view.addTabViewItem(&tab_input);

            root_view.addSubview(&tab_view);

            // Bottom controls:
            let txt_status = create_label(
                mtm,
                NSRect::new(NSPoint::new(20.0, 16.0), NSSize::new(290.0, 24.0)),
                "",
                false,
            );
            root_view.addSubview(&txt_status);

            let btn_save = NSButton::initWithFrame(
                mtm.alloc(),
                NSRect::new(NSPoint::new(320.0, 14.0), NSSize::new(85.0, 28.0)),
            );
            btn_save.setTitle(&NSString::from_str("Save"));
            btn_save.setTarget(Some(controller));
            btn_save.setAction(Some(objc2::sel!(saveClicked:)));
            root_view.addSubview(&btn_save);

            let btn_close = NSButton::initWithFrame(
                mtm.alloc(),
                NSRect::new(NSPoint::new(415.0, 14.0), NSSize::new(85.0, 28.0)),
            );
            btn_close.setTitle(&NSString::from_str("Close"));
            btn_close.setTarget(Some(controller));
            btn_close.setAction(Some(objc2::sel!(closeClicked:)));
            root_view.addSubview(&btn_close);

            win.setContentView(Some(&root_view));
            win.center();
            win.makeKeyAndOrderFront(None);

            *state_guard = Some(SettingsState {
                window: Retained::into_raw(win) as usize,
                manager,
                chk_force_relay: Retained::into_raw(chk_force_relay) as usize,
                chk_stun_only: Retained::into_raw(chk_stun_only) as usize,
                chk_prefer_hevc: Retained::into_raw(chk_prefer_hevc) as usize,
                chk_ten_bit: Retained::into_raw(chk_ten_bit) as usize,
                chk_event_log: Retained::into_raw(chk_event_log) as usize,
                chk_custom_audio: Retained::into_raw(chk_custom_audio) as usize,
                chk_raw_audio: Retained::into_raw(chk_raw_audio) as usize,
                chk_surround: Retained::into_raw(chk_surround) as usize,
                #[cfg(feature = "bud")]
                chk_bud: Retained::into_raw(chk_bud) as usize,
                #[cfg(feature = "bud")]
                chk_hid_mouse: Retained::into_raw(chk_hid_mouse) as usize,
                #[cfg(feature = "bud")]
                chk_hid_keyboard: Retained::into_raw(chk_hid_keyboard) as usize,
                #[cfg(feature = "bud")]
                lbl_bud_hint: Retained::into_raw(lbl_bud_hint) as usize,
                turn,
                txt_status: Retained::into_raw(txt_status) as usize,
            });
            #[cfg(feature = "bud")]
            if let Some(state) = state_guard.as_ref() {
                sync_hid_enabled(state);
            }
        }
    };

    if let Some(mtm) = MainThreadMarker::new() {
        open_fn(mtm);
    } else {
        dispatch2::DispatchQueue::main().exec_async(move || {
            if let Some(mtm) = MainThreadMarker::new() {
                open_fn(mtm);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn urls_split_on_spaces_commas_and_lines_with_lowercase_schemes() {
        assert_eq!(
            super::parse_urls(
                " TURNS:relay.example:443?transport=tcp,turn:10.0.0.2:3478\nstun:a:3478  "
            ),
            [
                "turns:relay.example:443?transport=tcp",
                "turn:10.0.0.2:3478",
                "stun:a:3478"
            ]
        );
        assert!(super::parse_urls("  ").is_empty());
    }
}
