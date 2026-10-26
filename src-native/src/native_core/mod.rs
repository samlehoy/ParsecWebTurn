//! EvoParseq native account and session entry point.
//! The production media, carrier, route retry and HID workers are reused.
mod account;
mod controls;
pub(crate) mod host_displays;
mod host_video;
mod input;
mod picker;
mod pointer;
pub(crate) mod profile;
mod session_badge;
pub(crate) mod session_model;
pub(crate) mod session_ui;
mod session_video;
mod signaling;
mod transport;
pub(crate) mod ui;

use anyhow::{ensure, Context, Result};
use session_model::{Notification, Status, View};
use std::{
    collections::VecDeque,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

pub fn main() {
    if let Err(error) = run() {
        // Every error crossing this entry is an application-authored message;
        // raw HTTP/WebSocket errors must never display their credential URL.
        unsafe {
            ui::message_box(std::ptr::null_mut(), &error.to_string());
        }
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--version"] {
        println!(
            "EvoParseq {} / native Rust core (BUD/WebRTC)",
            env!("CARGO_PKG_VERSION")
        );
        return Ok(());
    }
    let root = if args.is_empty() {
        std::env::current_exe()?
            .parent()
            .context("Application folder unavailable")?
            .to_path_buf()
    } else if args.len() == 2 && args[0] == "--data-dir" {
        std::path::PathBuf::from(&args[1])
    } else {
        anyhow::bail!("Unsupported native core argument");
    };
    let settings = crate::connection_settings::Manager::open(root.join("settings.json"));
    crate::event_log::initialize(
        root.join("client-events.jsonl"),
        settings.view().0.event_logging,
    )?;
    let _log = crate::event_log::Guard;
    crate::event_log::record(
        "INFO",
        "native_core_started",
        serde_json::json!({"transport":if settings.view().0.bud {"BUD"} else {"WebRTC"}}),
    );
    let mut session = None;
    loop {
        let Some(selection) = ui::choose(settings.clone(), session.take())? else {
            return Ok(());
        };
        let preferences = settings.view().0;
        let requested_main10 = preferences.prefer_hevc && preferences.experimental_ten_bit;
        let policy = transport::Policy {
            bud: preferences.bud,
        };
        let result = (|| {
            let socket = signaling::Socket::open(&selection.session)?;
            let window = crate::window::Window::create_native()?;
            window.install_connection_settings(settings.clone());
            let result = stream(&selection.peer, &socket, &window, settings.clone(), policy);
            window.end_session();
            window.request_stop();
            window.close();
            result
        })();
        if let Err(error) = result {
            crate::event_log::record(
                "ERROR",
                "native_core_failed",
                serde_json::json!({"stage":"signaling_or_session"}),
            );
            unsafe {
                let message = error
                    .downcast_ref::<transport::HostStatus>()
                    .map(|status| status.user_message(requested_main10))
                    .unwrap_or_else(|| error.to_string());
                ui::message_box(std::ptr::null_mut(), &message);
            }
        }
        session = Some(selection.session);
    }
}

fn stream(
    peer: &str,
    socket: &signaling::Socket,
    window: &Arc<crate::window::Window>,
    manager: Arc<crate::connection_settings::Manager>,
    policy: transport::Policy,
) -> Result<()> {
    let id = crate::bud_attempt::native_attempt_id()?;
    let attempt = transport::Attempt::spawn_native(
        &id,
        peer,
        socket.writer.clone(),
        window.clone(),
        manager.clone(),
        policy,
    )?;
    let services = crate::platform::Services::new(Box::new(
        crate::platform_windows::NativeDesktop(window.handle() as usize),
    ));
    let mut batch = VecDeque::new();
    let mut pointer = pointer::Pointer::default();
    let mut input = input::Input::default();
    let mut output: Option<Arc<crate::audio_windows::Playback>> = None;
    let mut established = false;
    let mut audio_source = None;
    let mut local_keys = std::collections::BTreeSet::new();
    window.events.set_consumer(std::thread::current());
    let mut video = manager.view().0.native_core.video;
    video.validate()?;
    let mut fullscreen = video.fullscreen;
    window.set_fullscreen(fullscreen);
    window.set_input_state(crate::input_state::State {
        mode: video.mode(),
        ..Default::default()
    });
    let view = window.as_ref();
    let mut host_settings = session_video::HostSettings::new(&video, view);
    let mut detached = false;
    let mut last_clipboard = String::new();
    while !window.closing.load(Ordering::Acquire) && !window.stop.stopped() {
        // The service can close its signaling socket after the BUD path is up.
        for _ in 0..64 {
            match socket.next() {
                Ok(Some(text)) => {
                    attempt.incoming(&text)?;
                }
                Ok(None) => break,
                Err(_) if established || attempt.native_connected() => break,
                Err(_) => anyhow::bail!(
                    "Account signaling ended before {} connected.",
                    attempt.name()
                ),
            }
        }
        while let Some(event) = attempt.pop_event() {
            if event["type"] == 7 {
                if event["status"] != 0 {
                    if let Some(code) = attempt.host_status() {
                        return Err(transport::HostStatus::from_window(code, window).into());
                    }
                }
                ensure!(
                    event["status"] == 0,
                    "The {} connection ended.",
                    attempt.name()
                );
                if !established {
                    host_settings.connected(&attempt, view)?;
                }
                established = true;
                window.set_wake_lock(true);
            }
        }
        if attempt.failure().is_some() {
            if let Some(code) = attempt.host_status() {
                return Err(transport::HostStatus::from_window(code, window).into());
            }
            anyhow::bail!(
                "{} connection failed. See the event log for its stage.",
                attempt.name()
            );
        }
        for _ in 0..64 {
            let Some(frame) = attempt.pop_control() else {
                break;
            };
            if frame.get(12) == Some(&9) {
                pointer.receive(&frame, window)?;
                continue;
            }
            match crate::control::decode(&frame)? {
                crate::control::Message::Buffer {
                    event,
                    payload: Some(range),
                } if event["type"] == 3 && event["id"] == 11 => {
                    let bytes = &frame[range];
                    let bytes = bytes.strip_suffix(&[0]).unwrap_or(bytes);
                    host_settings.receive(bytes, &attempt, view);
                }
                crate::control::Message::Buffer {
                    event,
                    payload: Some(range),
                } if event["type"] == 3 && event["id"] == 12 => {
                    let bytes = &frame[range];
                    host_settings.receive_outputs(
                        bytes.strip_suffix(&[0]).unwrap_or(bytes),
                        &attempt,
                        view,
                    );
                }
                crate::control::Message::Buffer {
                    event,
                    payload: Some(range),
                } if event["type"] == 3 && event["id"] == 7 => {
                    let bytes = &frame[range];
                    if let Ok(text) = std::str::from_utf8(bytes.strip_suffix(&[0]).unwrap_or(bytes))
                    {
                        if window.clipboard_allowed() && text != last_clipboard {
                            services.write_text(text);
                            last_clipboard = text.into();
                        }
                    }
                }
                crate::control::Message::Status(status) if status != 0 => {
                    crate::event_log::record(
                        "ERROR",
                        "native_host_status",
                        serde_json::json!({"transport":attempt.name(), "host_status":status}),
                    );
                    return Err(transport::HostStatus::from_window(status, window).into());
                }
                _ => {}
            }
        }
        host_settings.poll(&attempt, view)?;
        if audio_source.is_none() {
            audio_source = attempt.audio_pipeline();
            if let Some(audio) = &audio_source {
                audio.set_native_consumer(std::thread::current());
            }
        }
        if let Some(audio) = &audio_source {
            for index in 0..8 {
                if index == 7 {
                    std::thread::current().unpark();
                }
                let Some(block) = audio.take_native_pcm() else {
                    break;
                };
                if output.as_ref().is_none_or(|p| p.layout != block.layout) {
                    if let Some(old) = output.take() {
                        old.stop();
                    }
                    let playback = Arc::new(crate::audio_windows::Playback::create_layout(
                        window.clone(),
                        75,
                        150,
                        block.layout,
                    )?);
                    window.stats.set_audio(Arc::downgrade(&playback));
                    output = Some(playback);
                }
                if let Some(playback) = &output {
                    playback.queue(&block.pcm);
                }
                audio.recycle_handoff(block);
            }
        }
        window.events.take_batch(&mut batch);
        for event in batch.drain(..) {
            use crate::window::Event;
            if let Event::Key(true, code, mods) = &event {
                if mods & 3 == 3 && matches!(*code, "KeyQ" | "KeyW" | "KeyR") {
                    local_keys.insert(*code);
                    match *code {
                        "KeyQ" => window.request_stop(),
                        "KeyW" => {
                            fullscreen = !fullscreen;
                            window.set_fullscreen(fullscreen);
                        }
                        "KeyR" => {
                            if established {
                                attempt.send(&crate::control::header(24, 0, 0, 0))?;
                            }
                            detached = !detached;
                            window.set_input_state(crate::input_state::State {
                                detached,
                                mode: video.mode(),
                                ..Default::default()
                            });
                        }
                        _ => unreachable!(),
                    }
                    continue;
                }
            }
            if let Event::Key(false, code, _) = &event {
                if local_keys.remove(code) {
                    continue;
                }
            }
            if matches!(&event, Event::Clipboard)
                || matches!(&event,Event::Key(true,"KeyV",mods) if mods & 14==2)
            {
                window.take_clipboard_event();
                if established && window.clipboard_allowed() {
                    if let Some(text) = services.read_notification_text() {
                        if text != last_clipboard {
                            attempt.send(&crate::control::text(17, 7, &text)?)?;
                            last_clipboard = text;
                        }
                    }
                }
            }
            if let Event::NativeSession(action) = &event {
                match action {
                    session_model::Command::Disconnect => window.request_stop(),
                    session_model::Command::HideButton => {
                        if manager
                            .update_native(|p| p.hide_session_button = true)
                            .is_err()
                        {
                            crate::event_log::record(
                                "WARN",
                                "native_session_button_save_failed",
                                serde_json::json!({}),
                            );
                        }
                        window.native_refresh_overlay();
                    }
                    session_model::Command::SecureAttention => {
                        // Same application request as the stock menu. The host
                        // authorizes it and needs its installed service for SAS.
                        let result = (|| -> Result<()> {
                            ensure!(established, "Transport is not connected yet");
                            attempt.send(&crate::control::header(24, 0, 0, 0))?;
                            attempt.send(&crate::control::text(17, 14, "")?)
                        })();
                        crate::event_log::record(
                            if result.is_ok() { "INFO" } else { "WARN" },
                            "native_secure_attention_requested",
                            serde_json::json!({"sent":result.is_ok()}),
                        );
                        view.notify(Notification::Status(if result.is_ok() {
                            Status::AttentionSent
                        } else {
                            Status::AttentionFailed
                        }));
                    }
                    session_model::Command::SelectDisplay(id) => {
                        let result = if established {
                            host_settings.select_output(&attempt, view, id)
                        } else {
                            Err(anyhow::anyhow!("Transport is not connected"))
                        };
                        if matches!(result, Ok(true)) {
                            // A monitor switch releases the previous monitor's
                            // forced resolution. Keep the live and saved UI
                            // preference consistent with that host request.
                            video.resolution = profile::Resolution::Host;
                            if manager
                                .update_native(|p| p.video.resolution = profile::Resolution::Host)
                                .is_err()
                            {
                                crate::event_log::record(
                                    "WARN",
                                    "native_display_preference_save_failed",
                                    serde_json::json!({}),
                                );
                            }
                            view.notify(Notification::DisplaySelected);
                        }
                        if result.is_err() {
                            crate::event_log::record(
                                "WARN",
                                "native_host_display_request_failed",
                                serde_json::json!({"hid":attempt.hid_mouse_enabled()}),
                            );
                        }
                        host_settings.result(view, result.is_ok());
                        view.notify(Notification::DisplaysChanged);
                    }
                    session_model::Command::RefreshDisplays => {
                        if established && host_settings.refresh_outputs(&attempt, view).is_err() {
                            view.result(false);
                        }
                    }
                    session_model::Command::Apply(next) => {
                        let result = (|| -> Result<()> {
                            ensure!(established, "Transport is not connected yet");
                            let size = host_settings.apply(&attempt, view, next)?;
                            manager.update_native(|p| p.video = next.clone())?;
                            video = next.clone();
                            fullscreen = video.fullscreen;
                            host_settings.applied_locally(size);
                            attempt.send(&crate::control::header(24, 0, 0, 0))?;
                            window.set_input_state(crate::input_state::State {
                                mode: video.mode(),
                                detached,
                                ..Default::default()
                            });
                            window.set_fullscreen(fullscreen);
                            Ok(())
                        })();
                        if result.is_err() {
                            crate::event_log::record(
                                "WARN",
                                "native_session_settings_failed",
                                serde_json::json!({}),
                            );
                        }
                        host_settings.result(view, result.is_ok());
                    }
                }
                continue;
            }
            if let Event::Fullscreen(active) = &event {
                fullscreen = *active;
                video.fullscreen = *active;
                let _ = manager.update_native(|p| p.video.fullscreen = *active);
            }
            if matches!(event, Event::Size(..))
                && established
                && video.resolution == profile::Resolution::Client
            {
                host_settings.resized(&attempt, view, &video)?;
            }
            if established {
                input.send(&attempt, window, &event)?;
            }
        }
        // Existing FIFO wakes immediately for input; media decoding and presentation
        // stay on their original workers. This interval only services control/audio.
        std::thread::park_timeout(Duration::from_millis(100));
    }
    if established {
        let _ = attempt.send(&crate::control::header(24, 0, 0, 0));
    }
    drop(attempt);
    if let Some(output) = output {
        output.stop();
    }
    Ok(())
}
