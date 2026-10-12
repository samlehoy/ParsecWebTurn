//! Shared-mode, event-driven WASAPI PCM output. COM objects stay on the worker.
use anyhow::{bail, Result};
use serde::Serialize;
use std::{
    collections::{BTreeMap, VecDeque},
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    sync::{mpsc, Arc, Mutex},
    thread::JoinHandle,
};
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::*,
        Media::Audio::*,
        System::{Com::*, Threading::*},
    },
};
#[derive(Clone, Default, Serialize)]
pub struct Snapshot {
    pub min_buffer_ms: u32,
    pub max_buffer_ms: u32,
    pub resets: u64,
    pub overflow_recoveries: u64,
    pub buffering: bool,
    pub max_worker_gap_ms: Option<f64>,
    pub device_opened: bool,
    pub native_shared_buffer: bool,
    pub engine_period_frames: Option<u32>,
    pub device_buffer_frames: u32,
    pub mmcss_registered: bool,
    pub frames_queued: u64,
    pub frames_written: u64,
    pub frames_dropped: u64,
    pub underruns: u64,
    pub empty_device_wakes: u64,
    pub queued_frames: usize,
    pub device_padding: u32,
    pub failure_hresult: Option<String>,
    pub resources_released: bool,
}
#[derive(Default)]
pub struct Registry {
    next: u32,
    outputs: BTreeMap<u32, Arc<Playback>>,
    completed: VecDeque<Snapshot>,
}
impl Registry {
    pub fn snapshots(&self) -> Vec<Snapshot> {
        self.completed
            .iter()
            .cloned()
            .chain(self.outputs.values().map(|v| v.snapshot()))
            .collect()
    }
    pub fn retain_snapshot(&mut self, snapshot: Snapshot) {
        if self.completed.len() == 8 {
            self.completed.pop_front();
        }
        self.completed.push_back(snapshot);
    }
    pub fn insert(&mut self, output: Playback) -> Result<u32> {
        if self.outputs.len() >= 4 {
            bail!("audio context limit");
        }
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("audio handle overflow"))?;
        self.outputs.insert(self.next, Arc::new(output));
        Ok(self.next)
    }
    pub fn get(&self, handle: u32) -> Result<Arc<Playback>> {
        self.outputs
            .get(&handle)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("invalid audio context"))
    }
    pub fn remove(&mut self, handle: u32) -> Result<Arc<Playback>> {
        self.outputs
            .remove(&handle)
            .ok_or_else(|| anyhow::anyhow!("invalid audio context"))
    }
}
#[derive(Default)]
struct State {
    report: Snapshot,
    pcm: VecDeque<i16>,
    stopped: bool,
    reset: bool,
}
pub struct Playback {
    shared: Arc<Mutex<State>>,
    worker: Mutex<Option<JoinHandle<()>>>,
    max_frames: usize,
    wake: Arc<Event>,
}
impl Playback {
    pub fn create(
        window: Arc<crate::window::Window>,
        min_buffer: u32,
        max_buffer: u32,
    ) -> Result<Self> {
        if min_buffer > max_buffer || max_buffer == 0 || max_buffer > 500 {
            bail!("invalid audio latency limits");
        }
        // Pinned AudioWorklet converts its latency arguments from milliseconds to frames.
        let max_frames = max_buffer as usize * 48;
        let min_frames = min_buffer as usize * 48;
        let wake = Arc::new(Event::new()?);
        let shared = Arc::new(Mutex::new(State::default()));
        {
            let mut state = shared.lock().unwrap_or_else(|e| e.into_inner());
            state.report.min_buffer_ms = min_buffer;
            state.report.max_buffer_ms = max_buffer;
            state.report.buffering = true;
        }
        let state = shared.clone();
        let worker_wake = wake.clone();
        let (tx, rx) = mpsc::sync_channel(1);
        window
            .active_contexts
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        let worker = std::thread::spawn(move || {
            let result =
                unsafe { render(&state, &window, min_frames, max_frames, &worker_wake, &tx) };
            if let Err(error) = result {
                state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .report
                    .failure_hresult = Some(format!("0x{:08X}", error.code().0 as u32));
                let _ = tx.try_send(false);
            }
            state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .report
                .resources_released = true;
            window
                .active_contexts
                .fetch_sub(1, std::sync::atomic::Ordering::Release);
        });
        let output = Self {
            shared,
            worker: Mutex::new(Some(worker)),
            max_frames,
            wake,
        };
        if rx.recv_timeout(std::time::Duration::from_secs(3)).ok() != Some(true) {
            output.stop();
            bail!("WASAPI output unavailable");
        }
        Ok(output)
    }
    pub fn queue(&self, pcm: &[i16]) {
        let mut s = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        let frames = pcm.len() / 2;
        if s.stopped || s.report.failure_hresult.is_some() {
            s.report.frames_dropped += frames as u64;
            return;
        }
        // Bound staging memory. The renderer also budgets endpoint padding,
        // so moving PCM to WASAPI does not add a second latency allowance.
        let skip = frames.saturating_sub(self.max_frames) * 2;
        let incoming = &pcm[skip..frames * 2];
        s.pcm.extend(incoming);
        trim_to_budget(&mut s, 0, self.max_frames);
        s.report.frames_dropped += (skip / 2) as u64;
        s.report.frames_queued += frames as u64;
        s.report.queued_frames = s.pcm.len() / 2;
        self.wake.notify();
    }
    pub fn queued_frames(&self) -> u32 {
        let s = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        (s.pcm.len() / 2 + s.report.device_padding as usize) as u32
    }
    pub fn reset(&self) {
        let mut s = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        s.pcm.clear();
        s.report.queued_frames = 0;
        s.report.resets += 1;
        s.report.buffering = true;
        s.reset = true;
        self.wake.notify();
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
        self.wake.notify();
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
// OwnedHandle provides lifetime-safe Send/Sync for this kernel event; COM
// interfaces remain confined to the rendering worker.
struct Event(OwnedHandle);
impl Event {
    fn new() -> windows::core::Result<Self> {
        let handle = unsafe { CreateEventW(None, false, false, PCWSTR::null())? };
        // SAFETY: CreateEventW returned an owned, valid handle exactly once.
        Ok(Self(unsafe { OwnedHandle::from_raw_handle(handle.0) }))
    }
    fn handle(&self) -> HANDLE {
        HANDLE(self.0.as_raw_handle())
    }
    fn notify(&self) {
        unsafe {
            let _ = SetEvent(self.handle());
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
unsafe fn render(
    shared: &Mutex<State>,
    window: &crate::window::Window,
    min_frames: usize,
    max_frames: usize,
    event: &Event,
    ready: &mpsc::SyncSender<bool>,
) -> windows::core::Result<()> {
    CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
    let _com = Apartment;
    let scheduling = crate::media_scheduling::Registration::audio();
    let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
    let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
    let format = WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_PCM as u16,
        nChannels: 2,
        nSamplesPerSec: 48_000,
        nAvgBytesPerSec: 192_000,
        nBlockAlign: 4,
        wBitsPerSample: 16,
        cbSize: 0,
    };
    let client = initialize_client(&device, &format, max_frames)?;
    client.SetEventHandle(event.handle())?;
    let render: IAudioRenderClient = client.GetService()?;
    let capacity = client.GetBufferSize()?;
    {
        let mut state = shared.lock().unwrap_or_else(|e| e.into_inner());
        state.report.native_shared_buffer = true;
        let mut period = 0;
        if client.GetDevicePeriod(Some(&mut period), None).is_ok() {
            state.report.engine_period_frames = Some((period as u64 * 48_000 / 10_000_000) as u32);
        }
        state.report.device_buffer_frames = capacity;
        state.report.mmcss_registered = scheduling.is_some();
    }
    // Stage actual PCM before Start, as the native SDK/Matoya example does.
    // Capacity is headroom, not silence to prepend or a target to fill.
    let _started = Started(client.clone());
    shared
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .report
        .device_opened = true;
    let _ = ready.send(true);
    let mut previous_wake = std::time::Instant::now();
    let mut started = false;
    let mut starved = false;
    loop {
        WaitForSingleObject(event.handle(), 50);
        if window.closing.load(std::sync::atomic::Ordering::Acquire) {
            break;
        }
        let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
        if crate::io_trace::enabled() {
            let now = std::time::Instant::now();
            let gap = now.duration_since(previous_wake).as_secs_f64() * 1000.0;
            previous_wake = now;
            if started {
                s.report.max_worker_gap_ms =
                    Some(s.report.max_worker_gap_ms.unwrap_or(0.0).max(gap));
            }
        }
        if s.stopped {
            break;
        }
        if s.reset {
            client.Stop()?;
            client.Reset()?;
            started = false;
            starved = false;
            s.reset = false;
            s.report.buffering = true;
        }
        let padding = client.GetCurrentPadding()?;
        if crate::io_trace::enabled() && started && padding == 0 {
            s.report.empty_device_wakes += 1;
        }
        s.report.device_padding = padding;
        if started && padding == 0 && s.pcm.is_empty() {
            if !starved {
                s.report.underruns += 1;
            }
            starved = true;
        } else if !s.pcm.is_empty() {
            starved = false;
        }
        trim_to_budget(&mut s, padding as usize, max_frames);
        let available = capacity.saturating_sub(padding);
        if available == 0 {
            continue;
        }
        let buffering = next_buffering(s.report.buffering, s.pcm.len() / 2, min_frames);
        s.report.buffering = buffering;
        let count = render_frames(buffering, available, s.pcm.len() / 2);
        if count == 0 {
            // Windows renders starvation as silence. Once started, resume the
            // first real PCM without imposing another 75 ms source pause.
            continue;
        }
        let pointer = render.GetBuffer(count)?;
        // SAFETY: WASAPI grants exactly count interleaved 16-bit stereo frames.
        let buffer = std::slice::from_raw_parts_mut(pointer.cast::<i16>(), count as usize * 2);
        for sample in buffer {
            *sample = s.pcm.pop_front().unwrap();
        }
        render.ReleaseBuffer(count, 0)?;
        s.report.device_padding = padding + count;
        s.report.frames_written += count as u64;
        s.report.queued_frames = s.pcm.len() / 2;
        if !started {
            client.Start()?;
            started = true;
        }
    }
    Ok(())
}

fn next_buffering(buffering: bool, queued: usize, minimum: usize) -> bool {
    buffering && queued < minimum.max(1)
}

fn trim_to_budget(state: &mut State, padding: usize, maximum: usize) {
    let queued = state.pcm.len() / 2;
    let discard = queued.saturating_sub(maximum.saturating_sub(padding));
    if discard > 0 {
        state.pcm.drain(..discard * 2);
        state.report.frames_dropped += discard as u64;
        state.report.overflow_recoveries += 1;
        state.report.queued_frames = state.pcm.len() / 2;
    }
}

fn render_frames(buffering: bool, available: u32, queued: usize) -> u32 {
    if buffering {
        0
    } else {
        available.min(queued.min(u32::MAX as usize) as u32)
    }
}

/// Put the existing latency budget in the endpoint, where Windows can consume
/// it independently of our worker. Requested capacity is not playback latency.
unsafe fn initialize_client(
    device: &IMMDevice,
    format: &WAVEFORMATEX,
    max_frames: usize,
) -> windows::core::Result<IAudioClient> {
    let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
    client.Initialize(
        AUDCLNT_SHAREMODE_SHARED,
        AUDCLNT_STREAMFLAGS_EVENTCALLBACK
            | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
            | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
        (max_frames as i64 * 10_000_000 / 48_000).max(1),
        0,
        format,
        None,
    )?;
    Ok(client)
}

/// Quiet local device smoke test: no account/network and no audible test tone.
#[cfg(feature = "diagnostics")]
pub fn probe() -> Result<serde_json::Value> {
    let window = crate::window::Window::create_video_probe()?;
    let mut snapshot = None;
    if let Ok(output) = Playback::create(window.clone(), 0, 200) {
        for _ in 0..10 {
            output.queue(&[0; 1920]);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        output.reset();
        output.stop();
        snapshot = Some(output.snapshot());
    }
    window.close();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while !window.handle().is_null() && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    Ok(
        serde_json::json!({"scope":"synthetic-silent-wasapi-output","audio":snapshot,"real_account_used":false,"external_requests_enabled":false,"native_window_released":window.handle().is_null()}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_audio_packets_do_not_fill_the_endpoint_with_silence() {
        assert_eq!(render_frames(true, 960, 240), 0);
        assert_eq!(render_frames(false, 960, 240), 240);
        assert_eq!(render_frames(false, 960, 0), 0);
        // The next packet can resume immediately without startup rebuffering.
        assert_eq!(render_frames(false, 960, 120), 120);
        assert_eq!(render_frames(false, 120, 960), 120);
    }

    #[test]
    fn startup_buffers_but_starvation_does_not_add_another_source_pause() {
        assert!(next_buffering(true, 960, 3600));
        assert!(next_buffering(true, 2880, 3600));
        assert!(!next_buffering(true, 3840, 3600));
        assert!(!next_buffering(false, 0, 3600));
        assert!(!next_buffering(false, 960, 3600));
        assert!(!next_buffering(true, 1, 0));
    }

    #[test]
    fn endpoint_and_staging_share_one_budget_and_trim_only_excess_stereo_frames() {
        let output = Playback {
            shared: Arc::new(Mutex::new(State::default())),
            worker: Mutex::new(None),
            max_frames: 8,
            wake: Arc::new(Event::new().unwrap()),
        };
        output.queue(&[1, -1, 2, -2, 3, -3, 4, -4, 5, -5, 6, -6]);
        output.queue(&[7, -7, 8, -8, 9, -9]);
        let report = output.snapshot();
        assert_eq!(report.queued_frames, 8);
        assert_eq!(report.frames_dropped, 1);
        assert_eq!(report.overflow_recoveries, 1);
        assert!(!report.buffering);
        let mut state = output.shared.lock().unwrap();
        trim_to_budget(&mut state, 4, 8);
        assert_eq!(state.pcm, [6, -6, 7, -7, 8, -8, 9, -9]);
        assert_eq!(state.report.frames_dropped, 5);
        drop(state);
        output.queue(&[10, -10, 11, -11]);
        assert_eq!(output.snapshot().overflow_recoveries, 2);
        assert_eq!(output.snapshot().queued_frames, 6);
    }

    #[test]
    fn output_overflow_keeps_recent_stereo_frames_and_reset_releases_them() {
        let output = Playback {
            shared: Arc::new(Mutex::new(State::default())),
            worker: Mutex::new(None),
            max_frames: 3,
            wake: Arc::new(Event::new().unwrap()),
        };
        output.queue(&[1, -1, 2, -2]);
        output.queue(&[3, -3, 4, -4]);
        assert_eq!(output.shared.lock().unwrap().pcm, [2, -2, 3, -3, 4, -4]);
        output.queue(&[5, -5, 6, -6, 7, -7, 8, -8]);
        assert_eq!(output.shared.lock().unwrap().pcm, [6, -6, 7, -7, 8, -8]);
        assert_eq!(output.snapshot().frames_dropped, 5);
        output.reset();
        assert_eq!(output.snapshot().queued_frames, 0);
    }

    #[test]
    fn endpoint_headroom_preserves_continuous_pcm_across_worker_scheduling_stalls() {
        let output = Playback {
            shared: Arc::new(Mutex::new(State::default())),
            worker: Mutex::new(None),
            max_frames: 7200,
            wake: Arc::new(Event::new().unwrap()),
        };
        let mut endpoint = VecDeque::new();
        let mut buffering = true;
        let mut started = false;
        let mut source_frame = 0i16;
        let mut expected_frame = 0i16;
        for tick in 0..200 {
            if tick % 2 == 0 {
                let pcm: Vec<_> = (0..960)
                    .flat_map(|_| {
                        let value = source_frame;
                        source_frame = source_frame.wrapping_add(1);
                        [value, value.wrapping_neg()]
                    })
                    .collect();
                output.queue(&pcm);
            }
            // A 40 ms worker stall; the 10 ms engine and 20 ms source keep
            // running independently. The endpoint holds the startup reserve.
            if !(8..12).contains(&(tick % 20)) {
                let mut state = output.shared.lock().unwrap();
                trim_to_budget(&mut state, endpoint.len() / 2, 7200);
                buffering = next_buffering(buffering, state.pcm.len() / 2, 3600);
                let count = render_frames(
                    buffering,
                    7200 - endpoint.len() as u32 / 2,
                    state.pcm.len() / 2,
                );
                for _ in 0..count * 2 {
                    endpoint.push_back(state.pcm.pop_front().unwrap());
                }
                started |= count > 0;
                assert!(endpoint.len() / 2 + state.pcm.len() / 2 <= 7200);
            }
            if started {
                assert!(endpoint.len() >= 960, "endpoint starvation at tick {tick}");
                for _ in 0..480 {
                    assert_eq!(endpoint.pop_front(), Some(expected_frame));
                    assert_eq!(endpoint.pop_front(), Some(expected_frame.wrapping_neg()));
                    expected_frame = expected_frame.wrapping_add(1);
                }
            }
        }
        assert!(started);
        assert_eq!(output.snapshot().frames_dropped, 0);
    }
}
