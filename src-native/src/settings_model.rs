//! Editable Settings state and rules without window/control APIs.
use crate::connection_settings::{full_chroma_vendor_allowed, GpuPreference, Manager, Settings};
use anyhow::{Context, Result};
use zeroize::Zeroizing;

pub const TCP_QOS_LABEL: &str = "TCP/TLS QoS";
pub const TCP_QOS_HINT: &str = "Windows outgoing TURN traffic only; network support varies.\nUnsupported or refused: continue unmarked.\nReconnect after changing.";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Network,
    Video,
    Audio,
    Input,
}
impl Tab {
    pub fn heading(self) -> &'static str {
        match self {
            Self::Network => "NETWORK",
            Self::Video => "VIDEO",
            Self::Audio => "AUDIO",
            Self::Input => "INPUT",
        }
    }
    pub fn notice(self) -> &'static str {
        match self {
            Self::Network => "ICE chooses the route automatically unless Force TURN relay is enabled.\nChanges apply to your next connection.",
            Self::Video => "Video changes apply to your next connection.",
            Self::Audio => "Audio changes apply on your next connection.",
            Self::Input => "Enable BUD on the Network tab to use HID input.\nInput changes apply to your next connection.",
        }
    }
}

/// Compound choices retain the existing settings schema and transport rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoFormat {
    H264,
    Hevc8,
    Hevc10,
}
impl VideoFormat {
    pub const ALL: [Self; 3] = [Self::H264, Self::Hevc8, Self::Hevc10];
    pub const LABELS: [&'static str; 3] = [
        "H.264",
        "H.265 / HEVC (8-bit)",
        "H.265 / HEVC (10-bit, experimental)",
    ];
    pub fn current(s: &Settings) -> Self {
        if !s.prefer_hevc {
            Self::H264
        } else if s.experimental_ten_bit {
            Self::Hevc10
        } else {
            Self::Hevc8
        }
    }
    pub fn apply(self, s: &mut Settings) {
        s.prefer_hevc = self != Self::H264;
        // Keep a dormant preference when HEVC is disabled, as older settings do.
        if self != Self::H264 {
            s.experimental_ten_bit = self == Self::Hevc10;
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteMode {
    Automatic,
    DirectOnly,
    ForceRelay,
}
impl RouteMode {
    pub const ALL: [Self; 3] = [Self::Automatic, Self::DirectOnly, Self::ForceRelay];
    pub const LABELS: [&'static str; 3] = [
        "Automatic (direct or TURN)",
        "Direct only (STUN / LAN / VPN)",
        "Force TURN relay",
    ];
    pub fn current(s: &Settings) -> Self {
        if s.stun_only {
            Self::DirectOnly
        } else if s.force_relay {
            Self::ForceRelay
        } else {
            Self::Automatic
        }
    }
    pub fn apply(self, s: &mut Settings) {
        s.stun_only = self == Self::DirectOnly;
        s.force_relay = self == Self::ForceRelay;
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputMode {
    Standard,
    HidMouse,
    HidKeyboard,
    HidBoth,
}
impl InputMode {
    pub const ALL: [Self; 4] = [
        Self::Standard,
        Self::HidMouse,
        Self::HidKeyboard,
        Self::HidBoth,
    ];
    pub const LABELS: [&'static str; 4] = [
        "Standard input",
        "USB HID mouse (experimental)",
        "USB HID keyboard (experimental)",
        "USB HID mouse + keyboard (experimental)",
    ];
    pub fn current(s: &Settings) -> Self {
        #[cfg(feature = "bud")]
        {
            match (s.experimental_hid_mouse, s.experimental_hid_keyboard) {
                (false, false) => Self::Standard,
                (true, false) => Self::HidMouse,
                (false, true) => Self::HidKeyboard,
                (true, true) => Self::HidBoth,
            }
        }
        #[cfg(not(feature = "bud"))]
        {
            let _ = s;
            Self::Standard
        }
    }
    pub fn apply(self, s: &mut Settings) {
        #[cfg(feature = "bud")]
        {
            s.experimental_hid_mouse = matches!(self, Self::HidMouse | Self::HidBoth);
            s.experimental_hid_keyboard = matches!(self, Self::HidKeyboard | Self::HidBoth);
        }
        #[cfg(not(feature = "bud"))]
        {
            let _ = (self, s);
        }
    }
}
pub enum BinaryChoice {
    TcpQos,
    Chroma,
    Rendering,
    AmfMain10,
    AudioBuffer,
    AudioCodec,
    Speakers,
}
impl BinaryChoice {
    pub fn labels(self) -> [&'static str; 2] {
        match self {
            Self::TcpQos => ["Off (default)", "Audio/video QoS (experimental)"],
            Self::Chroma => ["4:2:0", "Prefer 4:4:4 (experimental)"],
            Self::Rendering => ["Normal GPU rendering", "Zero copy (experimental, HEVC)"],
            Self::AmfMain10 => [
                "Media Foundation (default)",
                "AMD AMF Main10 (experimental)",
            ],
            Self::AudioBuffer => ["Default (core limits)", "Custom limits"],
            Self::AudioCodec => ["Opus", "Allow raw PCM (BUD)"],
            Self::Speakers => ["Stereo", "Match output layout (experimental, BUD)"],
        }
    }
}

pub struct Availability {
    pub bud: bool,
    pub audio_limits: bool,
    pub amf_main10: bool,
    pub chroma_444: bool,
    pub zero_copy: bool,
    pub relay: bool,
    pub custom_turn: bool,
    pub cloudflare_turn: bool,
}

// Deliberately no Debug/Serialize: plaintext replacement secrets are transient.
pub struct Form {
    pub active_tab: Tab,
    pub values: Settings,
    pub gpu_choices: Vec<Option<GpuPreference>>,
    pub default_gpu_vendor: Option<u32>,
    pub gpu_selection: Option<usize>,
    pub audio_min: String,
    pub audio_max: String,
    pub ttl: String,
    pub stun_urls: String,
    pub turn_urls: String,
    pub password: Zeroizing<String>,
    pub token: Zeroizing<String>,
    pub forget_password: bool,
    pub forget_token: bool,
    pub feedback: String,
}
impl Form {
    pub fn new(values: Settings, default_gpu_vendor: Option<u32>) -> Self {
        Self {
            active_tab: Tab::Network,
            audio_min: values.audio_min_buffer_ms.to_string(),
            audio_max: values.audio_max_buffer_ms.to_string(),
            ttl: values.ttl.to_string(),
            stun_urls: values.stun_urls.join("\r\n"),
            turn_urls: values.turn_urls.join("\r\n"),
            values,
            gpu_choices: vec![None],
            default_gpu_vendor,
            gpu_selection: Some(0),
            password: Zeroizing::new(String::new()),
            token: Zeroizing::new(String::new()),
            forget_password: false,
            forget_token: false,
            feedback: String::new(),
        }
    }
    pub fn cloudflare(&self) -> bool {
        self.values.provider == "cloudflare"
    }
    pub fn availability(&mut self) -> Availability {
        let vendor = self
            .gpu_selection
            .and_then(|i| self.gpu_choices.get(i))
            .and_then(|gpu| {
                gpu.as_ref()
                    .map(|g| g.vendor_id)
                    .or(self.default_gpu_vendor)
            });
        let chroma_allowed = vendor.is_some_and(full_chroma_vendor_allowed);
        if !chroma_allowed {
            self.values.experimental_chroma_444 = false;
        }
        let relay = !self.values.stun_only;
        Availability {
            bud: {
                #[cfg(feature = "bud")]
                {
                    self.values.bud
                }
                #[cfg(not(feature = "bud"))]
                {
                    false
                }
            },
            audio_limits: self.values.custom_audio_buffer,
            amf_main10: cfg!(feature = "vendor-video")
                && vendor == Some(0x1002)
                && self.values.prefer_hevc
                && self.values.experimental_ten_bit
                && !self.values.experimental_chroma_444,
            chroma_444: self.values.prefer_hevc && chroma_allowed,
            zero_copy: self.values.prefer_hevc && !self.values.experimental_chroma_444,
            relay,
            custom_turn: relay && !self.cloudflare(),
            cloudflare_turn: relay && self.cloudflare(),
        }
    }
    fn candidate(&self) -> Result<Settings> {
        let mut settings = self.values.clone();
        // Preserve the existing parse order and validation even for inactive
        // numeric fields. Domain/secret validation remains in Manager::save.
        settings.audio_min_buffer_ms = self
            .audio_min
            .parse()
            .context("Enter an audio minimum in milliseconds")?;
        settings.audio_max_buffer_ms = self
            .audio_max
            .parse()
            .context("Enter an audio maximum in milliseconds")?;
        settings.video_gpu = self
            .gpu_selection
            .and_then(|i| self.gpu_choices.get(i))
            .context("Choose Automatic or an available video GPU")?
            .clone();
        settings.stun_urls = urls(&self.stun_urls);
        settings.turn_urls = urls(&self.turn_urls);
        settings.custom_username = settings.custom_username.trim().into();
        settings.turn_key_id = settings.turn_key_id.trim().into();
        settings.ttl = self
            .ttl
            .parse()
            .context("Enter a valid credential lifetime")?;
        Ok(settings)
    }
    pub fn save(&mut self, manager: &Manager) -> bool {
        let result = self.candidate().and_then(|settings| {
            manager.save(
                settings,
                &self.password,
                &self.token,
                self.forget_password,
                self.forget_token,
            )
        });
        match result {
            Ok(()) => {
                self.values = manager.view().0;
                self.password = Zeroizing::new(String::new());
                self.token = Zeroizing::new(String::new());
                self.forget_password = false;
                self.forget_token = false;
                self.feedback = "Settings saved. They apply on your next connection.".into();
                true
            }
            Err(error) => {
                self.feedback = error.to_string();
                false
            }
        }
    }
}
fn urls(text: &str) -> Vec<String> {
    text.lines()
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
}

#[cfg(test)]
mod tests {
    use super::*;
    fn form() -> Form {
        Form::new(
            Settings {
                provider: "custom".into(),
                ..Settings::default()
            },
            Some(0x10de),
        )
    }
    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            let mut bytes = [0; 8];
            getrandom::fill(&mut bytes).unwrap();
            let dir = std::env::temp_dir()
                .join(format!("evoparseq-form-{:x}", u64::from_le_bytes(bytes)));
            std::fs::create_dir(&dir).unwrap();
            Self(dir.join("settings.json"))
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
            let _ = std::fs::remove_dir(self.0.parent().unwrap());
        }
    }
    #[test]
    fn media_dependencies_preserve_dormant_preferences_and_do_not_infer_capabilities() {
        let mut form = form();
        form.values.experimental_ten_bit = true;
        form.values.experimental_chroma_444 = true;
        #[cfg(feature = "vendor-video")]
        {
            form.values.experimental_zero_copy = true;
        }
        for vendor in [Some(0x10de), Some(0x8086), Some(0x1002), None] {
            for hevc in [false, true] {
                form.default_gpu_vendor = vendor;
                form.values.prefer_hevc = hevc;
                form.values.experimental_chroma_444 = true;
                let eligible = matches!(vendor, Some(0x10de | 0x8086));
                let enabled = form.availability();
                assert_eq!(enabled.chroma_444, hevc && eligible);
                assert_eq!(enabled.zero_copy, hevc && !eligible);
                assert_eq!(form.values.experimental_chroma_444, eligible);
                assert!(form.values.experimental_ten_bit);
                #[cfg(feature = "vendor-video")]
                assert!(form.values.experimental_zero_copy);
            }
        }
        // A selected adapter overrides Automatic's vendor; a missing list
        // selection must not borrow the default adapter's eligibility.
        form.default_gpu_vendor = Some(0x10de);
        form.gpu_choices.push(Some(GpuPreference {
            name: "Synthetic AMD".into(),
            vendor_id: 0x1002,
            device_id: 1,
            subsystem_id: 0,
            revision: 0,
            luid: 1,
        }));
        form.gpu_selection = Some(1);
        assert!(!form.availability().chroma_444);
        form.gpu_selection = None;
        assert!(!form.availability().chroma_444);
    }
    #[test]
    fn network_audio_and_hid_dependencies_preserve_hidden_edits() {
        let mut form = form();
        form.values.allow_raw_audio = true;
        form.values.experimental_surround_audio = true;
        #[cfg(feature = "bud")]
        {
            form.values.experimental_hid_mouse = true;
            form.values.experimental_hid_keyboard = true;
        }
        form.turn_urls = "turns:example.invalid:443?transport=tcp".into();
        for cloudflare in [false, true] {
            form.values.provider = if cloudflare { "cloudflare" } else { "custom" }.into();
            for stun_only in [false, true] {
                (if stun_only {
                    RouteMode::DirectOnly
                } else {
                    RouteMode::Automatic
                })
                .apply(&mut form.values);
                let enabled = form.availability();
                assert_eq!(enabled.relay, !stun_only);
                assert_eq!(enabled.custom_turn, !stun_only && !cloudflare);
                assert_eq!(enabled.cloudflare_turn, !stun_only && cloudflare);
                assert!(!enabled.audio_limits);
                assert!(!enabled.bud);
            }
        }
        form.values.custom_audio_buffer = true;
        assert!(form.availability().audio_limits);
        form.values.force_relay = true;
        RouteMode::DirectOnly.apply(&mut form.values);
        assert!(!form.values.force_relay);
        RouteMode::Automatic.apply(&mut form.values);
        assert!(!form.values.force_relay); // No implicit re-enable.
        #[cfg(feature = "bud")]
        {
            form.values.bud = true;
            assert!(form.availability().bud);
            assert!(form.values.experimental_hid_mouse);
            assert!(form.values.experimental_hid_keyboard);
        }
        assert!(form.values.allow_raw_audio && form.values.experimental_surround_audio);
        assert!(form.turn_urls.ends_with("transport=tcp"));
    }
    #[test]
    fn parsing_keeps_existing_limits_error_order_and_url_transport_details() {
        let mut form = form();
        form.audio_min = "wrong".into();
        form.audio_max = "wrong".into();
        form.gpu_selection = None;
        assert_eq!(
            form.candidate().err().unwrap().to_string(),
            "Enter an audio minimum in milliseconds"
        );
        form.audio_min = "0".into();
        assert_eq!(
            form.candidate().err().unwrap().to_string(),
            "Enter an audio maximum in milliseconds"
        );
        form.audio_max = "100".into();
        assert!(form
            .candidate()
            .err()
            .unwrap()
            .to_string()
            .starts_with("Choose Automatic"));
        form.gpu_selection = Some(0);
        form.ttl = "bad".into();
        assert_eq!(
            form.candidate().err().unwrap().to_string(),
            "Enter a valid credential lifetime"
        );
        form.ttl = "86400".into();
        form.stun_urls = " STUN:stun.example.invalid:3478 \r\n\n".into();
        form.turn_urls = " TURNS:relay.example.invalid:443?transport=tcp \nTURN:relay.example.invalid:80?transport=tcp".into();
        form.values.custom_username = " fixture-user ".into();
        let parsed = form.candidate().unwrap();
        assert_eq!(parsed.custom_username, "fixture-user");
        assert_eq!(parsed.stun_urls, ["stun:stun.example.invalid:3478"]);
        assert_eq!(
            parsed.turn_urls,
            [
                "turns:relay.example.invalid:443?transport=tcp",
                "turn:relay.example.invalid:80?transport=tcp"
            ]
        );
    }
    #[test]
    fn failed_save_preserves_edits_success_encrypts_and_blank_edits_keep_secrets() {
        let fixture = Fixture::new();
        let manager = Manager::open(fixture.0.clone());
        let mut form = form();
        form.values.custom_audio_buffer = true;
        form.audio_min = "101".into();
        form.audio_max = "100".into();
        form.password = Zeroizing::new("fixture-password-sentinel".into());
        form.token = Zeroizing::new("fixture-token-sentinel".into());
        assert!(!form.save(&manager));
        assert!(form.feedback.starts_with("Audio buffer must satisfy"));
        assert_eq!(form.audio_min, "101");
        assert!(!form.feedback.contains("sentinel"));
        assert!(!fixture.0.exists());
        form.audio_min = "20".into();
        form.turn_urls = "turns:relay.example.invalid:443?transport=tcp".into();
        form.values.custom_username = "fixture-user".into();
        form.values
            .extra
            .insert("futureOption".into(), serde_json::json!({"keep":true}));
        // An unrelated profile edit while Settings is open must survive Save.
        manager
            .update_native(|p| p.email = "concurrent@example.invalid".into())
            .unwrap();
        form.values.native_core.hide_session_button = true;
        assert!(form.save(&manager));
        assert!(form.feedback.starts_with("Settings saved"));
        assert!(form.password.is_empty() && form.token.is_empty());
        let saved = manager.view().0;
        assert_eq!(saved.secret().unwrap(), "fixture-password-sentinel");
        assert_eq!(saved.native_core.email, "concurrent@example.invalid");
        assert!(saved.native_core.hide_session_button);
        assert_eq!(saved.extra["futureOption"]["keep"], true);
        let raw = std::fs::read_to_string(&fixture.0).unwrap();
        assert!(!raw.contains("fixture-password-sentinel"));
        assert!(!raw.contains("fixture-token-sentinel"));
        assert!(form.save(&manager));
        assert_eq!(
            manager.view().0.encrypted_custom_password,
            saved.encrypted_custom_password
        );
        assert_eq!(
            manager.view().0.encrypted_api_token,
            saved.encrypted_api_token
        );
        RouteMode::DirectOnly.apply(&mut form.values);
        form.forget_password = true;
        form.forget_token = true;
        assert!(form.save(&manager));
        assert!(manager.view().0.encrypted_custom_password.is_empty());
        assert!(manager.view().0.encrypted_api_token.is_empty());
        assert!(!form.forget_password && !form.forget_token);
    }
    #[test]
    fn disk_failure_keeps_the_draft_and_reports_failure_instead_of_success() {
        let fixture = Fixture::new();
        // A file in place of the parent directory gives a deterministic write
        // failure without changing permissions or requiring administrator rights.
        std::fs::write(&fixture.0, b"offline-blocker").unwrap();
        let manager = Manager::open(fixture.0.join("settings.json"));
        let mut form = form();
        form.values.prefer_hevc = true;
        form.password = Zeroizing::new("fixture-unsaved-sentinel".into());
        form.forget_token = true;
        assert!(!form.save(&manager));
        assert!(form.feedback.starts_with("Cannot write settings"));
        assert!(!form.feedback.contains("sentinel"));
        assert!(form.values.prefer_hevc && form.forget_token);
        assert_eq!(form.password.as_str(), "fixture-unsaved-sentinel");
        assert_eq!(std::fs::read(&fixture.0).unwrap(), b"offline-blocker");
    }
    #[test]
    fn amf_main10_is_opt_in_and_available_only_for_amd_hevc_ten_bit() {
        let mut form = form();
        assert!(!form.values.experimental_amf_main10);
        form.values.experimental_amf_main10 = true;
        for vendor in [Some(0x1002), Some(0x10de), Some(0x8086), None] {
            form.default_gpu_vendor = vendor;
            for hevc in [false, true] {
                form.values.prefer_hevc = hevc;
                for ten_bit in [false, true] {
                    form.values.experimental_ten_bit = ten_bit;
                    assert_eq!(
                        form.availability().amf_main10,
                        cfg!(feature = "vendor-video") && vendor == Some(0x1002) && hevc && ten_bit
                    );
                    assert!(form.values.experimental_amf_main10); // dormant preference
                }
            }
        }
        let saved = serde_json::to_vec(&form.values).unwrap();
        let restored: Settings = serde_json::from_slice(&saved).unwrap();
        assert!(restored.experimental_amf_main10);
    }
    #[test]
    fn compound_choices_round_trip_legacy_flags_without_changing_effective_behavior() {
        for hevc in [false, true] {
            for depth in [false, true] {
                let mut s = Settings {
                    prefer_hevc: hevc,
                    experimental_ten_bit: depth,
                    ..Settings::default()
                };
                let mode = VideoFormat::current(&s);
                mode.apply(&mut s);
                assert_eq!((s.prefer_hevc, s.experimental_ten_bit), (hevc, depth));
            }
        }
        for mode in RouteMode::ALL {
            let mut s = Settings::default();
            mode.apply(&mut s);
            assert_eq!(RouteMode::current(&s), mode);
            assert!(!(s.stun_only && s.force_relay));
        }
        for mode in VideoFormat::ALL {
            let mut s = Settings::default();
            mode.apply(&mut s);
            assert_eq!(VideoFormat::current(&s), mode);
        }
        #[cfg(feature = "bud")]
        for mode in InputMode::ALL {
            let mut s = Settings::default();
            mode.apply(&mut s);
            assert_eq!(InputMode::current(&s), mode);
            let restored: Settings =
                serde_json::from_slice(&serde_json::to_vec(&s).unwrap()).unwrap();
            assert_eq!(InputMode::current(&restored), mode);
        }
    }
}
