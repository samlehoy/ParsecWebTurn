//! Shared-mode, event-driven WASAPI PCM output. COM objects stay on the worker.
use anyhow::{bail, Result};
use serde::Serialize;
use std::{
    collections::{BTreeMap, VecDeque},
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
}
#[derive(Default)]
pub struct Registry {
    next: u32,
    outputs: BTreeMap<u32, Arc<Playback>>,
    completed: VecDeque<Snapshot>,
}
impl Registry {
    #[cfg(feature = "diagnostics")]
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
    custom_buffer: bool,
}
impl Playback {
    pub fn create(
        window: Arc<crate::window::Window>,
        min_buffer: u32,
        max_buffer: u32,
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
                "minimum_ms":min_buffer,"maximum_ms":max_buffer,"custom":custom.is_some()
            }),
        );
        let state = shared.clone();
        let (tx, rx) = mpsc::sync_channel(1);
        window
            .active_contexts
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        let worker = std::thread::spawn(move || {
            let result = unsafe { render(&state, &window, min_frames, &tx) };
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
            custom_buffer: custom.is_some(),
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
        if s.stopped
            || s.report.failure_hresult.is_some()
            || (!self.custom_buffer && s.pcm.len() / 2 + frames > self.max_frames)
        {
            s.report.frames_dropped += frames as u64;
            return;
        }
        if self.custom_buffer {
            // Keep the newest complete stereo frames within the combined budget.
            let room = self
                .max_frames
                .saturating_sub(s.report.device_padding as usize);
            let excess = (s.pcm.len() / 2 + frames).saturating_sub(room);
            let old = excess.min(s.pcm.len() / 2);
            s.pcm.drain(..old * 2);
            let incoming = excess.saturating_sub(old).min(frames);
            s.report.frames_dropped += (old + incoming) as u64;
            s.pcm.extend(&pcm[incoming * 2..]);
        } else {
            s.pcm.extend(pcm);
        }
        s.report.frames_queued += frames as u64;
        s.report.queued_frames = s.pcm.len() / 2;
    }
    pub fn queued_frames(&self) -> u32 {
        let s = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        (s.pcm.len() / 2 + s.report.device_padding as usize) as u32
    }
    pub fn reset(&self) {
        let mut s = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        s.pcm.clear();
        s.report.queued_frames = 0;
        s.reset = true;
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
struct Event(HANDLE);
impl Drop for Event {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
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
    ready: &mpsc::SyncSender<bool>,
) -> windows::core::Result<()> {
    CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
    let _com = Apartment;
    let _scheduling = crate::media_scheduling::Registration::audio();
    let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
    let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
    let mut client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
    let format = WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_PCM as u16,
        nChannels: 2,
        nSamplesPerSec: 48_000,
        nAvgBytesPerSec: 192_000,
        nBlockAlign: 4,
        wBitsPerSample: 16,
        cbSize: 0,
    };
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
                &format,
                &mut default,
                &mut fundamental,
                &mut minimum,
                &mut maximum,
            )?;
            modern.InitializeSharedAudioStream(
                AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                    | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                    | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                minimum,
                &format,
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
            &format,
            None,
        )?;
    }
    let event = Event(CreateEventW(None, false, false, PCWSTR::null())?);
    client.SetEventHandle(event.0)?;
    let render: IAudioRenderClient = client.GetService()?;
    let capacity = client.GetBufferSize()?;
    {
        let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
        s.report.low_latency_device = fast;
        s.report.device_capacity_frames = capacity;
    }
    // The unchanged automatic path primes silence. The experimental path starts
    // only when real PCM is ready and never pads its queue with a silence reserve.
    if !custom {
        let _ = render.GetBuffer(capacity)?;
        render.ReleaseBuffer(capacity, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)?;
        client.Start()?;
    }
    let mut started = !custom;
    let _started = Started(client.clone());
    shared
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .report
        .device_opened = true;
    let _ = ready.send(true);
    let mut buffering = true;
    loop {
        WaitForSingleObject(event.0, if custom && !started { 5 } else { 50 });
        if window.closing.load(std::sync::atomic::Ordering::Acquire) {
            break;
        }
        let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
        if s.stopped {
            break;
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
        if buffering && s.pcm.len() / 2 >= min_frames.max(1) {
            buffering = false;
        }
        let count = if buffering {
            0
        } else {
            available.min((s.pcm.len() / 2) as u32)
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
            let buffer = std::slice::from_raw_parts_mut(pointer.cast::<i16>(), count as usize * 2);
            for sample in buffer {
                *sample = s.pcm.pop_front().unwrap();
            }
            render.ReleaseBuffer(count, 0)?;
            if !started {
                client.Start()?;
                started = true;
            }
            s.report.frames_written += count as u64;
            s.report.queued_frames = s.pcm.len() / 2;
            s.report.device_padding = padding + count;
            continue;
        }
        let pointer = render.GetBuffer(available)?;
        // SAFETY: WASAPI grants exactly available interleaved 16-bit stereo frames.
        let buffer = std::slice::from_raw_parts_mut(pointer.cast::<i16>(), available as usize * 2);
        buffer.fill(0);
        for sample in buffer.iter_mut().take(count as usize * 2) {
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
        s.report.queued_frames = s.pcm.len() / 2;
        if count < available && !buffering {
            s.report.underruns += 1;
            buffering = true;
        }
    }
    Ok(())
}

/// Quiet local device smoke test: no account/network and no audible test tone.
#[cfg(feature = "diagnostics")]
pub fn probe() -> Result<serde_json::Value> {
    let mut baseline = probe_output(false)?;
    baseline["experimental"] = probe_output(true)?;
    Ok(baseline)
}
#[cfg(feature = "diagnostics")]
fn probe_output(custom: bool) -> Result<serde_json::Value> {
    let window = crate::window::Window::create_video_probe()?;
    let fixture_root = if custom {
        let root = std::env::temp_dir().join(format!(
            "parsec-audio-probe-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        std::fs::create_dir(&root)?;
        let manager = crate::connection_settings::Manager::open(root.join("settings.json"));
        let mut settings = manager.view().0;
        settings.provider = "custom".into();
        settings.experimental_audio_buffer = true;
        settings.audio_min_buffer_ms = 20;
        settings.audio_max_buffer_ms = 60;
        manager.save(settings, "", "", false, false)?;
        window.install_connection_settings(manager);
        Some(root)
    } else {
        None
    };
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
    if let Some(root) = fixture_root {
        std::fs::remove_file(root.join("settings.json"))?;
        std::fs::remove_dir(root)?;
    }
    Ok(
        serde_json::json!({"scope":"synthetic-silent-wasapi-output","audio":snapshot,"real_account_used":false,"external_requests_enabled":false,"native_window_released":window.handle().is_null()}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn experimental_budget_keeps_newest_stereo_with_device_padding() {
        let output = Playback {
            shared: Arc::new(Mutex::new(State::default())),
            worker: Mutex::new(None),
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
            shared: Arc::new(Mutex::new(State::default())),
            worker: Mutex::new(None),
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
