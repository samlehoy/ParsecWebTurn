//! Normal single-executable entry point. Diagnostic CLI is a separate build feature.
use crate::*;
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
    let root = if args.is_empty() {
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
    #[cfg(feature = "hid-input-trace")]
    let _hid_trace = hid_input_trace::start(root.join("hid-drag-trace.json"));
    let connection_settings = connection_settings::Manager::open(root.join("settings.json"));
    if event_log::initialize(
        root.join("client-events.jsonl"),
        connection_settings.view().0.event_logging,
    )
    .is_err()
    {
        let _ = event_log::initialize(root.join("client-events.jsonl"), false);
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
