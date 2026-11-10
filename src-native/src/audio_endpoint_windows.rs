//! Default-render-endpoint notifications. Callbacks only signal the audio worker.
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use windows::{
    core::PCWSTR,
    Win32::{Foundation::*, Media::Audio::*, System::Threading::*},
};

pub struct Event(pub HANDLE);
// SAFETY: a process-owned kernel event has no thread affinity. Shared owners
// retain it until all worker/callback references are gone; it is closed once.
unsafe impl Send for Event {}
unsafe impl Sync for Event {}
impl Event {
    pub fn new() -> windows::core::Result<Self> {
        unsafe { Ok(Self(CreateEventW(None, false, false, PCWSTR::null())?)) }
    }
    pub fn wake(&self) {
        unsafe {
            let _ = SetEvent(self.0);
        }
    }
}
impl Drop for Event {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
pub struct Signal {
    pub event: Event,
    generation: AtomicU64,
}
impl Signal {
    pub fn new() -> windows::core::Result<Arc<Self>> {
        Ok(Arc::new(Self {
            event: Event::new()?,
            generation: AtomicU64::new(0),
        }))
    }
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }
}
#[windows::core::implement(IMMNotificationClient)]
struct DefaultOutput {
    signal: Arc<Signal>,
}
impl IMMNotificationClient_Impl for DefaultOutput_Impl {
    fn OnDefaultDeviceChanged(
        &self,
        flow: EDataFlow,
        role: ERole,
        _: &PCWSTR,
    ) -> windows::core::Result<()> {
        if flow == eRender && role == eConsole {
            self.signal.generation.fetch_add(1, Ordering::AcqRel);
            self.signal.event.wake();
        }
        Ok(())
    }
    fn OnDeviceAdded(&self, _: &PCWSTR) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnDeviceRemoved(&self, _: &PCWSTR) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnPropertyValueChanged(&self, _: &PCWSTR, _: &PROPERTYKEY) -> windows::core::Result<()> {
        Ok(())
    }
}
pub struct Registration {
    enumerator: IMMDeviceEnumerator,
    callback: IMMNotificationClient,
}
impl Registration {
    pub fn new(
        enumerator: &IMMDeviceEnumerator,
        signal: Arc<Signal>,
    ) -> windows::core::Result<Self> {
        let callback: IMMNotificationClient = DefaultOutput { signal }.into();
        unsafe {
            enumerator.RegisterEndpointNotificationCallback(&callback)?;
        }
        Ok(Self {
            enumerator: enumerator.clone(),
            callback,
        })
    }
}
impl Drop for Registration {
    fn drop(&mut self) {
        // Never unregister or release the endpoint enumerator from a callback.
        unsafe {
            let _ = self
                .enumerator
                .UnregisterEndpointNotificationCallback(&self.callback);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callbacks_only_wake_for_the_selected_render_role_and_keep_event_alive() {
        let signal = Signal::new().unwrap();
        let callback: IMMNotificationClient = DefaultOutput {
            signal: signal.clone(),
        }
        .into();
        unsafe {
            callback
                .OnDefaultDeviceChanged(eCapture, eConsole, PCWSTR::null())
                .unwrap();
            callback
                .OnDefaultDeviceChanged(eRender, eCommunications, PCWSTR::null())
                .unwrap();
            assert_eq!(signal.generation(), 0);
            assert_eq!(WaitForSingleObject(signal.event.0, 0), WAIT_TIMEOUT);
            callback
                .OnDefaultDeviceChanged(eRender, eConsole, PCWSTR::null())
                .unwrap();
            assert_eq!(signal.generation(), 1);
            assert_eq!(WaitForSingleObject(signal.event.0, 0), WAIT_OBJECT_0);
            assert_eq!(WaitForSingleObject(signal.event.0, 0), WAIT_TIMEOUT);
            drop(callback);
            signal.event.wake();
            assert_eq!(WaitForSingleObject(signal.event.0, 0), WAIT_OBJECT_0);
        }
    }
}
