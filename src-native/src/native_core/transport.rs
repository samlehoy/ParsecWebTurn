//! Session-facing transport selection. Both paths reuse the existing native
//! decoders and renderer; WebRTC owns ICE/DTLS/SCTP, BUD keeps its HID adapter.
use crate::{connection_settings::Manager, window::Window};
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[derive(Clone, Copy)]
pub(super) struct Policy {
    pub bud: bool,
}
#[derive(Debug)]
pub(super) struct HostStatus {
    pub code: i32,
    pub before_video: bool,
}
impl std::fmt::Display for HostStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "The host ended the stream (status {}).", self.code)
    }
}
impl std::error::Error for HostStatus {}
impl HostStatus {
    pub fn from_window(code: i32, window: &Window) -> Self {
        let before_video = window
            .video_report
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_none_or(|video| video.frames_decoded == 0);
        Self { code, before_video }
    }
    pub fn user_message(&self, requested_main10: bool) -> String {
        if self.code == -15002 && self.before_video && requested_main10 {
            return "The host encoder failed (status -15002) while 10-bit HEVC was requested. Disable 10-bit HEVC in Settings > Video, then connect again. The client has not retried in 8-bit or changed your settings.".into();
        }
        self.to_string()
    }
}

pub(super) enum Attempt {
    Bud(crate::bud_attempt::Attempt),
    Web(WebRtc),
}
pub(super) struct WebRtc {
    transport: crate::attempt::Attempt,
    id: String,
    peer: String,
    writer: crate::bud_attempt::SignalingWrite,
    failed: AtomicBool,
}
fn envelope(action: &str, peer: &str, id: &str, data: Value) -> Value {
    json!({"version":1,"action":action,"payload":{"to":peer,"attempt_id":id,"data":data}})
}
fn offer(peer: &str, id: &str, credentials: &crate::signaling::Credentials) -> String {
    let mut value = envelope(
        "offer",
        peer,
        id,
        json!({
            "ver_data":1,"mode":2,"versions":{"p2p":1,"bud":1,"init":1,"video":1,"audio":1,"control":1},
            "creds":{"ice_ufrag":credentials.ufrag,"ice_pwd":credentials.password,"fingerprint":credentials.fingerprint}
        }),
    );
    value["payload"]["secret"] = json!("");
    value.to_string()
}

impl Attempt {
    pub fn spawn_native(
        id: &str,
        peer: &str,
        writer: crate::bud_attempt::SignalingWrite,
        window: Arc<Window>,
        manager: Arc<Manager>,
        policy: Policy,
    ) -> Result<Self> {
        ensure!(
            !peer.is_empty() && peer.len() <= 256 && !peer.chars().any(char::is_control),
            "Invalid host peer ID"
        );
        if policy.bud {
            return crate::bud_attempt::Attempt::spawn_native(id, peer, writer, window, manager)
                .map(Self::Bud);
        }
        let settings = manager.view().0;
        let (target, attempt_id, publish) = (peer.to_owned(), id.to_owned(), writer.clone());
        let output = crate::attempt::Output::new(Arc::new(move |description| {
            Ok(description.is_some_and(|description| {
                publish(offer(&target, &attempt_id, &description.credentials))
            }))
        }));
        let transport = crate::attempt::Attempt::spawn_configured(
            id,
            output,
            Some(crate::control::Config {
                video_protocol: crate::video_stream::VideoProtocol {
                    version: 1,
                    message_size: 11,
                    version_offset: 0,
                    flag_offset: 4,
                },
                allow_hevc: settings.prefer_hevc,
                ten_bit: settings.experimental_ten_bit,
                chroma_444: settings.experimental_chroma_444,
            }),
            crate::attempt::StunProvider::Parsec,
            true,
            Some(manager),
        )?;
        transport.start_video(window);
        Ok(Self::Web(WebRtc {
            transport,
            id: id.into(),
            peer: peer.into(),
            writer,
            failed: AtomicBool::new(false),
        }))
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::Bud(_) => "BUD",
            Self::Web(_) => "WebRTC",
        }
    }
    pub fn incoming(&self, text: &str) -> Result<()> {
        let Self::Web(web) = self else {
            if let Self::Bud(bud) = self {
                bud.incoming(text)?;
            }
            return Ok(());
        };
        let value: Value = serde_json::from_str(text)
            .map_err(|_| anyhow::anyhow!("Invalid account signaling message"))?;
        let payload = &value["payload"];
        if payload["attempt_id"].as_str() != Some(web.id.as_str()) {
            return Ok(());
        }
        let data = &payload["data"];
        match value["action"].as_str() {
            Some("answer_relay") => {
                ensure!(
                    payload["approved"] == true,
                    "The host declined the connection"
                );
                let creds = &data["creds"];
                let field = |key| {
                    creds[key]
                        .as_str()
                        .map(str::to_owned)
                        .context("Host credentials are incomplete")
                };
                web.transport.begin(
                    &web.id,
                    crate::signaling::Credentials {
                        ufrag: field("ice_ufrag")?,
                        password: field("ice_pwd")?,
                        fingerprint: field("fingerprint")?,
                    }
                    .normalize_compact(),
                )?;
            }
            Some("candex_relay") => {
                if data["sync"] == true {
                    web.transport.sync(&web.id)?;
                } else {
                    let ip = data["ip"]
                        .as_str()
                        .context("Host candidate address missing")?;
                    let port = data["port"]
                        .as_u64()
                        .and_then(|n| u16::try_from(n).ok())
                        .context("Host candidate port invalid")?;
                    web.transport.candidate(
                        &web.id,
                        crate::signaling::Candidate::new(
                            ip,
                            port,
                            data["from_stun"].as_bool().unwrap_or(false),
                        )?,
                    )?;
                }
            }
            Some("close") => {
                web.failed.store(true, Ordering::Release);
            }
            _ => {}
        }
        Ok(())
    }
    pub fn pop_event(&self) -> Option<Value> {
        match self {
            Self::Bud(bud) => bud.pop_event(),
            Self::Web(web) => {
                for _ in 0..64 {
                    let event = web.transport.pop_event()?;
                    if event["type"] != 8 {
                        return Some(event);
                    }
                    let data = json!({"ver_data":1,"versions":{"p2p":1,"bud":1,"init":1,"video":1,"audio":1,"control":1},
                        "ip":event["ip"],"port":event["port"],"lan":event["lan"],"from_stun":event["fromStun"],"sync":event["sync"]});
                    if !(web.writer)(envelope("candex", &web.peer, &web.id, data).to_string()) {
                        web.failed.store(true, Ordering::Release);
                        break;
                    }
                }
                None
            }
        }
    }
    pub fn native_connected(&self) -> bool {
        match self {
            Self::Bud(bud) => bud.native_connected(),
            Self::Web(web) => web.transport.control_ready(),
        }
    }
    pub fn failure(&self) -> Option<&'static str> {
        match self {
            Self::Bud(bud) => bud.failure(),
            Self::Web(web) => (web.failed.load(Ordering::Acquire)
                || web.transport.failure_stage().is_some())
            .then_some("WebRTC connection failed"),
        }
    }
    pub fn host_status(&self) -> Option<i32> {
        match self {
            Self::Bud(bud) => bud.host_status(),
            Self::Web(web) => web.transport.host_status(),
        }
    }
    pub fn send(&self, bytes: &[u8]) -> Result<()> {
        match self {
            Self::Bud(bud) => bud.send(bytes),
            Self::Web(web) => web
                .transport
                .send_binary(0, bytes::Bytes::copy_from_slice(bytes)),
        }
    }
    pub fn pop_control(&self) -> Option<bytes::Bytes> {
        match self {
            Self::Bud(bud) => bud.pop_control(),
            Self::Web(web) => web.transport.pop_binary().map(|(_, _, bytes)| bytes),
        }
    }
    pub fn audio_pipeline(&self) -> Option<crate::audio_stream::NativeReceiver> {
        match self {
            Self::Bud(bud) => bud.audio_pipeline().map(|audio| audio.native_receiver()),
            Self::Web(web) => web.transport.audio_receiver(),
        }
    }
    pub fn hid_mouse_enabled(&self) -> bool {
        matches!(self, Self::Bud(bud) if bud.hid_mouse_enabled())
    }
    pub fn native_input(
        &self,
        packet: &[u8],
        viewport: Option<crate::viewport::Viewport>,
    ) -> Result<()> {
        match self {
            Self::Bud(bud) => bud.native_input(packet, viewport),
            Self::Web(_) => self.send(packet),
        }
    }
}
impl Drop for WebRtc {
    fn drop(&mut self) {
        let _ =
            (self.writer)(envelope("offer_cancel", &self.peer, &self.id, Value::Null).to_string());
        self.transport.disconnect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn initial_main10_encoder_failure_explains_manual_recovery() {
        let status = HostStatus {
            code: -15002,
            before_video: true,
        };
        assert!(status.user_message(true).contains("Disable 10-bit HEVC"));
        assert_eq!(status.user_message(false), status.to_string());
        for (code, before_video) in [(-15002, false), (-6200, true), (0, true), (-15000, true)] {
            let status = HostStatus { code, before_video };
            assert_eq!(status.user_message(true), status.to_string());
        }
    }
    #[test]
    fn native_offer_uses_the_existing_web_transport_contract() {
        let wire = offer(
            "fixture-host",
            "fixture-attempt",
            &crate::signaling::Credentials {
                ufrag: "fixture-ufrag".into(),
                password: "fixture-password".into(),
                fingerprint: "fixture-fingerprint".into(),
            },
        );
        let value: Value = serde_json::from_str(&wire).unwrap();
        assert_eq!(value["action"], "offer");
        assert_eq!(value["payload"]["to"], "fixture-host");
        assert_eq!(value["payload"]["attempt_id"], "fixture-attempt");
        assert_eq!(value["payload"]["secret"], "");
        let data = &value["payload"]["data"];
        assert_eq!(data["mode"], 2);
        assert_eq!(data["versions"]["video"], 1);
        assert_eq!(data["creds"]["ice_pwd"], "fixture-password");
        assert!(data.get("aes256").is_none());
    }
}
