//! Isolated VPL original-surface presentation gate. No copy fallback.
use super::*;
use parsec_gpu_video::Layout;
use std::{ffi::c_void, mem::size_of, rc::Rc};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::{
            CompositionSwapchain::*, Direct3D::*, Direct3D10::ID3D10Multithread,
            DirectComposition::*, Dxgi::IDXGIDevice,
        },
        System::LibraryLoader::*,
        UI::WindowsAndMessaging::*,
    },
    core::{GUID, HRESULT, IUnknown, s, w},
};
struct DisplayDevice {
    native: ID3D11Device,
}

type Initialize = unsafe extern "C" fn(mfxInitializationParam, *mut mfxSession) -> mfxStatus;
type Close = unsafe extern "C" fn(mfxSession) -> mfxStatus;
type SetHandle = unsafe extern "C" fn(mfxSession, mfxHandleType, mfxHDL) -> mfxStatus;
type Header = unsafe extern "C" fn(mfxSession, *mut mfxBitstream, *mut mfxVideoParam) -> mfxStatus;
type Init = unsafe extern "C" fn(mfxSession, *mut mfxVideoParam) -> mfxStatus;
type Decode = unsafe extern "C" fn(
    mfxSession,
    *mut mfxBitstream,
    *mut mfxFrameSurface1,
    *mut *mut mfxFrameSurface1,
    *mut mfxSyncPoint,
) -> mfxStatus;
type Sync = unsafe extern "C" fn(mfxSession, mfxSyncPoint, u32) -> mfxStatus;
struct RawSession {
    raw: mfxSession,
    close: Close,
}
impl Drop for RawSession {
    fn drop(&mut self) {
        unsafe {
            (self.close)(self.raw);
        }
    }
}
struct Surface(*mut mfxFrameSurface1);
impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            if let Some(interface) = (*self.0).__bindgen_anon_1.FrameInterface.as_ref()
                && let Some(release) = interface.Release
            {
                release(self.0);
            }
        }
    }
}
fn check(status: mfxStatus, operation: &str) -> Result<()> {
    ensure!(status >= 0, "{operation}: VPL status {status}");
    Ok(())
}
fn bits(unit: &[u8], timestamp: u64) -> mfxBitstream {
    let mut bits: mfxBitstream = unsafe { std::mem::zeroed() };
    bits.Data = unit.as_ptr().cast_mut();
    bits.DataLength = unit.len() as u32;
    bits.MaxLength = bits.DataLength;
    bits.DataFlag = MFX_BITSTREAM_COMPLETE_FRAME as u16;
    bits.TimeStamp = timestamp;
    bits
}
unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // Keep the test window alive until all queued presentation leases retire.
    if message == WM_CLOSE {
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, message, w, l) }
}
fn pump() {
    unsafe {
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

pub(super) fn trial(probe_device: &Device, index: u32, units: &[&[u8]]) -> Result<Value> {
    // Vpl::for_adapter is retained by our caller and has loaded the selected
    // driver's current runtime by absolute driver-store path. No DLL search or
    // alternative software runtime is performed here. These are public VPL APIs.
    unsafe {
        let module = GetModuleHandleW(w!("libmfx64-gen.dll"))?;
        macro_rules! entry {
            ($name:literal, $ty:ty) => {{
                let address = GetProcAddress(module, s!($name)).context($name)?;
                std::mem::transmute::<unsafe extern "system" fn() -> isize, $ty>(address)
            }};
        }
        let initialize = entry!("MFXInitialize", Initialize);
        let close = entry!("MFXClose", Close);
        let set_handle = entry!("MFXVideoCORE_SetHandle", SetHandle);
        let header = entry!("MFXVideoDECODE_DecodeHeader", Header);
        let query = entry!("MFXVideoDECODE_Query", InitQuery);
        let init = entry!("MFXVideoDECODE_Init", Init);
        let decode = entry!("MFXVideoDECODE_DecodeFrameAsync", Decode);
        let sync = entry!("MFXVideoCORE_SyncOperation", Sync);
        let raw = probe_device.device().cast();
        let probe = ID3D11Device::from_raw_borrowed(&raw).context("probe device")?;
        let dxgi: IDXGIDevice = probe.cast()?;
        let adapter = dxgi.GetAdapter()?;
        let mut native = None;
        D3D11CreateDevice(
            &adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_VIDEO_SUPPORT
                | D3D11_CREATE_DEVICE_BGRA_SUPPORT
                | D3D11_CREATE_DEVICE_PREVENT_INTERNAL_THREADING_OPTIMIZATIONS,
            None,
            D3D11_SDK_VERSION,
            Some(&mut native),
            None,
            None,
        )?;
        let device = DisplayDevice {
            native: native.context("presentation D3D11 device")?,
        };
        let multithread: ID3D10Multithread = device.native.cast()?;
        let _ = multithread.SetMultithreadProtected(true);
        let mut initialization: mfxInitializationParam = std::mem::zeroed();
        initialization.AccelerationMode = MFX_ACCEL_MODE_VIA_D3D11;
        initialization.VendorImplID = index;
        let mut raw = std::ptr::null_mut();
        check(initialize(initialization, &mut raw), "MFXInitialize")?;
        ensure!(!raw.is_null(), "null VPL session");
        let session = RawSession { raw, close };
        check(
            set_handle(raw, MFX_HANDLE_D3D11_DEVICE, device.native.as_raw()),
            "SetHandle",
        )?;
        let mut param: mfxVideoParam = std::mem::zeroed();
        param.__bindgen_anon_1.mfx.CodecId = MFX_CODEC_HEVC as u32;
        check(
            header(raw, &mut bits(units[0], 1), &mut param),
            "DecodeHeader",
        )?;
        param.IOPattern = MFX_IOPATTERN_OUT_VIDEO_MEMORY as u16;
        param.AsyncDepth = 1;
        check(query(raw, &mut param, &mut param), "DecodeQuery")?;
        check(init(raw, &mut param), "DecodeInit")?;
        let instance = GetModuleHandleW(None)?;
        let class = w!("ParsecIntelOriginalSurfaceTest");
        ensure!(
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance.into(),
                lpszClassName: class,
                ..Default::default()
            }) != 0,
            "register test window"
        );
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class,
            w!("Intel VPL original NV12 - experimental test"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            960,
            600,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        let result = (|| -> Result<Value> {
            let mut presenter = Presenter::new(&device, hwnd)?;
            let mut held: Vec<(Surface, Rc<Lease>)> = Vec::new();
            let mut presented = 0;
            let mut busy = 0;
            let mut decoded = 0;
            let mut decode_us = Vec::new();
            let mut submit_us = Vec::new();
            let origin = Instant::now();
            let mut pending = BTreeMap::new();
            for (index, unit) in units.iter().enumerate() {
                pace(origin, index, 60);
                pump();
                held.retain(|(_, lease)| !lease.available());
                let start = Instant::now();
                pending.insert(index as u64 + 1, (index, start));
                let mut input = bits(unit, index as u64 + 1);
                loop {
                    ensure!(
                        start.elapsed() < Duration::from_secs(2),
                        "VPL original decode stalled"
                    );
                    let mut out = std::ptr::null_mut();
                    let mut point = std::ptr::null_mut();
                    let status =
                        decode(raw, &mut input, std::ptr::null_mut(), &mut out, &mut point);
                    // Own every returned reference, including warning/error paths.
                    let surface = (!out.is_null()).then(|| Surface(out));
                    if status == MFX_ERR_MORE_DATA {
                        break;
                    }
                    check(status, "DecodeFrameAsync")?;
                    if let Some(surface) = surface {
                        ensure!(!point.is_null(), "output has no completion point");
                        loop {
                            let status = sync(raw, point, 0);
                            check(status, "SyncOperation")?;
                            if status == 0 {
                                break;
                            }
                            ensure!(start.elapsed() < Duration::from_secs(2), "VPL sync stalled");
                            std::thread::yield_now();
                        }
                        let (sequence, submitted) = pending
                            .remove(&(*out).Data.TimeStamp)
                            .context("uncorrelated VPL original output")?;
                        if sequence >= 16 {
                            decode_us.push(submitted.elapsed().as_micros() as u64);
                        }
                        let interface = (*out)
                            .__bindgen_anon_1
                            .FrameInterface
                            .as_ref()
                            .context("surface interface")?;
                        let mut handle = std::ptr::null_mut();
                        let mut kind = 0;
                        check(
                            interface
                                .GetNativeHandle
                                .context("GetNativeHandle unavailable")?(
                                out,
                                &mut handle,
                                &mut kind,
                            ),
                            "GetNativeHandle",
                        )?;
                        ensure!(kind == MFX_RESOURCE_DX11_TEXTURE, "not a D3D11 texture");
                        let texture = ID3D11Texture2D::from_raw_borrowed(&handle)
                            .context("native texture")?;
                        let mut desc = D3D11_TEXTURE2D_DESC::default();
                        texture.GetDesc(&mut desc);
                        ensure!(
                            desc.Format == DXGI_FORMAT_NV12
                                && desc.CPUAccessFlags == 0
                                && desc.ArraySize == 1,
                            "original texture layout cannot be presented without a copy"
                        );
                        ensure!(
                            texture.GetDevice()?.as_raw() == device.native.as_raw(),
                            "cross-device surface refused"
                        );
                        decoded += 1;
                        if presenter.has_room()? {
                            let mut client = RECT::default();
                            GetClientRect(hwnd, &mut client)?;
                            let lease = presenter.present(texture, Layout { left:0, top:0, width:1280, height:720, full_range:false },
                                (client.right as u32, client.bottom as u32), [0,0,client.right,client.bottom])
                                .context("original VPL texture rejected by Presentation API; no copy fallback")?;
                            held.push((surface, lease));
                            presented += 1;
                            if sequence >= 16 {
                                submit_us.push(submitted.elapsed().as_micros() as u64);
                            }
                        } else {
                            busy += 1;
                        }
                        if input.DataLength == 0 {
                            break;
                        }
                    } else {
                        std::thread::yield_now();
                    }
                }
            }
            // Keep decoder surfaces leased until the compositor returns them.
            let deadline = Instant::now() + Duration::from_secs(3);
            while !held.is_empty() && Instant::now() < deadline {
                pump();
                held.retain(|(_, lease)| !lease.available());
                std::thread::sleep(Duration::from_millis(1));
            }
            // The currently displayed buffer may stay unavailable until detached.
            drop(presenter);
            drop(held);
            ensure!(
                decoded == units.len() && pending.is_empty() && presented > 0,
                "incomplete original-surface trial"
            );
            Ok(
                json!({"frames":decoded,"presented":presented,"busy_display_updates_skipped":busy,
                "submission_to_vpl_completion_us":decode_us,"submission_to_present_call_return_us":submit_us,
                "application_decoded_frame_copies":0,"application_conversion_passes":0,"cpu_pixel_readback":false,
                "same_d3d11_device":true,"scanout_verified":false,"runtime_internal_copies":"unknown"}),
            )
        })();
        let _ = DestroyWindow(hwnd);
        let _ = UnregisterClassW(class, Some(instance.into()));
        drop(session);
        result
    }
}
type InitQuery =
    unsafe extern "C" fn(mfxSession, *mut mfxVideoParam, *mut mfxVideoParam) -> mfxStatus;
struct OwnedHandle(HANDLE);
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
struct Lease {
    buffer: IPresentationBuffer,
}
impl Lease {
    fn available(&self) -> bool {
        unsafe { self.buffer.IsAvailable().is_ok_and(|v| v != 0) }
    }
}

// Same bounded presentation behavior as the normal double-buffered path:
// never let a burst pin the entire decoder picture buffer awaiting scanout.
const MAX_OUTSTANDING: usize = 2;
struct Presenter {
    manager: IPresentationManager,
    surface: IPresentationSurface,
    composition: IDCompositionDesktopDevice,
    target: IDCompositionTarget,
    _visual: IDCompositionVisual2,
    _content: IUnknown,
    _handle: OwnedHandle,
    buffers: Vec<(ID3D11Texture2D, Rc<Lease>)>,
}
impl Presenter {
    fn new(device: &DisplayDevice, hwnd: HWND) -> Result<Self> {
        // SAFETY: live device, texture and HWND on the owning video thread.
        unsafe {
            // Resolve only the Windows 11 entry point dynamically. Older systems
            // must still start and use the unchanged DXGI renderer.
            let module = GetModuleHandleW(w!("dcomp.dll"))?;
            let symbol = GetProcAddress(module, s!("CreatePresentationFactory"))
                .context("Windows Presentation API unavailable")?;
            type Create =
                unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT;
            let create: Create = std::mem::transmute(symbol);
            let mut raw = std::ptr::null_mut();
            create(device.native.as_raw(), &IPresentationFactory::IID, &mut raw).ok()?;
            let factory = IPresentationFactory::from_raw(raw);
            ensure!(
                factory.IsPresentationSupportedWithIndependentFlip() != 0,
                "direct presentation unsupported"
            );
            let manager = factory.CreatePresentationManager()?;
            let handle = OwnedHandle(DCompositionCreateSurfaceHandle(0x001f0003, None)?);
            let surface = manager.CreatePresentationSurface(handle.0)?;
            surface.SetAlphaMode(DXGI_ALPHA_MODE_IGNORE)?;
            let dxgi: IDXGIDevice = device.native.cast()?;
            let composition: IDCompositionDesktopDevice = DCompositionCreateDevice2(&dxgi)?;
            let target = composition.CreateTargetForHwnd(hwnd, true)?;
            let visual = composition.CreateVisual()?;
            let content = composition.CreateSurfaceFromHandle(handle.0)?;
            visual.SetContent(&content)?;
            target.SetRoot(&visual)?;
            composition.Commit()?;
            Ok(Self {
                manager,
                surface,
                composition,
                target,
                _visual: visual,
                _content: content,
                _handle: handle,
                buffers: Vec::new(),
            })
        }
    }
    fn has_room(&self) -> Result<bool> {
        let mut held = 0;
        for (_, lease) in &self.buffers {
            // No waiting: only decide whether to submit this display update.
            if unsafe { lease.buffer.IsAvailable()? } == 0 {
                held += 1;
                if held >= MAX_OUTSTANDING {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
    fn present(
        &mut self,
        texture: &ID3D11Texture2D,
        layout: Layout,
        client: (u32, u32),
        target: [i32; 4],
    ) -> Result<Rc<Lease>> {
        // SAFETY: live device, texture and HWND on the owning video thread.
        unsafe {
            let lease = if let Some((_, lease)) = self.buffers.iter().find(|(t, _)| t == texture) {
                lease.clone()
            } else {
                ensure!(self.buffers.len() < 31, "presentation buffer limit");
                let lease = Rc::new(Lease {
                    buffer: self.manager.AddBufferFromResource(texture)?,
                });
                self.buffers.push((texture.clone(), lease.clone()));
                lease
            };
            self.surface.SetSourceRect(&RECT {
                left: layout.left as i32,
                top: layout.top as i32,
                right: (layout.left + layout.width) as i32,
                bottom: (layout.top + layout.height) as i32,
            })?;
            self.surface.SetColorSpace(if layout.full_range {
                DXGI_COLOR_SPACE_YCBCR_FULL_G22_LEFT_P709
            } else {
                DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709
            })?;
            self.surface.SetLetterboxingMargins(
                target[0] as f32,
                target[1] as f32,
                (client.0 as i32 - target[2]) as f32,
                (client.1 as i32 - target[3]) as f32,
            )?;
            self.surface.SetTransform(&PresentationTransform {
                M11: (target[2] - target[0]) as f32 / layout.width as f32,
                M22: (target[3] - target[1]) as f32 / layout.height as f32,
                ..Default::default()
            })?;
            self.surface.SetBuffer(&lease.buffer)?;
            // Immediate readiness, no target-time pacing and no CPU completion wait.
            self.manager
                .SetTargetTime(SystemInterruptTime { value: 0 })?;
            self.manager.Present()?;
            Ok(lease)
        }
    }
}
impl Drop for Presenter {
    fn drop(&mut self) {
        unsafe {
            let _ = self.target.SetRoot(None);
            let _ = self.composition.Commit();
            // Detaching the visual is sufficient. Never submit a new GPU
            // present from Drop, especially before any buffer was registered
            // or after a decoder/device failure. Presentation keeps outstanding
            // buffer references alive until they can safely be retired.
        }
    }
}
