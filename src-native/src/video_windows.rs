//! Documented Media Foundation H.264/HEVC -> D3D11 NV12/P010 -> DXGI presentation.
//! COM and all GPU objects are created, used and released on one worker thread.
use crate::connection_settings::PresentationMode;
use crate::video_output::{Frame, Queue, Snapshot};
use std::{
    mem::ManuallyDrop,
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
use windows::{
    core::Interface,
    Win32::{
        Foundation::{CloseHandle, HANDLE, HMODULE, HWND, RECT, WAIT_OBJECT_0, WAIT_TIMEOUT},
        Graphics::{
            Direct3D::*,
            Direct3D10::ID3D10Multithread,
            Direct3D11::*,
            Dxgi::{Common::*, *},
        },
        Media::MediaFoundation::*,
        System::{
            Com::*,
            Threading::{
                CreateEventW, ResetEvent, SetEvent, WaitForMultipleObjects, WaitForSingleObject,
            },
            Variant::*,
        },
    },
};

#[derive(Debug)]
struct Failure {
    stage: &'static str,
    hresult: Option<String>,
}
type Result<T> = std::result::Result<T, Failure>;
fn api<T>(stage: &'static str, value: windows::core::Result<T>) -> Result<T> {
    value.map_err(|e| Failure {
        stage,
        hresult: Some(format!("0x{:08X}", e.code().0 as u32)),
    })
}
fn unavailable(stage: &'static str) -> Failure {
    Failure {
        stage,
        hresult: None,
    }
}

// Public MFT discovery, not a hard-coded codec-extension DLL/CLSID.
unsafe fn create_decoder(codec: crate::video_stream::Codec, ten_bit: bool) -> Result<IMFTransform> {
    if codec == crate::video_stream::Codec::H264 {
        return api(
            "video-decoder-create",
            CoCreateInstance(&CMSH264DecoderMFT, None, CLSCTX_INPROC_SERVER),
        );
    }
    let input = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: MFVideoFormat_HEVC,
    };
    let output = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: if ten_bit {
            MFVideoFormat_P010
        } else {
            MFVideoFormat_NV12
        },
    };
    let mut list = std::ptr::null_mut();
    let mut count = 0;
    api(
        "video-hevc-enumeration",
        MFTEnumEx(
            MFT_CATEGORY_VIDEO_DECODER,
            MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_LOCALMFT | MFT_ENUM_FLAG_SORTANDFILTER,
            Some(&input),
            Some(&output),
            &mut list,
            &mut count,
        ),
    )?;
    if list.is_null() {
        return Err(unavailable("video-hevc-unavailable"));
    }
    let mut found = None;
    for index in 0..count {
        let activation = std::ptr::read(list.add(index as usize));
        if found.is_none() {
            if let Some(activation) = &activation {
                if let Ok(decoder) = activation.ActivateObject::<IMFTransform>() {
                    if decoder
                        .GetAttributes()
                        .and_then(|a| a.GetUINT32(&MF_SA_D3D11_AWARE))
                        .is_ok_and(|v| v != 0)
                    {
                        found = Some(decoder);
                    }
                }
            }
        }
        drop(activation);
    }
    CoTaskMemFree(Some(list.cast()));
    found.ok_or_else(|| unavailable("video-hevc-unavailable"))
}

/// Check installed synchronous D3D11-aware decoder and selected GPU's Main/NV12
/// profile before advertising HEVC. This is capability, not a decoding guarantee.
pub fn hevc_available(preferred_gpu: Option<&crate::connection_settings::GpuPreference>) -> bool {
    unsafe {
        let probe = || -> Result<()> {
            let _runtime = Runtime::start()?;
            let (device, _context) = create_device(preferred_gpu)?;
            let video: ID3D11VideoDevice = api("video-hevc-device", device.cast())?;
            if !api(
                "video-hevc-profile",
                video.CheckVideoDecoderFormat(
                    &D3D11_DECODER_PROFILE_HEVC_VLD_MAIN,
                    DXGI_FORMAT_NV12,
                ),
            )?
            .as_bool()
            {
                return Err(unavailable("video-hevc-profile-unavailable"));
            }
            let _decoder = create_decoder(crate::video_stream::Codec::Hevc, false)?;
            Ok(())
        };
        probe().is_ok()
    }
}

/// Main 10 is advertised only with a D3D11-aware P010 decoder and GPU conversion.
pub fn hevc_ten_bit_available(
    preferred_gpu: Option<&crate::connection_settings::GpuPreference>,
) -> bool {
    unsafe {
        (|| -> Result<()> {
            let _runtime = Runtime::start()?;
            let (device, _) = create_device(preferred_gpu)?;
            let video: ID3D11VideoDevice = api("video-main10-device", device.cast())?;
            if !api(
                "video-main10-profile",
                video.CheckVideoDecoderFormat(
                    &D3D11_DECODER_PROFILE_HEVC_VLD_MAIN10,
                    DXGI_FORMAT_P010,
                ),
            )?
            .as_bool()
            {
                return Err(unavailable("video-main10-unavailable"));
            }
            let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
                InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                InputWidth: 1920,
                InputHeight: 1080,
                OutputWidth: 1920,
                OutputHeight: 1080,
                InputFrameRate: DXGI_RATIONAL {
                    Numerator: 60,
                    Denominator: 1,
                },
                OutputFrameRate: DXGI_RATIONAL {
                    Numerator: 60,
                    Denominator: 1,
                },
                Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
            };
            let enumerator = api(
                "video-main10-processor",
                video.CreateVideoProcessorEnumerator(&desc),
            )?;
            if api(
                "video-main10-input-format",
                enumerator.CheckVideoProcessorFormat(DXGI_FORMAT_P010),
            )? & D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_INPUT.0 as u32
                == 0
            {
                return Err(unavailable("video-main10-input-unavailable"));
            }
            let _decoder = create_decoder(crate::video_stream::Codec::Hevc, true)?;
            Ok(())
        })()
        .is_ok()
    }
}

/// Packed AYUV decoder surfaces and the existing processor input are both
/// required. No MF/NV12 conversion can preserve 4:4:4 chroma.
pub fn hevc_444_available(
    preferred_gpu: Option<&crate::connection_settings::GpuPreference>,
) -> bool {
    #[cfg(not(feature = "vendor-video"))]
    {
        let _ = preferred_gpu;
        false
    }
    #[cfg(feature = "vendor-video")]
    unsafe {
        (|| -> Result<()> {
            let _runtime = Runtime::start()?;
            let (device, _) = create_device(preferred_gpu)?;
            let video: ID3D11VideoDevice = api("video-444-device", device.cast())?;
            if !api(
                "video-444-profile",
                video.CheckVideoDecoderFormat(
                    &D3D12_VIDEO_DECODE_PROFILE_HEVC_MAIN_444,
                    DXGI_FORMAT_AYUV,
                ),
            )?
            .as_bool()
            {
                return Err(unavailable("video-444-unavailable"));
            }
            let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
                InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                InputWidth: 1920,
                InputHeight: 1080,
                OutputWidth: 1920,
                OutputHeight: 1080,
                InputFrameRate: DXGI_RATIONAL {
                    Numerator: 60,
                    Denominator: 1,
                },
                OutputFrameRate: DXGI_RATIONAL {
                    Numerator: 60,
                    Denominator: 1,
                },
                Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
            };
            let processor = api(
                "video-444-processor",
                video.CreateVideoProcessorEnumerator(&desc),
            )?;
            if api(
                "video-444-input",
                processor.CheckVideoProcessorFormat(DXGI_FORMAT_AYUV),
            )? & D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_INPUT.0 as u32
                == 0
            {
                return Err(unavailable("video-444-input-unavailable"));
            }
            Ok(())
        })()
        .is_ok()
    }
}

// A queue wake can interrupt the DXGI wait: never hold up new decode work
// waiting for the monitor. The Arc keeps this handle alive for every waiter.
struct WorkerWake(HANDLE);
#[derive(Debug, PartialEq, Eq)]
enum DisplayWake {
    Input,
    Display,
    Timeout,
}
// SAFETY: Win32 event operations are thread-safe; the final Arc owns CloseHandle.
unsafe impl Send for WorkerWake {}
unsafe impl Sync for WorkerWake {}
impl WorkerWake {
    fn create() -> Option<Self> {
        unsafe { CreateEventW(None, true, false, None).ok().map(Self) }
    }
    fn notify(&self) {
        unsafe {
            let _ = SetEvent(self.0);
        }
    }
    fn arm(&self) -> bool {
        unsafe { ResetEvent(self.0).is_ok() }
    }
    unsafe fn wait(&self, display: HANDLE) -> windows::core::Result<DisplayWake> {
        // Queue first: if both are ready, service input and let poll_slot take
        // the DXGI signal later. Auto-reset DXGI signals must be consumed once.
        let status = WaitForMultipleObjects(&[self.0, display], false, 100);
        if status == WAIT_OBJECT_0 {
            Ok(DisplayWake::Input)
        } else if status == WAIT_TIMEOUT {
            Ok(DisplayWake::Timeout)
        } else if status.0 == WAIT_OBJECT_0.0 + 1 {
            Ok(DisplayWake::Display)
        } else {
            Err(windows::core::Error::from_thread())
        }
    }
}
impl Drop for WorkerWake {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
type Shared = Arc<(Mutex<Queue>, Condvar, Option<WorkerWake>)>;
pub struct Pipeline {
    shared: Shared,
    started: Instant,
    window: Arc<crate::window::Window>,
    surface_generation: u64,
}
impl Pipeline {
    pub fn start(window: Arc<crate::window::Window>) -> Self {
        Self::start_mode(window, false, false)
    }
    #[cfg(feature = "bud")]
    pub fn start_bud(window: Arc<crate::window::Window>) -> Self {
        Self::start_mode(window, false, true)
    }
    fn start_mode(
        window: Arc<crate::window::Window>,
        verify_synthetic_pixels: bool,
        bud_catch_up: bool,
    ) -> Self {
        let surface_generation = window.begin_video();
        let surface_window = window.clone();
        let presentation_mode = window.presentation_mode();
        let wake = (presentation_mode == PresentationMode::LowLatency)
            .then(WorkerWake::create)
            .flatten();
        let shared = Arc::new((Mutex::new(Queue::default()), Condvar::new(), wake));
        shared
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .report
            .enabled = true;
        let state = shared.clone();
        // Guard HWND destruction even if thread startup or device creation fails.
        window
            .active_contexts
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        let activity = Activity(window.clone());
        let error_state = shared.clone();
        let preferred_gpu = window.video_gpu();
        shared
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .bud_catch_up = bud_catch_up;
        let spawned = std::thread::Builder::new()
            .name("native-video-d3d11".into())
            .spawn(move || {
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run(
                        &state,
                        &window,
                        verify_synthetic_pixels,
                        preferred_gpu.as_ref(),
                        surface_generation,
                        presentation_mode,
                    )
                }));
                let mut q = state.0.lock().unwrap_or_else(|e| e.into_inner());
                match outcome {
                    Ok(Err(failure)) => q.fail(failure.stage, failure.hresult),
                    Err(_) => q.fail("video-worker-panic", None),
                    Ok(Ok(())) => {}
                }
                q.stopped = true;
                // A retired pipeline must release its reuse pool even while
                // an attempt owner still holds the final statistics snapshot.
                q.clear();
                q.report.worker_finished = true;
                q.report.resources_released = true;
                window.finish_video(surface_generation);
                *window
                    .video_report
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = Some(q.report.clone());
                drop(activity);
            });
        if spawned.is_err() {
            let mut q = error_state.0.lock().unwrap_or_else(|e| e.into_inner());
            q.fail("video-worker-start", None);
            q.stopped = true;
            q.report.worker_finished = true;
            q.report.resources_released = true;
        }
        Self {
            shared,
            started: Instant::now(),
            window: surface_window,
            surface_generation,
        }
    }
    pub fn submit(&self, bytes: &[u8], info: crate::video_stream::PacketInfo) -> bool {
        self.submit_oriented(bytes, info, 0)
    }
    pub fn submit_oriented(
        &self,
        bytes: &[u8],
        info: crate::video_stream::PacketInfo,
        _rotation: u8,
    ) -> bool {
        let timestamp = i64::try_from(self.started.elapsed().as_nanos() / 100).unwrap_or(i64::MAX);
        let (mut buffer, generation) = {
            let mut q = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
            if q.stopped || q.report.failure_stage.is_some() {
                q.report.frames_dropped = q.report.frames_dropped.saturating_add(1);
                return false;
            }
            (q.take_buffer(), q.reset_generation)
        };
        buffer.clear();
        buffer.extend_from_slice(bytes);
        let accepted = {
            let mut q = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
            if q.reset_generation != generation {
                q.recycle(buffer);
                false
            } else {
                let accepted =
                    q.submit_codec(buffer, info.idr, info.parameters, timestamp, info.codec);
                #[cfg(feature = "native-core")]
                if accepted {
                    q.set_last_rotation(_rotation);
                }
                accepted
            }
        };
        self.shared.1.notify_one();
        if let Some(wake) = &self.shared.2 {
            wake.notify();
        }
        accepted
    }
    pub fn take_keyframe_request(&self) -> Option<u64> {
        let mut q = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
        q.take_keyframe_request(Instant::now())
            .map(|_| q.reset_generation)
    }
    pub fn recovery_request_failed(&self, generation: u64) {
        self.shared
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .request_failed(generation);
        self.shared.1.notify_one();
        if let Some(wake) = &self.shared.2 {
            wake.notify();
        }
    }
    pub fn snapshot(&self) -> Snapshot {
        self.shared
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .report
            .clone()
    }
    pub fn stop(&self) {
        self.window.finish_video(self.surface_generation);
        self.shared
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stopped = true;
        self.shared.1.notify_all();
        if let Some(wake) = &self.shared.2 {
            wake.notify();
        }
    }
}
impl Drop for Pipeline {
    fn drop(&mut self) {
        self.stop();
    }
}
struct Activity(Arc<crate::window::Window>);
impl Drop for Activity {
    fn drop(&mut self) {
        self.0
            .active_contexts
            .fetch_sub(1, std::sync::atomic::Ordering::Release);
    }
}

fn update(shared: &Shared, f: impl FnOnce(&mut Snapshot)) {
    f(&mut shared.0.lock().unwrap_or_else(|e| e.into_inner()).report);
}
fn publish_report(shared: &Shared, window: &crate::window::Window) {
    let report = shared
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .report
        .clone();
    *window
        .video_report
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(report);
}
fn run(
    shared: &Shared,
    window: &Arc<crate::window::Window>,
    verify_synthetic_pixels: bool,
    preferred_gpu: Option<&crate::connection_settings::GpuPreference>,
    surface_generation: u64,
    presentation_mode: PresentationMode,
) -> Result<()> {
    let scheduling = crate::media_scheduling::Registration::video();
    update(shared, |s| s.mmcss_registered = scheduling.is_some());
    let mut session: Option<Session> = None;
    let mut headers = Vec::new();
    let mut header_bytes = 0usize;
    let mut generation = 0;
    let mut parameters = crate::video_stream::ParameterCache::default();
    let mut awaiting_random_access = true;
    let mut vendor_failed = false;
    let mut vendor_fallback = None;
    let mut zero_failed = false;
    let mut zero_fallback = None;
    #[cfg(feature = "vendor-video")]
    let zero_requested = window.experimental_zero_copy();
    #[cfg(not(feature = "vendor-video"))]
    let zero_requested = false;
    loop {
        let retry_display = session.as_ref().is_some_and(Session::has_pending_display);
        let (next_generation, frame) = {
            let mut q = shared.0.lock().unwrap_or_else(|e| e.into_inner());
            while q.report.queue_depth == 0
                && !q.stopped
                && q.report.failure_stage.is_none()
                && q.reset_generation == generation
                && !window.closing.load(std::sync::atomic::Ordering::Acquire)
            {
                let display_wait = session.as_ref().and_then(Session::pending_wait_handle);
                if let (Some(display), Some(wake)) = (display_wait, &shared.2) {
                    // Reset while holding the queue lock, then release it before
                    // waiting. A producer publishing afterward sets the event;
                    // an already queued frame bypasses this wait altogether.
                    if wake.arm() {
                        drop(q);
                        let ready = unsafe { wake.wait(display) };
                        if let Some(renderer) = session.as_mut().and_then(|s| s.renderer.as_mut()) {
                            match ready {
                                Ok(DisplayWake::Display) => renderer.latency_slot = true,
                                Ok(DisplayWake::Input | DisplayWake::Timeout) => {}
                                Err(_) => renderer.disable_queue_control(shared),
                            }
                        }
                        q = shared.0.lock().unwrap_or_else(|e| e.into_inner());
                        q.check_recovery(Instant::now());
                        break;
                    }
                }
                q = shared
                    .1
                    .wait_timeout(
                        q,
                        Duration::from_millis(if retry_display { 2 } else { 100 }),
                    )
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
                q.check_recovery(Instant::now());
                if retry_display {
                    break;
                }
            }
            if q.stopped
                || q.report.failure_stage.is_some()
                || window.closing.load(std::sync::atomic::Ordering::Acquire)
            {
                break;
            }
            (q.reset_generation, q.pop())
        };
        if next_generation != generation {
            generation = next_generation;
            awaiting_random_access = true;
            session = None;
            headers.clear();
            header_bytes = 0;
            window.set_video_ready_for(surface_generation, false);
            if let Some(bytes) = parameters.bytes() {
                let codec = shared
                    .0
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .report
                    .codec
                    .ok_or_else(|| unavailable("video-recovery-codec-missing"))?;
                header_bytes = bytes.len();
                headers.push(Frame {
                    rotation: 0,
                    queued_at: Instant::now(),
                    codec,
                    bytes,
                    idr: false,
                    parameters: true,
                    timestamp_100ns: 0,
                });
            }
        }
        if frame.is_none() {
            if let Some(session) = &mut session {
                unsafe {
                    if session.retry_display(shared, window)? {
                        publish_report(shared, window);
                    }
                }
            }
        }
        let Some(mut frame) = frame else {
            continue;
        };
        // A display-mode transition retires the decoder through the existing
        // keyframe recovery path. Never mix old and new orientation on outputs.
        if session
            .as_ref()
            .is_some_and(|s| s.rotation != frame.rotation)
        {
            let mut q = shared.0.lock().unwrap_or_else(|e| e.into_inner());
            q.recover(Instant::now());
            q.recycle(frame.bytes);
            continue;
        }
        if frame.parameters {
            parameters.observe(&frame.bytes, frame.codec);
            if frame.codec == crate::video_stream::Codec::Hevc {
                if let (Some(previous), Some(next)) = (
                    session.as_ref().and_then(|session| session.hevc),
                    crate::video_stream::hevc_configuration(&frame.bytes),
                ) {
                    if previous != next {
                        // Retire the old GPU pools before feeding the new SPS.
                        // Keep this packet and its IDR: no extra frame copy,
                        // network round trip, or decoder fallback is needed.
                        session = None;
                        awaiting_random_access = true;
                        parameters.retain_hevc_configuration(next);
                        headers.clear();
                        header_bytes = 0;
                        if let Some(bytes) = parameters.bytes() {
                            header_bytes = bytes.len();
                            headers.push(Frame {
                                rotation: frame.rotation,
                                queued_at: Instant::now(),
                                codec: frame.codec,
                                bytes,
                                idr: false,
                                parameters: true,
                                timestamp_100ns: 0,
                            });
                        }
                        shared
                            .0
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .begin_format_change(Instant::now(), !frame.idr);
                        window.set_video_ready_for(surface_generation, false);
                        crate::event_log::record(
                            "INFO",
                            "video_format_changed",
                            serde_json::json!({
                                "previous_width":previous.width,"previous_height":previous.height,
                                "width":next.width,"height":next.height,
                                "bit_depth":next.bit_depth,"chroma":next.chroma,
                                "keyframe_in_packet":frame.idr,
                            }),
                        );
                    }
                }
            }
        }
        if awaiting_random_access && !frame.idr {
            // A parameter-bearing packet may also contain delta pictures. Only
            // feed its parameter NALs until this worker actually submits a key,
            // regardless of whether a future key is already in the queue.
            let mut bootstrap = crate::video_stream::ParameterCache::default();
            bootstrap.observe(&frame.bytes, frame.codec);
            let Some(bytes) = bootstrap.bytes() else {
                shared
                    .0
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .recycle(frame.bytes);
                continue;
            };
            let original = std::mem::replace(&mut frame.bytes, bytes);
            shared
                .0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .recycle(original);
        }
        if session.is_none() {
            let hevc = if frame.codec == crate::video_stream::Codec::Hevc {
                let config = crate::video_stream::hevc_configuration(&frame.bytes).or_else(|| {
                    headers
                        .iter()
                        .find_map(|f| crate::video_stream::hevc_configuration(&f.bytes))
                });
                if config.is_none() {
                    // A separate VPS can precede SPS. Retain compressed headers
                    // until the actual SPS supplies geometry; never guess a size.
                    header_bytes = header_bytes.saturating_add(frame.bytes.len());
                    if headers.len() >= crate::video_output::MAX_FRAMES
                        || header_bytes > crate::video_output::MAX_BYTES
                    {
                        return Err(unavailable("video-hevc-sps-missing"));
                    }
                    headers.push(frame);
                    continue;
                }
                config
            } else {
                None
            };
            let hwnd = window
                .ensure_video_surface()
                .map_err(|_| unavailable("video-surface-create"))?;
            let setup_started = Instant::now();
            let mut native = unsafe {
                Session::create(
                    HWND(hwnd as *mut _),
                    verify_synthetic_pixels,
                    preferred_gpu,
                    frame.codec,
                    hevc,
                    surface_generation,
                    (
                        !vendor_failed && !verify_synthetic_pixels,
                        zero_requested && !zero_failed && frame.rotation == 0,
                        presentation_mode,
                    ),
                )?
            };
            native.rotation = frame.rotation;
            if let Some(renderer) = &mut native.renderer {
                renderer.rotation = frame.rotation;
            }
            if zero_requested && frame.rotation != 0 {
                native.zero_copy_status =
                    Some("Rotated display uses the normal GPU conversion path".into());
            }
            update(shared, |s| {
                s.local_timing.decoder_setup.record(setup_started.elapsed());
                s.decoder_initialized = true;
                s.low_latency_request_accepted = native.low_latency_request_accepted;
                s.decoder = Some(native.name(frame.codec));
                s.decoder_fallback = if vendor_failed {
                    vendor_fallback.clone().or_else(|| native.fallback.clone())
                } else {
                    native.fallback.clone()
                };
                s.renderer = Some(native.renderer_name());
                s.presentation_mode = Some(
                    native
                        .renderer
                        .as_ref()
                        .map_or("Windows-managed (original surfaces)", |renderer| {
                            renderer.timing_status
                        }),
                );
                s.display_rgb_bit_depth = None;
                s.display_output_fallback = None;
                s.adapter = Some(native.adapter_name());
                s.zero_copy_status = if zero_failed {
                    zero_fallback.clone()
                } else {
                    native.zero_copy_status.clone()
                };
            });
            session = Some(native);
        }
        let mut failure = None;
        let mut retry_pending = false;
        for frame in headers.drain(..).chain(std::iter::once(frame)) {
            let feed_started = Instant::now();
            let (decoded_before, output_before) = {
                let q = shared.0.lock().unwrap_or_else(|e| e.into_inner());
                (
                    q.report.frames_decoded,
                    q.report.local_timing.output_service_ns,
                )
            };
            let result = unsafe { session.as_mut().unwrap().feed(&frame, shared, window) };
            update(shared, |s| {
                let output_ns = s
                    .local_timing
                    .output_service_ns
                    .saturating_sub(output_before);
                let service_ns = feed_started.elapsed().as_nanos().saturating_sub(output_ns);
                s.local_timing
                    .decoder_service
                    .record(Duration::from_nanos(service_ns.min(u64::MAX as u128) as u64));
                s.decode_feed_max_us = s
                    .decode_feed_max_us
                    .max(feed_started.elapsed().as_micros().min(u64::MAX as u128) as u64)
            });
            let mut q = shared.0.lock().unwrap_or_else(|e| e.into_inner());
            q.recycle(frame.bytes);
            if let Err(error) = result {
                q.report.frames_dropped = q.report.frames_dropped.saturating_add(1);
                failure = Some(error);
                break;
            }
            if frame.idr {
                awaiting_random_access = false;
            }
            if q.reset_generation == generation
                && !awaiting_random_access
                && q.report.frames_decoded > decoded_before
            {
                q.recovered();
            }
            retry_pending = q.reset_generation == generation
                && !q.stopped
                && q.report.failure_stage.is_none()
                && !crate::video_output::skip_display_for_backlog(
                    q.bud_catch_up,
                    q.report.queue_depth,
                );
        }
        header_bytes = 0;
        if let Some(error) = failure {
            crate::event_log::record(
                "WARN",
                "video_decode_or_presentation_error",
                serde_json::json!({
                    "stage":error.stage,"hresult":error.hresult,"surface_generation":surface_generation,
                }),
            );
            if !matches!(
                error.stage,
                "video-process-input"
                    | "video-process-output"
                    | "video-output-iteration-limit"
                    | "video-vendor-process"
                    | "video-zero-copy-process"
            ) {
                return Err(error);
            }
            if error.stage == "video-zero-copy-process" {
                zero_failed = true;
                zero_fallback = shared
                    .0
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .report
                    .zero_copy_status
                    .clone();
            }
            if error.stage == "video-vendor-process" {
                vendor_failed = true;
                vendor_fallback = shared
                    .0
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .report
                    .decoder_fallback
                    .clone();
            }
            let mut q = shared.0.lock().unwrap_or_else(|e| e.into_inner());
            if q.reset_generation == generation && !q.recover(Instant::now()) {
                return Err(error);
            }
            continue;
        }
        // Offer fresh input first: never present an older pending backbuffer
        // ahead of a newer decoded output. Still service pending output when
        // this feed produced no picture, without waiting for an empty queue.
        if presentation_mode == PresentationMode::LowLatency && retry_pending {
            unsafe {
                session.as_mut().unwrap().retry_display(shared, window)?;
            }
        }
        publish_report(shared, window);
    }
    // Release the transform before the device manager, renderer and MF runtime.
    drop(session);
    Ok(())
}

// Shared by capability probing and the live renderer; adapter selection is identical.
unsafe fn create_device(
    preferred_gpu: Option<&crate::connection_settings::GpuPreference>,
) -> Result<(ID3D11Device, ID3D11DeviceContext)> {
    let chosen = if let Some(preferred) = preferred_gpu {
        let mut adapters = api("video-gpu-enumeration", crate::gpu_windows::enumerate())?;
        let identities: Vec<_> = adapters.iter().map(|a| a.preference.clone()).collect();
        let index = crate::gpu_windows::match_index(preferred, &identities).ok_or_else(|| {
            unavailable("video-selected-gpu-unavailable-select-automatic-in-settings")
        })?;
        Some(api(
            "video-gpu-interface",
            adapters.swap_remove(index).handle.cast::<IDXGIAdapter>(),
        )?)
    } else {
        None
    };
    let mut device = None;
    let mut context = None;
    api(
        "video-d3d11-device",
        D3D11CreateDevice(
            chosen.as_ref(),
            if chosen.is_some() {
                D3D_DRIVER_TYPE_UNKNOWN
            } else {
                D3D_DRIVER_TYPE_HARDWARE
            },
            HMODULE::default(),
            D3D11_CREATE_DEVICE_VIDEO_SUPPORT | D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        ),
    )?;
    let device = device.ok_or_else(|| unavailable("video-d3d11-null-device"))?;
    let context = context.ok_or_else(|| unavailable("video-d3d11-null-context"))?;
    Ok((device, context))
}

struct Runtime;
impl Runtime {
    unsafe fn start() -> Result<Self> {
        api(
            "video-com-start",
            CoInitializeEx(None, COINIT_MULTITHREADED).ok(),
        )?;
        if let Err(e) = api("video-mf-start", MFStartup(MF_VERSION, MFSTARTUP_FULL)) {
            CoUninitialize();
            return Err(e);
        }
        Ok(Self)
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        unsafe {
            let _ = MFShutdown();
            CoUninitialize();
        }
    }
}

#[derive(Clone, Copy)]
struct OutputFormat {
    source: RECT,
    color: u32,
    matrix: Option<u32>,
    nominal: Option<u32>,
}
impl OutputFormat {
    unsafe fn read(media: &IMFMediaType) -> Result<Self> {
        let size = api("video-output-size", media.GetUINT64(&MF_MT_FRAME_SIZE))?;
        let width = (size >> 32) as u32;
        let height = size as u32;
        if width == 0 || height == 0 || width > 8192 || height > 8192 {
            return Err(unavailable("video-output-invalid-size"));
        }
        let mut source = RECT {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        };
        let mut aperture = [0u8; std::mem::size_of::<MFVideoArea>()];
        let mut aperture_size = 0;
        if media
            .GetBlob(
                &MF_MT_MINIMUM_DISPLAY_APERTURE,
                &mut aperture,
                Some(&mut aperture_size),
            )
            .is_ok()
            && aperture_size as usize == aperture.len()
        {
            let area = std::ptr::read_unaligned(aperture.as_ptr().cast::<MFVideoArea>());
            if area.OffsetX.fract != 0 || area.OffsetY.fract != 0 {
                return Err(unavailable("video-fractional-aperture"));
            }
            let left = i32::from(area.OffsetX.value);
            let top = i32::from(area.OffsetY.value);
            let right = left
                .checked_add(area.Area.cx)
                .ok_or_else(|| unavailable("video-aperture-overflow"))?;
            let bottom = top
                .checked_add(area.Area.cy)
                .ok_or_else(|| unavailable("video-aperture-overflow"))?;
            if left < 0
                || top < 0
                || right <= left
                || bottom <= top
                || right > width as i32
                || bottom > height as i32
            {
                return Err(unavailable("video-invalid-aperture"));
            }
            source = RECT {
                left,
                top,
                right,
                bottom,
            };
        }
        let matrix = media.GetUINT32(&MF_MT_YUV_MATRIX).ok().filter(|v| *v != 0);
        let nominal = media
            .GetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE)
            .ok()
            .filter(|v| *v != 0);
        if matrix.is_some_and(|v| v > 2) || nominal.is_some_and(|v| v > 2) {
            return Err(unavailable("video-unsupported-color-space"));
        }
        let color = crate::video_color::processor_color(matrix, nominal);
        Ok(Self {
            source,
            color,
            matrix,
            nominal,
        })
    }
}

struct Session {
    hevc: Option<crate::video_stream::HevcConfiguration>,
    rotation: u8,
    in_flight: crate::video_output::InFlight,
    ten_bit: bool,
    surface_generation: u64,
    low_latency_request_accepted: bool,
    decoder: Option<IMFTransform>,
    #[cfg(feature = "vendor-video")]
    vendor: Option<parsec_gpu_video::Decoder>,
    #[cfg(feature = "vendor-video")]
    display: Option<parsec_gpu_video::display::Session>,
    zero_copy_status: Option<String>,
    fallback: Option<String>,
    _manager: Option<IMFDXGIDeviceManager>,
    renderer: Option<Renderer>,
    output_format: Option<OutputFormat>,
    _runtime: Option<Runtime>,
}
impl Session {
    fn pending_wait_handle(&self) -> Option<HANDLE> {
        let renderer = self.renderer.as_ref()?;
        if renderer.mode != PresentationMode::LowLatency
            || !renderer.pending_present
            || renderer.latency_slot
        {
            return None;
        }
        renderer.latency_event.as_ref().map(|event| event.0)
    }
    fn has_pending_display(&self) -> bool {
        self.renderer.as_ref().is_some_and(|r| r.pending_present)
    }
    unsafe fn retry_display(
        &mut self,
        shared: &Shared,
        window: &crate::window::Window,
    ) -> Result<bool> {
        let Some(renderer) = &mut self.renderer else {
            return Ok(false);
        };
        if !renderer.pending_present {
            return Ok(false);
        }
        let refreshed = match renderer.try_present(shared)? {
            Some(Presentation::Presented) => {
                update(shared, |s| {
                    s.frames_presented = s.frames_presented.saturating_add(1)
                });
                window.set_video_ready_for(self.surface_generation, true);
                true
            }
            Some(Presentation::NotVisible) => {
                update(shared, |s| s.presentations_not_visible += 1);
                true
            }
            _ => false,
        };
        Ok(refreshed)
    }
    fn renderer_name(&self) -> &'static str {
        #[cfg(feature = "vendor-video")]
        if self.display.is_some() {
            return "Original NV12 / Windows Presentation API";
        }
        "D3D11 VideoProcessor / DXGI"
    }
    fn adapter_name(&self) -> String {
        #[cfg(feature = "vendor-video")]
        if let Some(display) = &self.display {
            return display.adapter().into();
        }
        self.renderer.as_ref().unwrap().adapter.clone()
    }
    unsafe fn create(
        hwnd: HWND,
        verify_synthetic_pixels: bool,
        preferred_gpu: Option<&crate::connection_settings::GpuPreference>,
        codec: crate::video_stream::Codec,
        hevc: Option<crate::video_stream::HevcConfiguration>,
        surface_generation: u64,
        policy: (bool, bool, PresentationMode),
    ) -> Result<Self> {
        let (_allow_vendor, _zero_copy, presentation_mode) = policy;
        let mut fallback = None;
        let zero_copy_status =
            _zero_copy.then(|| "Unavailable: select a supported GPU and enable HEVC".into());
        #[cfg(feature = "vendor-video")]
        let mut zero_copy_status = zero_copy_status;
        #[cfg(feature = "vendor-video")]
        if _allow_vendor
            && codec == crate::video_stream::Codec::Hevc
            && hevc.is_some_and(|h| h.bit_depth == 8)
        {
            let candidate = (|| -> anyhow::Result<Option<Self>> {
                let config = hevc.ok_or_else(|| anyhow::anyhow!("missing HEVC parameters"))?;
                anyhow::ensure!(
                    config.bit_depth == 8
                        && matches!((config.profile, config.chroma), (1, 1) | (4, 3)),
                    "unsupported vendor color format"
                );
                let (probe, probe_context) =
                    create_device(preferred_gpu).map_err(|e| anyhow::anyhow!("{}", e.stage))?;
                let dxgi: IDXGIDevice = probe.cast()?;
                let adapter = dxgi.GetAdapter()?.GetDesc()?;
                let mode = if config.chroma == 3 {
                    if _zero_copy {
                        zero_copy_status = Some("4:4:4 uses normal GPU rendering".into());
                    }
                    parsec_gpu_video::Mode::DirectD3d11
                } else {
                    let Some(mode) = parsec_gpu_video::preferred(adapter.VendorId, true) else {
                        return Ok(None);
                    };
                    mode
                };
                let luid = (u64::from(adapter.AdapterLuid.HighPart as u32) << 32)
                    | u64::from(adapter.AdapterLuid.LowPart);
                drop((probe_context, probe));
                if _zero_copy
                    && config.chroma == 1
                    && parsec_gpu_video::original_surface_vendor_allowed(adapter.VendorId)
                {
                    match parsec_gpu_video::display::Session::open(luid, hwnd) {
                        Ok(display) => {
                            return Ok(Some(Self {
                                hevc,
                                rotation: 0,
                                in_flight: Default::default(),
                                ten_bit: false,
                                surface_generation,
                                low_latency_request_accepted: false,
                                decoder: None,
                                vendor: None,
                                display: Some(display),
                                zero_copy_status: Some(
                                    "Starting original NV12 surface presentation".into(),
                                ),
                                fallback: None,
                                _manager: None,
                                renderer: None,
                                output_format: None,
                                _runtime: None,
                            }))
                        }
                        Err(error) => zero_copy_status = Some(format!("Fallback: {error}")),
                    }
                }
                let vendor =
                    parsec_gpu_video::Decoder::open(mode, luid, (config.width, config.height))?;
                let device = vendor.native_device()?;
                let context = device.GetImmediateContext()?;
                let renderer = Renderer::create_on_device(hwnd, device, context, presentation_mode)
                    .map_err(|e| anyhow::anyhow!("{}", e.stage))?;
                Ok(Some(Self {
                    hevc,
                    rotation: 0,
                    in_flight: Default::default(),
                    ten_bit: false,
                    surface_generation,
                    low_latency_request_accepted: vendor.low_latency_request_accepted(),
                    decoder: None,
                    vendor: Some(vendor),
                    display: None,
                    zero_copy_status: zero_copy_status.clone(),
                    fallback: None,
                    _manager: None,
                    renderer: Some(renderer),
                    output_format: None,
                    _runtime: None,
                }))
            })();
            match candidate {
                Ok(Some(session)) => return Ok(session),
                Ok(None) => {}
                Err(error) => fallback = Some(format!("Using Media Foundation: {error}")),
            }
        }
        if !_allow_vendor && !verify_synthetic_pixels {
            fallback = Some("Using Media Foundation after vendor decoder recovery".into());
        }
        let runtime = Runtime::start()?;
        let mut renderer = Renderer::create(hwnd, preferred_gpu, presentation_mode)?;
        renderer.verify_synthetic_pixels = verify_synthetic_pixels;
        let mut token = 0;
        let mut manager = None;
        api(
            "video-dxgi-manager",
            MFCreateDXGIDeviceManager(&mut token, &mut manager),
        )?;
        let manager = manager.ok_or_else(|| unavailable("video-dxgi-manager-null"))?;
        api(
            "video-dxgi-reset",
            manager.ResetDevice(&renderer.device, token),
        )?;
        let ten_bit = hevc.is_some_and(|h| h.bit_depth == 10);
        if _zero_copy && ten_bit {
            zero_copy_status = Some("Unavailable for Main 10; using normal GPU rendering".into());
        }
        let decoder = create_decoder(codec, ten_bit)?;
        let attrs = api("video-decoder-attributes", decoder.GetAttributes())?;
        if api(
            "video-decoder-d3d11-aware",
            attrs.GetUINT32(&MF_SA_D3D11_AWARE),
        )? == 0
        {
            return Err(unavailable("video-decoder-not-d3d11-aware"));
        }
        // The documented Microsoft H.264 decoder property uses VT_UI4 (unlike
        // other codecs' VT_BOOL). Request through ICodecAPI; rejection is optional.
        let low_latency_request_accepted = decoder.cast::<ICodecAPI>().is_ok_and(|codec_api| {
            let value = VARIANT {
                Anonymous: VARIANT_0 {
                    Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                        vt: if codec == crate::video_stream::Codec::H264 {
                            VT_UI4
                        } else {
                            VT_BOOL
                        },
                        Anonymous: if codec == crate::video_stream::Codec::H264 {
                            VARIANT_0_0_0 { ulVal: 1 }
                        } else {
                            VARIANT_0_0_0 {
                                boolVal: windows::Win32::Foundation::VARIANT_TRUE,
                            }
                        },
                        ..Default::default()
                    }),
                },
            };
            codec_api
                .SetValue(&CODECAPI_AVLowLatencyMode, &value)
                .is_ok()
        });
        // Documented MFT alternative for HEVC decoders that do not accept
        // ICodecAPI. MF_LOW_LATENCY stores BOOL as UINT32. Preserve the
        // existing H.264 setup; acceptance is not a latency measurement.
        let low_latency_request_accepted = low_latency_request_accepted
            || (codec == crate::video_stream::Codec::Hevc
                && attrs.SetUINT32(&MF_LOW_LATENCY, 1).is_ok());
        api(
            "video-decoder-set-manager",
            decoder.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, manager.as_raw() as usize),
        )?;
        let input = api("video-input-type-create", MFCreateMediaType())?;
        api(
            "video-input-major",
            input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video),
        )?;
        api(
            "video-input-codec",
            input.SetGUID(
                &MF_MT_SUBTYPE,
                &match codec {
                    crate::video_stream::Codec::H264 => MFVideoFormat_H264,
                    crate::video_stream::Codec::Hevc => MFVideoFormat_HEVC_ES,
                },
            ),
        )?;
        if codec == crate::video_stream::Codec::Hevc {
            let config = hevc.ok_or_else(|| unavailable("video-hevc-sps-missing"))?;
            if config.chroma != 1
                || !matches!((config.profile, config.bit_depth), (1 | 2, 8) | (2, 10))
            {
                return Err(unavailable("video-hevc-requires-main-or-main10-420"));
            }
            api(
                "video-input-hevc-main",
                input.SetUINT32(&MF_MT_MPEG2_PROFILE, config.profile),
            )?;
            // Unlike the H.264 transform, HEVC may enumerate a placeholder NV12
            // type yet reject SetOutputType without known input geometry.
            api(
                "video-input-hevc-size",
                input.SetUINT64(
                    &MF_MT_FRAME_SIZE,
                    (u64::from(config.width) << 32) | u64::from(config.height),
                ),
            )?;
        }
        api("video-input-type", decoder.SetInputType(0, &input, 0))?;
        let mut result = Self {
            hevc,
            rotation: 0,
            in_flight: Default::default(),
            ten_bit,
            surface_generation,
            low_latency_request_accepted,
            decoder: Some(decoder),
            #[cfg(feature = "vendor-video")]
            vendor: None,
            #[cfg(feature = "vendor-video")]
            display: None,
            zero_copy_status,
            fallback,
            _manager: Some(manager),
            renderer: Some(renderer),
            output_format: None,
            _runtime: Some(runtime),
        };
        result.output_type()?;
        api(
            "video-begin-streaming",
            result
                .mf()?
                .ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0),
        )?;
        api(
            "video-start-stream",
            result
                .mf()?
                .ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0),
        )?;
        Ok(result)
    }
    fn mf(&self) -> Result<&IMFTransform> {
        self.decoder
            .as_ref()
            .ok_or_else(|| unavailable("video-mf-unavailable"))
    }
    fn name(&self, codec: crate::video_stream::Codec) -> &'static str {
        #[cfg(feature = "vendor-video")]
        if self.display.is_some() {
            return "Direct D3D11 HEVC";
        }

        #[cfg(feature = "vendor-video")]
        if let Some(vendor) = &self.vendor {
            return vendor.name();
        }
        match codec {
            crate::video_stream::Codec::H264 => "Media Foundation H.264",
            crate::video_stream::Codec::Hevc => "Media Foundation HEVC",
        }
    }
    unsafe fn output_type(&mut self) -> Result<()> {
        for index in 0..32 {
            let media = api(
                "video-output-enumerate",
                self.mf()?.GetOutputAvailableType(0, index),
            )?;
            if api("video-output-subtype", media.GetGUID(&MF_MT_SUBTYPE))?
                == if self.ten_bit {
                    MFVideoFormat_P010
                } else {
                    MFVideoFormat_NV12
                }
            {
                api(
                    if self.ten_bit {
                        "video-output-set-p010"
                    } else {
                        "video-output-set-nv12"
                    },
                    self.mf()?.SetOutputType(0, &media, 0),
                )?;
                // Geometry can remain unspecified until the first decoded sample.
                self.output_format = None;
                // A stream change may replace the decoder's texture pool.
                self.renderer.as_mut().unwrap().processor = None;
                let stream = api(
                    "video-output-stream-info",
                    self.mf()?.GetOutputStreamInfo(0),
                )?;
                if stream.dwFlags
                    & ((MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0
                        | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0) as u32)
                    == 0
                {
                    return Err(unavailable("video-output-requires-cpu-buffer"));
                }
                return Ok(());
            }
        }
        Err(unavailable(if self.ten_bit {
            "video-output-p010-unavailable"
        } else {
            "video-output-nv12-unavailable"
        }))
    }
    unsafe fn feed(
        &mut self,
        frame: &Frame,
        shared: &Shared,
        window: &crate::window::Window,
    ) -> Result<()> {
        self.in_flight.begin(frame.timestamp_100ns);
        #[cfg(feature = "vendor-video")]
        if let Some(display) = &mut self.display {
            let catching_up = {
                let q = shared.0.lock().unwrap_or_else(|e| e.into_inner());
                crate::video_output::skip_display_for_backlog(q.bud_catch_up, q.report.queue_depth)
            };
            let hwnd = window
                .ensure_video_surface()
                .map_err(|_| unavailable("video-surface-create"))?;
            let mut rect = windows_sys::Win32::Foundation::RECT::default();
            if windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd as _, &mut rect) == 0
            {
                return Err(unavailable("video-surface-size"));
            }
            let size = (
                (rect.right - rect.left).max(1) as u32,
                (rect.bottom - rect.top).max(1) as u32,
            );
            let result = display.feed(
                &frame.bytes,
                window.direct_presentation_allowed() && !catching_up,
                size,
                |layout| {
                    crate::viewport::Viewport {
                        source: (layout.width, layout.height),
                        client: size,
                    }
                    .rect()
                    .ok_or_else(|| anyhow::anyhow!("invalid viewport"))
                },
                |sequence, ready, dispatch, prepare, present| {
                    update(shared, |s| {
                        if let Some(age) = self.in_flight.ready(Some(sequence), None, ready) {
                            s.local_timing.decoder_residence.record(age);
                        }
                        s.local_timing.surface_dispatch.record(dispatch);
                        s.local_timing.presentation_prepare.record(prepare);
                        s.local_timing.present_call.record(present);
                        s.local_timing.output_service_ns = s
                            .local_timing
                            .output_service_ns
                            .saturating_add(ready.elapsed().as_nanos());
                    })
                },
            );
            let output = result.map_err(|error| {
                update(shared, |s| {
                    s.zero_copy_status =
                        Some(format!("Fallback: {error}; using the normal GPU renderer"))
                });
                unavailable("video-zero-copy-process")
            })?;
            update(shared, |s| {
                s.frames_submitted += 1;
                s.frames_decoded += output.decoded as u64;
                if catching_up {
                    s.display_updates_skipped += output.decoded as u64;
                }
                s.frames_presented += output.presented as u64;
                s.presentations_busy += output.busy as u64;
                if let Some(layout) = output.layout {
                    s.width = Some(layout.width);
                    s.height = Some(layout.height);
                    s.nominal_range = Some(if layout.full_range { 1 } else { 2 });
                    s.decoded_bit_depth = Some(8);
                    s.decoded_chroma = Some("4:2:0");
                    s.gpu_surface_output = true;
                    s.d3d11_decoder_surface = Some(true);
                }
                if output.presented > 0 {
                    s.zero_copy_status = Some(
                        "Original NV12 surfaces; no app conversion pass. Scanout not verified."
                            .into(),
                    );
                }
            });
            if output.presented > 0 {
                window.set_video_ready_for(self.surface_generation, true);
            }
            return Ok(());
        }
        #[cfg(feature = "vendor-video")]
        if let Some(vendor) = &mut self.vendor {
            let renderer = self.renderer.as_mut().unwrap();
            let generation = self.surface_generation;
            let mut consumed = 0usize;
            return vendor
                .feed(&frame.bytes, |picture| {
                    let ready = Instant::now();
                    if let Some(age) = self.in_flight.ready(Some(picture.sequence), None, ready) {
                        update(shared, |s| s.local_timing.decoder_residence.record(age));
                    }
                    consumed += 1;
                    let layout = picture.layout;
                    let source = RECT {
                        left: layout.left as i32,
                        top: layout.top as i32,
                        right: (layout.left + layout.width) as i32,
                        bottom: (layout.top + layout.height) as i32,
                    };
                    let color = crate::video_color::processor_color(
                        Some(1),
                        Some(if layout.full_range { 1 } else { 2 }),
                    );
                    let mut desc = D3D11_TEXTURE2D_DESC::default();
                    picture.texture.GetDesc(&mut desc);
                    anyhow::ensure!(
                        matches!(desc.Format, DXGI_FORMAT_NV12 | DXGI_FORMAT_AYUV)
                            && desc.CPUAccessFlags == 0,
                        "vendor returned an unsupported GPU texture"
                    );
                    let owner = picture.texture.GetDevice()?;
                    anyhow::ensure!(
                        owner.as_raw() == renderer.device.as_raw(),
                        "cross-device vendor output refused"
                    );
                    let presented = renderer
                        .present_ready(
                            picture.texture,
                            picture.slice,
                            source,
                            color,
                            shared,
                            window,
                            ready,
                        )
                        .map_err(|e| anyhow::anyhow!("{}", e.stage))?;
                    update(shared, |s| {
                        s.frames_decoded = s.frames_decoded.saturating_add(1);
                        s.gpu_surface_output = true;
                        s.d3d11_decoder_surface =
                            Some(desc.BindFlags & D3D11_BIND_DECODER.0 as u32 != 0);
                        let (w, h) = display_extents(layout.width, layout.height, self.rotation);
                        s.width = Some(w);
                        s.height = Some(h);
                        s.nominal_range = Some(if layout.full_range { 1 } else { 2 });
                        s.decoded_bit_depth = Some(8);
                        s.decoded_chroma = Some(if desc.Format == DXGI_FORMAT_AYUV {
                            "4:4:4"
                        } else {
                            "4:2:0"
                        });
                        match presented {
                            Presentation::Presented => {
                                s.frames_presented = s.frames_presented.saturating_add(1)
                            }
                            Presentation::Busy => {
                                s.presentations_busy = s.presentations_busy.saturating_add(1)
                            }
                            Presentation::NotVisible => {
                                s.presentations_not_visible =
                                    s.presentations_not_visible.saturating_add(1)
                            }
                        }
                    });
                    if presented == Presentation::Presented {
                        window.set_video_ready_for(generation, true);
                    }
                    Ok(())
                })
                .map(|decoded| {
                    update(shared, |s| {
                        s.frames_submitted = s.frames_submitted.saturating_add(1);
                        // VPL can skip display updates while keeping every
                        // compressed picture decoded and its references intact.
                        let skipped = decoded.saturating_sub(consumed) as u64;
                        s.frames_decoded = s.frames_decoded.saturating_add(skipped);
                        s.presentations_busy = s.presentations_busy.saturating_add(skipped);
                    })
                })
                .map_err(|error| {
                    update(shared, |s| {
                        s.decoder_fallback = Some(format!(
                            "Using Media Foundation after vendor decoder error: {error:#}"
                        ));
                    });
                    unavailable("video-vendor-process")
                });
        }
        let buffer = api(
            "video-input-buffer",
            MFCreateMemoryBuffer(frame.bytes.len() as u32),
        )?;
        let mut ptr = std::ptr::null_mut();
        api("video-input-lock", buffer.Lock(&mut ptr, None, None))?;
        if ptr.is_null() {
            let _ = buffer.Unlock();
            return Err(unavailable("video-input-null-buffer"));
        }
        std::ptr::copy_nonoverlapping(frame.bytes.as_ptr(), ptr, frame.bytes.len());
        api("video-input-unlock", buffer.Unlock())?;
        api(
            "video-input-length",
            buffer.SetCurrentLength(frame.bytes.len() as u32),
        )?;
        let sample = api("video-input-sample", MFCreateSample())?;
        api("video-input-add-buffer", sample.AddBuffer(&buffer))?;
        api(
            "video-input-timestamp",
            sample.SetSampleTime(frame.timestamp_100ns),
        )?;
        if frame.idr {
            api(
                "video-input-clean-point",
                sample.SetUINT32(&MFSampleExtension_CleanPoint, 1),
            )?;
        }
        let mut result = self.mf()?.ProcessInput(0, &sample, 0);
        if result
            .as_ref()
            .err()
            .is_some_and(|e| e.code() == MF_E_NOTACCEPTING)
        {
            self.output(shared, window)?;
            result = self.mf()?.ProcessInput(0, &sample, 0);
        }
        api("video-process-input", result)?;
        update(shared, |s| {
            s.frames_submitted = s.frames_submitted.saturating_add(1)
        });
        self.output(shared, window)
    }
    unsafe fn output(&mut self, shared: &Shared, window: &crate::window::Window) -> Result<()> {
        for _ in 0..64 {
            let mut out = MFT_OUTPUT_DATA_BUFFER {
                dwStreamID: 0,
                pSample: ManuallyDrop::new(None),
                dwStatus: 0,
                pEvents: ManuallyDrop::new(None),
            };
            let mut status = 0;
            let result = self
                .mf()?
                .ProcessOutput(0, std::slice::from_mut(&mut out), &mut status);
            // These COM fields are caller-owned on both success and failure.
            let sample = ManuallyDrop::take(&mut out.pSample);
            drop(ManuallyDrop::take(&mut out.pEvents));
            match result {
                Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(()),
                Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                    self.output_type()?;
                    continue;
                }
                other => api("video-process-output", other)?,
            }
            let sample = sample.ok_or_else(|| unavailable("video-output-null-sample"))?;
            let ready = Instant::now();
            if let Some(age) = self
                .in_flight
                .ready(None, sample.GetSampleTime().ok(), ready)
            {
                update(shared, |s| s.local_timing.decoder_residence.record(age));
            }
            let buffer = api("video-output-buffer", sample.GetBufferByIndex(0))?;
            let dxgi: IMFDXGIBuffer = api("video-output-not-dxgi", buffer.cast())?;
            let mut raw = std::ptr::null_mut();
            api(
                "video-output-texture",
                dxgi.GetResource(&ID3D11Texture2D::IID, &mut raw),
            )?;
            if raw.is_null() {
                return Err(unavailable("video-output-null-texture"));
            }
            let texture = ID3D11Texture2D::from_raw(raw);
            // Read metadata only: no mapping, readback or additional frame copy.
            // Microsoft documents BIND_DECODER as decoder-API output usage.
            // It is evidence of the DXVA path, not an execution counter.
            let mut texture_desc = D3D11_TEXTURE2D_DESC::default();
            texture.GetDesc(&mut texture_desc);
            let decoder_surface = texture_desc.BindFlags & D3D11_BIND_DECODER.0 as u32 != 0;
            let subresource = api("video-output-subresource", dxgi.GetSubresourceIndex())?;
            if self.output_format.is_none() {
                let media = api(
                    "video-output-current-type",
                    self.mf()?.GetOutputCurrentType(0),
                )?;
                self.output_format = Some(OutputFormat::read(&media)?);
            }
            let OutputFormat {
                source,
                color,
                matrix,
                nominal,
            } = self
                .output_format
                .ok_or_else(|| unavailable("video-output-format-missing"))?;
            update(shared, |s| {
                s.frames_decoded = s.frames_decoded.saturating_add(1);
                s.gpu_surface_output = true;
                s.d3d11_decoder_surface = Some(decoder_surface);
                s.decoded_chroma = Some("4:2:0");
                s.decoded_bit_depth = Some(if texture_desc.Format == DXGI_FORMAT_P010 {
                    10
                } else {
                    8
                });
                let (w, h) = display_extents(
                    (source.right - source.left) as u32,
                    (source.bottom - source.top) as u32,
                    self.rotation,
                );
                s.width = Some(w);
                s.height = Some(h);
                s.color_matrix = matrix;
                s.nominal_range = nominal;
            });
            let presented = self.renderer.as_mut().unwrap().present_ready(
                &texture,
                subresource,
                source,
                color,
                shared,
                window,
                ready,
            )?;
            update(shared, |s| {
                s.synthetic_pixel_variation_verified =
                    self.renderer.as_ref().unwrap().synthetic_pixels_verified
            });
            update(shared, |s| match presented {
                Presentation::Presented => {
                    s.frames_presented = s.frames_presented.saturating_add(1)
                }
                Presentation::Busy => s.presentations_busy = s.presentations_busy.saturating_add(1),
                Presentation::NotVisible => {
                    s.presentations_not_visible = s.presentations_not_visible.saturating_add(1)
                }
            });
            if presented == Presentation::Presented {
                window.set_video_ready_for(self.surface_generation, true);
            }
        }
        Err(unavailable("video-output-iteration-limit"))
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            if let Some(decoder) = &self.decoder {
                let _ = decoder.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0);
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Presentation {
    Presented,
    Busy,
    NotVisible,
}
fn presentation_status(status: windows::core::HRESULT) -> Result<Presentation> {
    if status == DXGI_ERROR_WAS_STILL_DRAWING {
        return Ok(Presentation::Busy);
    }
    api("video-present", status.ok())?;
    Ok(if status == windows::core::HRESULT(0) {
        Presentation::Presented
    } else {
        Presentation::NotVisible
    })
}

struct ProcessorCache {
    rotation_set: bool,
    enumeration: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
    key: (u32, u32, u32, u32, DXGI_FORMAT, u32),
    inputs: Vec<(ID3D11Texture2D, u32, ID3D11VideoProcessorInputView)>,
    outputs: Vec<(ID3D11Texture2D, ID3D11VideoProcessorOutputView)>,
}

// Flush deferred destruction only after the renderer's views/swap chain have
// been released. Required before recreating a flip swap chain on the same HWND.
struct ContextCleanup(ID3D11DeviceContext);
impl Drop for ContextCleanup {
    fn drop(&mut self) {
        unsafe {
            self.0.Flush();
        }
    }
}
struct Renderer {
    rotation: u8,
    rotation_context: Option<ID3D11VideoContext1>,
    timing_status: &'static str,
    mode: PresentationMode,
    swap_flags: DXGI_SWAP_CHAIN_FLAG,
    latency_event: Option<LatencyEvent>,
    latency_slot: bool,
    pending_present: bool,
    last_present: Option<Instant>,
    device: ID3D11Device,
    video_device: ID3D11VideoDevice,
    video_context: ID3D11VideoContext,
    swap: IDXGISwapChain1,
    hwnd: HWND,
    adapter: String,
    verify_synthetic_pixels: bool,
    synthetic_pixels_verified: bool,
    #[cfg(any(test, feature = "diagnostics"))]
    rgb10_ramp_levels: Option<usize>,
    size: (u32, u32),
    output_format: DXGI_FORMAT,
    ten_bit_attempted: bool,
    ten_bit_fallback: Option<String>,
    processor: Option<ProcessorCache>,
    #[cfg(any(test, not(feature = "native-core")))]
    overlay: Option<crate::overlay_windows::Renderer>,
    #[cfg(any(test, not(feature = "native-core")))]
    overlay_failed: bool,
    // Fields drop in declaration order; keep the final flush after all views.
    cleanup: ContextCleanup,
}

struct LatencyEvent(HANDLE);
impl Drop for LatencyEvent {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn rgb10_input_color(color: u32) -> DXGI_COLOR_SPACE_TYPE {
    match (color & 4 != 0, (color >> 4) & 3 == 2) {
        (true, true) => DXGI_COLOR_SPACE_YCBCR_FULL_G22_LEFT_P709,
        (true, false) => DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709,
        (false, true) => DXGI_COLOR_SPACE_YCBCR_FULL_G22_LEFT_P601,
        (false, false) => DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P601,
    }
}

fn rgb10_output_rejected(error: &Failure) -> bool {
    matches!(
        error.stage,
        "video-swapchain-resize"
            | "video-output-colorspace"
            | "video-processor-output-view"
            | "video-processor-null-output"
            | "video-processor-blt"
    ) && !matches!(
        error.hresult.as_deref(),
        Some("0x887A0005" | "0x887A0006" | "0x887A0007")
    )
}

unsafe fn supports_rgb10(enumeration: &ID3D11VideoProcessorEnumerator, color: u32) -> bool {
    let output = enumeration
        .CheckVideoProcessorFormat(DXGI_FORMAT_R10G10B10A2_UNORM)
        .is_ok_and(|flags| flags & D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_OUTPUT.0 as u32 != 0);
    output
        && enumeration
            .cast::<ID3D11VideoProcessorEnumerator1>()
            .is_ok_and(|e| {
                e.CheckVideoProcessorFormatConversion(
                    DXGI_FORMAT_P010,
                    rgb10_input_color(color),
                    DXGI_FORMAT_R10G10B10A2_UNORM,
                    DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709,
                )
                .is_ok_and(|supported| supported.as_bool())
            })
}

impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            self.cleanup.0.ClearState();
        }
    }
}
impl Renderer {
    unsafe fn create(
        hwnd: HWND,
        preferred_gpu: Option<&crate::connection_settings::GpuPreference>,
        mode: PresentationMode,
    ) -> Result<Self> {
        let (device, context) = create_device(preferred_gpu)?;
        Self::create_on_device(hwnd, device, context, mode)
    }
    unsafe fn create_on_device(
        hwnd: HWND,
        device: ID3D11Device,
        context: ID3D11DeviceContext,
        mode: PresentationMode,
    ) -> Result<Self> {
        let multithread: ID3D10Multithread = api("video-d3d11-multithread", device.cast())?;
        // Return value is the previous protection state, not success/failure.
        let _ = multithread.SetMultithreadProtected(true);
        let video_device = api("video-d3d11-video-device", device.cast())?;
        let video_context = api("video-d3d11-video-context", context.cast())?;
        let dxgi: IDXGIDevice = api("video-dxgi-device", device.cast())?;
        let adapter = api("video-adapter", dxgi.GetAdapter())?;
        let desc = api("video-adapter-desc", adapter.GetDesc())?;
        let adapter_name = String::from_utf16_lossy(&desc.Description)
            .trim_end_matches('\0')
            .to_owned();
        let factory: IDXGIFactory2 = api("video-dxgi-factory", adapter.GetParent())?;
        let size = (1024, 720);
        let mut desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: size.0,
            Height: size.1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            AlphaMode: DXGI_ALPHA_MODE_IGNORE,
            Flags: if mode == PresentationMode::Automatic {
                0
            } else {
                DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT.0 as u32
            },
            ..Default::default()
        };
        let mut fallback = false;
        let created = factory.CreateSwapChainForHwnd(&device, hwnd, &desc, None, None);
        // Optional timing control cannot make a previously working GPU fail.
        let created = if created.is_err() && desc.Flags != 0 {
            fallback = true;
            desc.Flags = 0;
            factory.CreateSwapChainForHwnd(&device, hwnd, &desc, None, None)
        } else {
            created
        };
        let swap = api("video-swapchain", created)?;
        let latency_event = if desc.Flags != 0 {
            swap.cast::<IDXGISwapChain2>().ok().and_then(|s| {
                let handle = s.GetFrameLatencyWaitableObject();
                let event = (!handle.is_invalid()).then(|| LatencyEvent(handle));
                if s.SetMaximumFrameLatency(1).is_ok() {
                    event
                } else {
                    None
                }
            })
        } else {
            None
        };
        fallback |= mode != PresentationMode::Automatic && latency_event.is_none();
        let timing_status = match (mode, fallback) {
            (PresentationMode::Automatic, _) => "Automatic",
            (PresentationMode::LowLatency, false) => "Low latency; maximum queued presentations: 1",
            (PresentationMode::Vsync, false) => "VSync; maximum queued presentations: 1",
            (PresentationMode::LowLatency, true) => "Low latency unavailable; automatic timing",
            (PresentationMode::Vsync, true) => "VSync; queue control unavailable",
        };
        if fallback {
            crate::event_log::record(
                "WARN",
                "video_presentation_timing_fallback",
                serde_json::json!({"requested":mode}),
            );
        }
        api(
            "video-disable-dxgi-alt-enter",
            factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER),
        )?;
        Ok(Self {
            rotation: 0,
            rotation_context: None,
            timing_status,
            mode,
            swap_flags: DXGI_SWAP_CHAIN_FLAG(desc.Flags as i32),
            latency_event,
            latency_slot: false,
            pending_present: false,
            last_present: None,
            device,
            video_device,
            video_context,
            swap,
            hwnd,
            adapter: adapter_name,
            verify_synthetic_pixels: false,
            synthetic_pixels_verified: false,
            #[cfg(any(test, feature = "diagnostics"))]
            rgb10_ramp_levels: None,
            size,
            output_format: DXGI_FORMAT_B8G8R8A8_UNORM,
            ten_bit_attempted: false,
            ten_bit_fallback: None,
            processor: None,
            #[cfg(any(test, not(feature = "native-core")))]
            overlay: None,
            #[cfg(any(test, not(feature = "native-core")))]
            overlay_failed: false,
            cleanup: ContextCleanup(context),
        })
    }
    // Offline synthetic fixture only. Live paths never map/read pixel buffers.
    #[cfg(any(test, feature = "diagnostics"))]
    unsafe fn verify_pixels(&self, back: &ID3D11Texture2D, size: (u32, u32)) -> Result<bool> {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        back.GetDesc(&mut desc);
        desc.Usage = D3D11_USAGE_STAGING;
        desc.BindFlags = 0;
        desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        desc.MiscFlags = 0;
        let mut staging = None;
        api(
            "video-probe-staging",
            self.device.CreateTexture2D(&desc, None, Some(&mut staging)),
        )?;
        let staging = staging.ok_or_else(|| unavailable("video-probe-staging-null"))?;
        let context = api("video-probe-context", self.device.GetImmediateContext())?;
        context.CopyResource(&staging, back);
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        api(
            "video-probe-map",
            context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)),
        )?;
        let valid = !mapped.pData.is_null() && mapped.RowPitch >= size.0 * 4;
        let overlay_verified = if valid {
            let p = (mapped.pData as *const u8)
                .add(((size.1 / 16) * mapped.RowPitch + (size.0 / 16) * 4) as usize);
            *p < 4 && *p.add(1) < 4 && *p.add(2) > 250
        } else {
            false
        };
        let mut colors = std::collections::BTreeSet::new();
        if valid {
            for row in 1..8u32 {
                for col in 1..8u32 {
                    let x = col * size.0 / 8;
                    let y = row * size.1 / 8;
                    let p = (mapped.pData as *const u8).add((y * mapped.RowPitch + x * 4) as usize);
                    colors.insert([*p, *p.add(1), *p.add(2)]);
                }
            }
        }
        context.Unmap(&staging, 0);
        Ok(valid && colors.len() > 8 && overlay_verified)
    }

    // Synthetic ramp only, read before Present discards the backbuffer. This
    // diagnostic is never enabled for decoded live/account video.
    #[cfg(any(test, feature = "diagnostics"))]
    unsafe fn count_rgb10_levels(&self, back: &ID3D11Texture2D) -> Result<usize> {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        back.GetDesc(&mut desc);
        desc.Usage = D3D11_USAGE_STAGING;
        desc.BindFlags = 0;
        desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        desc.MiscFlags = 0;
        let mut staging = None;
        api(
            "video-rgb10-probe-staging",
            self.device.CreateTexture2D(&desc, None, Some(&mut staging)),
        )?;
        let staging = staging.ok_or_else(|| unavailable("video-rgb10-probe-staging-null"))?;
        let context = &self.cleanup.0;
        context.CopyResource(&staging, back);
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        api(
            "video-rgb10-probe-map",
            context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)),
        )?;
        let mut levels = std::collections::BTreeSet::new();
        if !mapped.pData.is_null() && mapped.RowPitch >= desc.Width * 4 {
            let row = mapped
                .pData
                .cast::<u8>()
                .add((desc.Height / 2 * mapped.RowPitch) as usize);
            for x in 0..desc.Width as usize {
                let packed = std::ptr::read_unaligned(row.add(x * 4).cast::<u32>());
                levels.insert(packed & 1023);
            }
        }
        context.Unmap(&staging, 0);
        Ok(levels.len())
    }

    #[cfg(feature = "diagnostics")]
    unsafe fn present(
        &mut self,
        texture: &ID3D11Texture2D,
        subresource: u32,
        source: RECT,
        color: u32,
        shared: &Shared,
        window: &crate::window::Window,
    ) -> Result<Presentation> {
        self.present_ready(
            texture,
            subresource,
            source,
            color,
            shared,
            window,
            Instant::now(),
        )
    }
    #[allow(clippy::too_many_arguments)] // GPU surface and its observed CPU-ready boundary.
    unsafe fn present_ready(
        &mut self,
        texture: &ID3D11Texture2D,
        subresource: u32,
        source: RECT,
        color: u32,
        shared: &Shared,
        window: &crate::window::Window,
        ready: Instant,
    ) -> Result<Presentation> {
        let dispatched = Instant::now();
        let (skip, more_input) = {
            let mut q = shared.0.lock().unwrap_or_else(|e| e.into_inner());
            q.report
                .local_timing
                .surface_dispatch
                .record(dispatched.duration_since(ready));
            let skip =
                crate::video_output::skip_display_for_backlog(q.bud_catch_up, q.report.queue_depth);
            if skip {
                q.report.display_updates_skipped += 1;
            }
            (skip, q.report.queue_depth > 0)
        };
        // Reference decoding continues. No conversion/output write for stale
        // display updates; no waiting for a monitor on the decoder worker.
        let slot_ready = skip || self.poll_slot(shared);
        let result = if skip || (!slot_ready && more_input) {
            Ok(Presentation::Busy)
        } else {
            // Preserve the newest RGB output even if the display slot is busy.
            // Its later retry needs only Present, never another surface/copy.
            self.present_with_fallback(texture, subresource, source, color, shared, window)
        };
        update(shared, |s| {
            s.local_timing.output_service_ns = s
                .local_timing
                .output_service_ns
                .saturating_add(ready.elapsed().as_nanos())
        });
        result
    }
    unsafe fn poll_slot(&mut self, shared: &Shared) -> bool {
        if !self.latency_slot {
            if let Some(event) = &self.latency_event {
                match WaitForSingleObject(event.0, 0) {
                    WAIT_OBJECT_0 => self.latency_slot = true,
                    WAIT_TIMEOUT => {}
                    _ => self.disable_queue_control(shared),
                }
            }
        }
        self.latency_event.is_none() || self.latency_slot
    }
    fn disable_queue_control(&mut self, shared: &Shared) {
        self.latency_event = None;
        self.timing_status = "Presentation queue control unavailable";
        update(shared, |s| s.presentation_mode = Some(self.timing_status));
        crate::event_log::record(
            "WARN",
            "video_presentation_readiness_failed",
            serde_json::json!({}),
        );
    }
    unsafe fn try_present(&mut self, shared: &Shared) -> Result<Option<Presentation>> {
        if !self.poll_slot(shared) {
            return Ok(None);
        }
        let started = Instant::now();
        let status = self.swap.Present(
            u32::from(self.mode == PresentationMode::Vsync),
            DXGI_PRESENT_DO_NOT_WAIT,
        );
        update(shared, |s| {
            s.local_timing.present_call.record(started.elapsed());
            s.present_max_us = s
                .present_max_us
                .max(started.elapsed().as_micros().min(u64::MAX as u128) as u64);
        });
        let result = presentation_status(status)?;
        if result == Presentation::Presented {
            self.latency_slot = false;
            let now = Instant::now();
            if let Some(previous) = self.last_present.replace(now) {
                update(shared, |s| {
                    s.local_timing
                        .present_interval
                        .record(now.duration_since(previous))
                });
            }
        }
        self.pending_present = result == Presentation::Busy;
        Ok(Some(result))
    }
    unsafe fn present_with_fallback(
        &mut self,
        texture: &ID3D11Texture2D,
        subresource: u32,
        source: RECT,
        color: u32,
        shared: &Shared,
        window: &crate::window::Window,
    ) -> Result<Presentation> {
        self.ten_bit_attempted = false;
        let result = self.present_frame(texture, subresource, source, color, shared, window);
        // Retry only a rejected 10-bit output operation, never a lost device or
        // decoder error. The decoded P010 surface is retained; no re-decode/copy.
        match result {
            Err(error)
                if self.ten_bit_attempted
                    && self.ten_bit_fallback.is_none()
                    && rgb10_output_rejected(&error) =>
            {
                self.ten_bit_fallback = Some(format!(
                    "{}: {}",
                    error.stage,
                    error.hresult.as_deref().unwrap_or("unsupported")
                ));
                self.processor = None;
                self.present_frame(texture, subresource, source, color, shared, window)
            }
            other => other,
        }
    }

    unsafe fn present_frame(
        &mut self,
        texture: &ID3D11Texture2D,
        subresource: u32,
        source: RECT,
        color: u32,
        shared: &Shared,
        window: &crate::window::Window,
    ) -> Result<Presentation> {
        let prepare_started = Instant::now();
        let width = (source.right - source.left) as u32;
        let height = (source.bottom - source.top) as u32;
        let mut td = D3D11_TEXTURE2D_DESC::default();
        texture.GetDesc(&mut td);
        if !matches!(td.Format, DXGI_FORMAT_NV12 | DXGI_FORMAT_P010)
            || td.MipLevels != 1
            || td.ArraySize == 0
            || subresource >= td.ArraySize
            || width > td.Width
            || height > td.Height
        {
            return Err(unavailable("video-output-texture-layout"));
        }
        let mut rect = windows_sys::Win32::Foundation::RECT::default();
        if windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(self.hwnd.0, &mut rect) == 0 {
            return Err(unavailable("video-surface-size"));
        }
        let size = (
            (rect.right - rect.left).max(1) as u32,
            (rect.bottom - rect.top).max(1) as u32,
        );
        let key = (td.Width, td.Height, size.0, size.1, td.Format, color);
        if self.processor.as_ref().is_none_or(|p| p.key != key) {
            let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
                InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                InputFrameRate: DXGI_RATIONAL {
                    Numerator: 60,
                    Denominator: 1,
                },
                InputWidth: td.Width,
                InputHeight: td.Height,
                OutputFrameRate: DXGI_RATIONAL {
                    Numerator: 60,
                    Denominator: 1,
                },
                OutputWidth: size.0,
                OutputHeight: size.1,
                Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
            };
            let enumeration = api(
                "video-processor-enumerate",
                self.video_device.CreateVideoProcessorEnumerator(&desc),
            )?;
            let mut output_format = DXGI_FORMAT_B8G8R8A8_UNORM;
            if td.Format == DXGI_FORMAT_P010 && self.ten_bit_fallback.is_none() {
                if supports_rgb10(&enumeration, color) {
                    self.ten_bit_attempted = true;
                    output_format = DXGI_FORMAT_R10G10B10A2_UNORM;
                } else {
                    self.ten_bit_fallback = Some("P010 to RGB10 SDR conversion unavailable".into());
                }
            }
            if size != self.size || output_format != self.output_format {
                // Drop every backbuffer view and unbind the GUI render target
                // before changing size/precision. No intermediate RGB texture.
                self.processor = None;
                self.cleanup.0.ClearState();
                api(
                    "video-swapchain-resize",
                    self.swap
                        .ResizeBuffers(2, size.0, size.1, output_format, self.swap_flags),
                )?;
                self.size = size;
                self.output_format = output_format;
            }
            if output_format == DXGI_FORMAT_R10G10B10A2_UNORM {
                let swap: IDXGISwapChain3 = api("video-output-colorspace", self.swap.cast())?;
                if api(
                    "video-output-colorspace",
                    swap.CheckColorSpaceSupport(DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709),
                )? & DXGI_SWAP_CHAIN_COLOR_SPACE_SUPPORT_FLAG_PRESENT.0 as u32
                    == 0
                {
                    return Err(unavailable("video-output-colorspace"));
                }
                // Ten-bit SDR. Never label SDR samples PQ/HDR or set HDR metadata.
                api(
                    "video-output-colorspace",
                    swap.SetColorSpace1(DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709),
                )?;
            }
            let processor = api(
                "video-processor-create",
                self.video_device.CreateVideoProcessor(&enumeration, 0),
            )?;
            self.video_context
                .VideoProcessorSetStreamAutoProcessingMode(&processor, 0, false);
            update(shared, |s| {
                s.auto_processing_disabled = !self
                    .video_context
                    .VideoProcessorGetStreamAutoProcessingMode(&processor, 0)
                    .as_bool();
            });
            self.processor = Some(ProcessorCache {
                rotation_set: false,
                enumeration,
                processor,
                key,
                inputs: Vec::new(),
                outputs: Vec::new(),
            });
        }
        self.ten_bit_attempted |= self.output_format == DXGI_FORMAT_R10G10B10A2_UNORM;
        let cached = self.processor.as_mut().unwrap();
        let enumeration = &cached.enumeration;
        let processor = &cached.processor;
        let input_desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
            ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_VPIV {
                    MipSlice: 0,
                    ArraySlice: subresource,
                },
            },
            ..Default::default()
        };
        let input = if let Some((_, _, view)) = cached
            .inputs
            .iter()
            .find(|(resource, slice, _)| resource == texture && *slice == subresource)
        {
            view.clone()
        } else {
            let mut input = None;
            api(
                "video-processor-input-view",
                self.video_device.CreateVideoProcessorInputView(
                    texture,
                    enumeration,
                    &input_desc,
                    Some(&mut input),
                ),
            )?;
            let input = input.ok_or_else(|| unavailable("video-processor-null-input"))?;
            // Bound retained views even if a driver rotates its texture pool.
            if cached.inputs.len() == 32 {
                cached.inputs.remove(0);
            }
            cached
                .inputs
                .push((texture.clone(), subresource, input.clone()));
            update(shared, |s| s.input_views_created += 1);
            input
        };
        let back: ID3D11Texture2D = api("video-backbuffer", self.swap.GetBuffer(0))?;
        let output_desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
            ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
            },
        };
        let output = if let Some((_, view)) = cached
            .outputs
            .iter()
            .find(|(resource, _)| resource == &back)
        {
            view.clone()
        } else {
            let mut output = None;
            api(
                "video-processor-output-view",
                self.video_device.CreateVideoProcessorOutputView(
                    &back,
                    enumeration,
                    &output_desc,
                    Some(&mut output),
                ),
            )?;
            let output = output.ok_or_else(|| unavailable("video-processor-null-output"))?;
            if cached.outputs.len() == 2 {
                cached.outputs.remove(0);
            }
            cached.outputs.push((back.clone(), output.clone()));
            update(shared, |s| s.output_views_created += 1);
            output
        };
        // Preserve aspect ratio and letterbox on the GPU, without CPU pixels.
        let [left, top, right, bottom] = crate::viewport::Viewport {
            source: display_extents(width, height, self.rotation),
            client: size,
        }
        .rect()
        .ok_or_else(|| unavailable("video-viewport"))?;
        let target = RECT {
            left,
            top,
            right,
            bottom,
        };
        let full = RECT {
            left: 0,
            top: 0,
            right: size.0 as i32,
            bottom: size.1 as i32,
        };
        self.video_context
            .VideoProcessorSetOutputTargetRect(processor, true, Some(&full));
        self.video_context
            .VideoProcessorSetStreamSourceRect(processor, 0, true, Some(&source));
        self.video_context
            .VideoProcessorSetStreamDestRect(processor, 0, true, Some(&target));
        if self.rotation != 0 && !cached.rotation_set {
            if self.rotation_context.is_none() {
                self.rotation_context =
                    Some(api("video-rotation-interface", self.video_context.cast())?);
            }
            let context = self.rotation_context.as_ref().unwrap();
            let mut caps = D3D11_VIDEO_PROCESSOR_CAPS::default();
            api(
                "video-rotation-capabilities",
                enumeration.GetVideoProcessorCaps(&mut caps),
            )?;
            if caps.FeatureCaps & D3D11_VIDEO_PROCESSOR_FEATURE_CAPS_ROTATION.0 as u32 == 0 {
                return Err(unavailable("video-rotation-unsupported"));
            }
            let turn = match self.rotation {
                1 => D3D11_VIDEO_PROCESSOR_ROTATION_90,
                2 => D3D11_VIDEO_PROCESSOR_ROTATION_180,
                _ => D3D11_VIDEO_PROCESSOR_ROTATION_270,
            };
            context.VideoProcessorSetStreamRotation(processor, 0, true, turn);
            cached.rotation_set = true;
        }
        self.video_context.VideoProcessorSetStreamFrameFormat(
            processor,
            0,
            D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
        );
        // Standard video default: BT.709 limited input, full-range RGB output.
        self.video_context.VideoProcessorSetStreamColorSpace(
            processor,
            0,
            &D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: color },
        );
        self.video_context.VideoProcessorSetOutputColorSpace(
            processor,
            &D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: 0 },
        );
        let mut stream = D3D11_VIDEO_PROCESSOR_STREAM {
            Enable: true.into(),
            pInputSurface: ManuallyDrop::new(Some(input)),
            ..Default::default()
        };
        let result = self.video_context.VideoProcessorBlt(
            processor,
            &output,
            0,
            std::slice::from_ref(&stream),
        );
        ManuallyDrop::drop(&mut stream.pInputSurface);
        api("video-processor-blt", result)?;
        update(shared, |s| {
            s.display_rgb_bit_depth =
                Some(if self.output_format == DXGI_FORMAT_R10G10B10A2_UNORM {
                    10
                } else {
                    8
                });
            s.display_output_fallback = if td.Format == DXGI_FORMAT_P010 {
                self.ten_bit_fallback.clone()
            } else {
                None
            };
        });
        #[cfg(any(test, not(feature = "native-core")))]
        if !self.overlay_failed
            && !window
                .software_overlay_enabled
                .load(std::sync::atomic::Ordering::Acquire)
        {
            let frame = {
                window
                    .overlay
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .frame
                    .clone()
            };
            if !frame.batches.is_empty() {
                let result = (|| -> windows::core::Result<u64> {
                    if self.overlay.is_none() {
                        self.overlay =
                            Some(crate::overlay_windows::Renderer::create(&self.device)?);
                    }
                    self.overlay.as_mut().unwrap().draw(frame, &back, size)
                })();
                let mut overlay = window.overlay.lock().unwrap_or_else(|e| e.into_inner());
                match result {
                    Ok(count) => overlay.report.draw_calls_composited += count,
                    Err(_) => {
                        overlay.report.failure = Some("d3d11-gui-composition-failed");
                        self.overlay_failed = true;
                        self.overlay = None;
                    }
                }
            }
        }
        #[cfg(any(test, feature = "diagnostics"))]
        if self.verify_synthetic_pixels && !self.synthetic_pixels_verified {
            self.synthetic_pixels_verified = self.verify_pixels(&back, size)?;
        }
        #[cfg(any(test, feature = "diagnostics"))]
        if self.rgb10_ramp_levels.is_some() && self.output_format == DXGI_FORMAT_R10G10B10A2_UNORM {
            self.rgb10_ramp_levels = Some(self.count_rgb10_levels(&back)?);
        }
        // The converted backbuffer can be retried while input is idle. Never
        // retain a decoded surface or repeat conversion just for a busy Present.
        self.pending_present = true;
        update(shared, |s| {
            s.local_timing
                .presentation_prepare
                .record(prepare_started.elapsed());
        });
        Ok(self.try_present(shared)?.unwrap_or(Presentation::Busy))
    }
}

/// Offline synthetic NV12 surfaces: exercise all timing modes and queue flags,
/// including a resize. CPU timings cannot establish scanout latency gains.
#[cfg(feature = "diagnostics")]
pub fn probe_presentation_timing() -> anyhow::Result<serde_json::Value> {
    let window = crate::window::Window::create_video_probe()?;
    let outcome = (|| -> anyhow::Result<serde_json::Value> {
        let hwnd = window.ensure_video_surface()?;
        let mut cases = Vec::new();
        for (mode, bud) in [
            (PresentationMode::Automatic, false),
            (PresentationMode::LowLatency, false),
            (PresentationMode::Vsync, false),
            (PresentationMode::Automatic, true),
        ] {
            let result = unsafe {
                (|| -> Result<serde_json::Value> {
                    let mut renderer = Renderer::create(HWND(hwnd as *mut _), None, mode)?;
                    let shared = Arc::new((Mutex::new(Queue::default()), Condvar::new(), None));
                    let data = [128u8; 64 * 96];
                    let desc = D3D11_TEXTURE2D_DESC {
                        Width: 64,
                        Height: 64,
                        MipLevels: 1,
                        ArraySize: 1,
                        Format: DXGI_FORMAT_NV12,
                        SampleDesc: DXGI_SAMPLE_DESC {
                            Count: 1,
                            Quality: 0,
                        },
                        ..Default::default()
                    };
                    let initial = D3D11_SUBRESOURCE_DATA {
                        pSysMem: data.as_ptr().cast(),
                        SysMemPitch: 64,
                        ..Default::default()
                    };
                    let mut texture = None;
                    api(
                        "timing-fixture-texture",
                        renderer
                            .device
                            .CreateTexture2D(&desc, Some(&initial), Some(&mut texture)),
                    )?;
                    let texture = texture.ok_or_else(|| unavailable("timing-fixture-texture"))?;
                    let mut presented = 0;
                    let mut busy = 0;
                    for index in 0..48 {
                        if index == 24 {
                            windows_sys::Win32::UI::WindowsAndMessaging::SetWindowPos(
                                hwnd as _,
                                std::ptr::null_mut(),
                                0,
                                0,
                                800,
                                600,
                                windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOMOVE
                                    | windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOZORDER
                                    | windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE,
                            );
                        }
                        match renderer.present(
                            &texture,
                            0,
                            RECT {
                                left: 0,
                                top: 0,
                                right: 64,
                                bottom: 64,
                            },
                            crate::video_color::processor_color(Some(1), Some(2)),
                            &shared,
                            &window,
                        )? {
                            Presentation::Presented => presented += 1,
                            Presentation::Busy => busy += 1,
                            Presentation::NotVisible => {}
                        }
                        // Fixture cadence only; live decoder workers never sleep for display.
                        std::thread::sleep(Duration::from_millis(4));
                    }
                    let before = shared
                        .0
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .report
                        .local_timing
                        .presentation_prepare
                        .count;
                    {
                        let mut q = shared.0.lock().unwrap_or_else(|e| e.into_inner());
                        q.bud_catch_up = bud;
                        for index in 0..24 {
                            q.submit_codec(
                                vec![0],
                                index == 0,
                                false,
                                index,
                                crate::video_stream::Codec::H264,
                            );
                        }
                    }
                    let burst_started = Instant::now();
                    for _ in 0..24 {
                        let _ = shared.0.lock().unwrap_or_else(|e| e.into_inner()).pop();
                        renderer.present(
                            &texture,
                            0,
                            RECT {
                                left: 0,
                                top: 0,
                                right: 64,
                                bottom: 64,
                            },
                            crate::video_color::processor_color(Some(1), Some(2)),
                            &shared,
                            &window,
                        )?;
                    }
                    let burst_ms = burst_started.elapsed().as_secs_f64() * 1000.;
                    let prepared_before_retry = shared
                        .0
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .report
                        .local_timing
                        .presentation_prepare
                        .count;
                    let until = Instant::now() + Duration::from_millis(500);
                    let mut idle_retries = 0;
                    while renderer.pending_present && Instant::now() < until {
                        let _ = renderer.try_present(&shared)?;
                        idle_retries += 1;
                        if renderer.pending_present {
                            std::thread::sleep(Duration::from_millis(2));
                        }
                    }
                    let snapshot = shared
                        .0
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .report
                        .clone();
                    if snapshot.local_timing.presentation_prepare.count != prepared_before_retry {
                        return Err(unavailable("timing-fixture-repeated-conversion"));
                    }
                    let display_work = snapshot.local_timing.presentation_prepare.count - before;
                    if mode == PresentationMode::Automatic
                        && display_work != if bud { 2 } else { 24 }
                    {
                        return Err(unavailable("timing-fixture-burst-display-count"));
                    }
                    Ok(
                        serde_json::json!({"requested":mode,"effective":renderer.timing_status,
                    "bud_catch_up":bud,"burst_cpu_wall_ms":burst_ms,"burst_display_preparations":display_work,
                    "idle_retries":idle_retries,"pending_after_idle_retry":renderer.pending_present,"idle_retry_extra_conversions":0,
                    "burst_input_messages_consumed":24,"synthetic_surface_not_decoder":true,
                    "queue_control":renderer.latency_event.is_some(),"presented":presented,"busy":busy,
                    "local_timing":snapshot.local_timing}),
                    )
                })()
            };
            cases.push(match result {
                Ok(report) => report,
                Err(error) if matches!(error.stage, "timing-fixture-burst-display-count" | "timing-fixture-repeated-conversion") => {
                    anyhow::bail!("Synthetic display catch-up did not preserve its expected work count");
                }
                Err(error) => serde_json::json!({"requested":mode,"unavailable_stage":error.stage,"hresult":error.hresult}),
            });
        }
        Ok(
            serde_json::json!({"cases":cases,"cpu_readback":false,"live_video":false,
            "instrumentation_cost":crate::video_output::timing_cost_probe(),
            "gpu_execution_measured":false,"scanout_measured":false,"memory":crate::performance::own_memory()}),
        )
    })();
    window.request_stop();
    window.close();
    outcome
}

/// Synthetic surfaces prove precision before Present and 8/10-bit transitions.
/// Readback here never sees live/account video.
#[cfg(feature = "diagnostics")]
pub fn probe_rgb10_output() -> anyhow::Result<serde_json::Value> {
    let window = crate::window::Window::create_video_probe()?;
    let result = (|| -> anyhow::Result<serde_json::Value> {
        let hwnd = window.ensure_video_surface()?;
        unsafe {
            let mut renderer =
                Renderer::create(HWND(hwnd as *mut _), None, PresentationMode::Automatic)
                    .map_err(|e| anyhow::anyhow!("GPU RGB output probe: {e:?}"))?;
            renderer.rgb10_ramp_levels = Some(0);
            let shared = Arc::new((Mutex::new(Queue::default()), Condvar::new(), None));
            let source = RECT {
                left: 0,
                top: 0,
                right: 1024,
                bottom: 64,
            };
            let mut results = Vec::new();
            for (index, (ten_bit, limited)) in [
                (false, false),
                (true, false),
                (true, true),
                (false, false),
                (true, false),
            ]
            .into_iter()
            .enumerate()
            {
                if index == 4 {
                    renderer.ten_bit_fallback =
                        Some("Synthetic unsupported-output fallback".into());
                    renderer.processor = None;
                }
                let format = if ten_bit {
                    DXGI_FORMAT_P010
                } else {
                    DXGI_FORMAT_NV12
                };
                let pitch = if ten_bit { 2048 } else { 1024 };
                let mut data = vec![0u8; pitch * 96];
                for y in 0..96 {
                    for x in 0..1024 {
                        if ten_bit {
                            let sample = if y >= 64 {
                                512
                            } else if limited {
                                64 + x * 876 / 1023
                            } else {
                                x
                            };
                            let value = ((sample as u16) << 6).to_le_bytes();
                            data[y * pitch + x * 2..y * pitch + x * 2 + 2].copy_from_slice(&value);
                        } else {
                            data[y * pitch + x] = if y >= 64 { 128 } else { (x / 4) as u8 };
                        }
                    }
                }
                let desc = D3D11_TEXTURE2D_DESC {
                    Width: 1024,
                    Height: 64,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: format,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    ..Default::default()
                };
                let initial = D3D11_SUBRESOURCE_DATA {
                    pSysMem: data.as_ptr().cast(),
                    SysMemPitch: pitch as u32,
                    ..Default::default()
                };
                let mut texture = None;
                renderer
                    .device
                    .CreateTexture2D(&desc, Some(&initial), Some(&mut texture))?;
                renderer
                    .present(
                        &texture.unwrap(),
                        0,
                        source,
                        crate::video_color::processor_color(
                            Some(1),
                            Some(if limited { 2 } else { 1 }),
                        ),
                        &shared,
                        &window,
                    )
                    .map_err(|e| anyhow::anyhow!("RGB output probe: {e:?}"))?;
                let snapshot = shared
                    .0
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .report
                    .clone();
                let rgb10 = snapshot.display_rgb_bit_depth == Some(10);
                if ten_bit && index != 4 && rgb10 {
                    anyhow::ensure!(
                        renderer.rgb10_ramp_levels.is_some_and(|n| n > 256),
                        "RGB10 output must retain more than 256 distinct ramp levels"
                    );
                }
                if !ten_bit || index == 4 {
                    anyhow::ensure!(
                        snapshot.display_rgb_bit_depth == Some(8),
                        "8-bit output/fallback failed"
                    );
                }
                results.push(
                    serde_json::json!({"p010_input":ten_bit,"limited_range":limited,
                    "forced_fallback":index == 4,"rgb_bit_depth":snapshot.display_rgb_bit_depth,
                    "fallback":snapshot.display_output_fallback,
                    "distinct_red_levels":if rgb10 { renderer.rgb10_ramp_levels } else { None }}),
                );
            }
            Ok(
                serde_json::json!({"scope":"synthetic-rgb10-output","cases":results,
                "external_requests_enabled":false,"real_account_used":false,
                "synthetic_readback_only":true,"cpu_readback_live_enabled":false}),
            )
        }
    })();
    window.close();
    result
}

/// No account, network or real content: create and release the actual GPU path.
#[cfg(feature = "diagnostics")]
pub fn probe(sustained: bool) -> anyhow::Result<serde_json::Value> {
    use std::sync::atomic::Ordering;
    let window = crate::window::Window::create_video_probe()?;
    // Test a real alpha-blended D3D11 GUI pass over GPU-decoded video.
    let texture = Arc::new(crate::overlay::Texture {
        version: 1,
        width: 1,
        height: 1,
        rgba: vec![255; 4],
    });
    let mut vertices = Vec::new();
    for (x, y) in [
        (-1.0f32, 1.0f32),
        (-0.75, 1.0),
        (-0.75, 0.75),
        (-1.0, 1.0),
        (-0.75, 0.75),
        (-1.0, 0.75),
    ] {
        vertices.extend_from_slice(&x.to_le_bytes());
        vertices.extend_from_slice(&y.to_le_bytes());
        vertices.extend_from_slice(&[0; 8]);
        vertices.extend_from_slice(&[255, 0, 0, 255]);
    }
    let client = *window.dimensions.lock().unwrap_or_else(|e| e.into_inner());
    window
        .overlay
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .frame = Arc::new(crate::overlay::Frame {
        size: (client.0 as u32, client.1 as u32),
        batches: vec![crate::overlay::Batch {
            vertices,
            texture,
            clip: [0, 0, client.0, client.1],
        }],
    });
    let pipeline = Pipeline::start_mode(window.clone(), true, false);
    let fixture = include_bytes!("../fixtures/synthetic-1920x1080.h264");
    let mut starts = Vec::new();
    for i in 0..fixture.len().saturating_sub(4) {
        if fixture[i..].starts_with(&[0, 0, 0, 1]) && fixture[i + 4] & 31 == 9 {
            starts.push(i);
        }
    }
    anyhow::ensure!(starts.len() == 8, "synthetic fixture access unit count");
    starts.push(fixture.len());
    let mut inspector = crate::video_stream::Inspector::default();
    let cycles = if sustained { 128 } else { 1 };
    'feeding: for cycle in 0..cycles {
        if sustained && cycle == cycles / 2 {
            // Exercise release of cached backbuffer views before ResizeBuffers.
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::SetWindowPos(
                    window.handle(),
                    std::ptr::null_mut(),
                    0,
                    0,
                    900,
                    640,
                    windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOMOVE
                        | windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOZORDER
                        | windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE,
                );
            }
        }
        for pair in starts.windows(2) {
            let bytes = &fixture[pair[0]..pair[1]];
            let info = inspector
                .receive(bytes)
                .ok_or_else(|| anyhow::anyhow!("invalid synthetic Annex B fixture"))?;
            pipeline.submit(bytes, info);
            std::thread::sleep(if sustained {
                Duration::from_micros(8333)
            } else {
                Duration::from_millis(40)
            });
            if pipeline.snapshot().failure_stage.is_some() {
                break 'feeding;
            }
        }
    }
    let framing = inspector.snapshot();
    anyhow::ensure!(
        framing.idr_messages > 0 && framing.sps_profile_idc == Some(100),
        "synthetic high-profile reference-picture fixture"
    );
    let until = Instant::now() + Duration::from_secs(12);
    while Instant::now() < until {
        let report = pipeline.snapshot();
        if report.failure_stage.is_some()
            || (report.frames_decoded == cycles * 8
                && report.frames_presented > 0
                && report.synthetic_pixel_variation_verified)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    pipeline.stop();
    let until = Instant::now() + Duration::from_secs(3);
    while !pipeline.snapshot().worker_finished && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
    let report = pipeline.snapshot();
    let released = window.active_contexts.load(Ordering::Acquire) == 0;
    if released {
        window.close();
    }
    let until = Instant::now() + Duration::from_secs(1);
    while !window.handle().is_null() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(
        serde_json::json!({"scope":"synthetic-native-h264-d3d11", "external_requests_enabled":false,"real_account_used":false,
        "cpu_readback_live_enabled":false,"synthetic_overlay":window.overlay.lock().unwrap_or_else(|e|e.into_inner()).report.clone(),"sustained":sustained,"synthetic_fixture_frames":cycles*8,"synthetic_fixture_idr_frames":framing.idr_messages,"video":report,
        "native_window_released":window.handle().is_null(),"gpu_resources_released":released}),
    )
}

#[cfg(feature = "diagnostics")]
pub fn probe_recovery() -> anyhow::Result<serde_json::Value> {
    // Hosted Windows runners expose D3D11 without the video interface. Report
    // this precise capability limitation; never skip an actual decoder failure.
    let (device, capability_context) = unsafe { create_device(None) }
        .map_err(|e| anyhow::anyhow!("recovery capability preflight: {}", e.stage))?;
    if let Err(error) = device.cast::<ID3D11VideoDevice>() {
        if error.code() == windows::Win32::Foundation::E_NOINTERFACE {
            return Ok(serde_json::json!({"scope":"synthetic-native-h264-recovery",
                "supported":false,"reason":"D3D11 video interface unavailable",
                "external_requests_enabled":false,"real_account_used":false}));
        }
        return Err(error.into());
    }
    drop((device, capability_context));
    let window = crate::window::Window::create_video_probe().unwrap();
    let pipeline = Pipeline::start(window.clone());
    let fixture = include_bytes!("../fixtures/synthetic-1920x1080.h264");
    let mut starts: Vec<_> = (0..fixture.len().saturating_sub(4))
        .filter(|&i| fixture[i..].starts_with(&[0, 0, 0, 1]) && fixture[i + 4] & 31 == 9)
        .collect();
    starts.push(fixture.len());
    let mut inspector = crate::video_stream::Inspector::default();
    let result = (|| -> anyhow::Result<()> {
        for pair in starts.windows(2) {
            let bytes = &fixture[pair[0]..pair[1]];
            anyhow::ensure!(pipeline.submit(bytes, inspector.receive(bytes).unwrap()));
            std::thread::sleep(Duration::from_millis(40));
        }
        let until = Instant::now() + Duration::from_secs(5);
        while pipeline.snapshot().frames_decoded < 8 && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        anyhow::ensure!(
            pipeline.snapshot().frames_decoded == 8,
            "initial synthetic decode"
        );
        anyhow::ensure!(pipeline.shared.0.lock().unwrap().recover(Instant::now()));
        pipeline.shared.1.notify_one();
        anyhow::ensure!(pipeline.take_keyframe_request().is_some());
        for (index, pair) in starts.windows(2).enumerate() {
            let mut bytes = &fixture[pair[0]..pair[1]];
            if index == 0 {
                // Omit SPS/PPS deliberately: only the saved parameters can
                // initialize the new decoder before this IDR picture.
                let idr = (0..bytes.len().saturating_sub(4))
                    .find(|&i| {
                        (bytes[i..].starts_with(&[0, 0, 0, 1]) && bytes[i + 4] & 31 == 5)
                            || (bytes[i..].starts_with(&[0, 0, 1]) && bytes[i + 3] & 31 == 5)
                    })
                    .ok_or_else(|| anyhow::anyhow!("synthetic IDR missing"))?;
                bytes = &bytes[idr..];
            }
            anyhow::ensure!(pipeline.submit(bytes, inspector.receive(bytes).unwrap()));
            std::thread::sleep(Duration::from_millis(40));
        }
        let until = Instant::now() + Duration::from_secs(5);
        while pipeline.snapshot().frames_decoded < 16 && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        let report = pipeline.snapshot();
        anyhow::ensure!(
            report.failure_stage.is_none(),
            "recovery failed: {:?}",
            report.failure_stage
        );
        anyhow::ensure!(
            report.frames_decoded == 16 && !report.recovering,
            "recovery did not decode all frames"
        );
        Ok(())
    })();
    pipeline.stop();
    let until = Instant::now() + Duration::from_secs(3);
    while !pipeline.snapshot().worker_finished && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
    window.close();
    result?;
    anyhow::ensure!(
        pipeline.snapshot().resources_released,
        "recovery resource cleanup"
    );
    Ok(
        serde_json::json!({"scope":"synthetic-native-h264-recovery", "supported":true,
        "external_requests_enabled":false, "real_account_used":false,
        "cpu_readback_live_enabled":false, "video":pipeline.snapshot()}),
    )
}

#[cfg(feature = "diagnostics")]
pub fn probe_main10() -> anyhow::Result<serde_json::Value> {
    if !hevc_ten_bit_available(None) {
        return Ok(
            serde_json::json!({"scope":"synthetic-main10-gpu","supported":false,
            "reason":"Main 10/P010 GPU decode and conversion unavailable"}),
        );
    }
    let window = crate::window::Window::create_video_probe()?;
    let pipeline = Pipeline::start(window.clone());
    let mut inspector = crate::video_stream::Inspector::default();
    inspector.enable_hevc(true);
    let result = (|| -> anyhow::Result<()> {
        let mut fixture: &[u8] = include_bytes!("../fixtures/synthetic-720p-hevc10.bin");
        let mut count = 0u64;
        while !fixture.is_empty() {
            anyhow::ensure!(fixture.len() >= 4, "truncated fixture size");
            let len = u32::from_le_bytes(fixture[..4].try_into()?) as usize;
            fixture = &fixture[4..];
            anyhow::ensure!(len <= fixture.len(), "truncated fixture unit");
            let bytes = &fixture[..len];
            fixture = &fixture[len..];
            if count == 0 {
                let sps = crate::video_stream::hevc_configuration(bytes)
                    .ok_or_else(|| anyhow::anyhow!("Main 10 fixture SPS missing"))?;
                anyhow::ensure!(
                    sps.profile == 2 && sps.bit_depth == 10 && sps.chroma == 1,
                    "fixture must be Main 10 4:2:0"
                );
            }
            anyhow::ensure!(
                pipeline.submit(
                    bytes,
                    inspector
                        .receive(bytes)
                        .ok_or_else(|| anyhow::anyhow!("fixture inspection failed"))?
                ),
                "fixture submission failed"
            );
            count += 1;
            std::thread::sleep(Duration::from_millis(33));
        }
        let until = Instant::now() + Duration::from_secs(5);
        while pipeline.snapshot().frames_decoded < count.saturating_sub(1) && Instant::now() < until
        {
            if pipeline.snapshot().failure_stage.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let report = pipeline.snapshot();
        anyhow::ensure!(
            report.failure_stage.is_none(),
            "Main 10 GPU pipeline failed: {:?}",
            report.failure_stage
        );
        // The live streaming MFT may retain one picture until the next input;
        // this test does not inject an EOS/drain into the live pipeline.
        anyhow::ensure!(
            report.frames_decoded >= count.saturating_sub(1)
                && report.decoded_bit_depth == Some(10)
                && report.gpu_surface_output,
            "Main 10 must produce P010 GPU output without dropping the stream"
        );
        Ok(())
    })();
    pipeline.stop();
    let until = Instant::now() + Duration::from_secs(3);
    while !pipeline.snapshot().worker_finished && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
    window.close();
    result?;
    anyhow::ensure!(
        pipeline.snapshot().resources_released,
        "Main 10 resources not released"
    );
    Ok(
        serde_json::json!({"scope":"synthetic-main10-gpu","supported":true,
        "pixel_readback":false,"external_requests_enabled":false,"video":pipeline.snapshot()}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn display_wait_prioritizes_input_without_consuming_the_display_signal() {
        let wake = WorkerWake::create().expect("input event");
        let display = LatencyEvent(unsafe { CreateEventW(None, false, false, None).unwrap() });
        assert!(wake.arm());
        wake.notify();
        unsafe {
            SetEvent(display.0).unwrap();
            assert_eq!(wake.wait(display.0).unwrap(), DisplayWake::Input);
            assert!(wake.arm());
            assert_eq!(wake.wait(display.0).unwrap(), DisplayWake::Display);
            // The worker caches this consumed signal in latency_slot instead
            // of polling the auto-reset object a second time and losing it.
            assert_eq!(WaitForSingleObject(display.0, 0), WAIT_TIMEOUT);
        }
    }
    #[test]
    fn new_input_interrupts_an_unsignalled_display_wait() {
        let wake = Arc::new(WorkerWake::create().expect("input event"));
        let display = LatencyEvent(unsafe { CreateEventW(None, false, false, None).unwrap() });
        assert!(wake.arm());
        let producer = wake.clone();
        let thread = std::thread::spawn(move || producer.notify());
        // The typed result distinguishes a queue wake from the safety timeout.
        assert_eq!(unsafe { wake.wait(display.0).unwrap() }, DisplayWake::Input);
        thread.join().unwrap();
        assert_eq!(unsafe { WaitForSingleObject(display.0, 0) }, WAIT_TIMEOUT);
    }
    #[test]
    fn rgb10_fallback_never_swallows_device_loss_or_decoder_errors() {
        let rejected = api::<()>(
            "video-processor-blt",
            Err(windows::Win32::Foundation::E_INVALIDARG.into()),
        )
        .unwrap_err();
        assert!(rgb10_output_rejected(&rejected));
        for status in [
            DXGI_ERROR_DEVICE_REMOVED,
            DXGI_ERROR_DEVICE_HUNG,
            DXGI_ERROR_DEVICE_RESET,
        ] {
            let error = api::<()>("video-processor-blt", Err(status.into())).unwrap_err();
            assert!(!rgb10_output_rejected(&error));
        }
        assert!(!rgb10_output_rejected(&unavailable("video-process-output")));
        assert!(!rgb10_output_rejected(&unavailable(
            "video-processor-input-view"
        )));
        assert!(rgb10_output_rejected(&unavailable(
            "video-output-colorspace"
        )));
    }
    #[test]
    fn rgb10_conversion_query_preserves_matrix_and_nominal_range() {
        for (matrix, nominal, expected) in [
            (1, 1, DXGI_COLOR_SPACE_YCBCR_FULL_G22_LEFT_P709),
            (1, 2, DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709),
            (2, 1, DXGI_COLOR_SPACE_YCBCR_FULL_G22_LEFT_P601),
            (2, 2, DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P601),
        ] {
            assert_eq!(
                rgb10_input_color(crate::video_color::processor_color(
                    Some(matrix),
                    Some(nominal)
                )),
                expected
            );
        }
    }
    #[cfg(feature = "diagnostics")]
    #[test]
    fn decoder_recovers_with_cached_parameters_and_a_headerless_keyframe() {
        let report = probe_recovery().unwrap();
        if report["supported"] == false {
            eprintln!("GPU recovery fixture unavailable: {}", report["reason"]);
            return;
        }
        assert_eq!(report["video"]["frames_decoded"], 16);
        assert_eq!(report["video"]["recovering"], false);
    }
    #[test]
    fn busy_or_occluded_presentation_is_not_a_decoder_failure() {
        assert_eq!(
            presentation_status(windows::core::HRESULT(0)).unwrap(),
            Presentation::Presented
        );
        assert_eq!(
            presentation_status(DXGI_ERROR_WAS_STILL_DRAWING).unwrap(),
            Presentation::Busy
        );
        assert_eq!(
            presentation_status(windows::Win32::Foundation::DXGI_STATUS_OCCLUDED).unwrap(),
            Presentation::NotVisible
        );
        let error = presentation_status(DXGI_ERROR_DEVICE_REMOVED).unwrap_err();
        assert_eq!(error.stage, "video-present");
        assert!(error.hresult.is_some());
    }
}

pub(crate) fn display_extents(width: u32, height: u32, rotation: u8) -> (u32, u32) {
    if rotation & 1 != 0 {
        (height, width)
    } else {
        (width, height)
    }
}
