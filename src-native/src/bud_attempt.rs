//! Explicit experimental BUD lifecycle. RTC attempts never enter this adapter.
use anyhow::{bail, ensure, Context, Result};
use parsec_native_experimental::{
    bud::{Event, Route, Session},
    signaling::Signaling,
};
use serde_json::{json, Value};
type SharedAudio = Arc<Mutex<Option<Arc<crate::audio_stream::Pipeline>>>>;

use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};

#[derive(Default)]
struct State {
    offer: Option<Value>,
    web_mode: Option<Value>,
    events: VecDeque<Value>,
    controls: VecDeque<bytes::Bytes>,
    ingress: crate::media_ingress::Ingress,
    failure: Option<&'static str>,
}

pub struct Attempt {
    id: String,
    state: Arc<Mutex<State>>,
    session: Arc<Mutex<Option<Session>>>,
    audio: SharedAudio,
    commands: mpsc::SyncSender<String>,
    cancel: Arc<AtomicBool>,
    thread: std::thread::Thread,
}

impl Attempt {
    pub fn spawn(
        id: &str,
        output: crate::attempt::Output,
        window: Arc<crate::window::Window>,
        manager: Arc<crate::connection_settings::Manager>,
    ) -> Result<Self> {
        let state = Arc::new(Mutex::new(State::default()));
        let session = Arc::new(Mutex::new(None));
        let audio = Arc::new(Mutex::new(None));
        let cancel = Arc::new(AtomicBool::new(false));
        let (commands, incoming) = mpsc::sync_channel(64);
        let (worker_state, worker_session, worker_audio, worker_cancel) = (
            state.clone(),
            session.clone(),
            audio.clone(),
            cancel.clone(),
        );
        let attempt_id = id.to_owned();
        let worker = std::thread::Builder::new().name("parsec-bud-media".into()).spawn(move || {
            let result = run(&attempt_id, &output, &window, &manager, &worker_state, &worker_session, &worker_audio, &worker_cancel, incoming);
            if result.is_err() && !worker_cancel.load(Ordering::Acquire) {
                let mut state = worker_state.lock().unwrap_or_else(|e| e.into_inner());
                state.failure.get_or_insert("BUD connection or media failed");
                // No tokens, AES keys, addresses or server responses in diagnostics.
                state.events.clear();
                state.controls.clear();
                state.events.push_back(json!({"type":7,"status":-6200,"state":4,"attemptID":attempt_id,"duration":0}));
                if state.offer.is_none() { let _ = output.finish_values(None); }
            }
            let mut retired = worker_session.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some(session) = &mut retired { session.close(); }

        })?;
        let thread = worker.thread().clone();
        // The worker owns its shutdown. Never join it under Backend/UI locks.
        drop(worker);
        Ok(Self {
            id: id.into(),
            state,
            session,
            audio,
            commands,
            cancel,
            thread,
        })
    }

    pub fn outgoing(&self, text: &str) -> Result<Option<String>> {
        let Ok(mut value) = serde_json::from_str::<Value>(text) else {
            return Ok(None);
        };
        if value["payload"]["attempt_id"].as_str() != Some(self.id.as_str()) {
            return Ok(None);
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if value["action"] == "offer" {
            state.web_mode = value["payload"]["data"].get("mode").cloned();
            let offer = state.offer.as_ref().context("BUD offer not ready")?;
            // Preserve account destination, authorization secret, and envelope.
            value["payload"]["data"] = offer.clone();
            return Ok(Some(serde_json::to_string(&value)?));
        }
        if value["action"] == "candex" {
            value["payload"]["data"]["ver_data"] = json!(1);
            value["payload"]["data"]["versions"] =
                json!({"p2p":1,"bud":1,"init":1,"video":1,"audio":1,"control":1});
            return Ok(Some(serde_json::to_string(&value)?));
        }
        Ok(None)
    }

    pub fn incoming(&self, text: &str) -> Result<Option<String>> {
        let Ok(mut value) = serde_json::from_str::<Value>(text) else {
            return Ok(None);
        };
        if !matches!(
            value["action"].as_str(),
            Some("answer_relay" | "candex_relay")
        ) || value["payload"]["attempt_id"].as_str() != Some(self.id.as_str())
        {
            return Ok(None);
        }
        ensure!(text.len() <= 16 * 1024, "BUD signaling exceeds limit");
        if value["action"] == "answer_relay" && value["payload"]["approved"] == true {
            ensure!(
                value["payload"]["data"].is_object()
                    && value["payload"]["data"]["creds"].is_object(),
                "malformed BUD answer"
            );
        }
        self.commands
            .try_send(text.to_owned())
            .context("BUD signaling queue unavailable")?;
        self.thread.unpark();
        if value["action"] == "answer_relay" && value["payload"]["data"].is_object() {
            let mode = self
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .web_mode
                .clone();
            if let Some(mode) = mode {
                value["payload"]["data"]["mode"] = mode;
            }
            if let Some(creds) = value["payload"]["data"]["creds"].as_object_mut() {
                creds.remove("aes256");
            }
            return Ok(Some(serde_json::to_string(&value)?));
        }
        Ok(None)
    }

    pub fn send(&self, frame: &[u8]) -> Result<()> {
        let mut session = self.session.lock().unwrap_or_else(|e| e.into_inner());
        let session = session.as_mut().context("BUD session unavailable")?;
        if frame.len() >= 14 && frame[12] == 17 {
            let id = u32::from_be_bytes(frame[4..8].try_into()?);
            let body = &frame[13..];
            ensure!(body.last() == Some(&0), "BUD user data lacks terminator");
            ensure!(
                session.send_user_data(id, &body[..body.len() - 1]),
                "BUD user data refused"
            );
            return Ok(());
        }
        session.send_control(frame)
    }
    pub fn poll_audio(
        &self,
        memory: &crate::memory::GuestMemory,
        pointer: u32,
        capacity: usize,
    ) -> Result<usize> {
        let audio = self.audio.lock().unwrap_or_else(|e| e.into_inner()).clone();
        audio.map_or(Ok(0), |audio| audio.poll(memory, pointer, capacity))
    }
    pub fn pop_event(&self) -> Option<Value> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .events
            .pop_front()
    }
    pub fn pop_control(&self) -> Option<bytes::Bytes> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .controls
            .pop_front()
    }
    pub fn ingress(&self) -> crate::media_ingress::Ingress {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .ingress
            .clone()
    }
    pub fn fail(&self) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).failure =
            Some("BUD application signaling failed");
        self.cancel.store(true, Ordering::Release);
        self.thread.unpark();
    }

    pub fn failure(&self) -> Option<&'static str> {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).failure
    }
}
impl Drop for Attempt {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        self.thread.unpark();
    }
}

// BUD uses an explicit direct route, or one UDP relay when Force TURN is selected.
// TCP/TLS URLs are never interpreted as UDP and there is no silent fallback.
async fn route(manager: &crate::connection_settings::Manager) -> Result<Route> {
    let (settings, servers) = manager.resolve_bud().await?;
    if settings.force_relay {
        for server in servers {
            for raw in server.urls {
                let url = webrtc::ice::url::Url::parse_url(&raw)?;
                if url.scheme == webrtc::ice::url::SchemeType::Turn
                    && url.proto == webrtc::ice::url::ProtoType::Udp
                {
                    let addr = tokio::net::lookup_host((url.host.as_str(), url.port))
                        .await?
                        .find(|a| a.is_ipv4())
                        .context("BUD UDP TURN has no IPv4 address")?;
                    return Ok(Route::turn_udp(addr, &server.username, &server.credential));
                }
            }
        }
        bail!("Experimental BUD requires a UDP TURN URL for Force TURN");
    }
    let mut stun = Vec::new();
    for server in servers {
        for raw in server.urls {
            let url = webrtc::ice::url::Url::parse_url(&raw)?;
            if url.scheme == webrtc::ice::url::SchemeType::Stun
                && url.proto == webrtc::ice::url::ProtoType::Udp
            {
                stun.extend(
                    tokio::net::lookup_host((url.host.as_str(), url.port))
                        .await?
                        .filter(|a| a.is_ipv4()),
                );
            }
        }
    }
    Ok(Route::Direct { stun })
}

#[allow(clippy::too_many_arguments)]
fn run(
    id: &str,
    output: &crate::attempt::Output,
    window: &Arc<crate::window::Window>,
    manager: &crate::connection_settings::Manager,
    state: &Arc<Mutex<State>>,
    shared_session: &Arc<Mutex<Option<Session>>>,
    audio: &SharedAudio,
    cancel: &AtomicBool,
    incoming: mpsc::Receiver<String>,
) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let route = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(15), route(manager)).await
    })??;
    if cancel.load(Ordering::Acquire) {
        return Ok(());
    }
    let relay = matches!(route, Route::TurnUdp(_));
    let (mut media, mut session) =
        crate::bud_media::Media::start(window.clone(), manager.view().0.prefer_hevc)?;
    session.wake_consumer(std::thread::current());
    let creds = session.offer(id, route)?;
    let mut source = session.take_media()?;
    let signaling = Signaling::new("host", id)?;
    state.lock().unwrap_or_else(|e| e.into_inner()).offer =
        Some(signaling.offer(&creds)["payload"]["data"].take());
    *audio.lock().unwrap_or_else(|e| e.into_inner()) = Some(media.audio_pipeline());
    *shared_session.lock().unwrap_or_else(|e| e.into_inner()) = Some(session);
    output.finish_values(Some([&creds.ufrag, &creds.pwd, &creds.fingerprint]))?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut established = false;
    let generation = window.stats.begin();
    let mut last_stats = Instant::now();
    while !cancel.load(Ordering::Acquire) {
        ensure!(
            established || Instant::now() < deadline,
            "BUD establishment deadline"
        );
        let mut guard = shared_session.lock().unwrap_or_else(|e| e.into_inner());
        let session = guard.as_mut().context("BUD session retired")?;
        for _ in 0..64 {
            let Ok(text) = incoming.try_recv() else {
                break;
            };
            signaling.receive(&text, session)?;
        }
        let mut events = Vec::new();
        for _ in 0..64 {
            let Some(event) = session.poll()? else {
                break;
            };
            events.push(event);
        }
        media.recover(session);
        drop(guard);
        let pumped = media.pump(&mut source)?;
        let mut shared = state.lock().unwrap_or_else(|e| e.into_inner());
        for event in events {
            ensure!(
                shared.events.len() < 64 && shared.controls.len() < 64,
                "BUD application queue full"
            );
            match event {
                Event::Candidate { addr, lan, from_stun } => shared.events.push_back(json!({"type":8,"attemptID":id,"ip":addr.ip().to_string(),"port":addr.port(),"lan":lan,"fromStun":from_stun,"sync":false})),
                Event::Ready => shared.events.push_back(json!({"type":8,"attemptID":id,"ip":"","port":0,"lan":false,"fromStun":false,"sync":true})),
                Event::Established { .. } => { established = true; shared.events.push_back(json!({"type":7,"status":0,"state":4,"attemptID":id,"duration":0})); },
                Event::Ended { .. } => bail!("BUD transport ended"),
                Event::ExternalControl { bytes } => shared.controls.push_back(bytes.into()),
                // Semantic SDK events are already represented by original control frames.
                _ => {},
            }
        }
        let mut ingress = media.ingress();
        ingress.video_packets_received =
            shared.ingress.video_packets_received + pumped.video_messages as u64;
        ingress.audio_packets_received =
            shared.ingress.audio_packets_received + pumped.audio_messages as u64;
        ingress.video_bytes_received =
            shared.ingress.video_bytes_received + pumped.video_bytes as u64;
        ingress.audio_bytes_received =
            shared.ingress.audio_bytes_received + pumped.audio_bytes as u64;
        shared.ingress = ingress;
        if window.stats.requested() && last_stats.elapsed() >= Duration::from_millis(250) {
            window.stats.publish(
                generation,
                crate::stats::Sample {
                    state: if established {
                        "Connected (experimental BUD)"
                    } else {
                        "Connecting (experimental BUD)"
                    }
                    .into(),
                    dtls: "Not applicable (native BUD)".into(),
                    protocol: "BUD / UDP".into(),
                    route: if established {
                        if relay {
                            "TURN relay"
                        } else {
                            "Direct"
                        }
                    } else {
                        "Unknown"
                    },
                    turn_protocol: relay.then(|| "udp".into()),
                    audio_bytes: shared.ingress.audio_bytes_received,
                    video: shared.ingress.video_output.clone(),
                    codec: shared.ingress.video_stream.codec,
                    profile: shared.ingress.video_stream.sps_profile_idc,
                    hevc_advertised: shared.ingress.video_stream.hevc_advertised,
                    audio_ready: shared
                        .ingress
                        .audio_output
                        .as_ref()
                        .is_some_and(|a| a.packets_decoded > 0),
                    ..Default::default()
                },
            );
            last_stats = Instant::now();
        }
        drop(shared);
        if pumped.video_messages >= 8 || pumped.audio_messages >= 8 {
            continue;
        }
        // Packet arrivals unpark this consumer; this timeout only checks lifecycle/stats.
        std::thread::park_timeout(Duration::from_millis(100));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn isolated() -> (Attempt, mpsc::Receiver<String>) {
        let (commands, receiver) = mpsc::sync_channel(2);
        (
            Attempt {
                id: "isolated".into(),
                state: Arc::new(Mutex::new(State {
                    offer: Some(json!({"mode":1,"creds":{"aes256":"isolated-key-sentinel"}})),
                    ..Default::default()
                })),
                session: Default::default(),
                audio: Default::default(),
                commands,
                cancel: Default::default(),
                thread: std::thread::current(),
            },
            receiver,
        )
    }
    #[test]
    fn native_offer_preserves_authorization_and_unrelated_messages() {
        let (attempt, _) = isolated();
        let source = json!({"action":"offer","payload":{"attempt_id":"isolated","to":"host","secret":"authorization-sentinel","data":{"mode":2,"creds":{"ice_pwd":"web-placeholder"}}}}).to_string();
        let wire: Value =
            serde_json::from_str(&attempt.outgoing(&source).unwrap().unwrap()).unwrap();
        assert_eq!(wire["payload"]["secret"], "authorization-sentinel");
        assert_eq!(wire["payload"]["to"], "host");
        assert_eq!(wire["payload"]["data"]["mode"], 1);
        assert_eq!(
            wire["payload"]["data"]["creds"]["aes256"],
            "isolated-key-sentinel"
        );
        assert!(attempt
            .outgoing(r#"{"action":"offer","payload":{"attempt_id":"stale"}}"#)
            .unwrap()
            .is_none());
        assert!(attempt.outgoing("__ping__").unwrap().is_none());
    }
    #[test]
    fn native_answer_keeps_aes_out_of_guest_and_rejects_stale_delivery() {
        let (attempt, incoming) = isolated();
        attempt.state.lock().unwrap().web_mode = Some(json!(2));
        let wire = json!({"action":"answer_relay","payload":{"attempt_id":"isolated","approved":true,"data":{"mode":1,"creds":{"aes256":"native-key-sentinel","ice_ufrag":"abc"}}}}).to_string();
        let guest: Value =
            serde_json::from_str(&attempt.incoming(&wire).unwrap().unwrap()).unwrap();
        assert_eq!(incoming.try_recv().unwrap(), wire);
        assert_eq!(guest["payload"]["data"]["mode"], 2);
        assert!(guest["payload"]["data"]["creds"].get("aes256").is_none());
        assert!(attempt
            .incoming(r#"{"action":"answer_relay","payload":{"attempt_id":"stale"}}"#)
            .unwrap()
            .is_none());
        assert!(incoming.try_recv().is_err());
    }
    #[test]
    fn malformed_active_answer_fails_without_mutating_guest_or_queue() {
        let (attempt, incoming) = isolated();
        attempt.state.lock().unwrap().web_mode = Some(json!(2));
        assert!(attempt.incoming(r#"{"action":"answer_relay","payload":{"attempt_id":"isolated","approved":true,"data":42}}"#).is_err());
        assert!(incoming.try_recv().is_err());
    }

    #[test]
    fn bud_is_explicit_and_default_settings_remain_webrtc() {
        let settings: crate::connection_settings::Settings = serde_json::from_str("{}").unwrap();
        assert!(!settings.experimental_bud);
        let settings: crate::connection_settings::Settings =
            serde_json::from_str(r#"{"experimentalBud":true}"#).unwrap();
        assert!(settings.experimental_bud);
    }
}
