//! Documented Media Foundation H.264/HEVC -> D3D11 NV12/P010 -> DXGI presentation.
//! COM and all GPU objects are created, used and released on one worker thread.
use crate::video_output::{Frame, Queue, Snapshot};
use std::{
    mem::ManuallyDrop,
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
use windows::{
    core::Interface,
    Win32::{
        Foundation::{HMODULE, HWND, RECT},
        Graphics::{
            Direct3D::*,
            Direct3D10::ID3D10Multithread,
            Direct3D11::*,
            Dxgi::{Common::*, *},
        },
        Media::MediaFoundation::*,
        System::{Com::*, Variant::*},
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

type Shared = Arc<(Mutex<Queue>, Condvar)>;
pub struct Pipeline {
    shared: Shared,
    started: Instant,
    window: Arc<crate::window::Window>,
    surface_generation: u64,
}
impl Pipeline {
    pub fn start(window: Arc<crate::window::Window>) -> Self {
        Self::start_mode(window, false)
    }
    fn start_mode(window: Arc<crate::window::Window>, verify_synthetic_pixels: bool) -> Self {
        let surface_generation = window.begin_video();
        let surface_window = window.clone();
        let shared = Arc::new((Mutex::new(Queue::default()), Condvar::new()));
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
                    )
                }));
                let mut q = state.0.lock().unwrap_or_else(|e| e.into_inner());
                match outcome {
                    Ok(Err(failure)) => q.fail(failure.stage, failure.hresult),
                    Err(_) => q.fail("video-worker-panic", None),
                    Ok(Ok(())) => {}
                }
                q.clear();
                q.stopped = true;
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
                q.submit_codec(buffer, info.idr, info.parameters, timestamp, info.codec)
            }
        };
        self.shared.1.notify_one();
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
fn run(
    shared: &Shared,
    window: &Arc<crate::window::Window>,
    verify_synthetic_pixels: bool,
    preferred_gpu: Option<&crate::connection_settings::GpuPreference>,
    surface_generation: u64,
) -> Result<()> {
    let scheduling = crate::media_scheduling::Registration::video();
    update(shared, |s| s.mmcss_registered = scheduling.is_some());
    let mut session = None;
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
        let (next_generation, frame) = {
            let mut q = shared.0.lock().unwrap_or_else(|e| e.into_inner());
            while q.report.queue_depth == 0
                && !q.stopped
                && q.report.failure_stage.is_none()
                && q.reset_generation == generation
                && !window.closing.load(std::sync::atomic::Ordering::Acquire)
            {
                q = shared
                    .1
                    .wait_timeout(q, Duration::from_millis(100))
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
                q.check_recovery(Instant::now());
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
                    codec,
                    bytes,
                    idr: false,
                    parameters: true,
                    timestamp_100ns: 0,
                });
            }
        }
        let Some(mut frame) = frame else {
            continue;
        };
        if frame.parameters {
            parameters.observe(&frame.bytes, frame.codec);
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
            let native = unsafe {
                Session::create(
                    HWND(hwnd as *mut _),
                    verify_synthetic_pixels,
                    preferred_gpu,
                    frame.codec,
                    hevc,
                    surface_generation,
                    (
                        !vendor_failed && !verify_synthetic_pixels,
                        zero_requested && !zero_failed,
                    ),
                )?
            };
            update(shared, |s| {
                s.decoder_initialized = true;
                s.low_latency_request_accepted = native.low_latency_request_accepted;
                s.decoder = Some(native.name(frame.codec));
                s.decoder_fallback = if vendor_failed {
                    vendor_fallback.clone().or_else(|| native.fallback.clone())
                } else {
                    native.fallback.clone()
                };
                s.renderer = Some(native.renderer_name());
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
        for frame in headers.drain(..).chain(std::iter::once(frame)) {
            let feed_started = Instant::now();
            let decoded_before = shared
                .0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .report
                .frames_decoded;
            let result = unsafe { session.as_mut().unwrap().feed(&frame, shared, window) };
            update(shared, |s| {
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
        *window
            .video_report
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(
            shared
                .0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .report
                .clone(),
        );
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
        policy: (bool, bool),
    ) -> Result<Self> {
        let (_allow_vendor, _zero_copy) = policy;
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
                    config.profile == 1 && config.bit_depth == 8 && config.chroma == 1,
                    "unsupported vendor color format"
                );
                let (probe, probe_context) =
                    create_device(preferred_gpu).map_err(|e| anyhow::anyhow!("{}", e.stage))?;
                let dxgi: IDXGIDevice = probe.cast()?;
                let adapter = dxgi.GetAdapter()?.GetDesc()?;
                let Some(mode) = parsec_gpu_video::preferred(adapter.VendorId, true) else {
                    return Ok(None);
                };
                let luid = (u64::from(adapter.AdapterLuid.HighPart as u32) << 32)
                    | u64::from(adapter.AdapterLuid.LowPart);
                drop((probe_context, probe));
                if _zero_copy && parsec_gpu_video::original_surface_vendor_allowed(adapter.VendorId)
                {
                    match parsec_gpu_video::display::Session::open(luid, hwnd) {
                        Ok(display) => {
                            return Ok(Some(Self {
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
                let renderer = Renderer::create_on_device(hwnd, device, context)
                    .map_err(|e| anyhow::anyhow!("{}", e.stage))?;
                Ok(Some(Self {
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
        let mut renderer = Renderer::create(hwnd, preferred_gpu)?;
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
        #[cfg(feature = "vendor-video")]
        if let Some(display) = &mut self.display {
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
                window.direct_presentation_allowed(),
                size,
                |layout| {
                    crate::viewport::Viewport {
                        source: (layout.width, layout.height),
                        client: size,
                    }
                    .rect()
                    .ok_or_else(|| anyhow::anyhow!("invalid viewport"))
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
                s.frames_presented += output.presented as u64;
                s.presentations_busy += output.busy as u64;
                if let Some(layout) = output.layout {
                    s.width = Some(layout.width);
                    s.height = Some(layout.height);
                    s.nominal_range = Some(if layout.full_range { 1 } else { 2 });
                    s.decoded_bit_depth = Some(8);
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
                        desc.Format == DXGI_FORMAT_NV12 && desc.CPUAccessFlags == 0,
                        "vendor returned a non-GPU NV12 texture"
                    );
                    let owner = picture.texture.GetDevice()?;
                    anyhow::ensure!(
                        owner.as_raw() == renderer.device.as_raw(),
                        "cross-device vendor output refused"
                    );
                    let presented = renderer
                        .present(
                            picture.texture,
                            picture.slice,
                            source,
                            color,
                            shared,
                            window,
                        )
                        .map_err(|e| anyhow::anyhow!("{}", e.stage))?;
                    update(shared, |s| {
                        s.frames_decoded = s.frames_decoded.saturating_add(1);
                        s.gpu_surface_output = true;
                        s.d3d11_decoder_surface =
                            Some(desc.BindFlags & D3D11_BIND_DECODER.0 as u32 != 0);
                        s.width = Some(layout.width);
                        s.height = Some(layout.height);
                        s.nominal_range = Some(if layout.full_range { 1 } else { 2 });
                        s.decoded_bit_depth = Some(8);
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
                s.decoded_bit_depth = Some(if texture_desc.Format == DXGI_FORMAT_P010 {
                    10
                } else {
                    8
                });
                s.width = Some((source.right - source.left) as u32);
                s.height = Some((source.bottom - source.top) as u32);
                s.color_matrix = matrix;
                s.nominal_range = nominal;
            });
            let presented = self.renderer.as_mut().unwrap().present(
                &texture,
                subresource,
                source,
                color,
                shared,
                window,
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
    enumeration: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
    key: (u32, u32, u32, u32),
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
    device: ID3D11Device,
    video_device: ID3D11VideoDevice,
    video_context: ID3D11VideoContext,
    swap: IDXGISwapChain1,
    hwnd: HWND,
    adapter: String,
    verify_synthetic_pixels: bool,
    synthetic_pixels_verified: bool,
    size: (u32, u32),
    processor: Option<ProcessorCache>,
    overlay: Option<crate::overlay_windows::Renderer>,
    overlay_failed: bool,
    // Fields drop in declaration order; keep the final flush after all views.
    cleanup: ContextCleanup,
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
    ) -> Result<Self> {
        let (device, context) = create_device(preferred_gpu)?;
        Self::create_on_device(hwnd, device, context)
    }
    unsafe fn create_on_device(
        hwnd: HWND,
        device: ID3D11Device,
        context: ID3D11DeviceContext,
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
        let desc = DXGI_SWAP_CHAIN_DESC1 {
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
            ..Default::default()
        };
        let swap = api(
            "video-swapchain",
            factory.CreateSwapChainForHwnd(&device, hwnd, &desc, None, None),
        )?;
        api(
            "video-disable-dxgi-alt-enter",
            factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER),
        )?;
        Ok(Self {
            device,
            video_device,
            video_context,
            swap,
            hwnd,
            adapter: adapter_name,
            verify_synthetic_pixels: false,
            synthetic_pixels_verified: false,
            size,
            processor: None,
            overlay: None,
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

    unsafe fn present(
        &mut self,
        texture: &ID3D11Texture2D,
        subresource: u32,
        source: RECT,
        color: u32,
        shared: &Shared,
        window: &crate::window::Window,
    ) -> Result<Presentation> {
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
        if size != self.size {
            // ResizeBuffers requires all cached backbuffer views to be released.
            self.processor = None;
            api(
                "video-swapchain-resize",
                self.swap.ResizeBuffers(
                    2,
                    size.0,
                    size.1,
                    DXGI_FORMAT_UNKNOWN,
                    DXGI_SWAP_CHAIN_FLAG(0),
                ),
            )?;
            self.size = size;
        }
        let key = (td.Width, td.Height, size.0, size.1);
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
                enumeration,
                processor,
                key,
                inputs: Vec::new(),
                outputs: Vec::new(),
            });
        }
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
            source: (width, height),
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
        // Never pace reference-picture decoding against display refresh. A busy
        // swap chain skips only this display submission, not encoded pictures.
        let started = Instant::now();
        let status = self.swap.Present(0, DXGI_PRESENT_DO_NOT_WAIT);
        update(shared, |s| {
            s.present_max_us = s
                .present_max_us
                .max(started.elapsed().as_micros().min(u64::MAX as u128) as u64)
        });
        presentation_status(status)
    }
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
    let pipeline = Pipeline::start_mode(window.clone(), true);
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
