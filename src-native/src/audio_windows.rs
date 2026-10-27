//! Shared-mode, event-driven WASAPI PCM output. COM objects stay on the worker.
use crate::audio_endpoint_windows::{Event, Registration, Signal};
use crate::audio_layout::Layout;
use crate::audio_recovery::Recovery;
use anyhow::{bail, Result};
use serde::Serialize;
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::JoinHandle,
};
use windows::Win32::{
    Foundation::*,
    Media::Audio::*,
    System::{Com::*, Threading::*},
};
#[derive(Clone, Default, Serialize)]
pub struct Snapshot {
    pub layout: Option<Layout>,
    pub minimum_ms: u32,
    pub maximum_ms: u32,
    pub custom_buffer: bool,
    pub low_latency_device: bool,
    pub device_capacity_frames: u32,
    pub device_opened: bool,
    pub frames_queued: u64,
    pub frames_written: u64,
    pub frames_dropped: u64,
    pub underruns: u64,
    pub queued_frames: usize,
    pub device_padding: u32,
    pub failure_hresult: Option<String>,
    pub resources_released: bool,
    pub recovering: bool,
    pub recovery_attempts: u64,
    pub recoveries: u64,
    pub last_device_error: Option<String>,
}
#[derive(Default)]
struct State {
    report: Snapshot,
    pcm: VecDeque<i16>,
    stopped: bool,
    reset: bool,
}
pub struct Playback {
    pub layout: Layout,
    shared: Arc<Mutex<State>>,
    worker: Mutex<Option<JoinHandle<()>>>,
    max_frames: usize,
    custom_buffer: bool,
    signal: Arc<Signal>,
    flush_source: Arc<AtomicBool>,
}
impl Playback {
    pub fn create_layout(
        window: Arc<crate::window::Window>,
        min_buffer: u32,
        max_buffer: u32,
        layout: Layout,
    ) -> Result<Self> {
        let custom = window.audio_buffer();
        let (min_buffer, max_buffer) = custom.unwrap_or((min_buffer, max_buffer));
        if min_buffer > max_buffer || max_buffer == 0 || max_buffer > 500 {
            bail!("invalid audio latency limits");
        }
        // Pinned AudioWorklet converts its latency arguments from milliseconds to frames.
        let max_frames = max_buffer as usize * 48;
        let min_frames = min_buffer as usize * 48;
        let shared = Arc::new(Mutex::new(State {
            report: Snapshot {
                layout: Some(layout),
                minimum_ms: min_buffer,
                maximum_ms: max_buffer,
                custom_buffer: custom.is_some(),
                ..Default::default()
            },
            ..Default::default()
        }));
        crate::event_log::record(
            "INFO",
            "audio_output_configured",
            serde_json::json!({
                "minimum_ms":min_buffer,"maximum_ms":max_buffer,"custom":custom.is_some(),
                "channels":layout.channels,"channel_mask":layout.mask
            }),
        );
        let state = shared.clone();
        let signal = Signal::new()?;
        let worker_signal = signal.clone();
        let flush_source = Arc::new(AtomicBool::new(false));
        let worker_flush = flush_source.clone();
        let (tx, rx) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            let result = unsafe {
                run(
                    &state,
                    &window,
                    min_frames,
                    &tx,
                    layout,
                    worker_signal,
                    worker_flush,
                )
            };
            if let Err(error) = result {
                state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .report
                    .failure_hresult = Some(format!("0x{:08X}", error.code().0 as u32));
                crate::event_log::record(
                    "ERROR",
                    "audio_output_failed",
                    serde_json::json!({"hresult": format!("0x{:08X}", error.code().0 as u32)}),
                );
                let _ = tx.try_send(false);
            }
            let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
            state.report.resources_released = true;
            state.report.device_opened = false;
            state.report.recovering = false;
        });
        let output = Self {
            layout,
            shared,
            worker: Mutex::new(Some(worker)),
            max_frames,
            custom_buffer: custom.is_some(),
            signal,
            flush_source,
        };
        if rx.recv_timeout(std::time::Duration::from_secs(3)).ok() != Some(true) {
            output.stop();
            bail!("WASAPI output unavailable");
        }
        Ok(output)
    }
    pub fn queue(&self, pcm: &[i16]) {
        let mut s = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        if !pcm.len().is_multiple_of(usize::from(self.layout.channels)) {
            return;
        }
        let channels = usize::from(self.layout.channels);
        let frames = pcm.len() / channels;
        if s.stopped
            || s.report.recovering
            || self.flush_source.load(Ordering::Acquire)
            || s.report.failure_hresult.is_some()
            || (!self.custom_buffer && s.pcm.len() / channels + frames > self.max_frames)
        {
            s.report.frames_dropped += frames as u64;
            return;
        }
        if self.custom_buffer {
            // Keep the newest complete stereo frames within the combined budget.
            let room = self
                .max_frames
                .saturating_sub(s.report.device_padding as usize);
            let excess = (s.pcm.len() / channels + frames).saturating_sub(room);
            let old = excess.min(s.pcm.len() / channels);
            s.pcm.drain(..old * channels);
            let incoming = excess.saturating_sub(old).min(frames);
            s.report.frames_dropped += (old + incoming) as u64;
            s.pcm.extend(&pcm[incoming * channels..]);
        } else {
            s.pcm.extend(pcm);
        }
        s.report.frames_queued += frames as u64;
        s.report.queued_frames = s.pcm.len() / channels;
    }
    #[cfg(test)]
    pub fn queued_frames(&self) -> u32 {
        let s = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        (s.pcm.len() / usize::from(self.layout.channels) + s.report.device_padding as usize) as u32
    }
    #[cfg(test)]
    pub fn reset(&self) {
        let mut s = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        s.pcm.clear();
        s.report.queued_frames = 0;
        s.reset = true;
    }
    /// A single consumer flushes its upstream PCM/packet generation before
    /// accepting audio on a restored endpoint. Ordinary playback is one read.
    pub fn take_recovery_flush(&self) -> bool {
        self.flush_source.load(Ordering::Acquire) && self.flush_source.swap(false, Ordering::AcqRel)
    }
    pub fn snapshot(&self) -> Snapshot {
        self.shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .report
            .clone()
    }
    pub fn stop(&self) {
        self.shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stopped = true;
        self.signal.event.wake();
        if let Some(worker) = self.worker.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = worker.join();
        }
    }
}
impl Drop for Playback {
    fn drop(&mut self) {
        self.stop();
    }
}
struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}
struct Started(IAudioClient);
impl Drop for Started {
    fn drop(&mut self) {
        unsafe {
            let _ = self.0.Stop();
            let _ = self.0.Reset();
        }
    }
}

fn wave_format(layout: Layout) -> WAVEFORMATEXTENSIBLE {
    WAVEFORMATEXTENSIBLE {
        Format: WAVEFORMATEX {
            wFormatTag: 0xfffe,
            nChannels: u16::from(layout.channels),
            nSamplesPerSec: 48_000,
            nAvgBytesPerSec: 48_000 * u32::from(layout.channels) * 2,
            nBlockAlign: u16::from(layout.channels) * 2,
            wBitsPerSample: 16,
            cbSize: 22,
        },
        Samples: WAVEFORMATEXTENSIBLE_0 {
            wValidBitsPerSample: 16,
        },
        dwChannelMask: layout.mask,
        SubFormat: windows::core::GUID::from_u128(0x00000001_0000_0010_8000_00aa00389b71),
    }
}

/// Read the default render endpoint, never infer surround from a device name.
/// Unsupported layouts or formats retain the existing stereo declaration.
pub fn device_layout() -> Result<Layout> {
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        let _com = Apartment;
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
        let mix = client.GetMixFormat()?;
        let header = mix.read_unaligned();
        let layout = if header.wFormatTag == 0xfffe && header.cbSize >= 22 {
            let mask = std::ptr::addr_of!((*mix.cast::<WAVEFORMATEXTENSIBLE>()).dwChannelMask)
                .read_unaligned();
            Layout::new(u8::try_from(header.nChannels).unwrap_or(0), mask).ok()
        } else {
            None
        };
        CoTaskMemFree(Some(mix.cast()));
        let Some(layout) = layout.filter(|v| *v != Layout::STEREO) else {
            return Ok(Layout::STEREO);
        };
        let format = wave_format(layout);
        // Validate the real output's shared-mode conversion. The endpoint mix
        // is often float, which need not exactly match our 16-bit PCM client.
        // This temporary client is never started and writes no audio.
        let supported = client
            .Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                200_000,
                0,
                &format.Format,
                None,
            )
            .is_ok();
        Ok(if supported { layout } else { Layout::STEREO })
    }
}
enum DeviceExit {
    Stopped,
    DefaultChanged,
}
struct Devices {
    enumerator: IMMDeviceEnumerator,
    signal: Arc<Signal>,
    flush_source: Arc<AtomicBool>,
}
fn clear_pcm(state: &mut State, channels: usize) {
    state.report.frames_dropped += (state.pcm.len() / channels) as u64;
    state.pcm.clear();
    state.report.queued_frames = 0;
    state.report.device_padding = 0;
}
fn mark_recovering(state: &mut State, channels: usize, error: Option<windows::core::HRESULT>) {
    clear_pcm(state, channels);
    state.report.recovering = true;
    state.report.device_opened = false;
    state.report.device_capacity_frames = 0;
    if let Some(code) = error {
        state.report.last_device_error = Some(format!("0x{:08X}", code.0 as u32));
    }
}
fn mark_open(state: &mut State, channels: usize, restored: bool, flush: &AtomicBool) {
    if restored {
        clear_pcm(state, channels);
        flush.store(true, Ordering::Release);
        state.report.recoveries = state.report.recoveries.saturating_add(1);
    }
    state.report.device_opened = true;
    state.report.recovering = false;
}
fn recoverable(error: windows::core::HRESULT) -> bool {
    matches!(
        error,
        AUDCLNT_E_DEVICE_INVALIDATED
            | AUDCLNT_E_RESOURCES_INVALIDATED
            | AUDCLNT_E_SERVICE_NOT_RUNNING
            | AUDCLNT_E_ENDPOINT_CREATE_FAILED
            | AUDCLNT_E_DEVICE_IN_USE
            | AUDCLNT_E_BUFFER_ERROR
    ) || error == windows::core::HRESULT::from_win32(ERROR_NOT_FOUND.0)
}
unsafe fn run(
    shared: &Mutex<State>,
    window: &crate::window::Window,
    min_frames: usize,
    ready: &mpsc::SyncSender<bool>,
    layout: Layout,
    signal: Arc<Signal>,
    flush_source: Arc<AtomicBool>,
) -> windows::core::Result<()> {
    CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
    let _com = Apartment;
    let _scheduling = crate::media_scheduling::Registration::audio();
    let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
    let _notifications = match Registration::new(&enumerator, signal.clone()) {
        Ok(value) => Some(value),
        Err(error) => {
            crate::event_log::record(
                "WARN",
                "audio_endpoint_notifications_unavailable",
                serde_json::json!({"hresult":format!("0x{:08X}", error.code().0 as u32)}),
            );
            None // Keep invalid-device recovery even if notifications are unavailable.
        }
    };
    let devices = Devices {
        enumerator,
        signal,
        flush_source,
    };
    let mut recovery = Recovery::default();
    let mut last_error = None;
    loop {
        if window.closing.load(Ordering::Acquire)
            || shared.lock().unwrap_or_else(|e| e.into_inner()).stopped
        {
            return Ok(());
        }
        let generation = devices.signal.generation();
        if recovery.recovering() {
            let mut state = shared.lock().unwrap_or_else(|e| e.into_inner());
            state.report.recovery_attempts = state.report.recovery_attempts.saturating_add(1);
        }
        let error = match render(
            shared,
            window,
            min_frames,
            ready,
            layout,
            &devices,
            &mut recovery,
        ) {
            Ok(DeviceExit::Stopped) => return Ok(()),
            Ok(DeviceExit::DefaultChanged) => None,
            Err(error)
                if recoverable(error.code())
                    || (recovery.recovering() && error.code() == AUDCLNT_E_UNSUPPORTED_FORMAT) =>
            {
                Some(error.code())
            }
            Err(error) => return Err(error),
        };
        let first = recovery.begin();
        {
            let mut state = shared.lock().unwrap_or_else(|e| e.into_inner());
            mark_recovering(&mut state, usize::from(layout.channels), error);
        }
        if first || (error.is_some() && error != last_error) {
            crate::event_log::record(
                if error.is_some() { "WARN" } else { "INFO" },
                "audio_output_recovery_started",
                serde_json::json!({"reason":if error.is_some(){"endpoint_error"}else{"default_device_changed"},"hresult":error.map(|e|format!("0x{:08X}",e.0 as u32))}),
            );
        }
        last_error = error;
        // No endpoint at startup is an audio outage, not a reason to stop video.
        let _ = ready.try_send(true);
        if error.is_none() || devices.signal.generation() != generation {
            recovery.endpoint_changed();
            continue;
        }
        let delay = recovery.failed();
        if WaitForSingleObject(devices.signal.event.0, delay.as_millis() as u32) == WAIT_FAILED {
            return Err(windows::core::Error::from_thread());
        }
        if devices.signal.generation() != generation {
            recovery.endpoint_changed();
        }
    }
}

unsafe fn render(
    shared: &Mutex<State>,
    window: &crate::window::Window,
    min_frames: usize,
    ready: &mpsc::SyncSender<bool>,
    layout: Layout,
    devices: &Devices,
    recovery: &mut Recovery,
) -> windows::core::Result<DeviceExit> {
    let generation = devices.signal.generation();
    let device = devices
        .enumerator
        .GetDefaultAudioEndpoint(eRender, eConsole)?;
    let mut client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
    let stereo = WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_PCM as u16,
        nChannels: 2,
        nSamplesPerSec: 48_000,
        nAvgBytesPerSec: 192_000,
        nBlockAlign: 4,
        wBitsPerSample: 16,
        cbSize: 0,
    };
    let extended = wave_format(layout);
    let format = if layout == Layout::STEREO {
        &stereo
    } else {
        &extended.Format
    };
    let channels = usize::from(layout.channels);
    let custom = shared
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .report
        .custom_buffer;
    let fast = if custom {
        (|| -> windows::core::Result<bool> {
            use windows::core::Interface;
            let modern: IAudioClient3 = client.cast()?;
            let (mut default, mut fundamental, mut minimum, mut maximum) = (0, 0, 0, 0);
            modern.GetSharedModeEnginePeriod(
                format,
                &mut default,
                &mut fundamental,
                &mut minimum,
                &mut maximum,
            )?;
            modern.InitializeSharedAudioStream(
                AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                minimum,
                format,
                None,
            )?;
            Ok(true)
        })()
        .unwrap_or(false)
    } else {
        false
    };
    if !fast {
        if custom {
            client = device.Activate(CLSCTX_ALL, None)?;
        }
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            200_000,
            0,
            format,
            None,
        )?;
    }
    let event = Event::new()?;
    client.SetEventHandle(event.0)?;
    let render: IAudioRenderClient = client.GetService()?;
    let capacity = client.GetBufferSize()?;
    {
        let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
        s.report.low_latency_device = fast;
        s.report.device_capacity_frames = capacity;
    }
    // The unchanged automatic path primes silence. The custom-buffer path starts
    // only when real PCM is ready and never pads its queue with a silence reserve.
    if !custom {
        let _ = render.GetBuffer(capacity)?;
        render.ReleaseBuffer(capacity, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)?;
        client.Start()?;
    }
    let mut started = !custom;
    let _started = Started(client.clone());
    if devices.signal.generation() != generation {
        return Ok(DeviceExit::DefaultChanged);
    }
    let restored = recovery.restored();
    {
        let mut state = shared.lock().unwrap_or_else(|e| e.into_inner());
        mark_open(&mut state, channels, restored, &devices.flush_source);
    }
    if restored {
        crate::event_log::record(
            "INFO",
            "audio_output_recovered",
            serde_json::json!({"channels":layout.channels,"low_latency_device":fast}),
        );
    }
    let _ = ready.try_send(true);
    let mut buffering = true;
    loop {
        let wait = WaitForMultipleObjects(
            &[event.0, devices.signal.event.0],
            false,
            if custom && !started { 5 } else { 50 },
        );
        if wait == WAIT_FAILED {
            return Err(windows::core::Error::from_thread());
        }
        if window.closing.load(std::sync::atomic::Ordering::Acquire) {
            break;
        }
        let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
        if s.stopped {
            break;
        }
        if devices.signal.generation() != generation {
            return Ok(DeviceExit::DefaultChanged);
        }
        if s.reset {
            client.Stop()?;
            client.Reset()?;
            if !custom {
                client.Start()?;
            }
            started = !custom;
            s.reset = false;
            buffering = true;
        }
        let padding = client.GetCurrentPadding()?;
        s.report.device_padding = padding;
        let available = capacity.saturating_sub(padding);
        if available == 0 {
            continue;
        }
        if buffering && s.pcm.len() / channels >= min_frames.max(1) {
            buffering = false;
        }
        let count = if buffering {
            0
        } else {
            available.min((s.pcm.len() / channels) as u32)
        };
        if custom {
            if count == 0 {
                if started && !buffering && padding == 0 {
                    client.Stop()?;
                    client.Reset()?;
                    started = false;
                    buffering = true;
                    s.report.underruns += 1;
                }
                continue;
            }
            let pointer = render.GetBuffer(count)?;
            let buffer =
                std::slice::from_raw_parts_mut(pointer.cast::<i16>(), count as usize * channels);
            for sample in buffer {
                *sample = s.pcm.pop_front().unwrap();
            }
            render.ReleaseBuffer(count, 0)?;
            if !started {
                client.Start()?;
                started = true;
            }
            s.report.frames_written += count as u64;
            s.report.queued_frames = s.pcm.len() / channels;
            s.report.device_padding = padding + count;
            continue;
        }
        let pointer = render.GetBuffer(available)?;
        // SAFETY: WASAPI grants exactly available interleaved 16-bit stereo frames.
        let buffer =
            std::slice::from_raw_parts_mut(pointer.cast::<i16>(), available as usize * channels);
        buffer.fill(0);
        for sample in buffer.iter_mut().take(count as usize * channels) {
            *sample = s.pcm.pop_front().unwrap();
        }
        render.ReleaseBuffer(
            available,
            if count == 0 {
                AUDCLNT_BUFFERFLAGS_SILENT.0 as u32
            } else {
                0
            },
        )?;
        s.report.frames_written += count as u64;
        s.report.queued_frames = s.pcm.len() / channels;
        if count < available && !buffering {
            s.report.underruns += 1;
            buffering = true;
        }
    }
    Ok(DeviceExit::Stopped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(custom_buffer: bool) -> Playback {
        Playback {
            layout: Layout::STEREO,
            shared: Arc::new(Mutex::new(State::default())),
            worker: Mutex::new(None),
            max_frames: 4800,
            custom_buffer,
            signal: Signal::new().unwrap(),
            flush_source: Arc::new(AtomicBool::new(false)),
        }
    }
    #[test]
    fn reopening_drops_old_pcm_and_waits_for_one_upstream_retirement() {
        for custom in [false, true] {
            let output = fixture(custom);
            output.queue(&[1, -1, 2, -2]);
            {
                let mut state = output.shared.lock().unwrap();
                state.report.device_padding = 480;
                mark_recovering(&mut state, 2, Some(AUDCLNT_E_DEVICE_INVALIDATED));
                assert!(state.pcm.is_empty());
                assert_eq!(state.report.device_padding, 0);
            }
            output.queue(&[3, -3]);
            assert!(output.shared.lock().unwrap().pcm.is_empty());
            {
                let mut state = output.shared.lock().unwrap();
                mark_open(&mut state, 2, true, &output.flush_source);
                assert_eq!(state.report.recoveries, 1);
                assert!(state.report.failure_hresult.is_none());
            }
            output.queue(&[4, -4]); // The consumer has not retired its upstream queue yet.
            assert!(output.shared.lock().unwrap().pcm.is_empty());
            assert!(output.take_recovery_flush());
            assert!(!output.take_recovery_flush());
            output.queue(&[5, -5]);
            assert_eq!(output.shared.lock().unwrap().pcm, [5, -5]);
            assert_eq!(output.snapshot().frames_dropped, 4);
        }
    }
    #[test]
    fn stop_wakes_a_worker_waiting_in_recovery() {
        let output = fixture(false);
        let shared = output.shared.clone();
        let signal = output.signal.clone();
        let worker = std::thread::spawn(move || {
            unsafe {
                assert_eq!(WaitForSingleObject(signal.event.0, 2000), WAIT_OBJECT_0);
            }
            let mut state = shared.lock().unwrap();
            assert!(state.stopped);
            state.report.resources_released = true;
        });
        *output.worker.lock().unwrap() = Some(worker);
        output.stop();
        assert!(output.snapshot().resources_released);
    }
    #[test]
    fn device_errors_are_retryable_but_programming_errors_remain_terminal() {
        assert!(recoverable(AUDCLNT_E_DEVICE_INVALIDATED));
        assert!(recoverable(AUDCLNT_E_RESOURCES_INVALIDATED));
        assert!(recoverable(AUDCLNT_E_SERVICE_NOT_RUNNING));
        assert!(recoverable(windows::core::HRESULT::from_win32(
            ERROR_NOT_FOUND.0
        )));
        assert!(!recoverable(E_INVALIDARG));
        assert!(!recoverable(E_POINTER));
    }

    #[test]
    fn surround_queue_budget_counts_frames_and_keeps_channels_together() {
        let layout = Layout::new(6, 0x60f).unwrap();
        let output = Playback {
            layout,
            shared: Arc::new(Mutex::new(State::default())),
            worker: Mutex::new(None),
            signal: Signal::new().unwrap(),
            flush_source: Arc::new(AtomicBool::new(false)),
            max_frames: 3,
            custom_buffer: true,
        };
        output.shared.lock().unwrap().report.device_padding = 1;
        output.queue(&[1; 12]);
        output.queue(&[2; 6]);
        let state = output.shared.lock().unwrap();
        assert_eq!(state.pcm, [1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2]);
        assert_eq!(state.report.frames_dropped, 1);
        drop(state);
        assert_eq!(output.queued_frames(), 3);
    }

    #[test]
    fn budget_keeps_newest_stereo_with_device_padding() {
        let output = Playback {
            layout: Layout::STEREO,
            shared: Arc::new(Mutex::new(State::default())),
            worker: Mutex::new(None),
            signal: Signal::new().unwrap(),
            flush_source: Arc::new(AtomicBool::new(false)),
            max_frames: 4,
            custom_buffer: true,
        };
        output.shared.lock().unwrap().report.device_padding = 1;
        output.queue(&[1, -1, 2, -2]);
        output.queue(&[3, -3, 4, -4]);
        assert_eq!(output.shared.lock().unwrap().pcm, [2, -2, 3, -3, 4, -4]);
        assert_eq!(output.queued_frames(), 4);
        output.queue(&[5, -5, 6, -6, 7, -7, 8, -8, 9, -9]);
        assert_eq!(output.shared.lock().unwrap().pcm, [7, -7, 8, -8, 9, -9]);
        assert_eq!(output.queued_frames(), 4);
        assert_eq!(output.snapshot().frames_dropped, 6);
    }

    #[test]
    fn original_queue_limit_rejects_whole_new_block_without_trimming_old_pcm() {
        let output = Playback {
            layout: Layout::STEREO,
            shared: Arc::new(Mutex::new(State::default())),
            worker: Mutex::new(None),
            signal: Signal::new().unwrap(),
            flush_source: Arc::new(AtomicBool::new(false)),
            max_frames: 3,
            custom_buffer: false,
        };
        output.queue(&[1, -1, 2, -2]);
        output.queue(&[3, -3, 4, -4]);
        assert_eq!(output.shared.lock().unwrap().pcm, [1, -1, 2, -2]);
        assert_eq!(output.snapshot().frames_dropped, 2);
        output.queue(&[3, -3]);
        assert_eq!(output.shared.lock().unwrap().pcm, [1, -1, 2, -2, 3, -3]);
        output.reset();
        assert!(output.shared.lock().unwrap().pcm.is_empty());
        assert!(output.shared.lock().unwrap().reset);
        assert_eq!(output.snapshot().queued_frames, 0);
    }
}
