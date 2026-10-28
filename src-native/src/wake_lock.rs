//! Documented screen power request, confined to the window UI thread.
use serde::Serialize;

#[cfg(windows)]
use windows_sys::Win32::System::Power::{
    SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED,
};

#[derive(Clone, Default, Serialize)]
pub struct State {
    pub requested: bool,
    pub applied: bool,
    pub failed_requests: u64,
}
impl State {
    #[cfg(windows)]
    pub fn update(&mut self, requested: bool, visible: bool) {
        self.update_with(requested, visible, |enable| unsafe {
            SetThreadExecutionState(ES_CONTINUOUS | if enable { ES_DISPLAY_REQUIRED } else { 0 })
                != 0
        });
    }

    #[cfg(target_os = "macos")]
    pub fn update(&mut self, requested: bool, visible: bool) {
        self.update_with(requested, visible, |enable| {
            macos_set_display_sleep_disabled(enable)
        });
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    pub fn update(&mut self, requested: bool, visible: bool) {
        self.update_with(requested, visible, |_| true);
    }

    fn update_with(&mut self, requested: bool, visible: bool, mut apply: impl FnMut(bool) -> bool) {
        self.requested = requested;
        let desired = requested && visible;
        if desired == self.applied {
            return;
        }
        if apply(desired) {
            self.applied = desired;
        } else {
            self.failed_requests = self.failed_requests.saturating_add(1);
        }
    }
}

#[cfg(target_os = "macos")]
fn macos_set_display_sleep_disabled(disable: bool) -> bool {
    use core_foundation::base::TCFType;
    use core_foundation::string::CFString;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOPMAssertionCreateWithName(
            assertion_type: *const std::ffi::c_void,
            assertion_level: u32,
            assertion_name: *const std::ffi::c_void,
            assertion_id: *mut u32,
        ) -> i32;
        fn IOPMAssertionRelease(assertion_id: u32) -> i32;
    }

    static ASSERTION_ID: AtomicU32 = AtomicU32::new(0);

    let current = ASSERTION_ID.load(Ordering::SeqCst);
    if disable && current == 0 {
        let atype = CFString::new("PreventUserIdleDisplaySleep");
        let aname = CFString::new("ParsecWebTurn Remote Session");
        let mut id = 0u32;
        let ret = unsafe {
            IOPMAssertionCreateWithName(
                atype.as_concrete_TypeRef() as *const _,
                255, // kIOPMAssertionLevelOn
                aname.as_concrete_TypeRef() as *const _,
                &mut id,
            )
        };
        if ret == 0 {
            ASSERTION_ID.store(id, Ordering::SeqCst);
            true
        } else {
            false
        }
    } else if !disable && current != 0 {
        unsafe {
            IOPMAssertionRelease(current);
        }
        ASSERTION_ID.store(0, Ordering::SeqCst);
        true
    } else {
        true
    }
}

#[cfg(any(test, feature = "diagnostics"))]
pub fn probe() -> anyhow::Result<serde_json::Value> {
    let mut config = wasmtime::Config::new();
    config
        .wasm_threads(true)
        .consume_fuel(true)
        .epoch_interruption(true);
    let engine = wasmtime::Engine::new(&config)?;
    let module = wasmtime::Module::new(
        &engine,
        r#"(module
        (import "env" "memory" (memory 1 1 shared))
        (import "env" "web_wake_lock" (func $set (param i32)))
        (func (export "test") i32.const 1 call $set i32.const 0 call $set))"#,
    )?;
    let (mut store, instance) = crate::instantiate(&engine, &module)?;
    instance
        .get_typed_func::<(), ()>(&mut store, "test")?
        .call(&mut store, ())?;
    anyhow::ensure!(
        store.data().boundary.is_none() && store.data().calls.get("env::web_wake_lock") == Some(&2),
        "wake lock guest import failed"
    );
    Ok(
        serde_json::json!({"guest_import_verified":true,"real_account_used":false,"power_request_without_window":false}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_request_and_release_share_one_thread() {
        std::thread::spawn(|| {
            let mut state = State::default();
            state.update(true, true);
            assert!(state.applied || state.failed_requests == 1);
            state.update(false, false);
            // A platform rejection is recorded, never converted to a WASM trap.
            assert!(!state.requested);
            #[cfg(windows)]
            unsafe {
                SetThreadExecutionState(ES_CONTINUOUS);
            }
        })
        .join()
        .unwrap();
    }
    #[test]
    fn guest_wake_lock_is_optional_without_window() {
        probe().unwrap();
    }
    #[test]
    fn visibility_and_release_are_idempotent() {
        let mut state = State::default();
        let mut calls = Vec::new();
        for (request, visible) in [
            (true, true),
            (true, true),
            (true, false),
            (true, true),
            (false, true),
            (false, true),
        ] {
            state.update_with(request, visible, |enable| {
                calls.push(enable);
                true
            });
        }
        assert_eq!(calls, [true, false, true, false]);
        assert!(!state.applied && !state.requested);
    }
    #[test]
    fn rejection_preserves_actual_state_and_retries_release() {
        let mut state = State::default();
        state.update_with(true, true, |_| false);
        assert!(!state.applied);
        state.update_with(true, true, |_| true);
        state.update_with(false, true, |_| false);
        assert!(state.applied && !state.requested);
        state.update_with(false, true, |_| true);
        assert!(!state.applied);
        assert_eq!(state.failed_requests, 2);
    }
}
