//! Normal single-executable entry point. Diagnostic CLI is a separate build feature.
use crate::*;

#[cfg(windows)]
pub fn main() {
    let result = std::thread::Builder::new()
        .name("parsec-runtime".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(run)
        .map_err(anyhow::Error::from)
        .and_then(|thread| {
            thread
                .join()
                .map_err(|_| anyhow::anyhow!("Native runtime stopped unexpectedly"))
        })
        .and_then(|result| result);
    if result.is_err() {
        // Never display guest errors, URLs, tokens or profile contents.
        let text:Vec<u16> = "ParsecWebTurn could not start or finish safely. Your saved profile was preserved. Check that another instance is not running and that the profile directory is accessible.\0".encode_utf16().collect();
        let title: Vec<u16> = "ParsecWebTurn\0".encode_utf16().collect();
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                std::ptr::null_mut(),
                text.as_ptr(),
                title.as_ptr(),
                windows_sys::Win32::UI::WindowsAndMessaging::MB_OK
                    | windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
            );
        }
        std::process::exit(1);
    }
}

#[cfg(target_os = "macos")]
pub fn main() {
    let mtm = objc2_foundation::MainThreadMarker::new().expect("Must run on main thread");
    let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(objc2_app_kit::NSApplicationActivationPolicy::Regular);

    let (done_tx, done_rx) = std::sync::mpsc::sync_channel(1);
    if let Err(e) = std::thread::Builder::new()
        .name("parsec-runtime".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            let res = run();
            let _ = done_tx.send(res);
            // Wake the event loop below, which otherwise sleeps until the next UI event.
            dispatch2::DispatchQueue::main().exec_async(|| {
                let mtm = objc2_foundation::MainThreadMarker::new().expect("main queue");
                let wake = objc2_app_kit::NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
                    objc2_app_kit::NSEventType::ApplicationDefined,
                    objc2_foundation::NSPoint::ZERO,
                    objc2_app_kit::NSEventModifierFlags(0),
                    0.0,
                    0,
                    None,
                    0,
                    0,
                    0,
                );
                if let Some(wake) = wake {
                    objc2_app_kit::NSApplication::sharedApplication(mtm).postEvent_atStart(&wake, true);
                }
            });
        })
    {
        eprintln!("ParsecWebTurn error: {e:?}");
        std::process::exit(1);
    }

    app.finishLaunching();

    let result = loop {
        match done_rx.try_recv() {
            Ok(result) => break result,
            // The sender is dropped without a result only if run() panicked.
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                break Err(anyhow::anyhow!("Native runtime stopped unexpectedly"))
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
        objc2::rc::autoreleasepool(|_| {
            // Main-queue blocks (window, settings, title updates) still run while this waits.
            let timeout = objc2_foundation::NSDate::distantFuture();
            let event = unsafe {
                app.nextEventMatchingMask_untilDate_inMode_dequeue(
                    objc2_app_kit::NSEventMask(u64::MAX),
                    Some(&timeout),
                    objc2_foundation::NSDefaultRunLoopMode,
                    true,
                )
            };
            if let Some(event) = event {
                if !crate::window_macos::handle_cocoa_event(&event) {
                    app.sendEvent(&event);
                }
            }
        });
    };

    if let Err(e) = result {
        eprintln!("ParsecWebTurn error: {e:?}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let mut args: Vec<_> = std::env::args().skip(1).collect();
    let software_overlay = !args.iter().any(|arg| arg == "--d3d-overlay");
    args.retain(|arg| arg != "--software-overlay" && arg != "--d3d-overlay");
    if args == ["--version"] {
        println!(
            "ParsecWebTurn {} / Parsec WASM {}",
            env!("CARGO_PKG_VERSION"),
            PARSEC_CORE_VERSION
        );
        return Ok(());
    }
    let root = if args.is_empty() && cfg!(target_os = "macos") {
        // A signed .app bundle must not be modified and may run translocated
        // read-only, so macOS keeps settings beside the native profile.
        let root =
            std::path::PathBuf::from(std::env::var("HOME").context("Home directory unavailable")?)
                .join("Library/Application Support/ParsecWebTurn");
        std::fs::create_dir_all(&root)?;
        root
    } else if args.is_empty() {
        std::env::current_exe()?
            .parent()
            .context("Application directory unavailable")?
            .to_path_buf()
    } else if args.len() == 2 && args[0] == "--data-dir" {
        let root = std::path::PathBuf::from(&args[1]);
        std::fs::create_dir_all(&root)?;
        root
    } else {
        bail!("Unsupported argument");
    };
    let connection_settings = connection_settings::Manager::open(root.join("settings.json"));
    #[cfg(target_os = "macos")]
    crate::connection_settings_ui_macos::set_default_manager(connection_settings.clone());
    if event_log::initialize(
        root.join("client-events.jsonl"),
        connection_settings.view().0.event_logging,
    )
    .is_err()
    {
        let _ = event_log::initialize(root.join("client-events.jsonl"), false);
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                std::ptr::null_mut(),
                "Event logging could not be started. The client will continue without it. Check the data directory.\0".encode_utf16().collect::<Vec<_>>().as_ptr(),
                "ParsecWebTurn\0".encode_utf16().collect::<Vec<_>>().as_ptr(),
                windows_sys::Win32::UI::WindowsAndMessaging::MB_OK | windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONWARNING,
            );
        }
    }
    let _event_log = event_log::Guard;
    let profile = profile::Profile::user()?;
    let mut saved = profile.load()?;
    saved.profile = Some(profile.clone());
    performance::record_memory("before_core_load", None);
    let engine = Engine::new(&core_config::config())?;
    let module = core_runtime::load(&engine)?;
    performance::record_memory("core_loaded", None);
    let window = window::Window::create(false, true, true, false)?;
    window
        .software_overlay_enabled
        .store(software_overlay, Ordering::Release);
    window.install_connection_settings(connection_settings.clone());
    if std::env::var_os("PARSECWEBTURN_NO_UPDATE_CHECK").is_none() {
        window.updates.check(true);
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                window.handle(),
                updater_ui::START,
                0,
                0,
            );
        }
    }
    let (mut store, instance) = instantiate_mode(&engine, &module, Some(window.clone()))?;
    *store
        .data()
        .filesystem
        .lock()
        .map_err(|_| anyhow::anyhow!("Profile lock failed"))? = saved;
    {
        let mut backend = store
            .data()
            .backend
            .lock()
            .map_err(|_| anyhow::anyhow!("Backend lock failed"))?;
        // Compatibility with the pinned host's legacy identity certificate.
        // Signature and certificate-fingerprint validation remain mandatory.
        backend.legacy_rsa_1024_enabled = true;
        backend.stun_provider = attempt::StunProvider::Parsec;
        backend.connection_settings = Some(connection_settings);
    }
    let outcome = (|| -> Result<()> {
        performance::record_memory(
            "before_guest_start",
            Some(store.data().memory.0.data_size()),
        );
        #[cfg(test)]
        {
            store.data_mut().execution_stage = Some("guest-start");
        }
        let start = instance.get_typed_func::<(), ()>(&mut store, "_start")?;
        let result = start.call(&mut store, ());
        if store.data().event_loop.is_some() {
            desktop::run(&mut store, &instance)
        } else {
            result
        }
    })();
    if let Err(e) = &outcome {
        eprintln!("Guest execution error: {e:?}");
    }
    // Also notify the host when the user closes the application window.
    store
        .data()
        .backend
        .lock()
        .map_err(|_| anyhow::anyhow!("Backend lock failed"))?
        .destroy();
    // Persistence is also committed when guest files close. This final checkpoint
    // includes preferences still open at a normal application shutdown.
    let saved_result = store
        .data()
        .filesystem
        .lock()
        .map_err(|_| anyhow::anyhow!("Profile lock failed"))?
        .flush();
    window.request_stop();
    engine.increment_epoch();
    if let Some(runtime) = &store.data().threads {
        runtime.http.shutdown();
        runtime.websocket.shutdown();
    }
    let until = std::time::Instant::now() + Duration::from_secs(3);
    while window.active_contexts.load(Ordering::Acquire) != 0 && std::time::Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
    if window.active_contexts.load(Ordering::Acquire) == 0 {
        window.close();
    }
    if saved_result.is_err() {
        event_log::record(
            "ERROR",
            "profile_save_failed",
            serde_json::json!({"stage":"shutdown"}),
        );
    }
    saved_result?;
    match outcome {
        Err(e) if lifecycle::is_cancellation(&e) => Ok(()),
        other => other,
    }
}
