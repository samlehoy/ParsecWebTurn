//! Experimental BUD-only entry: no WASM load, execution or guest callbacks.
//! The production media, carrier, route retry and HID workers are reused.
mod account;
mod input;
mod pointer;
pub(crate) mod profile;
pub(crate) mod session_ui;
mod signaling;
pub(crate) mod ui;

use anyhow::{ensure, Context, Result};
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
            "ParsecWebTurn {} / native Rust core (BUD)",
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
        serde_json::json!({"transport":"BUD","wasm":false}),
    );
    let mut session = None;
    loop {
        let Some(selection) = ui::choose(settings.clone(), session.take())? else {
            return Ok(());
        };
        let result = (|| {
            let socket = signaling::Socket::open(&selection.session)?;
            let window = crate::window::Window::create_native()?;
            window.install_connection_settings(settings.clone());
            let result = stream(&selection.peer, &socket, &window, settings.clone());
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
                ui::message_box(std::ptr::null_mut(), &error.to_string());
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
) -> Result<()> {
    let id = crate::bud_attempt::native_attempt_id()?;
    let attempt = crate::bud_attempt::Attempt::spawn_native(
        &id,
        peer,
        socket.writer.clone(),
        window.clone(),
        manager.clone(),
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
    window.set_input_state(crate::core_input::State {
        mode: video.mode(),
        ..Default::default()
    });
    let mut requested_display = None;
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
                Err(_) => anyhow::bail!("Account signaling ended before BUD connected."),
            }
        }
        while let Some(event) = attempt.pop_event() {
            if event["type"] == 7 {
                ensure!(event["status"] == 0, "The BUD connection ended.");
                if !established {
                    let size = window.native_display_size();
                    attempt.send(&video.request(size)?)?;
                    requested_display = Some(size);
                }
                established = true;
                window.set_wake_lock(true);
            }
        }
        if attempt.failure().is_some() {
            anyhow::bail!("BUD connection failed. See the event log for its stage.");
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
                _ => {}
            }
        }
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
                if mods & 3 == 3 && matches!(*code, "KeyQ" | "KeyW" | "KeyR" | "KeyO") {
                    local_keys.insert(*code);
                }
                if mods & 3 == 3 && *code == "KeyO" {
                    window.native_toggle_overlay();
                    continue;
                }
                if mods & 3 == 3 && *code == "KeyQ" {
                    window.request_stop();
                    continue;
                }
                if mods & 3 == 3 && *code == "KeyW" {
                    fullscreen = !fullscreen;
                    window.set_fullscreen(fullscreen);
                    continue;
                }
                if mods & 3 == 3 && *code == "KeyR" {
                    if established {
                        attempt.send(&crate::control::header(24, 0, 0, 0))?;
                    }
                    detached = !detached;
                    window.set_input_state(crate::core_input::State {
                        detached,
                        mode: video.mode(),
                        ..Default::default()
                    });
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
                    session_ui::Action::Disconnect => window.request_stop(),
                    session_ui::Action::Apply(next) => {
                        let result = (|| -> Result<()> {
                            ensure!(established, "BUD is not connected yet");
                            let size = window.native_display_size();
                            attempt.send(&next.request(size)?)?;
                            manager.update_native(|p| p.video = next.clone())?;
                            video = next.clone();
                            fullscreen = video.fullscreen;
                            requested_display = Some(size);
                            attempt.send(&crate::control::header(24, 0, 0, 0))?;
                            window.set_input_state(crate::core_input::State {
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
                        session_ui::result(window, result.is_ok());
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
                let size = window.native_display_size();
                if requested_display != Some(size) {
                    attempt.send(&video.request(size)?)?;
                    requested_display = Some(size);
                }
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
