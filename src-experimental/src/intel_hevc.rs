//! Offline HEVC route identification. No account, network or live-client change.
//! A played frame is NOT automatically classified as hardware decoded.
use anyhow::{Context, Result, ensure};
use lowlat_drivers::d3d11::D3d11;
use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::{
            Direct3D::*, Direct3D10::ID3D10Multithread, Direct3D11::*, DirectComposition::*,
            Dxgi::*,
        },
        Media::MediaFoundation::*,
        System::{Com::StructuredStorage::*, Com::*, LibraryLoader::*},
        UI::WindowsAndMessaging::*,
    },
    core::{BSTR, Interface, implement, w},
};
mod counters;
use counters::DecodeCounter;

#[derive(Default)]
struct Events {
    ready: AtomicBool,
    error: Mutex<Option<(usize, u32)>>,
}
#[implement(IMFMediaEngineNotify)]
struct Notify(Arc<Events>);
impl IMFMediaEngineNotify_Impl for Notify_Impl {
    fn EventNotify(&self, event: u32, param1: usize, param2: u32) -> windows::core::Result<()> {
        if event == MF_MEDIA_ENGINE_EVENT_CANPLAY.0 as u32 {
            self.0.ready.store(true, Ordering::Release);
        } else if event == MF_MEDIA_ENGINE_EVENT_ERROR.0 as u32 {
            *self.0.error.lock().unwrap_or_else(|e| e.into_inner()) = Some((param1, param2));
        }
        Ok(())
    }
}

struct Runtime;
impl Runtime {
    fn start() -> Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
            if let Err(error) = MFStartup(MF_VERSION, MFSTARTUP_FULL) {
                CoUninitialize();
                return Err(error.into());
            }
            Ok(Self)
        }
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
struct Window(HWND);
impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}
unsafe extern "system" fn procedure(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}
fn window() -> Result<Window> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = w!("ParsecIntelHevcOfflineProbe");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: instance.into(),
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassW(&wc);
        Ok(Window(CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP,
            class,
            w!("Offline HEVC Media Engine test - synthetic video"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            960,
            600,
            None,
            None,
            Some(instance.into()),
            None,
        )?))
    }
}

struct Engine {
    engine: IMFMediaEngine,
    composition: Option<IDCompositionDesktopDevice>,
    target: Option<IDCompositionTarget>,
    visual: Option<IDCompositionVisual2>,
    handle: Option<HANDLE>,
}
impl Drop for Engine {
    fn drop(&mut self) {
        unsafe {
            if let Some(target) = &self.target {
                let _ = target.SetRoot(None);
            }
            if let Some(composition) = &self.composition {
                let _ = composition.Commit();
            }
            let _ = self.engine.Shutdown();
            if let Some(handle) = self.handle.take() {
                let _ = CloseHandle(handle);
            }
        }
    }
}

unsafe fn device(luid: u64) -> Result<ID3D11Device> {
    unsafe {
        let factory: IDXGIFactory4 = CreateDXGIFactory1()?;
        let adapter: IDXGIAdapter = factory.EnumAdapterByLuid(LUID {
            LowPart: luid as u32,
            HighPart: (luid >> 32) as i32,
        })?;
        let mut device = None;
        D3D11CreateDevice(
            &adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_VIDEO_SUPPORT
                | D3D11_CREATE_DEVICE_BGRA_SUPPORT
                | D3D11_CREATE_DEVICE_PREVENT_INTERNAL_THREADING_OPTIMIZATIONS,
            Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            None,
        )?;
        let device = device.context("D3D11 device missing")?;
        let protection: ID3D10Multithread = device.cast()?;
        let _ = protection.SetMultithreadProtected(true);
        ensure!(
            protection.GetMultithreadProtected().as_bool(),
            "D3D11 multithread protection unavailable"
        );
        Ok(device)
    }
}
unsafe fn statistic(engine: &IMFMediaEngineEx, kind: MF_MEDIA_ENGINE_STATISTIC) -> Result<u32> {
    unsafe {
        let mut value = engine.GetStatistics(kind)?;
        let number = PropVariantToUInt32(&value);
        PropVariantClear(&mut value)?;
        Ok(number?)
    }
}

fn engine_trial(luid: u64, fixture: &std::path::Path, realtime: bool) -> Value {
    let mut stage = "COM/MF startup";
    let result = (|| -> Result<Value> {
        let _runtime = Runtime::start()?;
        stage = "window creation";
        let window = window()?;
        stage = "D3D11 device creation (Chromium Media Engine flags)";
        let device = unsafe { device(luid)? };
        let mut token = 0;
        let mut manager = None;
        stage = "DXGI manager creation/reset";
        unsafe {
            MFCreateDXGIDeviceManager(&mut token, &mut manager)?;
        }
        let manager = manager.context("DXGI manager missing")?;
        unsafe {
            manager.ResetDevice(&device, token)?;
        }
        let events = Arc::new(Events::default());
        let notify: IMFMediaEngineNotify = Notify(events.clone()).into();
        let mut attrs = None;
        unsafe {
            MFCreateAttributes(&mut attrs, 4)?;
        }
        let attrs = attrs.context("Media Engine attributes missing")?;
        unsafe {
            attrs.SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &notify)?;
            attrs.SetUnknown(&MF_MEDIA_ENGINE_DXGI_MANAGER, &manager)?;
            attrs.SetUINT64(&MF_MEDIA_ENGINE_OPM_HWND, window.0.0 as usize as u64)?;
        }
        stage = "Media Engine creation";
        let factory: IMFMediaEngineClassFactory = unsafe {
            CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_INPROC_SERVER)?
        };
        let engine = unsafe { factory.CreateInstance(0, &attrs)? };
        let mut owner = Engine {
            engine,
            composition: None,
            target: None,
            visual: None,
            handle: None,
        };
        let ex: IMFMediaEngineEx = owner.engine.cast()?;
        stage = "windowless swapchain / realtime mode";
        unsafe {
            ex.EnableWindowlessSwapchainMode(true)?;
            ex.SetRealTimeMode(realtime)?;
            owner.engine.SetMuted(true)?;
            owner.engine.SetLoop(true)?;
        }
        let mut counter = DecodeCounter::open(luid);
        stage = "local synthetic source loading";
        unsafe {
            owner
                .engine
                .SetSource(&BSTR::from(fixture.to_string_lossy().as_ref()))?;
            owner.engine.Load()?;
        }
        let began = Instant::now();
        let mut playing: Option<Instant> = None;
        let mut last_sample = Instant::now();
        let mut peak_frames = 0;
        while began.elapsed() < Duration::from_secs(20) {
            unsafe {
                let mut msg = MSG::default();
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                ensure!(IsWindow(Some(window.0)).as_bool(), "test window closed");
            }
            if let Some((code, hr)) = *events.error.lock().unwrap_or_else(|e| e.into_inner()) {
                anyhow::bail!("Media Engine error category {code}, HRESULT 0x{hr:08x}");
            }
            if events.ready.load(Ordering::Acquire) && playing.is_none() {
                stage = "DirectComposition swapchain attachment";
                unsafe {
                    let handle = ex.GetVideoSwapchainHandle()?;
                    owner.handle = Some(handle);
                    let dxgi: IDXGIDevice = device.cast()?;
                    let composition: IDCompositionDesktopDevice = DCompositionCreateDevice2(&dxgi)?;
                    let target = composition.CreateTargetForHwnd(window.0, true)?;
                    let visual = composition.CreateVisual()?;
                    let content = composition.CreateSurfaceFromHandle(handle)?;
                    visual.SetContent(&content)?;
                    target.SetRoot(&visual)?;
                    owner.composition = Some(composition);
                    owner.target = Some(target);
                    owner.visual = Some(visual);
                    let mut rect = RECT::default();
                    GetClientRect(window.0, &mut rect)?;
                    ex.UpdateVideoStream(None, Some(&rect), None)?;
                    owner.composition.as_ref().unwrap().Commit()?;
                    stage = "Media Engine playback";
                    owner.engine.Play()?;
                }
                playing = Some(Instant::now());
            }
            if last_sample.elapsed() >= Duration::from_millis(200) {
                // Preserve a witnessed snapshot when the short source loops;
                // do not invent a cumulative count by summing counter resets.
                if let Ok(frames) =
                    unsafe { statistic(&ex, MF_MEDIA_ENGINE_STATISTIC_FRAMES_RENDERED) }
                {
                    peak_frames = peak_frames.max(frames);
                }
                if let Some(counter) = &mut counter {
                    counter.sample();
                }
                last_sample = Instant::now();
            }
            if playing.is_some_and(|start| start.elapsed() >= Duration::from_secs(8)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        ensure!(playing.is_some(), "CANPLAY deadline expired");
        stage = "decoded/rendered frame statistics";
        let (frames, dropped) = unsafe {
            (
                statistic(&ex, MF_MEDIA_ENGINE_STATISTIC_FRAMES_RENDERED)?,
                statistic(&ex, MF_MEDIA_ENGINE_STATISTIC_FRAMES_DROPPED)?,
            )
        };
        let (mut width, mut height) = (0, 0);
        unsafe {
            owner
                .engine
                .GetNativeVideoSize(Some(&mut width), Some(&mut height))?;
        }
        ensure!(
            (width, height) == (1280, 720),
            "unexpected decoded geometry {width}x{height}"
        );
        peak_frames = peak_frames.max(frames);
        Ok(
            json!({"played":peak_frames>=60,"frames_rendered_last_snapshot":frames,
            "frames_rendered_peak_snapshot":peak_frames,"frames_dropped_last_snapshot":dropped,"size":[width,height],
            "decode_engine":counter.as_ref().map(|counter|counter.report(peak_frames)),
            "cpu_pixel_readback":false,"app_decoded_texture_copy":false,
            "gpu_internal_copies":"unknown", "latency_improvement_verified":false}),
        )
    })();
    json!({"path":"media_engine_directcomposition", "realtime":realtime,
        "result":match result{Ok(value)=>value,Err(error)=>json!({"failed":format!("{error:#}"),"stage":stage})}})
}

pub fn probe() -> Result<Value> {
    let control = std::env::args().any(|arg| arg == "--control");
    let _runtime = Runtime::start()?;
    let d3d = D3d11::load()?;
    let fixture = std::env::current_exe()?.with_file_name("synthetic-720p-hevc.mp4");
    // Offline-only exact fixture; arbitrary media/URLs are not accepted.
    let expected = include_bytes!("../fixtures/synthetic-720p-hevc.mp4");
    if fixture.exists() {
        ensure!(
            std::fs::read(&fixture)?.as_slice() == expected,
            "existing fixture differs; extract this probe into a separate directory"
        );
    } else {
        std::fs::write(&fixture, expected)?;
    }
    let mut adapters = Vec::new();
    for adapter in d3d
        .adapters()?
        .into_iter()
        .filter(|a| a.decodes_here() && (a.vendor == 0x8086 || control))
    {
        eprintln!(
            "Testing {} (separate synthetic video windows)",
            adapter.description
        );
        let luid = adapter.luid.value();
        let baseline = crate::gpu::intel_hevc_baseline(luid);
        let trials = [
            engine_trial(luid, &fixture, false),
            engine_trial(luid, &fixture, true),
        ];
        adapters.push(json!({"adapter":adapter.description,"vendor":adapter.vendor,"luid":adapter.luid.to_string(),"driver":adapter.driver,
            "baseline":match baseline{Ok(value)=>value,Err(error)=>json!({"failed":format!("{error:#}")})},"media_engine":trials}));
    }
    Ok(
        json!({"scope":"offline HEVC Main 8-bit 4:2:0 route identification", "control_gpu_run":control,
        "adapters":adapters, "live_client_modified":false,"network_used":false,
        "note":"A control GPU does not prove Intel compatibility. Played frames without process/adapter-scoped VideoDecode activity do not establish hardware decoding. No latency or global zero-copy claim."}),
    )
}

/// The baseline decoder stays on its COM owner thread. PDH sampling alone runs
/// separately so blocking decode calls cannot hide the hardware engine activity.
pub(crate) fn observe_trial(
    luid: u64,
    trial: impl FnOnce() -> Result<Value>,
) -> (Result<Value>, Value) {
    use std::sync::{atomic::AtomicU32, mpsc};
    let stop = AtomicBool::new(false);
    let frames = AtomicU32::new(0);
    struct Stop<'a>(&'a AtomicBool);
    impl Drop for Stop<'_> {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }
    std::thread::scope(|scope| {
        let (ready, wait) = mpsc::sync_channel(1);
        let observed = (&stop, &frames);
        let observer = scope.spawn(move || {
            let mut counter = DecodeCounter::open(luid);
            let _ = ready.send(());
            while !observed.0.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(200));
                if let Some(counter) = &mut counter {
                    counter.sample();
                }
            }
            counter
                .as_ref()
                .map_or(Value::Null, |c| c.report(observed.1.load(Ordering::Relaxed)))
        });
        let guard = Stop(&stop);
        let _ = wait.recv();
        let result = trial();
        frames.store(
            result
                .as_ref()
                .ok()
                .and_then(|r| r["frames"].as_u64())
                .unwrap_or(0)
                .min(u64::from(u32::MAX)) as u32,
            Ordering::Relaxed,
        );
        drop(guard);
        (result, observer.join().unwrap_or(Value::Null))
    })
}
