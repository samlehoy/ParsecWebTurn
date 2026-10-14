//! Explicit experimental BUD lifecycle. RTC attempts never enter this adapter.
use anyhow::{bail, ensure, Context, Result};
use parsec_native_experimental::{
    bud::{Event, Route, Session},
    signaling::Signaling,
};
use serde_json::{json, Value};
pub type SignalingWrite = Arc<dyn Fn(String) -> bool + Send + Sync>;
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
    output_finished: bool,
    web_mode: Option<Value>,
    wire_id: String,
    retired_wire: Option<String>,
    template: Option<Value>,
    writer: Option<SignalingWrite>,
    established: bool,
    guest_answer_sent: bool,
    events: VecDeque<Value>,
    controls: VecDeque<bytes::Bytes>,
    ingress: crate::media_ingress::Ingress,
    failure: Option<&'static str>,
}

// Keep BUD identity through connecting/disconnected states too. An old worker
// finishing after reconnect may only publish into its own stats generation.
struct Statistics {
    shared: Arc<crate::stats::Shared>,
    generation: u64,
}
impl Statistics {
    fn start(shared: Arc<crate::stats::Shared>) -> Self {
        let generation = shared.begin();
        shared.publish(
            generation,
            crate::stats::Sample {
                bud: Some(Default::default()),
                state: "Connecting (experimental BUD)".into(),
                ..Default::default()
            },
        );
        Self { shared, generation }
    }
}
impl Drop for Statistics {
    fn drop(&mut self) {
        self.shared.publish(
            self.generation,
            crate::stats::Sample {
                bud: Some(Default::default()),
                state: "Disconnected (experimental BUD)".into(),
                ..Default::default()
            },
        );
    }
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
            if result.is_err() {
                let mut state = worker_state.lock().unwrap_or_else(|e| e.into_inner());
                if !worker_cancel.load(Ordering::Acquire) {
                state.failure.get_or_insert("BUD connection or media failed");
                // No tokens, AES keys, addresses or server responses in diagnostics.
                state.events.clear();
                state.controls.clear();
                state.events.push_back(json!({"type":7,"status":-6200,"state":4,"attemptID":attempt_id,"duration":0}));
                if !state.output_finished { state.output_finished = true; let _ = output.finish_values(None); }
                }
            }
            withdraw_pending(&worker_state);
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

    pub fn outgoing(&self, text: &str, writer: Option<SignalingWrite>) -> Result<Option<String>> {
        let Ok(mut value) = serde_json::from_str::<Value>(text) else {
            return Ok(None);
        };
        if value["payload"]["attempt_id"].as_str() != Some(self.id.as_str()) {
            return Ok(None);
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if value["action"] == "offer" {
            if let Some(writer) = writer {
                state.writer = Some(writer);
            }
            state.template = Some(value.clone());
            state.web_mode = value["payload"]["data"].get("mode").cloned();
            let offer = state.offer.as_ref().context("BUD offer not ready")?;
            // Preserve account destination, authorization secret, and envelope.
            value["payload"]["data"] = offer.clone();
            if !state.wire_id.is_empty() {
                value["payload"]["attempt_id"] = json!(state.wire_id);
            }
            return Ok(Some(serde_json::to_string(&value)?));
        }
        if value["action"] == "candex" {
            return Ok(Some(String::new()));
        }
        if value["action"] == "offer_cancel" && !state.wire_id.is_empty() {
            value["payload"]["attempt_id"] = json!(state.wire_id);
            return Ok(Some(serde_json::to_string(&value)?));
        }
        Ok(None)
    }

    pub fn incoming(&self, text: &str) -> Result<Option<String>> {
        let Ok(mut value) = serde_json::from_str::<Value>(text) else {
            return Ok(None);
        };
        let wire = value["payload"]["attempt_id"].as_str().unwrap_or("");
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.retired_wire.as_deref() == Some(wire) {
            return Ok(Some(String::new()));
        }
        let current = if state.wire_id.is_empty() {
            self.id.as_str()
        } else {
            &state.wire_id
        };
        if wire != current
            || !matches!(
                value["action"].as_str(),
                Some("answer_relay" | "candex_relay" | "close")
            )
        {
            return Ok(None);
        }
        let duplicate_answer = state.guest_answer_sent && value["action"] == "answer_relay";
        drop(state);
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
        if value["action"] == "answer_relay" {
            self.state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .guest_answer_sent = true;
            if duplicate_answer {
                return Ok(Some(String::new()));
            }
        }
        value["payload"]["attempt_id"] = json!(self.id);
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
        Ok(Some(serde_json::to_string(&value)?))
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
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.failure = Some("BUD application signaling failed");
        self.cancel.store(true, Ordering::Release);
        self.thread.unpark();
    }

    pub fn failure(&self) -> Option<&'static str> {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).failure
    }
}
impl Drop for Attempt {
    fn drop(&mut self) {
        let _state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.cancel.store(true, Ordering::Release);
        self.thread.unpark();
    }
}

fn send_candidate(state: &Mutex<State>, event: &Event) -> Result<()> {
    let (writer, message) = {
        let state = state.lock().unwrap_or_else(|e| e.into_inner());
        let peer = state
            .template
            .as_ref()
            .and_then(|v| v["payload"]["to"].as_str())
            .context("BUD account destination missing")?;
        let message = Signaling::new(peer, &state.wire_id)?
            .candidate(event)
            .context("not a BUD candidate")?;
        (
            state.writer.clone().context("BUD account writer missing")?,
            serde_json::to_string(&message)?,
        )
    };
    ensure!(writer(message), "BUD candidate send failed");
    Ok(())
}

fn switch_offer(state: &Mutex<State>, cancel: &AtomicBool, id: &str, data: Value) -> Result<bool> {
    let (writer, previous, next) = {
        let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
        if cancel.load(Ordering::Acquire) {
            return Ok(false);
        }
        let mut next = state
            .template
            .clone()
            .context("BUD account offer missing")?;
        let mut previous = next.clone();
        previous["action"] = json!("offer_cancel");
        previous["payload"]["attempt_id"] = json!(state.wire_id);
        previous["payload"]["data"] = json!({});
        next["payload"]["attempt_id"] = json!(id);
        next["payload"]["data"] = data.clone();
        state.retired_wire = Some(std::mem::replace(&mut state.wire_id, id.into()));
        state.offer = Some(data);
        state.events.clear();
        state.controls.clear();
        (
            state.writer.clone().context("BUD account writer missing")?,
            serde_json::to_string(&previous)?,
            serde_json::to_string(&next)?,
        )
    };
    ensure!(
        writer(previous) && writer(next),
        "BUD relay offer send failed"
    );
    Ok(true)
}

// A cancelled relay retry may not have reached the retained guest yet. Retire
// its native account offer here, outside the UI/session locks. Established
// sessions leave through the SDK's existing authenticated teardown instead.
fn withdraw_pending(state: &Mutex<State>) {
    let pending = {
        let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
        if state.established {
            return;
        }
        state.template.take().and_then(|mut message| {
            let writer = state.writer.clone()?;
            message["action"] = json!("offer_cancel");
            message["payload"]["attempt_id"] = json!(state.wire_id);
            message["payload"]["data"] = json!({});
            Some((writer, message.to_string()))
        })
    };
    if let Some((writer, message)) = pending {
        let _ = writer(message);
    }
}

// Serialize guest completion against retirement. A delayed offer must never
// write credentials or wake a guest buffer reused by the next attempt.
fn complete_offer(
    state: &Mutex<State>,
    cancel: &AtomicBool,
    output: &crate::attempt::Output,
    offer: Value,
    values: [&str; 3],
) -> Result<bool> {
    let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
    if cancel.load(Ordering::Acquire) {
        return Ok(false);
    }
    ensure!(!state.output_finished, "BUD offer already completed");
    state.output_finished = true;
    ensure!(
        output.finish_values(Some(values))?,
        "BUD credential output too small"
    );
    state.offer = Some(offer);
    Ok(true)
}

// Force TURN starts on UDP relay; otherwise direct discovery is tried first.
// The worker may retry via UDP relay without changing the selected BUD transport.
async fn route(
    manager: &crate::connection_settings::Manager,
) -> Result<(Route, crate::connection_settings::Settings)> {
    let (settings, servers) = manager.resolve_bud().await?;
    if settings.force_relay {
        return Ok((udp_relay(servers).await?, settings));
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
    Ok((Route::Direct { stun }, settings))
}

async fn relay_route(
    manager: &crate::connection_settings::Manager,
    settings: &crate::connection_settings::Settings,
) -> Result<Route> {
    udp_relay(manager.resolve_bud_relay(settings).await?).await
}

async fn udp_relay(servers: Vec<webrtc::ice_transport::ice_server::RTCIceServer>) -> Result<Route> {
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
    bail!("BUD automatic relay requires a configured UDP TURN server")
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
    let (route, settings) = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(15), route(manager)).await
    })??;
    if cancel.load(Ordering::Acquire) {
        return Ok(());
    }
    // Probe before the lifecycle lock: a slow driver must not stall Cancel.
    let hevc_available =
        settings.prefer_hevc && crate::video_windows::hevc_available(window.video_gpu().as_ref());
    let main10_available = hevc_available
        && settings.bud_hevc_main10
        && crate::video_windows::hevc_main10_available(window.video_gpu().as_ref());
    let (mut media, mut session, statistics) = {
        let _state = state.lock().unwrap_or_else(|e| e.into_inner());
        if cancel.load(Ordering::Acquire) {
            return Ok(());
        }
        // Creation only starts the existing workers. Surface/stats generation
        // is serialized against retirement, so a cancelled old attempt cannot
        // hide the next attempt's video or reset its statistics.
        let (media, session) = crate::bud_media::Media::start_verified(
            window.clone(),
            hevc_available,
            main10_available,
        )?;
        (media, session, Statistics::start(window.stats.clone()))
    };
    state.lock().unwrap_or_else(|e| e.into_inner()).wire_id = id.into();
    session.wake_consumer(std::thread::current());
    let creds = session.offer(id, route)?;
    let mut source = session.take_media()?;
    let mut signaling = Signaling::new("host", id)?;
    *audio.lock().unwrap_or_else(|e| e.into_inner()) = Some(media.audio_pipeline());
    *shared_session.lock().unwrap_or_else(|e| e.into_inner()) = Some(session);
    if !complete_offer(
        state,
        cancel,
        output,
        signaling.offer(&creds)["payload"]["data"].take(),
        [&creds.ufrag, &creds.pwd, &creds.fingerprint],
    )? {
        return Ok(());
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut established = false;
    let mut answered_at: Option<Instant> = None;
    let mut retried = settings.force_relay
        || settings.stun_only
        || (settings.provider == "custom" && settings.turn_urls.is_empty());
    let mut wire_id = id.to_owned();
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
            if let Ok(value) = serde_json::from_str::<Value>(&text) {
                if value["action"] == "answer_relay"
                    && value["payload"]["attempt_id"].as_str() == Some(&wire_id)
                {
                    answered_at = Some(Instant::now());
                }
            }
        }
        let mut events = Vec::new();
        for _ in 0..64 {
            let Some(event) = session.poll()? else {
                break;
            };
            events.push(event);
        }
        let transport = session.snapshot();
        media.recover(session);
        drop(guard);
        let pumped = media.pump(&mut source)?;
        let mut ended = false;
        for event in &events {
            if matches!(event, Event::Candidate { .. } | Event::Ready) {
                send_candidate(state, event)?;
            }
        }
        let mut shared = state.lock().unwrap_or_else(|e| e.into_inner());
        for event in events {
            ensure!(
                shared.events.len() < 64 && shared.controls.len() < 64,
                "BUD application queue full"
            );
            match event {
                Event::Candidate { .. } | Event::Ready => {}
                Event::Established { .. } => {
                    established = true;
                    shared.established = true;
                    shared.events.push_back(
                        json!({"type":7,"status":0,"state":4,"attemptID":id,"duration":0}),
                    );
                }
                Event::Ended { .. } => ended = true,
                Event::ExternalControl { bytes } => shared.controls.push_back(bytes.into()),
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
                statistics.generation,
                crate::stats::Sample {
                    state: if established {
                        "Connected (experimental BUD)"
                    } else {
                        "Connecting (experimental BUD)"
                    }
                    .into(),
                    bud: Some(transport.clone()),
                    rtt_ms: transport.rtt_ms,
                    received: Some(transport.udp_received),
                    sent: Some(transport.udp_sent),
                    dtls: String::new(),
                    protocol: "BUD / UDP".into(),
                    route: if transport.connected {
                        if transport.relay {
                            "TURN relay"
                        } else {
                            "Direct"
                        }
                    } else {
                        "Unknown"
                    },
                    turn_protocol: transport.relay.then(|| "udp".into()),
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
        let fallback_due = !established
            && (ended || answered_at.is_some_and(|t| t.elapsed() >= Duration::from_secs(4)));
        if fallback_due && !retried {
            retried = true;
            drop(source);
            let mut old = shared_session
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take();
            if let Some(old) = &mut old {
                old.close();
            }
            drop(old);
            let relay = runtime.block_on(async {
                tokio::time::timeout(Duration::from_secs(8), relay_route(manager, &settings)).await
            })??;
            if cancel.load(Ordering::Acquire) {
                return Ok(());
            }
            wire_id = format!("{}-turn-1", id.chars().take(200).collect::<String>());
            let mut next = Session::external_video_formats(hevc_available, main10_available)?;
            next.wake_consumer(std::thread::current());
            let creds = next.offer(&wire_id, relay)?;
            source = next.take_media()?;
            signaling = Signaling::new("host", &wire_id)?;
            *shared_session.lock().unwrap_or_else(|e| e.into_inner()) = Some(next);
            if !switch_offer(
                state,
                cancel,
                &wire_id,
                signaling.offer(&creds)["payload"]["data"].take(),
            )? {
                return Ok(());
            }
            answered_at = None;
            continue;
        }
        ensure!(!ended, "BUD transport ended");
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
    fn old_bud_shutdown_cannot_replace_new_stats_or_show_webrtc_fields() {
        let shared = Arc::new(crate::stats::Shared::default());
        let old = Statistics::start(shared.clone());
        let current = Statistics::start(shared.clone());
        let generation = current.generation;
        drop(old);
        let sample = shared.read().unwrap();
        assert_eq!(sample.generation, generation);
        assert!(sample.state.starts_with("Connecting"));
        assert!(sample.bud.is_some());
        drop(current);
        let sample = shared.read().unwrap();
        assert!(sample.state.starts_with("Disconnected"));
        assert!(!sample.bud.unwrap().connected);
        assert!(sample.video.is_none());
    }

    #[test]
    fn native_offer_preserves_authorization_and_unrelated_messages() {
        let (attempt, _) = isolated();
        let source = json!({"action":"offer","payload":{"attempt_id":"isolated","to":"host","secret":"authorization-sentinel","data":{"mode":2,"creds":{"ice_pwd":"web-placeholder"}}}}).to_string();
        let wire: Value =
            serde_json::from_str(&attempt.outgoing(&source, None).unwrap().unwrap()).unwrap();
        assert_eq!(wire["payload"]["secret"], "authorization-sentinel");
        assert_eq!(wire["payload"]["to"], "host");
        assert_eq!(wire["payload"]["data"]["mode"], 1);
        assert_eq!(
            wire["payload"]["data"]["creds"]["aes256"],
            "isolated-key-sentinel"
        );
        assert!(attempt
            .outgoing(
                r#"{"action":"offer","payload":{"attempt_id":"stale"}}"#,
                None
            )
            .unwrap()
            .is_none());
        assert!(attempt.outgoing("__ping__", None).unwrap().is_none());
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
    fn relay_retry_withdraws_old_offer_preserves_authorization_and_ignores_old_answer() {
        let (attempt, incoming) = isolated();
        attempt.state.lock().unwrap().wire_id = "isolated".into();
        let writes = Arc::new(Mutex::new(Vec::<Value>::new()));
        let collected = writes.clone();
        let writer: SignalingWrite = Arc::new(move |text| {
            collected
                .lock()
                .unwrap()
                .push(serde_json::from_str(&text).unwrap());
            true
        });
        let offer = json!({"action":"offer","payload":{"attempt_id":"isolated","to":"real-host","secret":"authorization-sentinel","data":{"mode":2}}}).to_string();
        attempt.outgoing(&offer, Some(writer)).unwrap();
        assert!(switch_offer(
            &attempt.state,
            &attempt.cancel,
            "isolated-turn-1",
            json!({"mode":1,"creds":{"aes256":"new-native-key"}})
        )
        .unwrap());
        let sent = writes.lock().unwrap();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0]["action"], "offer_cancel");
        assert_eq!(sent[0]["payload"]["attempt_id"], "isolated");
        assert_eq!(sent[1]["action"], "offer");
        assert_eq!(sent[1]["payload"]["attempt_id"], "isolated-turn-1");
        assert_eq!(sent[1]["payload"]["secret"], "authorization-sentinel");
        drop(sent);
        assert_eq!(attempt.incoming(r#"{"action":"answer_relay","payload":{"attempt_id":"isolated","approved":true,"data":{}}}"#).unwrap(), Some(String::new()));
        assert!(incoming.try_recv().is_err());
        let response = json!({"action":"answer_relay","payload":{"attempt_id":"isolated-turn-1","approved":true,"data":{"mode":1,"creds":{"aes256":"key"}}}}).to_string();
        let guest: Value =
            serde_json::from_str(&attempt.incoming(&response).unwrap().unwrap()).unwrap();
        assert_eq!(guest["payload"]["attempt_id"], "isolated");
        assert_eq!(incoming.try_recv().unwrap(), response);
        assert_eq!(attempt.outgoing(r#"{"action":"candex","payload":{"attempt_id":"isolated","data":{"ip":"stale"}}}"#, None).unwrap(), Some(String::new()));
        attempt.state.lock().unwrap().established = true;
        withdraw_pending(&attempt.state);
        assert_eq!(writes.lock().unwrap().len(), 2);
        assert!(attempt.state.lock().unwrap().template.is_some());
        attempt.state.lock().unwrap().established = false;
        send_candidate(&attempt.state, &Event::Ready).unwrap();
        let sent = writes.lock().unwrap();
        assert_eq!(sent[2]["payload"]["to"], "real-host");
        assert_eq!(sent[2]["payload"]["attempt_id"], "isolated-turn-1");
        assert_eq!(sent[2]["payload"]["data"]["sync"], true);
        drop(sent);
        withdraw_pending(&attempt.state);
        withdraw_pending(&attempt.state);
        let sent = writes.lock().unwrap();
        assert_eq!(sent.len(), 4, "pending offer is withdrawn only once");
        assert_eq!(sent[3]["action"], "offer_cancel");
        assert_eq!(sent[3]["payload"]["attempt_id"], "isolated-turn-1");
    }

    #[test]
    fn malformed_active_answer_fails_without_mutating_guest_or_queue() {
        let (attempt, incoming) = isolated();
        attempt.state.lock().unwrap().web_mode = Some(json!(2));
        assert!(attempt.incoming(r#"{"action":"answer_relay","payload":{"attempt_id":"isolated","approved":true,"data":42}}"#).is_err());
        assert!(incoming.try_recv().is_err());
    }

    #[test]
    fn retired_offer_cannot_overwrite_reused_guest_buffers() {
        let mut config = wasmtime::Config::new();
        config.wasm_threads(true);
        let memory = crate::memory::GuestMemory(
            wasmtime::SharedMemory::new(
                &wasmtime::Engine::new(&config).unwrap(),
                wasmtime::MemoryType::shared(1, 1),
            )
            .unwrap(),
        );
        let output =
            crate::attempt::Output::new(memory.clone(), [100, 400, 700], 128, 64, 80).unwrap();
        memory.c_string(100, 128, "next-attempt-sentinel").unwrap();
        let state = Mutex::new(State::default());
        let cancelled = AtomicBool::new(true);
        assert!(!complete_offer(
            &state,
            &cancelled,
            &output,
            json!({"mode":1}),
            ["old", "old", "old"]
        )
        .unwrap());
        assert_eq!(memory.string(100, 128).unwrap(), "next-attempt-sentinel");
        assert_eq!(memory.u32(64).unwrap(), 0);
        cancelled.store(false, Ordering::Release);
        assert!(complete_offer(
            &state,
            &cancelled,
            &output,
            json!({"mode":1}),
            ["new", "pwd", "fp"]
        )
        .unwrap());
        assert_eq!(memory.string(100, 128).unwrap(), "new");
        assert!(complete_offer(
            &state,
            &cancelled,
            &output,
            json!({"mode":1}),
            ["duplicate", "pwd", "fp"]
        )
        .is_err());
        assert_eq!(memory.string(100, 128).unwrap(), "new");
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
