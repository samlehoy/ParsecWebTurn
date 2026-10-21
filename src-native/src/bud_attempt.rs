//! Explicit BUD lifecycle. RTC attempts never enter this adapter.
use anyhow::{ensure, Context, Result};
use parsec_native_bud::{
    bud::{Event, Outcome, Route, Session},
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
    retired_wires: Vec<String>,
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
    fn consume_timing(&self, event: &Event) -> bool {
        if let Event::ExternalControl { bytes } = event {
            self.shared.host_timing.receive(self.generation, bytes)
        } else {
            false
        }
    }
    fn start(shared: Arc<crate::stats::Shared>) -> Self {
        let generation = shared.begin();
        shared.publish(
            generation,
            crate::stats::Sample {
                bud: Some(Default::default()),
                state: "Connecting (BUD)".into(),
                ..Default::default()
            },
        );
        Self { shared, generation }
    }
}
impl Drop for Statistics {
    fn drop(&mut self) {
        self.shared.host_timing.end(self.generation);
        self.shared.publish(
            self.generation,
            crate::stats::Sample {
                bud: Some(Default::default()),
                state: "Disconnected (BUD)".into(),
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
    hid_input: (bool, bool),
}

impl Attempt {
    pub fn spawn(
        id: &str,
        output: crate::attempt::Output,
        window: Arc<crate::window::Window>,
        manager: Arc<crate::connection_settings::Manager>,
    ) -> Result<Self> {
        let settings = manager.view().0;
        let hid_input = (
            settings.hid_mouse_enabled(),
            settings.hid_keyboard_enabled(),
        );
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
            let mut trace = crate::bud_trace::Trace::start();
            let result = run(&attempt_id, &output, &window, &manager, &worker_state, &worker_session, &worker_audio, &worker_cancel, incoming, &mut trace, hid_input);
            trace.record(if result.is_err() { "worker_error" } else { "worker_returned" }, &[("cancelled", u64::from(worker_cancel.load(Ordering::Acquire)))]);
            if result.is_err() {
                if !worker_cancel.load(Ordering::Acquire) {
                    crate::event_log::record("ERROR", "connection_failed", json!({"transport":"BUD","stage":trace.stage(),"route":window.stats.read().map(|s|s.route)}));
                }
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
            hid_input,
        })
    }

    pub fn hid_mouse_enabled(&self) -> bool {
        self.hid_input.0
    }

    pub fn hid_input_enabled(&self) -> bool {
        self.hid_input.0 || self.hid_input.1
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
        if state.retired_wires.iter().any(|id| id == wire) {
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
        // Native attempt termination belongs to the BUD worker. Forwarding it
        // to the retained web guest would cancel the whole connection before
        // that worker can replace a failed direct attempt with a relay offer.
        if value["action"] == "close" {
            return Ok(Some(String::new()));
        }
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
    pub fn audio_pipeline(&self) -> Option<Arc<crate::audio_stream::Pipeline>> {
        self.audio.lock().unwrap_or_else(|e| e.into_inner()).clone()
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
        ensure!(state.retired_wires.len() < 32, "BUD relay retry limit");
        let previous_id = std::mem::replace(&mut state.wire_id, id.into());
        state.retired_wires.push(previous_id);
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

// Force TURN starts on the selected relay carrier; otherwise try direct UDP first.
async fn route(
    manager: &crate::connection_settings::Manager,
) -> Result<(
    Route,
    Option<VecDeque<Route>>,
    crate::connection_settings::Settings,
)> {
    let (settings, servers) = manager.resolve_bud().await?;
    if settings.force_relay {
        let mut routes = turn_relays(servers).await?;
        let first = routes
            .pop_front()
            .context("BUD requires a supported TURN URL")?;
        return Ok((first, Some(routes), settings));
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
    Ok((Route::Direct { stun }, None, settings))
}

async fn relay_route(
    manager: &crate::connection_settings::Manager,
    settings: &crate::connection_settings::Settings,
) -> Result<VecDeque<Route>> {
    if settings.stun_only || (settings.provider == "custom" && settings.turn_urls.is_empty()) {
        return Ok(VecDeque::new());
    }
    turn_relays(manager.resolve_bud_relay(settings).await?).await
}

fn relay_carrier(url: &webrtc::ice::url::Url) -> Option<parsec_native_bud::bud::RelayTransport> {
    use parsec_native_bud::bud::RelayTransport;
    use webrtc::ice::url::{ProtoType, SchemeType};
    match (url.scheme, url.proto) {
        (SchemeType::Turn, ProtoType::Udp) => Some(RelayTransport::Udp),
        (SchemeType::Turn, ProtoType::Tcp) => Some(RelayTransport::Tcp),
        (SchemeType::Turns, ProtoType::Tcp) => Some(RelayTransport::Tls {
            server_name: url.host.clone(),
        }),
        _ => None,
    }
}

async fn turn_relays(
    servers: Vec<webrtc::ice_transport::ice_server::RTCIceServer>,
) -> Result<VecDeque<Route>> {
    let mut urls = Vec::new();
    for server in &servers {
        for raw in &server.urls {
            let url = webrtc::ice::url::Url::parse_url(raw)?;
            if let Some(carrier) = relay_carrier(&url) {
                let priority = match carrier {
                    parsec_native_bud::bud::RelayTransport::Udp => 0,
                    parsec_native_bud::bud::RelayTransport::Tls { .. } => 1,
                    _ if url.port == 80 => 2,
                    _ => 3,
                };
                urls.push((priority, url, carrier, server));
            }
        }
    }
    urls.sort_by_key(|entry| entry.0);
    // Resolve concurrently so an unavailable DNS entry does not serially hold
    // up other routes. join_all preserves the explicitly sorted URL priority.
    let routes: VecDeque<_> = futures_util::future::join_all(urls.into_iter().take(32).map(
        |(_, url, carrier, server)| async move {
            let addresses = tokio::time::timeout(
                Duration::from_secs(3),
                tokio::net::lookup_host((url.host.as_str(), url.port)),
            )
            .await
            .ok()?
            .ok()?;
            let addr = addresses.into_iter().find(|a| a.is_ipv4())?;
            Some(Route::turn(
                addr,
                &server.username,
                &server.credential,
                carrier,
            ))
        },
    ))
    .await
    .into_iter()
    .flatten()
    .collect();
    ensure!(
        !routes.is_empty(),
        "BUD requires a reachable UDP, TLS or TCP TURN URL"
    );
    Ok(routes)
}

fn log_route_attempt(route: &Route) {
    use parsec_native_bud::bud::RelayTransport;
    let carrier = match route {
        Route::Direct { .. } => None,
        Route::Turn(relay) => Some(match &relay.transport {
            RelayTransport::Udp => "udp",
            RelayTransport::Tcp => "tcp",
            RelayTransport::Tls { .. } => "tls",
        }),
    };
    crate::event_log::record(
        "INFO",
        "bud_route_attempt",
        json!({"route":if carrier.is_some() {"TURN relay"} else {"Direct"}, "requested_turn_transport":carrier}),
    );
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
    trace: &mut crate::bud_trace::Trace,
    hid_input: (bool, bool),
) -> Result<()> {
    let (hid_mouse, hid_keyboard) = hid_input;
    trace.mark("initial_route_resolution");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let (route, mut relay_routes, settings) = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(15), route(manager)).await
    })??;
    trace.mark(if settings.force_relay {
        "initial_relay_selected"
    } else {
        "initial_direct_selected"
    });
    if cancel.load(Ordering::Acquire) {
        return Ok(());
    }
    // Probe before the lifecycle lock: a slow driver must not stall Cancel.
    let hevc_available =
        settings.prefer_hevc && crate::video_windows::hevc_available(window.video_gpu().as_ref());
    let ten_bit = hevc_available
        && settings.experimental_ten_bit
        && crate::video_windows::hevc_ten_bit_available(window.video_gpu().as_ref());
    crate::event_log::record(
        "INFO",
        "video_capabilities_declared",
        json!({"transport":"BUD","flags":8 | u32::from(hevc_available) | if ten_bit {16} else {0},"hevc":hevc_available,"main10":ten_bit}),
    );
    let hid_generation;
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
            ten_bit,
            settings.allow_raw_audio,
            settings.experimental_surround_audio,
        )?;
        {
            hid_generation = window.hid_input_generation();
        }
        (media, session, Statistics::start(window.stats.clone()))
    };
    let _hid_lifetime = crate::hid_input::Lifetime::new(window.clone(), hid_generation);
    session.enable_hid_input(hid_mouse, hid_keyboard);
    let mut hid_states = [(false, 0); 2];
    for (enabled, device) in [(hid_mouse, "mouse"), (hid_keyboard, "keyboard")] {
        if enabled {
            crate::event_log::record(
                "INFO",
                "hid_input_enabled",
                json!({"transport":"BUD","device":device,"experimental":true}),
            );
        }
    }
    state.lock().unwrap_or_else(|e| e.into_inner()).wire_id = id.into();
    session.wake_consumer(std::thread::current());
    trace.mark("initial_native_offer");
    log_route_attempt(&route);
    let creds = session.offer(id, route)?;
    let mut source = session.take_media()?;
    let mut signaling = Signaling::new("host", id)?;
    *audio.lock().unwrap_or_else(|e| e.into_inner()) = Some(media.audio_pipeline());
    *shared_session.lock().unwrap_or_else(|e| e.into_inner()) = Some(session);
    trace.mark("initial_guest_offer_completion");
    if !complete_offer(
        state,
        cancel,
        output,
        signaling.offer(&creds)["payload"]["data"].take(),
        [&creds.ufrag, &creds.pwd, &creds.fingerprint],
    )? {
        return Ok(());
    }
    let mut deadline = Instant::now() + Duration::from_secs(30);
    let mut established = false;
    let mut timing_requests = crate::frame_timing::Requests::default();
    let mut timing_active = false;
    let mut answered_at: Option<Instant> = None;
    let mut direct_attempt = !settings.force_relay;
    let mut wire_id = id.to_owned();
    let mut last_stats = Instant::now();
    let mut last_trace = Instant::now();
    trace.mark("initial_waiting_for_host");
    while !cancel.load(Ordering::Acquire) {
        if !established && Instant::now() >= deadline {
            trace.mark("establishment_deadline");
        }
        let mut guard = shared_session.lock().unwrap_or_else(|e| e.into_inner());
        let session = guard.as_mut().context("BUD session retired")?;
        let mut ended = !established && Instant::now() >= deadline;
        for _ in 0..64 {
            let Ok(text) = incoming.try_recv() else {
                break;
            };
            if native_close(&text, &wire_id) {
                trace.mark("host_close_received");
                ended = true;
                break;
            }
            if let Ok(value) = serde_json::from_str::<Value>(&text) {
                if value["action"] == "answer_relay" {
                    trace.record(
                        "host_answer_received",
                        &[("approved", u64::from(value["payload"]["approved"] == true))],
                    );
                }
            }
            trace.mark("native_signal_processing");
            if let Err(error) = signaling.receive(&text, session) {
                let native = error.downcast_ref::<parsec_native_bud::bud::AnswerFailure>();
                let io = native.and_then(|e| e.io_details());
                crate::event_log::record(
                    "ERROR",
                    "bud_signaling_failed",
                    json!({
                    "native_error":native.map(|e|e.code()), "io_stage":io.map(|e|e.0),
                    "io_kind":io.map(|e|format!("{:?}",e.1)), "os_error":io.and_then(|e|e.2)}),
                );
                if io.is_some() {
                    // A local carrier/thread setup failure is a failed route,
                    // just like the worker's TransportFailed outcome.
                    ended = true;
                    break;
                }
                return Err(error);
            }
            trace.mark("native_signal_processed");
            if let Ok(value) = serde_json::from_str::<Value>(&text) {
                if value["action"] == "answer_relay"
                    && value["payload"]["attempt_id"].as_str() == Some(&wire_id)
                {
                    answered_at.get_or_insert_with(Instant::now);
                    trace.mark("host_answer_accepted");
                } else if value["action"] == "candex_relay" {
                    trace.mark(if value["payload"]["data"]["sync"] == true {
                        "host_readiness_received"
                    } else {
                        "host_candidate_received"
                    });
                }
            }
        }
        let mut events = Vec::new();
        for _ in 0..64 {
            trace.set_stage("native_event_poll");
            let Some(event) = session.poll()? else {
                break;
            };
            events.push(event);
        }
        let transport = session.snapshot();
        if established && statistics.shared.host_timing.current(statistics.generation) {
            let desired = statistics.shared.requested();
            if let Some(request) = timing_requests.update(desired) {
                if desired {
                    timing_active = true;
                }
                statistics
                    .shared
                    .host_timing
                    .set_requested(statistics.generation, desired);
                if session.send_control(&request).is_err() {
                    timing_requests.fail();
                    statistics.shared.host_timing.failed(statistics.generation);
                    crate::event_log::record(
                        "WARN",
                        "host_timing_request_failed",
                        json!({"transport":"BUD"}),
                    );
                } else if !desired {
                    timing_active = false;
                }
            }
        }
        let hid_current = session.hid_input_state();
        window.set_hid_input_active(hid_generation, hid_current[0].0, hid_current[1].0);
        for (index, (enabled, device)) in [(hid_mouse, "mouse"), (hid_keyboard, "keyboard")]
            .into_iter()
            .enumerate()
        {
            let current = hid_current[index];
            let previous = hid_states[index];
            if enabled && current != previous {
                if current.1 != 0 && current.1 != previous.1 {
                    let reason = match current.1 {
                        1 => "host_setup_timeout",
                        2 => "invalid_interrupt_request",
                        3 => "input_queue_limit",
                        4 => "transport_send_failed",
                        _ => "host_device_ended",
                    };
                    crate::event_log::record(
                        "ERROR",
                        "hid_input_failed",
                        json!({"device":device,"reason":reason,"normal_input_fallback":false}),
                    );
                }
                if current.0 != previous.0 {
                    crate::event_log::record(
                        "INFO",
                        "hid_input_state_changed",
                        json!({"device":device,"active":current.0}),
                    );
                }
                hid_states[index] = current;
            }
        }
        if !established && last_trace.elapsed() >= Duration::from_secs(1) {
            trace.record(
                "transport_progress",
                &[
                    ("udp_received", transport.udp_received),
                    ("udp_sent", transport.udp_sent),
                    ("connected", u64::from(transport.connected)),
                    ("relay", u64::from(transport.relay)),
                ],
            );
            last_trace = Instant::now();
        }
        media.recover(session);
        drop(guard);
        trace.set_stage("native_media_processing");
        let pumped = media.pump(&mut source)?;
        trace.set_stage("native_media_processed");
        for event in &events {
            if matches!(event, Event::Candidate { .. } | Event::Ready) {
                trace.mark(if matches!(event, Event::Ready) {
                    "local_readiness_sending"
                } else {
                    "local_candidate_sending"
                });
                send_candidate(state, event)?;
                trace.mark("local_signal_sent");
            }
        }
        let mut shared = state.lock().unwrap_or_else(|e| e.into_inner());
        for event in events {
            if statistics.consume_timing(&event) {
                continue;
            }
            ensure!(
                shared.events.len() < 64 && shared.controls.len() < 64,
                "BUD application queue full"
            );
            match event {
                Event::IoFailure {
                    stage,
                    kind,
                    os_error,
                } => {
                    crate::event_log::record(
                        "ERROR",
                        "bud_transport_failed",
                        json!({"stage":stage.name(), "io_kind":format!("{kind:?}"), "os_error":os_error}),
                    );
                }
                Event::Candidate { .. } | Event::Ready => {}
                Event::Established { .. } => {
                    trace.mark("transport_established");
                    crate::event_log::record(
                        "INFO",
                        "bud_connection_established",
                        json!({"route":if transport.relay {"TURN relay"} else {"Direct"},
                            "turn_transport":transport.relay_transport}),
                    );
                    established = true;
                    shared.established = true;
                    shared.events.push_back(
                        json!({"type":7,"status":0,"state":4,"attemptID":id,"duration":0}),
                    );
                }
                Event::Ended { outcome } => {
                    let reason = match outcome {
                        Outcome::ConnectivityFailed => "ended_connectivity_failed",
                        Outcome::PeerGone => "ended_peer_gone",
                        Outcome::Undeliverable => "ended_undeliverable",
                        Outcome::TransportFailed => "ended_transport_failed",
                        Outcome::Disconnected(_) => "ended_host_disconnected",
                        Outcome::Unreadable => "ended_unreadable",
                        Outcome::RelayUnreachable => "ended_relay_unreachable",
                        Outcome::RelayRefused => "ended_relay_refused",
                        Outcome::RelayLost => "ended_relay_lost",
                    };
                    trace.mark(reason);
                    crate::event_log::record(
                        "WARN",
                        "bud_connection_ended",
                        json!({"reason":reason,"attempt_route":if direct_attempt {"Direct"} else {"TURN relay"},"turn_transport":transport.relay_transport}),
                    );
                    ended = true;
                }
                Event::ExternalControl { bytes } => {
                    shared.controls.push_back(bytes.into());
                }
            }
        }
        let mut ingress = media.ingress();
        ingress.network_rtt = transport.rtt_ms.map(|ms| (Instant::now(), ms));
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
                        "Connected (BUD)"
                    } else {
                        "Connecting (BUD)"
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
                    turn_protocol: transport.relay_transport.map(str::to_owned),
                    audio_bytes: shared.ingress.audio_bytes_received,
                    audio_codec: shared.ingress.audio_output.as_ref().and_then(|a| a.codec),
                    audio_layout: shared.ingress.audio_output.as_ref().and_then(|a| a.layout),
                    audio_decoded_queue_ms: shared
                        .ingress
                        .audio_output
                        .as_ref()
                        .map(|a| a.queued_frames as f64 / 48.0),
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
            && (ended
                || (direct_attempt
                    && answered_at.is_some_and(|t| t.elapsed() >= Duration::from_secs(4))));
        if fallback_due {
            trace.mark("relay_retry_started");
            if relay_routes.is_none() {
                relay_routes = Some(runtime.block_on(async {
                    tokio::time::timeout(Duration::from_secs(15), relay_route(manager, &settings))
                        .await
                })??);
            }
            let relay = relay_routes
                .as_mut()
                .and_then(VecDeque::pop_front)
                .context("BUD configured routes exhausted")?;
            direct_attempt = false;
            timing_requests = Default::default();
            timing_active = false;
            statistics.shared.host_timing.begin(statistics.generation);
            drop(source);
            let mut old = shared_session
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take();
            if let Some(old) = &mut old {
                old.close();
            }
            drop(old);
            trace.mark("relay_credentials_ready");
            if cancel.load(Ordering::Acquire) {
                return Ok(());
            }
            // The native client uses six random 8-hex groups per attempt.
            // Do not append a diagnostic suffix to an account protocol ID:
            // local SDK acceptance does not establish service compatibility.
            wire_id = native_attempt_id()?;
            let mut next = crate::bud_media::session(
                hevc_available,
                ten_bit,
                settings.allow_raw_audio,
                media.audio_layout(),
            )?;
            next.enable_hid_input(hid_mouse, hid_keyboard);
            next.wake_consumer(std::thread::current());
            trace.mark("relay_native_offer");
            log_route_attempt(&relay);
            let creds = next.offer(&wire_id, relay)?;
            source = next.take_media()?;
            signaling = Signaling::new("host", &wire_id)?;
            *shared_session.lock().unwrap_or_else(|e| e.into_inner()) = Some(next);
            trace.mark("relay_account_offer_sending");
            if !switch_offer(
                state,
                cancel,
                &wire_id,
                signaling.offer(&creds)["payload"]["data"].take(),
            )? {
                return Ok(());
            }
            trace.mark("relay_account_offer_sent");
            answered_at = None;
            // A fresh relay attempt needs the same establishment allowance as
            // Force relay, independent of time spent discovering a direct path.
            deadline = Instant::now() + Duration::from_secs(30);
            continue;
        }
        ensure!(!ended, "BUD transport ended");
        if pumped.video_messages >= 8 || pumped.audio_messages >= 8 {
            continue;
        }
        // Packet arrivals unpark this consumer; this timeout only checks lifecycle/stats.
        std::thread::park_timeout(Duration::from_millis(100));
    }
    if timing_active {
        if let Some(session) = shared_session
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
        {
            let _ = session.send_control(&crate::frame_timing::request(false));
        }
    }
    Ok(())
}

fn native_close(text: &str, attempt: &str) -> bool {
    serde_json::from_str::<Value>(text).is_ok_and(|value| {
        value["action"] == "close" && value["payload"]["attempt_id"].as_str() == Some(attempt)
    })
}

fn native_attempt_id() -> Result<String> {
    let mut random = [0u8; 24];
    getrandom::fill(&mut random).map_err(|_| anyhow::anyhow!("BUD attempt entropy unavailable"))?;
    Ok(random
        .as_chunks::<4>()
        .0
        .iter()
        .map(|part| {
            format!(
                "{:02x}{:02x}{:02x}{:02x}",
                part[0], part[1], part[2], part[3]
            )
        })
        .collect::<Vec<_>>()
        .join("-"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn timing_reports_bypass_application_events_and_retire_with_their_generation() {
        let bus = std::sync::Arc::new(crate::stats::Shared::default());
        let statistics = super::Statistics::start(bus.clone());
        bus.host_timing.set_requested(statistics.generation, true);
        let event = parsec_native_bud::bud::Event::ExternalControl {
            bytes: crate::frame_timing::tests::packet(1, 0, &[8., 0., 20.]),
        };
        for _ in 0..10_000 {
            assert!(statistics.consume_timing(&event));
        }
        assert_eq!(
            bus.host_timing
                .snapshot(statistics.generation)
                .audio
                .unwrap()
                .count,
            10_000
        );
        assert!(
            !statistics.consume_timing(&parsec_native_bud::bud::Event::ExternalControl {
                bytes: vec![0; 13]
            })
        );
        let newer = super::Statistics::start(bus.clone());
        bus.host_timing.set_requested(newer.generation, true);
        assert!(statistics.consume_timing(&event));
        drop(statistics);
        let report = bus.host_timing.snapshot(newer.generation);
        assert!(report.requested);
        assert!(report.audio.is_none());
    }
    use super::*;
    #[test]
    fn turn_urls_prefer_udp_then_tls_then_tcp80_without_rewriting_ports() {
        use parsec_native_bud::bud::RelayTransport;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let routes = runtime
            .block_on(turn_relays(vec![
                webrtc::ice_transport::ice_server::RTCIceServer {
                    urls: vec![
                        "turn:127.0.0.1:3478?transport=tcp".into(),
                        "turn:127.0.0.1:80?transport=tcp".into(),
                        "turns:localhost:443?transport=tcp".into(),
                        "stun:127.0.0.1:3478".into(),
                        "turn:127.0.0.1:3478?transport=udp".into(),
                    ],
                    username: "isolated".into(),
                    credential: "fixture-only".into(),
                },
            ]))
            .unwrap();
        let relays: Vec<_> = routes
            .into_iter()
            .map(|route| match route {
                Route::Turn(relay) => relay,
                _ => panic!("TURN URL leaked a direct route"),
            })
            .collect();
        assert_eq!(relays.len(), 4);
        assert_eq!(
            relays.iter().map(|r| r.server.port()).collect::<Vec<_>>(),
            [3478, 443, 80, 3478]
        );
        assert_eq!(relays[0].transport, RelayTransport::Udp);
        assert_eq!(
            relays[1].transport,
            RelayTransport::Tls {
                server_name: "localhost".into()
            }
        );
        assert_eq!(relays[2].transport, RelayTransport::Tcp);
        assert_eq!(relays[3].transport, RelayTransport::Tcp);
    }
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
                hid_input: (false, false),
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
    fn direct_close_is_owned_by_worker_and_cannot_cancel_guest_before_relay_retry() {
        let (attempt, incoming) = isolated();
        let close = r#"{"action":"close","payload":{"attempt_id":"isolated"}}"#;
        assert_eq!(attempt.incoming(close).unwrap(), Some(String::new()));
        let queued = incoming.try_recv().unwrap();
        assert!(native_close(&queued, "isolated"));
        assert!(!native_close(&queued, "isolated-turn-1"));
        assert!(!attempt.cancel.load(Ordering::Acquire));
        assert!(attempt.failure().is_none());
        // A late close from the retired direct route must not terminate relay.
        {
            let mut state = attempt.state.lock().unwrap();
            state.retired_wires.push("isolated".into());
            state.wire_id = "isolated-turn-1".into();
        }
        assert_eq!(attempt.incoming(close).unwrap(), Some(String::new()));
        assert!(incoming.try_recv().is_err());
        let relay_close = r#"{"action":"close","payload":{"attempt_id":"isolated-turn-1"}}"#;
        assert_eq!(attempt.incoming(relay_close).unwrap(), Some(String::new()));
        assert!(native_close(
            &incoming.try_recv().unwrap(),
            "isolated-turn-1"
        ));
    }

    #[test]
    fn late_signals_from_all_retired_carriers_are_suppressed() {
        let (attempt, incoming) = isolated();
        let offer = json!({"action":"offer","payload":{"attempt_id":"isolated","to":"fixture-host","data":{}}}).to_string();
        attempt.state.lock().unwrap().wire_id = "isolated".into();
        attempt.outgoing(&offer, Some(Arc::new(|_| true))).unwrap();
        for current in ["udp-attempt", "tls-attempt", "tcp-attempt"] {
            assert!(switch_offer(
                &attempt.state,
                &attempt.cancel,
                current,
                json!({"creds":{}})
            )
            .unwrap());
        }
        for retired in ["isolated", "udp-attempt", "tls-attempt"] {
            for action in ["close", "answer_relay", "candex_relay"] {
                let signal = json!({"action":action,"payload":{"attempt_id":retired}}).to_string();
                assert_eq!(attempt.incoming(&signal).unwrap(), Some(String::new()));
            }
        }
        assert!(incoming.try_recv().is_err());
        let state = attempt.state.lock().unwrap();
        assert_eq!(state.wire_id, "tcp-attempt");
        assert_eq!(state.retired_wires.len(), 3);
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
        assert!(!settings.bud);
        let settings: crate::connection_settings::Settings =
            serde_json::from_str(r#"{"bud":true}"#).unwrap();
        assert!(settings.bud);
    }

    #[test]
    fn relay_retry_uses_fresh_native_account_identifier_shape() {
        let first = native_attempt_id().unwrap();
        let second = native_attempt_id().unwrap();
        assert_ne!(first, second);
        for id in [&first, &second] {
            assert_eq!(id.len(), 53);
            let groups: Vec<_> = id.split('-').collect();
            assert_eq!(groups.len(), 6);
            assert!(groups.iter().all(
                |group| group.len() == 8 && group.bytes().all(|byte| byte.is_ascii_hexdigit())
            ));
        }
        // Check the actual wire handoff, not just the random generator.
        let (attempt, _) = isolated();
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
        let offer = json!({"action":"offer","payload":{"attempt_id":"isolated","to":"host","secret":"authorization-sentinel","data":{"mode":2}}}).to_string();
        attempt.outgoing(&offer, Some(writer)).unwrap();
        switch_offer(&attempt.state, &attempt.cancel, &first, json!({"mode":1})).unwrap();
        send_candidate(&attempt.state, &Event::Ready).unwrap();
        let writes = writes.lock().unwrap();
        assert_eq!(writes[1]["payload"]["attempt_id"], first);
        assert_eq!(writes[2]["payload"]["attempt_id"], first);
        assert_eq!(writes[1]["payload"]["secret"], "authorization-sentinel");
        drop(writes);
        assert_eq!(
            attempt
                .incoming(r#"{"action":"close","payload":{"attempt_id":"isolated"}}"#)
                .unwrap(),
            Some(String::new())
        );
    }
}
