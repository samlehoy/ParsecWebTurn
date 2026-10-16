//! Native, modeless settings page inside the existing client window.
//! All HWND/GDI ownership and control access stay on the window's UI thread.
#[path = "settings_controls.rs"]
pub(crate) mod controls;
use crate::connection_settings::{Manager, Settings};
use std::{
    ptr::{null, null_mut},
    sync::Arc,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::*,
    UI::{
        Controls::{
            SetScrollInfo, DRAWITEMSTRUCT, EM_SETLIMITTEXT, MEASUREITEMSTRUCT, ODS_DISABLED,
            ODS_FOCUS, ODS_SELECTED,
        },
        Input::KeyboardAndMouse::*,
        WindowsAndMessaging::*,
    },
};
pub const OPEN: usize = 4100;
pub const ABOUT: usize = 4300;
// Give the large heading and subtitle enough space, including font overhang.
const HEADER_EXTRA: i32 = 20;
const SAVE: usize = 4101;
const BACK: usize = 4102;
const PROVIDER: usize = 4103;
const STUN_ONLY: usize = 4104;
const STUN: usize = 4105;
const TURN: usize = 4106;
const USERNAME: usize = 4107;
const PASSWORD: usize = 4108;
const KEY: usize = 4109;
const TOKEN: usize = 4110;
const TTL: usize = 4111;
const CACHE: usize = 4112;
const FORGET_PASSWORD: usize = 4113;
const FORGET_TOKEN: usize = 4114;
const STATUS: usize = 4115;
const GPU: usize = 4116;
const HEVC: usize = 4117;
const FORCE_RELAY: usize = 4118;
#[cfg(feature = "experimental-bud")]
const BUD: usize = 4119;
const BUD_EXTRA: i32 = if cfg!(feature = "experimental-bud") {
    94
} else {
    0
};
#[cfg(feature = "vendor-video")]
const ZERO_COPY: usize = 4120;
const BG: u32 = 0x00282828;
const FIELD: u32 = 0x00212121;
const FG: u32 = 0x00eeeeee;
const MUTED: u32 = 0x00bcbcbc;
const ACCENT: u32 = 0x00ffbb00;
const CUSTOM: u8 = 1;
const CLOUDFLARE: u8 = 2;
const HELP: usize = 4200;
const NETWORK_TAB: usize = 4201;
const VIDEO_TAB: usize = 4121;
const TEN_BIT: usize = 4122;
const EVENT_LOG: usize = 4123;
const AUDIO_TAB: usize = 4124;
const AUDIO_CUSTOM: usize = 4125;
const AUDIO_MIN: usize = 4126;
const AUDIO_MAX: usize = 4127;
const AUDIO_RAW: usize = 4128;
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Network,
    Video,
    Audio,
}
const SECTION: usize = 4202;
const NOTICE: usize = 4203;
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}
struct Item {
    hwnd: HWND,
    rect: RECT,
    group: u8,
    tab: Option<Tab>,
}
struct Page {
    window_owned: bool,
    manager: Arc<Manager>,
    saved: Settings,
    gpu_choices: Vec<Option<crate::connection_settings::GpuPreference>>,
    gpu_labels: Vec<String>,
    background: HBRUSH,
    field: HBRUSH,
    font: HFONT,
    heading: HFONT,
    small: HFONT,
    items: Vec<Item>,
    active_tab: Tab,
    building_tab: Option<Tab>,
    scroll: i32,
    reflowing: bool,
}
impl Drop for Page {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.background);
            DeleteObject(self.field);
            DeleteObject(self.font);
            DeleteObject(self.heading);
            DeleteObject(self.small);
        }
    }
}
unsafe fn font(size: i32, weight: i32) -> HFONT {
    CreateFontW(
        -size,
        0,
        0,
        0,
        weight,
        0,
        0,
        0,
        DEFAULT_CHARSET as u32,
        0,
        0,
        CLEARTYPE_QUALITY as u32,
        0,
        wide("Segoe UI").as_ptr(),
    )
}
/// A native menu command and keyboard shortcut; no private Parsec UI callbacks.
pub unsafe fn install_menu(parent: HWND) {
    if !GetMenu(parent).is_null() {
        return;
    }
    let menu = CreateMenu();
    AppendMenuW(menu, MF_STRING, OPEN, wide("Settings").as_ptr());
    AppendMenuW(
        menu,
        MF_STRING,
        crate::stats_ui::OPEN,
        wide("Connection stats").as_ptr(),
    );
    let help = CreatePopupMenu();
    AppendMenuW(help, MF_STRING, ABOUT, wide("About ParsecWebTurn").as_ptr());
    AppendMenuW(
        help,
        MF_STRING,
        crate::updater_ui::OPEN,
        wide("Check for updates").as_ptr(),
    );
    AppendMenuW(menu, MF_POPUP, help as usize, wide("Help").as_ptr());
    SetMenu(parent, menu);
    DrawMenuBar(parent);
}
pub unsafe fn open(parent: HWND, manager: Arc<Manager>) -> HWND {
    let name = wide("ParsecWebTurnSettings");
    let instance = GetModuleHandleW(null());
    let class = WNDCLASSW {
        lpfnWndProc: Some(page_proc),
        hInstance: instance,
        hCursor: LoadCursorW(null_mut(), IDC_ARROW),
        lpszClassName: name.as_ptr(),
        ..std::mem::zeroed()
    };
    RegisterClassW(&class);
    let (saved, error) = manager.view();
    let mut page = Box::new(Page {
        window_owned: false,
        manager,
        saved,
        gpu_choices: vec![None],
        gpu_labels: vec!["Automatic (Windows default)".into()],
        background: CreateSolidBrush(BG),
        field: CreateSolidBrush(FIELD),
        font: font(16, 400),
        heading: font(46, 300),
        small: font(13, 400),
        items: Vec::new(),
        active_tab: Tab::Network,
        building_tab: None,
        scroll: 0,
        reflowing: false,
    });
    // Failed CreateWindowEx may send WM_NCDESTROY. Transfer ownership only on success.
    let pointer = (&mut *page) as *mut Page;
    let mut rect: RECT = std::mem::zeroed();
    GetClientRect(parent, &mut rect);
    if rect.right < 860 || rect.bottom < 600 {
        SetWindowPos(
            parent,
            null_mut(),
            0,
            0,
            1000,
            780,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
        GetClientRect(parent, &mut rect);
    }
    let hwnd = CreateWindowExW(
        // Buffer the GDI page and its native child controls together. This
        // class has no CS_OWNDC/CS_CLASSDC/CS_PARENTDC; the GL parent is unchanged.
        WS_EX_CONTROLPARENT | WS_EX_COMPOSITED,
        name.as_ptr(),
        wide("Settings").as_ptr(),
        // Build the entire page offscreen before making any control visible.
        WS_CHILD | WS_CLIPCHILDREN | WS_VSCROLL,
        0,
        0,
        rect.right,
        rect.bottom,
        parent,
        null_mut(),
        instance,
        pointer.cast(),
    );
    if hwnd.is_null() {
        return hwnd;
    }
    page.window_owned = true;
    let page = &mut *Box::into_raw(page);
    let s = page.saved.clone();
    add(
        hwnd,
        page,
        "STATIC",
        "Settings",
        0,
        [0, 24, 820, 64],
        0,
        0,
        page.heading,
    );
    add(
        hwnd,
        page,
        "STATIC",
        "Customize your connection, video and audio playback.",
        0,
        [4, 88, 816, 24],
        0,
        0,
        page.font,
    );
    for (id, label, rect) in [
        (NETWORK_TAB, "Network", [0, 142, 128, 40]),
        (VIDEO_TAB, "Video", [140, 142, 128, 40]),
        (AUDIO_TAB, "Audio", [280, 142, 128, 40]),
        (BACK, "Back", [550, 142, 110, 40]),
        (SAVE, "Save settings", [676, 142, 144, 40]),
    ] {
        add(
            hwnd,
            page,
            "BUTTON",
            label,
            BS_OWNERDRAW as u32 | WS_TABSTOP,
            rect,
            id,
            0,
            page.font,
        );
    }
    add(
        hwnd,
        page,
        "STATIC",
        "NETWORK",
        0,
        [0, 204, 820, 25],
        SECTION,
        0,
        page.font,
    );
    page.building_tab = Some(Tab::Network);
    row(hwnd, page, "STUN servers",
        "Discover a direct route. One stun: URL per line.\nLeave blank to use the default discovery server.",
        STUN, &s.stun_urls.join("\r\n"), 242, 76, 0, ES_MULTILINE as u32 | ES_WANTRETURN as u32 | ES_AUTOVSCROLL as u32 | WS_CLIPCHILDREN);
    row_label(
        hwnd,
        page,
        "STUN only",
        "For LAN or VPN connections that do not need a relay.",
        344,
        0,
        40,
    );
    add(
        hwnd,
        page,
        "BUTTON",
        "Use STUN only (no TURN relay)",
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
        [420, 344, 400, 36],
        STUN_ONLY,
        0,
        page.font,
    );
    check(hwnd, STUN_ONLY, s.stun_only);
    row_label(
        hwnd,
        page,
        "TURN provider",
        "An optional relay when a direct connection is unavailable.",
        418,
        0,
        40,
    );
    let provider = add(
        hwnd,
        page,
        "COMBOBOX",
        "",
        CBS_DROPDOWNLIST as u32
            | CBS_OWNERDRAWFIXED as u32
            | CBS_HASSTRINGS as u32
            | WS_VSCROLL
            | WS_TABSTOP,
        [420, 418, 400, 160],
        PROVIDER,
        0,
        page.font,
    );
    controls::combo(provider);
    for name in ["Custom servers", "Cloudflare TURN"] {
        SendMessageW(provider, CB_ADDSTRING, 0, wide(name).as_ptr() as isize);
    }
    SendMessageW(
        provider,
        CB_SETCURSEL,
        usize::from(s.provider == "cloudflare"),
        0,
    );
    row(hwnd, page, "TURN servers",
        "coturn, eturnal, ExpressTURN and other providers.\nUDP, TCP or TLS; e.g. turns:host:443. One URL per line.",
        TURN, &s.turn_urls.join("\r\n"), 502, 76, CUSTOM,
        ES_MULTILINE as u32 | ES_WANTRETURN as u32 | ES_AUTOVSCROLL as u32 | WS_CLIPCHILDREN);
    row(
        hwnd,
        page,
        "Username",
        "The username supplied by your TURN provider.",
        USERNAME,
        &s.custom_username,
        608,
        36,
        CUSTOM,
        ES_AUTOHSCROLL as u32,
    );
    row(
        hwnd,
        page,
        "Password",
        "Leave blank to keep your saved password.",
        PASSWORD,
        "",
        688,
        36,
        CUSTOM,
        ES_AUTOHSCROLL as u32 | ES_PASSWORD as u32,
    );
    add(
        hwnd,
        page,
        "BUTTON",
        "Forget saved password",
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
        [420, 734, 400, 28],
        FORGET_PASSWORD,
        CUSTOM,
        page.small,
    );
    row(
        hwnd,
        page,
        "TURN Key ID",
        "The key ID from your Cloudflare TURN configuration.",
        KEY,
        &s.turn_key_id,
        502,
        36,
        CLOUDFLARE,
        ES_AUTOHSCROLL as u32,
    );
    row(
        hwnd,
        page,
        "API token",
        "Leave blank to keep your saved token.",
        TOKEN,
        "",
        582,
        36,
        CLOUDFLARE,
        ES_AUTOHSCROLL as u32 | ES_PASSWORD as u32,
    );
    add(
        hwnd,
        page,
        "BUTTON",
        "Forget saved API token",
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
        [420, 628, 400, 28],
        FORGET_TOKEN,
        CLOUDFLARE,
        page.small,
    );
    row(
        hwnd,
        page,
        "Credential lifetime",
        "How long generated TURN credentials remain valid (seconds).",
        TTL,
        &s.ttl.to_string(),
        678,
        36,
        CLOUDFLARE,
        ES_NUMBER as u32,
    );
    row_label(
        hwnd,
        page,
        "Credential cache",
        "Reuse valid credentials until you close ParsecWebTurn.",
        758,
        CLOUDFLARE,
        24,
    );
    add(
        hwnd,
        page,
        "BUTTON",
        "Reuse unexpired credentials",
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
        [420, 758, 400, 36],
        CACHE,
        CLOUDFLARE,
        page.font,
    );
    check(hwnd, CACHE, s.cache_credentials);
    page.building_tab = Some(Tab::Video);
    row_label(
        hwnd,
        page,
        "Video GPU",
        "GPU for stream decoding and rendering.\nAutomatic uses Windows' default. Next connection.",
        242,
        0,
        40,
    );
    let gpu = add(
        hwnd,
        page,
        "COMBOBOX",
        "",
        CBS_DROPDOWNLIST as u32
            | CBS_OWNERDRAWFIXED as u32
            | CBS_HASSTRINGS as u32
            | WS_VSCROLL
            | WS_TABSTOP,
        [420, 242, 400, 200],
        GPU,
        0,
        page.font,
    );
    controls::combo(gpu);
    SendMessageW(
        gpu,
        CB_ADDSTRING,
        0,
        wide("Automatic (Windows default)").as_ptr() as isize,
    );
    if let Ok(adapters) = crate::gpu_windows::enumerate() {
        for (index, adapter) in adapters.iter().enumerate() {
            let p = adapter.preference.clone();
            let label = if adapters
                .iter()
                .filter(|a| a.preference.name == p.name)
                .count()
                > 1
            {
                format!("{} (GPU {})", p.name, index + 1)
            } else {
                p.name.clone()
            };
            SendMessageW(gpu, CB_ADDSTRING, 0, wide(&label).as_ptr() as isize);
            page.gpu_choices.push(Some(p));
            page.gpu_labels.push(label);
        }
    }
    let selected = s
        .video_gpu
        .as_ref()
        .map(|preferred| {
            let available: Vec<_> = page.gpu_choices.iter().skip(1).flatten().cloned().collect();
            match crate::gpu_windows::match_index(preferred, &available) {
                Some(index) => index + 1,
                None => {
                    SendMessageW(
                        gpu,
                        CB_ADDSTRING,
                        0,
                        wide(&format!("Unavailable: {}", preferred.name)).as_ptr() as isize,
                    );
                    page.gpu_choices.push(Some(preferred.clone()));
                    page.gpu_labels
                        .push(format!("Unavailable: {}", preferred.name));
                    page.gpu_choices.len() - 1
                }
            }
        })
        .unwrap_or(0);
    SendMessageW(gpu, CB_SETCURSEL, selected, 0);
    row_label(
        hwnd,
        page,
        "Video codec",
        "Use HEVC when supported by this GPU and driver.\nReconnect after changing.",
        344,
        0,
        40,
    );
    add(
        hwnd,
        page,
        "BUTTON",
        "Prefer H.265 / HEVC",
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
        [420, 344, 400, 36],
        HEVC,
        0,
        page.font,
    );
    check(hwnd, HEVC, s.prefer_hevc);
    row_label(hwnd, page, "Color depth", "Request HEVC Main 10 (experimental). Host support required.\nGPU decode; normal 8-bit RGB display output. Reconnect to apply.", 438, 0, 40);
    add(
        hwnd,
        page,
        "BUTTON",
        "10-bit HEVC (experimental)",
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
        [420, 438, 400, 36],
        TEN_BIT,
        0,
        page.font,
    );
    check(hwnd, TEN_BIT, s.experimental_ten_bit);
    page.building_tab = Some(Tab::Network);
    row_label(
        hwnd,
        page,
        "Connection route",
        "Use a TURN relay even when a direct route is available.",
        844,
        0,
        40,
    );
    add(
        hwnd,
        page,
        "BUTTON",
        "Force TURN relay",
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
        [420, 844, 400, 36],
        FORCE_RELAY,
        0,
        page.font,
    );
    check(hwnd, FORCE_RELAY, s.force_relay);
    #[cfg(feature = "experimental-bud")]
    {
        row_label(hwnd, page, "Transport", "Experimental BUD: direct UDP or TURN over UDP/TLS/TCP.\nUses the TURN URLs above. Reconnect after changing.", 938, 0, 40);
        add(
            hwnd,
            page,
            "BUTTON",
            "Experimental BUD",
            BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
            [420, 938, 400, 36],
            BUD,
            0,
            page.font,
        );
        check(hwnd, BUD, s.experimental_bud);
    }
    #[cfg(feature = "vendor-video")]
    {
        page.building_tab = Some(Tab::Video);
        let y = 532;
        let hint = "Present original HEVC decoder surfaces on supported GPUs.\nWindows 11 / compatible driver. Reconnect after changing.";
        row_label(hwnd, page, "Video presentation", hint, y, 0, 40);
        add(
            hwnd,
            page,
            "BUTTON",
            "Zero copy (experimental, HEVC)",
            BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
            [420, y, 400, 36],
            ZERO_COPY,
            0,
            page.font,
        );
        check(hwnd, ZERO_COPY, s.experimental_zero_copy);
    }
    page.building_tab = Some(Tab::Network);
    row_label(hwnd, page, "Event log", "Log connection changes and decoder errors.\nSaved beside settings.json; no credentials or media payloads.\nApplies immediately after saving.", 938 + BUD_EXTRA, 0, 60);
    add(
        hwnd,
        page,
        "BUTTON",
        "Enable event logging",
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
        [420, 938 + BUD_EXTRA, 400, 36],
        EVENT_LOG,
        0,
        page.font,
    );
    check(hwnd, EVENT_LOG, s.event_logging);
    page.building_tab = Some(Tab::Audio);
    row_label(hwnd, page, "Playback buffer", "Experimental: custom limits apply only when enabled.\nOff uses the core's limits (normally 75 / 150 ms).", 242, 0, 48);
    add(
        hwnd,
        page,
        "BUTTON",
        "Custom audio buffer (experimental)",
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
        [420, 242, 400, 36],
        AUDIO_CUSTOM,
        0,
        page.font,
    );
    check(hwnd, AUDIO_CUSTOM, s.experimental_audio_buffer);
    row(hwnd, page, "Minimum buffer (ms)", "PCM reserve before starting or refilling playback.\nSmaller values can increase audible gaps.", AUDIO_MIN,
        &s.audio_min_buffer_ms.to_string(), 344, 36, 0, ES_NUMBER as u32);
    row(hwnd, page, "Maximum buffer (ms)", "Local PCM plus audio queued on the Windows device.\n0 <= minimum <= maximum <= 500; maximum above zero.", AUDIO_MAX,
        &s.audio_max_buffer_ms.to_string(), 438, 36, 0, ES_NUMBER as u32);
    row_label(hwnd, page, "Audio format", "Experimental RAW: BUD only; host must also enable RAW.\n48 kHz stereo PCM uses about 1.54 Mbps before overhead.\nThe actual codec is shown in Connection stats.", 532, 0, 60);
    add(
        hwnd,
        page,
        "BUTTON",
        "Allow raw PCM audio (experimental, BUD)",
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
        [420, 532, 400, 36],
        AUDIO_RAW,
        0,
        page.font,
    );
    check(hwnd, AUDIO_RAW, s.experimental_raw_audio);
    page.building_tab = None;
    add(hwnd, page, "STATIC",
        "ICE chooses the route automatically unless Force TURN relay is enabled.\nChanges apply to your next connection.",
        0, [0, 1056 + BUD_EXTRA, 820, 48], NOTICE, 0, page.small);
    let status = error.unwrap_or_else(|| {
        "Saved in settings.json. Secrets are protected for your Windows user.".into()
    });
    add(
        hwnd,
        page,
        "STATIC",
        &status,
        0,
        [0, 1118 + BUD_EXTRA, 820, 40],
        STATUS,
        0,
        page.small,
    );
    update_enabled(hwnd);
    ShowWindow(hwnd, SW_SHOW);
    RedrawWindow(
        hwnd,
        null(),
        null_mut(),
        RDW_INVALIDATE | RDW_ERASE | RDW_FRAME | RDW_ALLCHILDREN | RDW_UPDATENOW,
    );
    SetFocus(GetDlgItem(hwnd, STUN as i32));
    hwnd
}
#[allow(clippy::too_many_arguments)] // Native control descriptors, UI-thread only.
unsafe fn add(
    parent: HWND,
    page: &mut Page,
    class: &str,
    text: &str,
    style: u32,
    rect: [i32; 4],
    id: usize,
    group: u8,
    font: HFONT,
) -> HWND {
    let [x, y, w, h] = rect;
    let hwnd = control(parent, class, text, style, x, y, w, h, id, font);
    page.items.push(Item {
        hwnd,
        rect: RECT {
            left: x,
            top: y,
            right: x + w,
            bottom: y + h,
        },
        group,
        tab: page.building_tab,
    });
    hwnd
}
unsafe fn row_label(
    hwnd: HWND,
    page: &mut Page,
    label: &str,
    help: &str,
    y: i32,
    group: u8,
    help_height: i32,
) {
    add(
        hwnd,
        page,
        "STATIC",
        label,
        0,
        [0, y + 2, 392, 24],
        0,
        group,
        page.font,
    );
    add(
        hwnd,
        page,
        "STATIC",
        help,
        0,
        [0, y + 32, 392, help_height],
        HELP,
        group,
        page.small,
    );
}
#[allow(clippy::too_many_arguments)] // Label, help and native editor describe one settings row.
unsafe fn row(
    hwnd: HWND,
    page: &mut Page,
    label: &str,
    help: &str,
    id: usize,
    value: &str,
    y: i32,
    height: i32,
    group: u8,
    style: u32,
) {
    row_label(hwnd, page, label, help, y, group, 48);
    add(
        hwnd,
        page,
        "EDIT",
        value,
        style | WS_BORDER | WS_TABSTOP,
        [420, y, 400, height],
        id,
        group,
        page.font,
    );
    if style & ES_MULTILINE as u32 != 0 {
        controls::multiline(GetDlgItem(hwnd, id as i32));
    }
}
/// Reflow on resize/provider changes. Hidden controls retain unsaved values.
unsafe fn layout(hwnd: HWND) {
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Page;
    if pointer.is_null() {
        return;
    }
    let page = &mut *pointer;
    if page.items.is_empty() || page.reflowing {
        return;
    }
    page.reflowing = true;
    let mut client = std::mem::zeroed();
    GetClientRect(hwnd, &mut client);
    let cloudflare = SendMessageW(GetDlgItem(hwnd, PROVIDER as i32), CB_GETCURSEL, 0, 0) == 1;
    let footer = match page.active_tab {
        Tab::Network => 1056 + BUD_EXTRA,
        Tab::Audio => 650,
        Tab::Video => {
            if cfg!(feature = "vendor-video") {
                626
            } else {
                532
            }
        }
    };
    let height = footer + 122 + HEADER_EXTRA;
    page.scroll = page.scroll.clamp(0, (height - client.bottom).max(0));
    let info = SCROLLINFO {
        cbSize: size_of::<SCROLLINFO>() as u32,
        fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
        nMin: 0,
        nMax: height - 1,
        nPage: client.bottom.max(0) as u32,
        nPos: page.scroll,
        nTrackPos: 0,
    };
    SetScrollInfo(hwnd, SB_VERT, &info, 1);
    // Showing/hiding the scrollbar can synchronously send WM_SIZE. Reflow once,
    // using the final client width instead of recursively moving every control.
    GetClientRect(hwnd, &mut client);
    let x = ((client.right - 820) / 2).max(24);
    for item in &page.items {
        let visible = item.tab.is_none_or(|tab| tab == page.active_tab)
            && (item.group == 0 || item.group == if cloudflare { CLOUDFLARE } else { CUSTOM });
        let mut r = item.rect;
        let id = GetDlgCtrlID(item.hwnd) as usize;
        let new_top = match id {
            NOTICE => Some(footer),
            STATUS => Some(footer + 62),
            _ => None,
        };
        if let Some(top) = new_top {
            r.bottom += top - r.top;
            r.top = top;
        }
        // Moving overlapping child windows individually must not preserve old
        // pixels or paint halfway through reflow. A parent-only invalidation
        // excludes children (WS_CLIPCHILDREN), leaving clipped text/buttons.
        SetWindowPos(
            item.hwnd,
            null_mut(),
            x + r.left,
            r.top + if r.top >= 88 { HEADER_EXTRA } else { 0 } - page.scroll,
            r.right - r.left,
            r.bottom - r.top,
            SWP_NOZORDER
                | SWP_NOACTIVATE
                | SWP_NOCOPYBITS
                | SWP_NOREDRAW
                | if visible {
                    SWP_SHOWWINDOW
                } else {
                    SWP_HIDEWINDOW
                },
        );
    }
    page.reflowing = false;
    RedrawWindow(
        hwnd,
        null(),
        null_mut(),
        RDW_INVALIDATE | RDW_ERASE | RDW_FRAME | RDW_ALLCHILDREN | RDW_UPDATENOW,
    );
}

unsafe fn scroll_to(hwnd: HWND, requested: i32) {
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Page;
    if pointer.is_null() {
        return;
    }
    let mut info = SCROLLINFO {
        cbSize: size_of::<SCROLLINFO>() as u32,
        fMask: SIF_RANGE | SIF_PAGE,
        ..std::mem::zeroed()
    };
    if GetScrollInfo(hwnd, SB_VERT, &mut info) == 0 {
        return;
    }
    let limit = (info.nMax - info.nPage as i32 + 1).max(info.nMin);
    let position = requested.clamp(info.nMin, limit);
    if (*pointer).scroll != position {
        (*pointer).scroll = position;
        layout(hwnd);
    }
}

unsafe fn paint_page(hwnd: HWND, dc: HDC, page: &Page) {
    let mut rect: RECT = std::mem::zeroed();
    GetClientRect(hwnd, &mut rect);
    // Paint is self-contained even when Windows supplies a paint region without
    // requesting a separate background erase (e.g. after an exposed child).
    FillRect(dc, &rect, page.background);
    let x = ((rect.right - 820) / 2).max(24);
    let line = RECT {
        left: x,
        top: 127 + HEADER_EXTRA - page.scroll,
        right: x + 820,
        bottom: 129 + HEADER_EXTRA - page.scroll,
    };
    SetDCBrushColor(dc, 0x003b3b3b);
    FillRect(dc, &line, GetStockObject(DC_BRUSH) as HBRUSH);
}
#[allow(clippy::too_many_arguments)] // Flat Win32 control descriptor, UI-thread only.
unsafe fn control(
    parent: HWND,
    class: &str,
    text: &str,
    style: u32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    id: usize,
    font: HFONT,
) -> HWND {
    let hwnd = CreateWindowExW(
        0,
        wide(class).as_ptr(),
        wide(text).as_ptr(),
        WS_CHILD
            | WS_VISIBLE
            | style
            | if class == "BUTTON" {
                BS_NOTIFY as u32
            } else {
                0
            },
        x,
        y,
        width,
        height,
        parent,
        id as HMENU,
        GetModuleHandleW(null()),
        null(),
    );
    SendMessageW(hwnd, WM_SETFONT, font as usize, 1);
    if class == "EDIT" {
        SendMessageW(hwnd, EM_SETLIMITTEXT, 16384, 0);
    }
    hwnd
}
unsafe fn text(hwnd: HWND, id: usize) -> String {
    let field = GetDlgItem(hwnd, id as i32);
    let mut buffer = vec![0; GetWindowTextLengthW(field).min(16384) as usize + 1];
    let length = GetWindowTextW(field, buffer.as_mut_ptr(), buffer.len() as i32);
    String::from_utf16_lossy(&buffer[..length as usize])
}
unsafe fn check(hwnd: HWND, id: usize, value: bool) {
    SendMessageW(
        GetDlgItem(hwnd, id as i32),
        BM_SETCHECK,
        usize::from(value),
        0,
    );
}
unsafe fn checked(hwnd: HWND, id: usize) -> bool {
    SendMessageW(GetDlgItem(hwnd, id as i32), BM_GETCHECK, 0, 0) == 1
}
unsafe fn update_enabled(hwnd: HWND) {
    for id in [AUDIO_MIN, AUDIO_MAX] {
        EnableWindow(
            GetDlgItem(hwnd, id as i32),
            checked(hwnd, AUDIO_CUSTOM) as i32,
        );
    }
    #[cfg(feature = "experimental-bud")]
    EnableWindow(
        GetDlgItem(hwnd, AUDIO_RAW as i32),
        checked(hwnd, BUD) as i32,
    );
    #[cfg(not(feature = "experimental-bud"))]
    EnableWindow(GetDlgItem(hwnd, AUDIO_RAW as i32), 0);
    let hevc = checked(hwnd, HEVC) as i32;
    EnableWindow(GetDlgItem(hwnd, TEN_BIT as i32), hevc);
    #[cfg(feature = "vendor-video")]
    EnableWindow(GetDlgItem(hwnd, ZERO_COPY as i32), hevc);
    let turn = !checked(hwnd, STUN_ONLY);
    let cloudflare = SendMessageW(GetDlgItem(hwnd, PROVIDER as i32), CB_GETCURSEL, 0, 0) == 1;
    for id in [
        PROVIDER,
        TURN,
        USERNAME,
        PASSWORD,
        FORGET_PASSWORD,
        KEY,
        TOKEN,
        FORGET_TOKEN,
        TTL,
        CACHE,
        FORCE_RELAY,
    ] {
        let enabled = turn
            && match id {
                TURN | USERNAME | PASSWORD | FORGET_PASSWORD => !cloudflare,
                KEY | TOKEN | FORGET_TOKEN | TTL | CACHE => cloudflare,
                _ => true,
            };
        EnableWindow(GetDlgItem(hwnd, id as i32), enabled as i32);
    }
    layout(hwnd);
}
unsafe fn save(hwnd: HWND, page: &mut Page) -> anyhow::Result<()> {
    let mut s = page.saved.clone();
    s.prefer_hevc = checked(hwnd, HEVC);
    s.experimental_ten_bit = checked(hwnd, TEN_BIT);
    s.experimental_audio_buffer = checked(hwnd, AUDIO_CUSTOM);
    s.audio_min_buffer_ms = text(hwnd, AUDIO_MIN)
        .parse()
        .map_err(|_| anyhow::anyhow!("Enter an audio minimum in milliseconds"))?;
    s.audio_max_buffer_ms = text(hwnd, AUDIO_MAX)
        .parse()
        .map_err(|_| anyhow::anyhow!("Enter an audio maximum in milliseconds"))?;
    s.experimental_raw_audio = checked(hwnd, AUDIO_RAW);
    s.event_logging = checked(hwnd, EVENT_LOG);
    #[cfg(feature = "vendor-video")]
    {
        s.experimental_zero_copy = checked(hwnd, ZERO_COPY);
    }
    #[cfg(feature = "experimental-bud")]
    {
        s.experimental_bud = checked(hwnd, BUD);
    }
    let selected = SendMessageW(GetDlgItem(hwnd, GPU as i32), CB_GETCURSEL, 0, 0);
    s.video_gpu = usize::try_from(selected)
        .ok()
        .and_then(|i| page.gpu_choices.get(i))
        .ok_or_else(|| anyhow::anyhow!("Choose Automatic or an available video GPU"))?
        .clone();
    let urls = |id| {
        text(hwnd, id)
            .lines()
            .map(str::trim)
            .filter(|u| !u.is_empty())
            .map(|u| {
                if let Some((scheme, rest)) = u.split_once(':') {
                    format!("{}:{rest}", scheme.to_ascii_lowercase())
                } else {
                    u.into()
                }
            })
            .collect()
    };
    s.provider = if SendMessageW(GetDlgItem(hwnd, PROVIDER as i32), CB_GETCURSEL, 0, 0) == 1 {
        "cloudflare"
    } else {
        "custom"
    }
    .into();
    s.stun_urls = urls(STUN);
    s.turn_urls = urls(TURN);
    s.stun_only = checked(hwnd, STUN_ONLY);
    s.force_relay = checked(hwnd, FORCE_RELAY);
    s.custom_username = text(hwnd, USERNAME).trim().into();
    s.turn_key_id = text(hwnd, KEY).trim().into();
    s.ttl = text(hwnd, TTL)
        .parse()
        .map_err(|_| anyhow::anyhow!("Enter a valid credential lifetime"))?;
    s.cache_credentials = checked(hwnd, CACHE);
    page.manager.save(
        s,
        &text(hwnd, PASSWORD),
        &text(hwnd, TOKEN),
        checked(hwnd, FORGET_PASSWORD),
        checked(hwnd, FORGET_TOKEN),
    )?;
    page.saved = page.manager.view().0;
    for id in [PASSWORD, TOKEN] {
        SetWindowTextW(GetDlgItem(hwnd, id as i32), wide("").as_ptr());
    }
    check(hwnd, FORGET_PASSWORD, false);
    check(hwnd, FORGET_TOKEN, false);
    Ok(())
}

unsafe extern "system" fn page_proc(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if message == WM_NCCREATE {
        SetWindowLongPtrW(
            hwnd,
            GWLP_USERDATA,
            (*(lp as *const CREATESTRUCTW)).lpCreateParams as isize,
        );
    }
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Page;
    if !pointer.is_null() {
        let page = &mut *pointer;
        match message {
            WM_SIZE => {
                layout(hwnd);
                return 0;
            }
            WM_MOUSEWHEEL => {
                let requested = page.scroll - ((wp >> 16) as u16 as i16 as i32) * 72 / 120;
                scroll_to(hwnd, requested);
                return 0;
            }
            WM_VSCROLL => {
                let mut info = SCROLLINFO {
                    cbSize: size_of::<SCROLLINFO>() as u32,
                    fMask: SIF_ALL,
                    ..std::mem::zeroed()
                };
                GetScrollInfo(hwnd, SB_VERT, &mut info);
                let requested = match (wp & 0xffff) as i32 {
                    SB_LINEUP => page.scroll - 36,
                    SB_LINEDOWN => page.scroll + 36,
                    SB_PAGEUP => page.scroll - info.nPage as i32,
                    SB_PAGEDOWN => page.scroll + info.nPage as i32,
                    SB_THUMBTRACK | SB_THUMBPOSITION => info.nTrackPos,
                    SB_TOP => 0,
                    SB_BOTTOM => info.nMax,
                    _ => page.scroll,
                };
                scroll_to(hwnd, requested);
                return 0;
            }
            WM_MEASUREITEM if wp == PROVIDER || wp == GPU => {
                (*(lp as *mut MEASUREITEMSTRUCT)).itemHeight = 30;
                return 1;
            }
            WM_DRAWITEM => {
                let draw = &*(lp as *const DRAWITEMSTRUCT);
                if [
                    SAVE as u32,
                    BACK as u32,
                    NETWORK_TAB as u32,
                    VIDEO_TAB as u32,
                    AUDIO_TAB as u32,
                    PROVIDER as u32,
                    GPU as u32,
                ]
                .contains(&draw.CtlID)
                {
                    let dc = draw.hDC;
                    let state = SaveDC(dc);
                    let primary = draw.CtlID == SAVE as u32;
                    let selected_tab = (draw.CtlID == NETWORK_TAB as u32
                        && page.active_tab == Tab::Network)
                        || (draw.CtlID == VIDEO_TAB as u32 && page.active_tab == Tab::Video)
                        || (draw.CtlID == AUDIO_TAB as u32 && page.active_tab == Tab::Audio);
                    let color = if draw.itemState & ODS_SELECTED != 0 {
                        0x00404040
                    } else if primary || selected_tab {
                        ACCENT
                    } else {
                        FIELD
                    };
                    let brush = CreateSolidBrush(color);
                    FillRect(dc, &draw.rcItem, brush);
                    DeleteObject(brush);
                    SelectObject(dc, page.font);
                    SetBkMode(dc, TRANSPARENT as i32);
                    SetTextColor(
                        dc,
                        if draw.itemState & ODS_DISABLED != 0 {
                            MUTED
                        } else if (primary || selected_tab) && draw.itemState & ODS_SELECTED == 0 {
                            FIELD
                        } else {
                            FG
                        },
                    );
                    let combo = draw.CtlID == PROVIDER as u32 || draw.CtlID == GPU as u32;
                    let label = if draw.CtlID == PROVIDER as u32 {
                        match draw.itemID {
                            0 => "Custom servers".into(),
                            1 => "Cloudflare TURN".into(),
                            _ => String::new(),
                        }
                    } else if draw.CtlID == GPU as u32 {
                        page.gpu_labels
                            .get(draw.itemID as usize)
                            .cloned()
                            .unwrap_or_default()
                    } else if draw.CtlID == NETWORK_TAB as u32 {
                        "Network".into()
                    } else if draw.CtlID == VIDEO_TAB as u32 {
                        "Video".into()
                    } else if draw.CtlID == AUDIO_TAB as u32 {
                        "Audio".into()
                    } else if primary {
                        "Save settings".into()
                    } else {
                        "Back".into()
                    };
                    let mut rect = draw.rcItem;
                    rect.left += 10;
                    rect.right -= 10;
                    DrawTextW(
                        dc,
                        wide(&label).as_ptr(),
                        -1,
                        &mut rect,
                        DT_SINGLELINE | DT_VCENTER | if combo { DT_LEFT } else { DT_CENTER },
                    );
                    if draw.itemState & ODS_FOCUS != 0 {
                        InflateRect(&mut rect, -2, -4);
                        DrawFocusRect(dc, &rect);
                    }
                    RestoreDC(dc, state);
                    return 1;
                }
            }
            WM_COMMAND if matches!((wp >> 16) as u32, EN_SETFOCUS | BN_SETFOCUS | CBN_SETFOCUS) => {
                let mut rect: RECT = std::mem::zeroed();
                GetWindowRect(lp as HWND, &mut rect);
                MapWindowPoints(null_mut(), hwnd, (&mut rect as *mut RECT).cast(), 2);
                let mut client = std::mem::zeroed();
                GetClientRect(hwnd, &mut client);
                if rect.top < 8 {
                    scroll_to(hwnd, page.scroll + rect.top - 8);
                } else if rect.bottom > client.bottom - 8 {
                    scroll_to(hwnd, page.scroll + rect.bottom - client.bottom + 8);
                } else {
                    return 0;
                }
                return 0;
            }
            WM_COMMAND => match wp & 0xffff {
                NETWORK_TAB | VIDEO_TAB | AUDIO_TAB if (wp >> 16) as u32 == BN_CLICKED => {
                    page.active_tab = if wp & 0xffff == VIDEO_TAB {
                        Tab::Video
                    } else if wp & 0xffff == AUDIO_TAB {
                        Tab::Audio
                    } else {
                        Tab::Network
                    };
                    page.scroll = 0;
                    SetWindowTextW(
                        GetDlgItem(hwnd, SECTION as i32),
                        wide(if page.active_tab == Tab::Video {
                            "VIDEO"
                        } else if page.active_tab == Tab::Audio {
                            "AUDIO"
                        } else {
                            "NETWORK"
                        })
                        .as_ptr(),
                    );
                    SetWindowTextW(GetDlgItem(hwnd, NOTICE as i32), wide(if page.active_tab == Tab::Video {
                        "Video changes apply to your next connection."
                    } else if page.active_tab == Tab::Audio {
                        "Experimental audio changes apply on your next connection."
                    } else {
                        "ICE chooses the route automatically unless Force TURN relay is enabled.\nChanges apply to your next connection."
                    }).as_ptr());
                    layout(hwnd);
                    return 0;
                }
                SAVE | 1 => {
                    let status = match save(hwnd, page) {
                        Ok(()) => "Settings saved. They apply on your next connection.".into(),
                        Err(e) => e.to_string(),
                    };
                    SetWindowTextW(GetDlgItem(hwnd, STATUS as i32), wide(&status).as_ptr());
                    return 0;
                }
                BACK => {
                    PostMessageW(GetParent(hwnd), WM_APP + 9, 0, 0);
                    return 0;
                }
                PROVIDER if (wp >> 16) as u32 == CBN_SELCHANGE => {
                    update_enabled(hwnd);
                    return 0;
                }
                HEVC if (wp >> 16) as u32 == BN_CLICKED => {
                    update_enabled(hwnd);
                    return 0;
                }
                AUDIO_CUSTOM if (wp >> 16) as u32 == BN_CLICKED => {
                    update_enabled(hwnd);
                    return 0;
                }
                #[cfg(feature = "experimental-bud")]
                BUD if (wp >> 16) as u32 == BN_CLICKED => {
                    update_enabled(hwnd);
                    return 0;
                }
                STUN_ONLY if (wp >> 16) as u32 == BN_CLICKED => {
                    if checked(hwnd, STUN_ONLY) {
                        check(hwnd, FORCE_RELAY, false);
                    }
                    update_enabled(hwnd);
                    return 0;
                }
                _ => {}
            },
            WM_CTLCOLORSTATIC | WM_CTLCOLORBTN | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => {
                let dc = wp as HDC;
                let edit = matches!(message, WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX);
                let id = GetDlgCtrlID(lp as HWND) as usize;
                SetTextColor(
                    dc,
                    if id == SECTION {
                        ACCENT
                    } else if matches!(id, HELP | NOTICE | STATUS)
                        || IsWindowEnabled(lp as HWND) == 0
                    {
                        MUTED
                    } else {
                        FG
                    },
                );
                SetBkColor(dc, if edit { FIELD } else { BG });
                return if edit { page.field } else { page.background } as isize;
            }
            WM_ERASEBKGND => {
                let mut rect = std::mem::zeroed();
                GetClientRect(hwnd, &mut rect);
                FillRect(wp as HDC, &rect, page.background);
                return 1;
            }
            WM_PAINT => {
                let mut paint = std::mem::zeroed();
                let dc = BeginPaint(hwnd, &mut paint);
                paint_page(hwnd, dc, page);
                EndPaint(hwnd, &paint);
                return 0;
            }
            WM_PRINTCLIENT => {
                paint_page(hwnd, wp as HDC, page);
                return 0;
            }
            WM_NCDESTROY => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                if page.window_owned {
                    drop(Box::from_raw(pointer));
                }
            }
            _ => {}
        }
    }
    DefWindowProcW(hwnd, message, wp, lp)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_controls_save_and_reopen_without_guest_or_real_profile() {
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce).unwrap();
        let suffix: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
        let directory = std::env::temp_dir().join(format!("parsec-settings-ui-{suffix}"));
        std::fs::create_dir(&directory).unwrap();
        let manager = Manager::open(directory.join("settings.json"));
        unsafe {
            let parent = CreateWindowExW(
                0,
                wide("STATIC").as_ptr(),
                wide("Settings fixture").as_ptr(),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                920,
                750,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            );
            assert!(!parent.is_null());
            install_menu(parent);
            assert_eq!(GetMenuItemCount(GetMenu(parent)), 3);
            let help = GetSubMenu(GetMenu(parent), 2);
            assert!(!help.is_null());
            assert_eq!(GetMenuItemID(help, 0), ABOUT as u32);
            assert_eq!(GetMenuItemID(help, 1), crate::updater_ui::OPEN as u32);
            let page = open(parent, manager.clone());
            assert!(!page.is_null());
            assert_ne!(
                GetWindowLongW(GetDlgItem(page, TURN as i32), GWL_STYLE) as u32 & WS_VISIBLE,
                0
            );
            assert_eq!(
                GetWindowLongW(GetDlgItem(page, KEY as i32), GWL_STYLE) as u32 & WS_VISIBLE,
                0
            );
            assert_no_overlaps(page);
            capture(page, "custom");
            controls::verify_multiline(GetDlgItem(page, STUN as i32));
            SetWindowTextW(
                GetDlgItem(page, USERNAME as i32),
                wide("unsaved-user").as_ptr(),
            );
            SendMessageW(GetDlgItem(page, PROVIDER as i32), CB_SETCURSEL, 1, 0);
            update_enabled(page);
            assert_eq!(
                GetWindowLongW(GetDlgItem(page, TURN as i32), GWL_STYLE) as u32 & WS_VISIBLE,
                0
            );
            assert_ne!(
                GetWindowLongW(GetDlgItem(page, KEY as i32), GWL_STYLE) as u32 & WS_VISIBLE,
                0
            );
            assert_no_overlaps(page);
            capture(page, "cloudflare");
            assert_combo_label_visible(page, PROVIDER);
            paint_regression(page);
            scroll_boundary_does_not_repaint(page);
            // The last control remains reachable at a small window size and
            // keyboard focus scrolls it into view without touching real data.
            SendMessageW(
                page,
                WM_COMMAND,
                SAVE | ((BN_SETFOCUS as usize) << 16),
                GetDlgItem(page, SAVE as i32) as isize,
            );
            let mut button = std::mem::zeroed();
            let mut client = std::mem::zeroed();
            GetWindowRect(GetDlgItem(page, SAVE as i32), &mut button);
            MapWindowPoints(null_mut(), page, (&mut button as *mut RECT).cast(), 2);
            GetClientRect(page, &mut client);
            assert!(button.top >= 0 && button.bottom <= client.bottom);
            SendMessageW(page, WM_VSCROLL, SB_BOTTOM as usize, 0);
            capture(page, "cloudflare-bottom");
            SetWindowTextW(
                GetDlgItem(page, TOKEN as i32),
                wide("fixture-token").as_ptr(),
            );
            #[cfg(feature = "experimental-bud")]
            check(page, BUD, true);
            SendMessageW(page, WM_COMMAND, VIDEO_TAB, 0);
            assert_eq!(IsWindowVisible(GetDlgItem(page, PROVIDER as i32)), 0);
            assert_ne!(IsWindowVisible(GetDlgItem(page, GPU as i32)), 0);
            check(page, HEVC, true);
            assert_no_overlaps(page);
            capture(page, "video");
            assert_combo_label_visible(page, GPU);
            SendMessageW(page, WM_COMMAND, AUDIO_TAB, 0);
            assert_ne!(IsWindowVisible(GetDlgItem(page, AUDIO_CUSTOM as i32)), 0);
            assert_eq!(IsWindowEnabled(GetDlgItem(page, AUDIO_MIN as i32)), 0);
            check(page, AUDIO_CUSTOM, true);
            update_enabled(page);
            assert_ne!(IsWindowEnabled(GetDlgItem(page, AUDIO_MIN as i32)), 0);
            SetWindowTextW(GetDlgItem(page, AUDIO_MIN as i32), wide("20").as_ptr());
            SetWindowTextW(GetDlgItem(page, AUDIO_MAX as i32), wide("60").as_ptr());
            check(page, AUDIO_RAW, true);
            assert_no_overlaps(page);
            capture(page, "audio");
            SendMessageW(page, WM_COMMAND, NETWORK_TAB, 0);
            assert_eq!(IsWindowVisible(GetDlgItem(page, GPU as i32)), 0);
            assert!(checked(page, HEVC));
            assert_eq!(text(page, TOKEN), "fixture-token");
            assert_eq!(text(page, USERNAME), "unsaved-user");
            SendMessageW(GetDlgItem(page, PROVIDER as i32), CB_SETCURSEL, 0, 0);
            update_enabled(page);
            assert_eq!(text(page, USERNAME), "unsaved-user");
            controls::verify_enter(
                page,
                GetDlgItem(page, STUN as i32),
                "stun:stun.example.org:3478",
                "stun:second.example.invalid:3478",
            );
            controls::verify_enter(
                page,
                GetDlgItem(page, TURN as i32),
                "turn:relay.example.invalid:3478?transport=udp",
                "turns:relay.example.invalid:443?transport=tcp",
            );
            SetWindowTextW(
                GetDlgItem(page, PASSWORD as i32),
                wide("fixture-password").as_ptr(),
            );
            check(page, FORCE_RELAY, true);
            check(page, TEN_BIT, true);
            check(page, EVENT_LOG, true);
            SendMessageW(page, WM_COMMAND, SAVE, 0);
            assert!(manager.view().0.force_relay);
            assert!(manager.view().0.experimental_ten_bit);
            assert!(manager.view().0.event_logging);
            assert!(manager.view().0.experimental_audio_buffer);
            assert_eq!(manager.view().0.audio_min_buffer_ms, 20);
            assert_eq!(manager.view().0.audio_max_buffer_ms, 60);
            assert!(manager.view().0.experimental_raw_audio);
            #[cfg(feature = "experimental-bud")]
            assert!(manager.view().0.experimental_bud);
            assert_eq!(manager.view().0.stun_urls.len(), 2);
            assert_eq!(manager.view().0.turn_urls.len(), 2);
            check(page, STUN_ONLY, true);
            SendMessageW(
                page,
                WM_COMMAND,
                STUN_ONLY | ((BN_CLICKED as usize) << 16),
                0,
            );
            assert!(!checked(page, FORCE_RELAY));
            check(page, HEVC, true);
            update_enabled(page);
            assert_eq!(IsWindowEnabled(GetDlgItem(page, TURN as i32)), 0);
            let state = &*(GetWindowLongPtrW(page, GWLP_USERDATA) as *const Page);
            let selected = usize::from(state.gpu_choices.len() > 1);
            let expected_gpu = state.gpu_choices[selected].clone();
            SendMessageW(GetDlgItem(page, GPU as i32), CB_SETCURSEL, selected, 0);
            SendMessageW(page, WM_COMMAND, SAVE, 0);
            assert!(text(page, STATUS).starts_with("Settings saved"));
            assert_eq!(manager.view().0.video_gpu, expected_gpu);
            #[cfg(feature = "vendor-video")]
            {
                assert!(!manager.view().0.experimental_zero_copy);
                check(page, ZERO_COPY, true);
                SendMessageW(page, WM_COMMAND, SAVE, 0);
                assert!(manager.view().0.experimental_zero_copy);
            }
            assert!(manager.view().0.prefer_hevc);
            DestroyWindow(page);
            let page = open(parent, manager.clone());
            assert_eq!(
                text(page, STUN),
                "stun:stun.example.org:3478\r\nstun:second.example.invalid:3478"
            );
            assert_eq!(text(page, TURN), "turn:relay.example.invalid:3478?transport=udp\r\nturns:relay.example.invalid:443?transport=tcp");
            assert!(checked(page, STUN_ONLY));
            #[cfg(feature = "vendor-video")]
            assert!(checked(page, ZERO_COPY));
            assert!(checked(page, HEVC));
            assert_eq!(
                SendMessageW(GetDlgItem(page, GPU as i32), CB_GETCURSEL, 0, 0),
                selected as isize
            );
            SendMessageW(GetDlgItem(page, GPU as i32), CB_SETCURSEL, 0, 0);
            SendMessageW(page, WM_COMMAND, SAVE, 0);
            assert!(manager.view().0.video_gpu.is_none());
            DestroyWindow(page);
            DestroyWindow(parent);
        }
        assert!(
            Manager::open(directory.join("settings.json"))
                .view()
                .0
                .stun_only
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    unsafe fn assert_combo_label_visible(hwnd: HWND, id: usize) {
        let mut rect: RECT = std::mem::zeroed();
        GetWindowRect(GetDlgItem(hwnd, id as i32), &mut rect);
        MapWindowPoints(null_mut(), hwnd, (&mut rect as *mut RECT).cast(), 2);
        let (width, height, pixels) = snapshot(hwnd, false);
        assert!(rect.top >= 0 && rect.bottom <= height as i32);
        let mut foreground = 0;
        // Exclude the border and arrow; a blank owner-drawn selection must fail.
        for y in rect.top + 5..rect.bottom - 5 {
            for x in rect.left + 10..rect.right - 28 {
                let offset = (y as usize * width as usize + x as usize) * 4;
                if pixels[offset..offset + 3].iter().all(|v| *v > 140) {
                    foreground += 1;
                }
            }
        }
        assert!(foreground > 20, "Missing selected text in combo {id}");
    }

    unsafe fn assert_no_overlaps(hwnd: HWND) {
        let page = &*(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Page);
        let rectangles: Vec<_> = page
            .items
            .iter()
            .filter(|item| GetWindowLongW(item.hwnd, GWL_STYLE) as u32 & WS_VISIBLE != 0)
            .map(|item| {
                let mut rect: RECT = std::mem::zeroed();
                GetWindowRect(item.hwnd, &mut rect);
                (item.hwnd, rect)
            })
            .collect();
        for (i, (first, a)) in rectangles.iter().enumerate() {
            for (second, b) in &rectangles[i + 1..] {
                assert!(
                    a.left >= b.right
                        || b.left >= a.right
                        || a.top >= b.bottom
                        || b.top >= a.bottom,
                    "Overlapping settings controls: {:?} and {:?}",
                    first,
                    second
                );
            }
        }
    }

    unsafe fn scroll_boundary_does_not_repaint(hwnd: HWND) {
        use std::cell::Cell;
        use windows_sys::Win32::UI::Shell::{
            DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass,
        };
        unsafe extern "system" fn observe(
            hwnd: HWND,
            msg: u32,
            wp: WPARAM,
            lp: LPARAM,
            _id: usize,
            data: usize,
        ) -> LRESULT {
            if msg == WM_PAINT {
                let count = &*(data as *const Cell<usize>);
                count.set(count.get() + 1);
            }
            DefSubclassProc(hwnd, msg, wp, lp)
        }
        assert_ne!(
            GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_COMPOSITED,
            0
        );
        assert_eq!(
            GetClassLongW(hwnd, GCL_STYLE) & (CS_OWNDC | CS_CLASSDC | CS_PARENTDC),
            0
        );
        let paints = Cell::new(0usize);
        let id = 0x5057;
        assert_ne!(
            SetWindowSubclass(
                hwnd,
                Some(observe),
                id,
                (&paints as *const Cell<usize>) as usize
            ),
            0
        );
        struct Remove(HWND, usize);
        impl Drop for Remove {
            fn drop(&mut self) {
                unsafe {
                    RemoveWindowSubclass(self.0, Some(observe), self.1);
                }
            }
        }
        let _remove = Remove(hwnd, id);
        for (edge, delta, line, page) in [
            (SB_BOTTOM, -120i16, SB_LINEDOWN, SB_PAGEDOWN),
            (SB_TOP, 120i16, SB_LINEUP, SB_PAGEUP),
        ] {
            SendMessageW(hwnd, WM_VSCROLL, edge as usize, 0);
            paints.set(0);
            for _ in 0..5 {
                SendMessageW(hwnd, WM_MOUSEWHEEL, (delta as u16 as usize) << 16, 0);
                SendMessageW(hwnd, WM_VSCROLL, line as usize, 0);
                SendMessageW(hwnd, WM_VSCROLL, page as usize, 0);
                SendMessageW(hwnd, WM_VSCROLL, SB_ENDSCROLL as usize, 0);
            }
            assert_eq!(
                paints.get(),
                0,
                "Scrolling beyond the boundary repainted Settings"
            );
        }
        SendMessageW(hwnd, WM_MOUSEWHEEL, (-120i16 as u16 as usize) << 16, 0);
        assert!(paints.get() > 0, "Real scrolling must repaint");
    }

    // Compare the actual incremental window painting against a clean repaint.
    // WM_PRINT alone cannot catch stale on-screen pixels: it always paints fresh.
    unsafe fn paint_regression(hwnd: HWND) {
        let parent = GetParent(hwnd);
        SetFocus(parent); // Suppress edit carets so both snapshots are deterministic.
        SetWindowPos(
            parent,
            HWND_TOPMOST,
            0,
            0,
            920,
            750,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        for (width, height) in [(920, 600), (1000, 740), (920, 750)] {
            SetWindowPos(
                parent,
                null_mut(),
                0,
                0,
                width,
                height,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
            let mut client: RECT = std::mem::zeroed();
            GetClientRect(parent, &mut client);
            SetWindowPos(
                hwnd,
                null_mut(),
                0,
                0,
                client.right,
                client.bottom,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            for direction in [SB_LINEDOWN, SB_LINEUP] {
                for _ in 0..6 {
                    SendMessageW(hwnd, WM_VSCROLL, direction as usize, 0);
                    assert_no_overlaps(hwnd);
                    let actual = snapshot(hwnd, true);
                    RedrawWindow(
                        hwnd,
                        null(),
                        null_mut(),
                        RDW_INVALIDATE | RDW_ERASE | RDW_FRAME | RDW_ALLCHILDREN | RDW_UPDATENOW,
                    );
                    let clean = snapshot(hwnd, true);
                    let changed = actual
                        .2
                        .iter()
                        .zip(&clean.2)
                        .filter(|(a, b)| a != b)
                        .count();
                    assert_eq!(
                        changed, 0,
                        "Stale pixels after scroll/resize at {width}x{height}"
                    );
                }
            }
        }
        capture(hwnd, "cloudflare-after-scroll-resize");
    }

    // Optional CI-only visual fixtures. This cannot capture an account, stream
    // or production profile: the fixture above owns only synthetic controls.
    unsafe fn capture(hwnd: HWND, name: &str) {
        // Visual assertions also need a painted window when PNG export is off.
        ShowWindow(GetParent(hwnd), SW_SHOWNOACTIVATE);
        RedrawWindow(
            GetParent(hwnd),
            null(),
            null_mut(),
            RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN | RDW_UPDATENOW,
        );
        let Some(directory) = std::env::var_os("PARSEC_SETTINGS_SCREENSHOTS") else {
            return;
        };
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        let (width, height, rgba) = snapshot(hwnd, false);
        image::save_buffer(
            directory.join(format!("{name}.png")),
            &rgba,
            width,
            height,
            image::ColorType::Rgba8,
        )
        .unwrap();
    }

    unsafe fn snapshot(hwnd: HWND, live: bool) -> (u32, u32, Vec<u8>) {
        let mut rect: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut rect);
        let dc = CreateCompatibleDC(null_mut());
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = rect.right;
        info.bmiHeader.biHeight = -rect.bottom;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let mut pixels = null_mut();
        let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut pixels, null_mut(), 0);
        assert!(!bitmap.is_null() && !dc.is_null() && !pixels.is_null());
        let previous = SelectObject(dc, bitmap);
        if live {
            let source = GetDC(hwnd);
            assert!(!source.is_null());
            let copied = BitBlt(dc, 0, 0, rect.right, rect.bottom, source, 0, 0, SRCCOPY);
            ReleaseDC(hwnd, source);
            assert_ne!(copied, 0);
        } else {
            SendMessageW(
                hwnd,
                WM_PRINT,
                dc as usize,
                (PRF_CLIENT | PRF_NONCLIENT | PRF_CHILDREN | PRF_ERASEBKGND) as isize,
            );
        }
        GdiFlush();
        let mut rgba = std::slice::from_raw_parts(
            pixels.cast::<u8>(),
            (rect.right * rect.bottom * 4) as usize,
        )
        .to_vec();
        for pixel in rgba.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
            pixel[3] = 255;
        }
        SelectObject(dc, previous);
        DeleteObject(bitmap);
        DeleteDC(dc);
        (rect.right as u32, rect.bottom as u32, rgba)
    }
}
