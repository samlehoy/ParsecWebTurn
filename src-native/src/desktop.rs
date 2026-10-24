//! Bounded offline native UI experiment, separate from connected video.
use crate::{host::HostState, window::Event};
use anyhow::{bail, Context, Result};
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
use wasmtime::{Caller, Instance, Store, Val};

pub fn handles(name: &str) -> bool {
    name.starts_with("gl")
        || matches!(
            name,
            "web_set_gfx"
                | "web_gl_flush"
                | "web_present"
                | "web_set_canvas_size"
                | "web_run_and_yield"
                | "web_set_app"
                | "web_set_title"
                | "MTY_DecompressImage"
        )
}

fn invoke(caller: &mut Caller<'_, HostState>, name: &str, args: &[Val]) -> Result<()> {
    let f = caller
        .get_export(name)
        .and_then(|e| e.into_func())
        .with_context(|| format!("Missing {name}"))?;
    f.call(caller, args, &mut [])
}

pub fn dispatch(
    caller: &mut Caller<'_, HostState>,
    name: &str,
    args: &[Val],
    results: &mut [Val],
) -> Result<()> {
    let window = caller
        .data()
        .window
        .clone()
        .context("native window missing")?;
    let m = caller.data().memory.clone();
    let int = |n: usize| {
        args.get(n)
            .and_then(Val::i32)
            .context("native bridge expected i32")
    };
    match name {
        "web_set_gfx" => {
            if caller.data().graphics.is_some() {
                bail!("GPU context already initialized");
            }
            caller.data_mut().graphics = Some(crate::graphics::Graphics::create(window)?);
            crate::performance::record_memory("graphics_initialized", Some(m.0.data_size()));
        }
        "web_present" => caller
            .data_mut()
            .graphics
            .as_mut()
            .context("GPU context missing")?
            .present()?,
        "web_set_canvas_size" => {
            let (w, h) = (int(0)?, int(1)?);
            if w < 0 || h < 0 || w > 8192 || h > 8192 {
                bail!("Invalid native canvas dimensions");
            }
            // Win32 owns the swapchain size. Matoya sets viewport in its GL calls.
        }
        "web_run_and_yield" => {
            if caller.data().guest_worker {
                return run_worker_callback(caller, int(0)? as u32, int(1)?);
            }
            caller.data_mut().event_loop = Some((int(0)? as u32, int(1)? as u32));
            bail!("native event-loop handoff");
        }
        "web_set_app" => {
            let app = int(0)?;
            caller.data_mut().app_pointer = Some(app as u32);
            window.gui_visibility.publish_app(app as u32);
            let (w, h) = *window.dimensions.lock().unwrap_or_else(|e| e.into_inner());
            let (x, y, screen_w, screen_h, focused) = window.initial_geometry();
            for (name, x, y) in [
                ("mty_window_update_position", x, y),
                ("mty_window_update_screen", screen_w, screen_h),
                ("mty_window_update_size", w, h),
            ] {
                invoke(
                    caller,
                    name,
                    &[
                        Val::I32(app),
                        Val::F64((x as f64).to_bits()),
                        Val::F64((y as f64).to_bits()),
                    ],
                )?;
            }
            for (name, values) in [
                ("mty_window_update_focus", vec![app, focused as i32]),
                ("mty_window_update_fullscreen", vec![app, 0]),
                ("mty_window_update_visibility", vec![app, 1]),
                ("mty_window_update_relative_mouse", vec![app, 0]),
            ] {
                invoke(
                    caller,
                    name,
                    &values.into_iter().map(Val::I32).collect::<Vec<_>>(),
                )?;
            }
            invoke(
                caller,
                "mty_window_update_pixel_ratio",
                &[Val::I32(app), Val::F64(1.0_f64.to_bits())],
            )?;
        }
        "web_set_title" => {
            m.string(int(0)? as u32, 1024)?;
            let title = crate::APP_TITLE.to_owned();
            let wide: Vec<u16> = title.encode_utf16().chain([0]).collect();
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::SetWindowTextW(
                    window.handle(),
                    wide.as_ptr(),
                );
            }
            caller.data_mut().title = Some(title);
        }
        "MTY_DecompressImage" => {
            let size = usize::try_from(int(1)?)?;
            if size > 16 * 1024 * 1024 {
                bail!("Encoded UI image limit exceeded");
            }
            let bytes = m.read(int(0)? as u32, size)?;
            let mut decoder =
                image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(4096);
            limits.max_image_height = Some(4096);
            limits.max_alloc = Some(32 * 1024 * 1024);
            decoder.limits(limits);
            let image = decoder.decode()?.into_rgba8();
            let (w, h) = image.dimensions();
            let alloc = caller
                .get_export("mty_system_alloc")
                .and_then(|e| e.into_func())
                .context("guest allocator missing")?
                .typed::<(i32, i32), i32>(&*caller)?;
            let p = alloc.call(&mut *caller, (image.as_raw().len() as i32, 1))? as u32;
            if p == 0 {
                bail!("Guest UI image allocation failed");
            }
            m.write(p, image.as_raw())?;
            m.set_u32(int(2)? as u32, w)?;
            m.set_u32(int(3)? as u32, h)?;
            if let Some(slot) = results.first_mut() {
                *slot = Val::I32(p as i32);
            }
        }
        _ => {
            let value = caller
                .data_mut()
                .graphics
                .as_mut()
                .context("GPU context missing")?
                .call(name, &m, args)?;
            if let Some(value) = value {
                if let Some(slot) = results.first_mut() {
                    *slot = Val::I32(value);
                }
            }
        }
    }
    Ok(())
}

fn export(
    store: &mut Store<HostState>,
    instance: &Instance,
    name: &'static str,
    args: &[Val],
) -> Result<()> {
    #[cfg(any(test, feature = "diagnostics"))]
    {
        store.data_mut().execution_stage = Some(name);
    }
    instance
        .get_func(&mut *store, name)
        .with_context(|| format!("Missing {name}"))?
        .call(store, args, &mut [])
}

pub fn run(store: &mut Store<HostState>, instance: &Instance) -> Result<()> {
    run_loop(store, instance, true)
}

fn run_worker_callback(caller: &mut Caller<'_, HostState>, index: u32, opaque: i32) -> Result<()> {
    // Keep the WASM call stack alive: returning from this import must run the
    // audio cleanup and libc pthread exit/notify before a guest join completes.
    // Trapping and running callbacks in a new invocation skips that epilogue.
    let window = caller
        .data()
        .window
        .clone()
        .context("native window missing")?;
    let table = caller
        .get_export("__indirect_function_table")
        .and_then(|e| e.into_table())
        .context("callback table missing")?;
    let callback = match table.get(&mut *caller, index as u64) {
        Some(wasmtime::Ref::Func(Some(f))) => f.typed::<i32, i32>(&*caller)?,
        _ => bail!("invalid worker callback"),
    };
    let until = Instant::now() + Duration::from_secs(window.run_seconds);
    let mut first = true;
    let memory_warmup_at = Instant::now() + Duration::from_secs(3);
    let mut memory_warmup_recorded = false;
    while !window.closing.load(Ordering::Acquire) && (window.live || Instant::now() < until) {
        let started = Instant::now();
        caller.set_fuel(if first { 500_000_000 } else { 50_000_000 })?;
        // Only the GUI worker owns a graphics context. Audio/other worker
        // callbacks and their polling cadence remain unchanged. Consume at
        // most one phase per callback so the core observes hidden -> visible
        // even if Settings was opened and closed between render iterations.
        if caller.data().graphics.is_some() {
            if let Some((app, visible)) = window.gui_visibility.take_transition() {
                invoke(
                    caller,
                    "mty_window_update_visibility",
                    &[Val::I32(app as i32), Val::I32(visible as i32)],
                )?;
                if !visible {
                    // An idle core may skip present() entirely while covered.
                    // Acknowledge the last completed GL callback here instead
                    // of waiting for another suppressed SwapBuffers request.
                    window.repaint_settings_after_gl_pause();
                }
            }
        }
        if callback.call(&mut *caller, opaque)? == 0 {
            return Ok(());
        }
        if caller.data().graphics.is_some() {
            if first {
                crate::performance::record_memory(
                    "first_graphics_callback",
                    Some(caller.data().memory.0.data_size()),
                );
            }
            if !memory_warmup_recorded && started >= memory_warmup_at {
                memory_warmup_recorded = true;
                crate::performance::record_memory(
                    "graphics_warmup",
                    Some(caller.data().memory.0.data_size()),
                );
            }
        }
        first = false;
        // Worker callbacks (including the pinned audio loop) already wait via
        // libc nanosleep. They are not display callbacks: a 60 Hz throttle here
        // delays audio polling and creates avoidable underruns. Only guard a
        // callback that returns immediately against spinning a CPU core.
        let remaining = Duration::from_millis(1).saturating_sub(started.elapsed());
        if !remaining.is_zero() {
            std::thread::sleep(remaining);
        }
    }
    if window.closing.load(Ordering::Acquire) {
        return Err(crate::lifecycle::cancellation_error());
    }
    bail!("worker callback deadline reached before normal completion")
}

struct CallbackCadence {
    next: Instant,
    period: Duration,
}
impl CallbackCadence {
    fn due(&mut self, now: Instant, cursor_ready: bool) -> bool {
        if !cursor_ready && now < self.next {
            return false;
        }
        // Skip overdue ticks instead of running a burst of catch-up callbacks.
        self.next = now + self.period;
        true
    }
}

fn dispatch_input_event(
    store: &mut Store<HostState>,
    instance: &Instance,
    app: i32,
    event: Event,
) -> Result<()> {
    // Clipboard contents must arrive before a paste key on the same ordered
    // control path, including when BUD translates keyboard input to USB HID.
    if matches!(&event, Event::Key(true, "KeyV", mods) if mods & 14 == 2)
        || matches!(&event, Event::Key(true, "Insert", mods) if mods & 15 == 1)
    {
        sync_clipboard(store, instance, app);
    }
    #[cfg(feature = "bud")]
    if matches!(&event, Event::Focus(false) | Event::HidInputRelease) {
        // Explicit focus release also resets the USB devices' held input
        // and queued movement. The core's own focus/UI path still runs below.
        let backend = store
            .data()
            .backend
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(attempt) = &backend.bud_attempt {
            if attempt.hid_input_enabled()
                && attempt.send(&crate::control::header(24, 0, 0, 0)).is_err()
            {
                // Failed cleanup ends the connection instead of keeping a held button.
                attempt.fail();
            }
        }
    }
    #[cfg(feature = "bud")]
    if let Event::RelativeMode(active) = &event {
        let backend = store
            .data()
            .backend
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(attempt) = &backend.bud_attempt {
            // Adapter-only mode control follows actual native capture, before
            // subsequent motion/buttons. Never sent on WebRTC or to the host.
            if attempt.hid_mouse_enabled()
                && attempt
                    .send(&crate::control::header(3, 3, i32::from(*active), 0))
                    .is_err()
            {
                attempt.fail();
            }
        }
    }
    let (name, values, count) = match event {
        #[cfg(feature = "native-core")]
        Event::NativeSession(_) => return Ok(()), // Native UI commands never enter guest callbacks.
        Event::CursorReady => return Ok(()),
        Event::Clipboard => {
            if let Some(window) = &store.data().window {
                window.take_clipboard_event();
            }
            sync_clipboard(store, instance, app);
            return Ok(());
        }
        #[cfg(feature = "bud")]
        Event::HidInputRelease => return Ok(()),
        Event::Fullscreen(active) => (
            "mty_window_update_fullscreen",
            [app, active as i32, 0, 0, 0],
            2,
        ),
        Event::Size(w, h) => {
            export(
                store,
                instance,
                "mty_window_update_size",
                &[
                    Val::I32(app),
                    Val::F64((w as f64).to_bits()),
                    Val::F64((h as f64).to_bits()),
                ],
            )?;
            ("mty_window_size", [app, 0, 0, 0, 0], 1)
        }
        Event::Focus(f) => {
            export(
                store,
                instance,
                "mty_window_update_focus",
                &[Val::I32(app), Val::I32(f as i32)],
            )?;
            ("mty_window_focus", [app, f as i32, 0, 0, 0], 2)
        }
        Event::RelativeMode(active) => (
            "mty_window_update_relative_mouse",
            [app, active as i32, 0, 0, 0],
            2,
        ),
        Event::RelativeMotion(x, y) => ("mty_window_motion", [app, 1, x, y, 0], 4),
        Event::Motion(x, y) => ("mty_window_motion", [app, 0, x, y, 0], 4),
        Event::Button(down, button, x, y) => {
            ("mty_window_button", [app, down as i32, button, x, y], 5)
        }
        Event::Scroll(x, y) => ("mty_window_scroll", [app, x, y, 0, 0], 3),
        Event::Key(down, code, mods) => {
            let Some(key) = store.data().key_codes.get(code).copied() else {
                return Ok(());
            };
            ("mty_window_keyboard", [app, down as i32, key, 0, mods], 5)
        }
        Event::Text(code) => {
            let Some(ch) = char::from_u32(code) else {
                return Ok(());
            };
            if ch.is_control() {
                return Ok(());
            }
            let mut bytes = [0; 4];
            ch.encode_utf8(&mut bytes);
            (
                "mty_window_keyboard",
                [app, 1, 0, u32::from_le_bytes(bytes) as i32, 0],
                5,
            )
        }
    };
    let arguments = values.map(Val::I32);
    export(store, instance, name, &arguments[..count])
}

fn sync_clipboard(store: &mut Store<HostState>, instance: &Instance, app: i32) {
    if !store
        .data()
        .window
        .as_ref()
        .is_some_and(|w| w.clipboard_allowed())
        || store
            .data()
            .backend
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .status
            != Some(0)
    {
        return;
    }
    if dispatch_clipboard_event(store, instance, app as u32).is_err() {
        crate::event_log::record(
            "WARN",
            "clipboard_notification_failed",
            serde_json::json!({}),
        );
    }
}

/// Native Matoya MTY_EVENT_CLIPBOARD (16), using the immutable 150-104a
/// window-callback ABI, just as its exported mty_window_* shims do. No core
/// memory structures or code are patched. The core checks client_clipboard
/// and sends user-data ID 7 through the selected existing transport.
pub(crate) fn dispatch_clipboard_event(
    store: &mut Store<HostState>,
    instance: &Instance,
    app: u32,
) -> Result<()> {
    let memory = store.data().memory.clone();
    let at = |offset| {
        app.checked_add(offset)
            .context("clipboard app address overflow")
    };
    let index = memory.u32(at(40)?)?;
    let opaque = memory.u32(at(884)?)?;
    let table = instance
        .get_table(&mut *store, "__indirect_function_table")
        .context("clipboard callback table missing")?;
    let callback = match table.get(&mut *store, u64::from(index)) {
        Some(wasmtime::Ref::Func(Some(f))) => f.typed::<(i32, i32), ()>(&*store)?,
        _ => bail!("clipboard callback unavailable"),
    };
    let alloc = instance.get_typed_func::<(i32, i32), i32>(&mut *store, "mty_system_alloc")?;
    let free = instance.get_typed_func::<i32, ()>(&mut *store, "mty_system_free")?;
    let event = alloc.call(&mut *store, (216, 1))?;
    if event == 0 {
        bail!("clipboard event allocation failed");
    }
    let mut bytes = [0; 216];
    bytes[..4].copy_from_slice(&16u32.to_le_bytes());
    let outcome = memory.write(event as u32, &bytes).and_then(|()| {
        let old = std::mem::replace(&mut store.data_mut().clipboard_notification, true);
        let result = callback.call(&mut *store, (event, opaque as i32));
        store.data_mut().clipboard_notification = old;
        result
    });
    let released = free.call(&mut *store, event);
    outcome.and(released)
}

fn run_loop(store: &mut Store<HostState>, instance: &Instance, input: bool) -> Result<()> {
    let window = store
        .data()
        .window
        .clone()
        .context("native window missing")?;
    let (index, opaque) = store.data().event_loop.context("event loop missing")?;
    let table = instance
        .get_table(&mut *store, "__indirect_function_table")
        .context("callback table missing")?;
    let callback = match table.get(&mut *store, index as u64) {
        Some(wasmtime::Ref::Func(Some(f))) => f.typed::<i32, i32>(&*store)?,
        _ => bail!("invalid event-loop callback"),
    };
    if input {
        export(store, instance, "mty_app_set_keys", &[])?;
        if let Some(app) = store.data().app_pointer {
            window.bind_core_hotkeys(crate::core_hotkeys::Binding {
                memory: store.data().memory.clone(),
                app,
                keys: store.data().key_codes.clone(),
            });
        }
    }
    let until = Instant::now() + Duration::from_secs(window.run_seconds);
    #[cfg(any(test, feature = "diagnostics"))]
    let mut script_frame = 0;
    let mut first = true;
    let frame_period = Duration::from_nanos(1_000_000_000 / 60);
    let mut cadence = CallbackCadence {
        next: Instant::now(),
        period: frame_period,
    };
    let mut input_batch = std::collections::VecDeque::new();
    while !window.closing.load(Ordering::Acquire) && (window.live || Instant::now() < until) {
        let frame_started = Instant::now();
        // The first render callback decompresses/rasterizes the embedded font
        // atlas. Give that one-time initialization its own bounded budget;
        // subsequent iterations retain the smaller per-frame allowance.
        store.set_fuel(if first && !input {
            500_000_000
        } else {
            50_000_000
        })?;
        let mut cursor_ready = false;
        if input {
            #[cfg(any(test, feature = "diagnostics"))]
            if window.synthetic_login {
                // Isolated offline test of the pinned UI layout, not login
                // automation for real accounts. Wait for rendered widgets and
                // separate press/release/text stages by actual presentations.
                let frames = window
                    .graphics
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_ref()
                    .map_or(0, |g| g.frames_presented);
                let step = window.script_steps.load(Ordering::Acquire);
                if frames >= 2 && frames > script_frame && step < 9 {
                    let (width, height) =
                        *window.dimensions.lock().unwrap_or_else(|e| e.into_inner());
                    let x = width / 2;
                    let email_y = height / 2 - 34;
                    let login_y = height / 2 + 138;
                    let events = &window.events;
                    match step {
                        0 => {
                            events.push(Event::Focus(true));
                            events.push(Event::Motion(x, email_y));
                            events.push(Event::Button(true, 0, x, email_y));
                        }
                        1 => events.push(Event::Button(false, 0, x, email_y)),
                        2 => {
                            for c in "native-audit@example.invalid".chars() {
                                events.push(Event::Text(c as u32));
                            }
                        }
                        3 => events.push(Event::Key(true, "Tab", 0)),
                        4 => events.push(Event::Key(false, "Tab", 0)),
                        5 => {
                            events.push(Event::Key(true, "ControlLeft", 2));
                            events.push(Event::Key(true, "KeyV", 2));
                        }
                        6 => {
                            events.push(Event::Key(false, "KeyV", 2));
                            events.push(Event::Key(false, "ControlLeft", 0));
                        }
                        7 => {
                            events.push(Event::Motion(x, login_y));
                            events.push(Event::Button(true, 0, x, login_y));
                        }
                        8 => events.push(Event::Button(false, 0, x, login_y)),
                        _ => unreachable!(),
                    }
                    script_frame = frames;
                    window.script_steps.store(step + 1, Ordering::Release);
                }
            }
            let app = store.data().app_pointer.context("app pointer missing")? as i32;
            window.events.take_batch(&mut input_batch);
            for event in input_batch.drain(..) {
                cursor_ready |= matches!(&event, Event::CursorReady);
                dispatch_input_event(store, instance, app, event)?;
            }
        }
        // Input exports can send synchronously, without waiting for the next
        // regular callback. Ordinary input never accelerates UI ticks. A host
        // cursor capture transition may bring one callback forward; cadence
        // then starts from that callback, avoiding a redundant follow-up tick.
        // The separate graphics worker and all video presentation are unchanged.
        if !input || cadence.due(Instant::now(), cursor_ready) {
            #[cfg(any(test, feature = "diagnostics"))]
            {
                store.data_mut().execution_stage = Some("event-loop-callback");
            }
            if callback.call(&mut *store, opaque as i32)? == 0 {
                break;
            }
            if input {
                let state = store.data().app_pointer.and_then(|app| {
                    crate::core_input::effective_state(&store.data().memory, app).ok()
                });
                let state = state.unwrap_or(crate::core_input::State {
                    detached: true,
                    ..Default::default()
                });
                window.set_input_state(state);
            }
            if first {
                crate::performance::record_memory(
                    if input {
                        "first_input_callback"
                    } else {
                        "first_ui_callback"
                    },
                    Some(store.data().memory.0.data_size()),
                );
            }
            first = false;
        }
        // SwapBuffers can already wait for vblank. Do not add a full second
        // frame delay on top of that; idle callbacks still avoid busy polling.
        if input {
            let deadline = if window.live {
                cadence.next
            } else {
                cadence.next.min(until)
            };
            window.events.wait_until(deadline);
        } else {
            let remaining = frame_period.saturating_sub(frame_started.elapsed());
            if !remaining.is_zero() {
                std::thread::sleep(remaining);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stack_input_arguments_preserve_the_guest_abi_and_signed_motion() {
        let mut config = wasmtime::Config::new();
        config
            .wasm_threads(true)
            .consume_fuel(true)
            .epoch_interruption(true);
        let engine = wasmtime::Engine::new(&config).unwrap();
        let mut wat = String::from("(module (import \"env\" \"memory\" (memory 1 1 shared))");
        for (name, count) in [
            ("mty_window_motion", 4),
            ("mty_window_button", 5),
            ("mty_window_scroll", 3),
            ("mty_window_keyboard", 5),
            ("mty_window_update_fullscreen", 2),
            ("mty_window_update_relative_mouse", 2),
        ] {
            wat.push_str(&format!(
                "(func (export \"{name}\") (param {} )",
                "i32 ".repeat(count)
            ));
            for argument in 0..count {
                wat.push_str(&format!(
                    " i32.const {} local.get {argument} i32.store",
                    argument * 4
                ));
            }
            wat.push_str(&format!(" i32.const 32 i32.const {count} i32.store)"));
        }
        wat.push(')');
        let module = wasmtime::Module::new(&engine, wat).unwrap();
        let (mut store, instance) = crate::instantiate(&engine, &module).unwrap();
        store.data_mut().key_codes.insert("KeyA".into(), 99);
        for (event, expected) in [
            (Event::RelativeMotion(-7, 13), vec![42, 1, -7, 13]),
            (Event::Motion(123, 456), vec![42, 0, 123, 456]),
            (Event::Button(false, 2, 17, 19), vec![42, 0, 2, 17, 19]),
            (Event::Scroll(-1, 3), vec![42, -1, 3]),
            (Event::Key(true, "KeyA", 5), vec![42, 1, 99, 0, 5]),
            (Event::Text('a' as u32), vec![42, 1, 0, 97, 0]),
            (Event::Fullscreen(true), vec![42, 1]),
            (Event::RelativeMode(false), vec![42, 0]),
        ] {
            let memory = store.data().memory.clone();
            memory.write(0, &[0xff; 36]).unwrap();
            dispatch_input_event(&mut store, &instance, 42, event).unwrap();
            assert_eq!(memory.u32(32).unwrap() as usize, expected.len());
            for (index, expected) in expected.into_iter().enumerate() {
                assert_eq!(memory.u32((index * 4) as u32).unwrap() as i32, expected);
            }
        }
        let memory = store.data().memory.clone();
        memory.write(0, &[0xff; 36]).unwrap();
        dispatch_input_event(&mut store, &instance, 42, Event::CursorReady).unwrap();
        assert_eq!(memory.read(0, 36).unwrap(), vec![0xff; 36]);
    }

    #[test]
    fn input_wakeups_do_not_accelerate_regular_callbacks_or_trigger_catchup() {
        let start = Instant::now();
        let period = Duration::from_nanos(1_000_000_000 / 60);
        let mut cadence = CallbackCadence {
            next: start,
            period,
        };
        assert!(cadence.due(start, false));
        for _ in 0..60 {
            let deadline = cadence.next;
            for us in 1..1000 {
                assert!(!cadence.due(deadline - period + Duration::from_micros(us), false));
                assert_eq!(cadence.next, deadline);
            }
            assert!(cadence.due(deadline, false));
        }
        let late = cadence.next + Duration::from_secs(2);
        assert!(cadence.due(late, false));
        assert_eq!(cadence.next, late + period);
        assert!(!cadence.due(late, false));
    }

    #[test]
    fn host_cursor_wakeup_brings_callback_forward_without_a_followup_tick() {
        let start = Instant::now();
        let period = Duration::from_nanos(1_000_000_000 / 60);
        let mut cadence = CallbackCadence {
            next: start,
            period,
        };
        assert!(cadence.due(start, false));
        let original_deadline = cadence.next;
        let arrival = start + Duration::from_millis(2);
        assert!(!cadence.due(arrival, false));
        assert!(cadence.due(arrival, true));
        assert!(!cadence.due(original_deadline, false));
        assert!(!cadence.due(arrival, false));
        assert!(cadence.due(arrival + period, false));
    }
}
