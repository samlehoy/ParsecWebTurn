//! RFC 6716 reference decoder. The pinned Parsec channel carries raw Opus,
//! 48 kHz stereo; containers/RTP headers must not be invented around it.
use anyhow::{bail, Result};
use serde::Serialize;
use std::{
    collections::VecDeque,
    sync::{mpsc, Arc, Mutex},
    thread::JoinHandle,
};
const MAX_SAMPLES: usize = 48_000; // 500 ms, interleaved stereo
const PCM_SAMPLES: usize = 5760 * 2; // RFC maximum 120 ms, stereo
const RECYCLED_BUFFERS: usize = 32;
#[derive(Clone, Default, Serialize)]
pub struct Snapshot {
    pub poll_calls: u64,
    pub max_poll_gap_ms: Option<f64>,
    pub packets_decoded: u64,
    pub packets_dropped: u64,
    pub decode_errors: u64,
    pub pcm_frames_polled: u64,
    pub queued_samples: usize,
    pub worker_finished: bool,
}
#[derive(Default)]
struct State {
    #[cfg(windows)]
    previous_poll: Option<std::time::Instant>,
    report: Snapshot,
    pcm: VecDeque<Vec<i16>>,
    free_pcm: Vec<Vec<i16>>,
    free_packets: Vec<Vec<u8>>,
    stopped: bool,
}
#[derive(Clone)]
pub(crate) struct Sender {
    tx: mpsc::SyncSender<Vec<u8>>,
    state: Arc<Mutex<State>>,
}
impl Sender {
    pub fn submit(&self, bytes: &[u8]) -> bool {
        let mut packet = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.stopped || bytes.is_empty() || bytes.len() > 65_536 {
                state.report.packets_dropped += 1;
                return false;
            }
            state.free_packets.pop().unwrap_or_default()
        };
        packet.clear();
        packet.extend_from_slice(bytes);
        match self.tx.try_send(packet) {
            Ok(()) => true,
            Err(error) => {
                let packet = match error {
                    mpsc::TrySendError::Full(packet) | mpsc::TrySendError::Disconnected(packet) => {
                        packet
                    }
                };
                let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                state.report.packets_dropped += 1;
                if !state.stopped && state.free_packets.len() < RECYCLED_BUFFERS {
                    state.free_packets.push(packet);
                }
                false
            }
        }
    }
}
pub struct Pipeline {
    tx: Option<mpsc::SyncSender<Vec<u8>>>,
    state: Arc<Mutex<State>>,
    poll_lock: Mutex<()>,
    worker: Option<JoinHandle<()>>,
}
struct Decoder(*mut libopus_sys::OpusDecoder);
impl Decoder {
    fn new() -> Result<Self> {
        let mut error = 0;
        // SAFETY: libopus owns this opaque state; this worker alone uses it.
        let pointer = unsafe { libopus_sys::opus_decoder_create(48_000, 2, &mut error) };
        if pointer.is_null() || error != 0 {
            bail!("Opus decoder initialization failed");
        }
        Ok(Self(pointer))
    }
    fn decode_into(&mut self, packet: &[u8], pcm: &mut Vec<i16>) -> Result<()> {
        if packet.is_empty() || packet.len() > 65_536 {
            bail!("invalid Opus packet boundary");
        }
        pcm.resize(PCM_SAMPLES, 0);
        // SAFETY: both slices remain alive, capacity matches frame_size*channels.
        let frames = unsafe {
            libopus_sys::opus_decode(
                self.0,
                packet.as_ptr(),
                packet.len() as i32,
                pcm.as_mut_ptr(),
                5760,
                0,
            )
        };
        if !(1..=5760).contains(&frames) {
            bail!("Opus packet rejected");
        }
        pcm.truncate(frames as usize * 2);
        Ok(())
    }
    #[cfg(test)]
    fn decode(&mut self, packet: &[u8]) -> Result<Vec<i16>> {
        let mut pcm = Vec::new();
        self.decode_into(packet, &mut pcm)?;
        Ok(pcm)
    }
}
impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe { libopus_sys::opus_decoder_destroy(self.0) }
    }
}
impl Pipeline {
    pub fn start() -> Self {
        let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(64);
        let state = Arc::new(Mutex::new(State::default()));
        let shared = state.clone();
        let worker = std::thread::spawn(move || {
            if let Ok(mut decoder) = Decoder::new() {
                loop {
                    if shared.lock().unwrap_or_else(|e| e.into_inner()).stopped {
                        break;
                    }
                    let packet = match rx.recv_timeout(std::time::Duration::from_millis(20)) {
                        Ok(packet) => packet,
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    };
                    let recycled = shared
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .free_pcm
                        .pop();
                    let mut pcm = recycled.unwrap_or_else(|| Vec::with_capacity(PCM_SAMPLES));
                    let decoded = decoder.decode_into(&packet, &mut pcm);
                    let mut state = shared.lock().unwrap_or_else(|e| e.into_inner());
                    if !state.stopped && state.free_packets.len() < RECYCLED_BUFFERS {
                        state.free_packets.push(packet);
                    }
                    if state.stopped {
                        continue;
                    }
                    match decoded {
                        Ok(()) => {
                            state.report.packets_decoded += 1;
                            // Drop oldest decoded audio on backpressure; never grow latency indefinitely.
                            while state.report.queued_samples + pcm.len() > MAX_SAMPLES {
                                if let Some(old) = state.pcm.pop_front() {
                                    state.report.queued_samples -= old.len();
                                    state.report.packets_dropped += 1;
                                    if state.free_pcm.len() < RECYCLED_BUFFERS {
                                        state.free_pcm.push(old);
                                    }
                                } else {
                                    break;
                                }
                            }
                            state.report.queued_samples += pcm.len();
                            state.pcm.push_back(pcm);
                        }
                        Err(_) => {
                            state.report.decode_errors += 1;
                            if state.free_pcm.len() < RECYCLED_BUFFERS {
                                state.free_pcm.push(pcm);
                            }
                        }
                    }
                }
            } else {
                shared
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .report
                    .decode_errors += 1;
            }
            shared
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .report
                .worker_finished = true;
        });
        Self {
            tx: Some(tx),
            state,
            poll_lock: Mutex::new(()),
            worker: Some(worker),
        }
    }
    #[cfg(test)]
    pub fn submit(&self, bytes: &[u8]) -> bool {
        self.sender().is_some_and(|sender| sender.submit(bytes))
    }
    pub(crate) fn sender(&self) -> Option<Sender> {
        self.tx.as_ref().map(|tx| Sender {
            tx: tx.clone(),
            state: self.state.clone(),
        })
    }
    pub fn poll(
        &self,
        memory: &crate::memory::GuestMemory,
        pointer: u32,
        capacity: usize,
    ) -> Result<usize> {
        // Serialize guest writes while leaving the decoder's state lock free.
        let _poll = self.poll_lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.report.poll_calls += 1;
        #[cfg(windows)]
        if crate::io_trace::enabled() {
            let now = std::time::Instant::now();
            if let Some(previous) = state.previous_poll.replace(now) {
                let gap = now.duration_since(previous).as_secs_f64() * 1000.0;
                state.report.max_poll_gap_ms =
                    Some(state.report.max_poll_gap_ms.unwrap_or(0.0).max(gap));
            }
        }
        let Some(pcm) = state.pcm.front() else {
            return Ok(0);
        };
        if pcm.len() > capacity {
            bail!("guest audio buffer too small");
        }
        memory.range(pointer, pcm.len() * 2)?;
        let samples = pcm.len();
        let pcm = state.pcm.pop_front().unwrap();
        state.report.queued_samples -= samples;
        drop(state);
        // Bounds were checked before dequeue; shared memory can only grow.
        memory.write_i16_le(pointer, &pcm)?;
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.report.pcm_frames_polled += samples as u64 / 2;
        if !state.stopped && state.free_pcm.len() < RECYCLED_BUFFERS {
            state.free_pcm.push(pcm);
        }
        Ok(samples / 2)
    }
    pub fn snapshot(&self) -> Snapshot {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .report
            .clone()
    }
    pub fn stop(&mut self) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).stopped = true;
        self.tx.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.pcm.clear();
        state.free_pcm.clear();
        state.free_packets.clear();
        state.report.queued_samples = 0;
    }
}
impl Drop for Pipeline {
    fn drop(&mut self) {
        self.stop();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_decode_reuses_pcm_storage_and_stop_invalidates_sender_handles() {
        let mut decoder = Decoder::new().unwrap();
        let mut pcm = Vec::with_capacity(PCM_SAMPLES);
        let pointer = pcm.as_ptr();
        for _ in 0..20 {
            decoder.decode_into(&[0xf8, 0xff, 0xfe], &mut pcm).unwrap();
            assert_eq!(pcm.len(), 1920);
            assert_eq!(pcm.as_ptr(), pointer);
        }
        let mut pipeline = Pipeline::start();
        let sender = pipeline.sender().unwrap();
        pipeline.stop();
        assert!(pipeline.snapshot().worker_finished);
        assert!(!sender.submit(&[0xf8, 0xff, 0xfe]));
        let state = pipeline.state.lock().unwrap();
        assert!(state.free_packets.is_empty());
        assert!(state.free_pcm.is_empty());
    }
    #[test]
    fn reference_decoder_accepts_standard_silence_and_rejects_invalid_packet() {
        let mut decoder = Decoder::new().unwrap();
        let samples = decoder.decode(&[0xf8, 0xff, 0xfe]).unwrap();
        assert_eq!(samples.len(), 960 * 2);
        assert!(samples.iter().all(|v| v.abs() < 4));
        assert!(decoder.decode(&[]).is_err());
        assert!(decoder.decode(&[3]).is_err());
    }
    #[test]
    fn reference_opus_roundtrip_retains_stereo_pcm_signal() {
        let mut error = 0;
        let encoder = unsafe {
            libopus_sys::opus_encoder_create(
                48_000,
                2,
                libopus_sys::OPUS_APPLICATION_AUDIO as i32,
                &mut error,
            )
        };
        assert!(!encoder.is_null());
        assert_eq!(error, 0);
        let input: Vec<i16> = (0..960)
            .flat_map(|i| {
                let v = ((i as f32 * 0.08).sin() * 8000.) as i16;
                [v, -v]
            })
            .collect();
        let mut packet = vec![0; 4096];
        let size = unsafe {
            libopus_sys::opus_encode(
                encoder,
                input.as_ptr(),
                960,
                packet.as_mut_ptr(),
                packet.len() as i32,
            )
        };
        unsafe { libopus_sys::opus_encoder_destroy(encoder) };
        assert!(size > 0);
        packet.truncate(size as usize);
        let pcm = Decoder::new().unwrap().decode(&packet).unwrap();
        assert_eq!(pcm.len(), 1920);
        assert!(pcm.iter().map(|v| i64::from(*v).abs()).sum::<i64>() > 100_000);
    }
    #[test]
    fn decoded_audio_is_retained_until_a_valid_guest_copy_and_released_on_stop() {
        let mut config = wasmtime::Config::new();
        config.wasm_threads(true);
        let engine = wasmtime::Engine::new(&config).unwrap();
        let memory = crate::memory::GuestMemory(
            wasmtime::SharedMemory::new(&engine, wasmtime::MemoryType::shared(1, 1)).unwrap(),
        );
        let mut pipeline = Pipeline::start();
        assert!(pipeline.submit(&[0xf8, 0xff, 0xfe]));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while pipeline.snapshot().packets_decoded == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(pipeline.snapshot().queued_samples, 1920);
        assert!(pipeline.poll(&memory, 0, 1919).is_err());
        assert!(pipeline.poll(&memory, u32::MAX, 1920).is_err());
        assert_eq!(pipeline.snapshot().queued_samples, 1920);
        assert_eq!(pipeline.poll(&memory, 0, 1920).unwrap(), 960);
        assert_eq!(pipeline.poll(&memory, 0, 1920).unwrap(), 0);
        pipeline.stop();
        assert!(pipeline.snapshot().worker_finished);
        assert_eq!(pipeline.snapshot().queued_samples, 0);
    }
}
