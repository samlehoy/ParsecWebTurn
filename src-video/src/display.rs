//! Experimental original NV12 decoder surfaces -> Windows Presentation API.
//! No processor, shader, pixel upload/readback, RGB target or frame copy.
use crate::{Layout, direct};
use anyhow::{Context as _, Result, ensure};
use lowlat_decode::Fed;
use lowlat_drivers::{
    d3d11::{Com, D3d11, Error, Luid},
    ffi::d3d11 as ffi,
};
use self_cell::self_cell;
use std::{ffi::c_void, rc::Rc};
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE, HMODULE, HWND, RECT},
        Graphics::{
            CompositionSwapchain::*,
            Direct3D::D3D_DRIVER_TYPE_UNKNOWN,
            Direct3D10::ID3D10Multithread,
            Direct3D11::*,
            DirectComposition::*,
            Dxgi::{Common::*, IDXGIDevice},
        },
        System::{
            Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize},
            LibraryLoader::{GetModuleHandleW, GetProcAddress},
        },
    },
    core::{GUID, HRESULT, IUnknown, Interface, s, w},
};

struct Device {
    native: ID3D11Device,
    context: ID3D11DeviceContext,
    video: ID3D11VideoDevice,
    video_context: ID3D11VideoContext,
    name: String,
}
impl Device {
    fn open(luid: u64) -> Result<Self> {
        let runtime = D3d11::load()?;
        let probe = runtime.open(Luid {
            high: (luid >> 32) as i32,
            low: luid as u32,
        })?;
        let raw = probe.device().cast();
        // SAFETY: borrow the live lowlat device only while selecting its adapter.
        let probe = unsafe { ID3D11Device::from_raw_borrowed(&raw) }.context("missing device")?;
        unsafe {
            let dxgi: IDXGIDevice = probe.cast()?;
            let adapter = dxgi.GetAdapter()?;
            let desc = adapter.GetDesc()?;
            ensure!(desc.VendorId == 0x10de, "Zero copy requires an NVIDIA GPU");
            let mut native = None;
            let mut context = None;
            // Presentation API requires PREVENT_INTERNAL_THREADING_OPTIMIZATIONS.
            // Do not mark SINGLETHREADED: the existing UI retains its own layer.
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
                Some(&mut context),
            )?;
            let native = native.context("missing display device")?;
            let context = context.context("missing display context")?;
            let multithread: ID3D10Multithread = native.cast()?;
            let _ = multithread.SetMultithreadProtected(true);
            let mut support = D3D11_FEATURE_DATA_DISPLAYABLE::default();
            native.CheckFeatureSupport(
                D3D11_FEATURE_DISPLAYABLE,
                (&mut support as *mut D3D11_FEATURE_DATA_DISPLAYABLE).cast(),
                size_of::<D3D11_FEATURE_DATA_DISPLAYABLE>() as u32,
            )?;
            ensure!(
                support.DisplayableTexture.as_bool(),
                "displayable textures unavailable"
            );
            Ok(Self {
                video: native.cast()?,
                video_context: context.cast()?,
                native,
                context,
                name: String::from_utf16_lossy(&desc.Description)
                    .trim_end_matches('\0')
                    .into(),
            })
        }
    }
}
impl direct::DeviceAccess for Device {
    fn video(&self) -> *mut ffi::ID3D11VideoDevice {
        self.video.as_raw().cast()
    }
    fn video_context(&self) -> *mut ffi::ID3D11VideoContext {
        self.video_context.as_raw().cast()
    }
    fn texture(
        &self,
        desc: &ffi::D3D11_TEXTURE2D_DESC,
    ) -> std::result::Result<Com<ffi::ID3D11Texture2D>, Error> {
        // SAFETY: both bindings describe the same documented C ABI structure.
        const {
            assert!(size_of::<ffi::D3D11_TEXTURE2D_DESC>() == size_of::<D3D11_TEXTURE2D_DESC>());
        }
        unsafe {
            let mut texture = None;
            self.native
                .CreateTexture2D(
                    (desc as *const ffi::D3D11_TEXTURE2D_DESC).cast(),
                    None,
                    Some(&mut texture),
                )
                .map_err(|e| Error::Status(e.code().0))?;
            Com::from_raw(texture.ok_or(Error::Unavailable)?.into_raw().cast())
                .ok_or(Error::Unavailable)
        }
    }
}
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
impl direct::SurfaceLease for Lease {
    fn available(&self) -> bool {
        unsafe { self.buffer.IsAvailable().is_ok_and(|v| v != 0) }
    }
}

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
    fn new(device: &Device, hwnd: HWND) -> Result<Self> {
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
    fn present(
        &mut self,
        texture: &ID3D11Texture2D,
        layout: Layout,
        target: [i32; 4],
    ) -> Result<Rc<dyn direct::SurfaceLease>> {
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
            self.surface.SetTransform(&PresentationTransform {
                M11: (target[2] - target[0]) as f32 / layout.width as f32,
                M22: (target[3] - target[1]) as f32 / layout.height as f32,
                M31: target[0] as f32,
                M32: target[1] as f32,
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
            let _ = self.surface.SetBuffer(None);
            let _ = self.manager.Present();
        }
    }
}
struct State<'a> {
    presenter: Presenter,
    decoder: direct::Backend<'a>,
}
self_cell!(
    struct Cell {
        owner: Device,
        #[covariant]
        dependent: State,
    }
);
struct ComApartment;
impl ComApartment {
    fn enter() -> Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        }
        Ok(Self)
    }
}
impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}
pub struct Session {
    cell: Cell,
    sequence: usize,
    _apartment: ComApartment,
}
#[derive(Default)]
pub struct Output {
    pub decoded: usize,
    pub presented: usize,
    pub layout: Option<Layout>,
}
impl Session {
    /// All creation, feed and destruction must take place on the video worker.
    /// `hwnd` must remain valid until this session is dropped.
    pub fn open(luid: u64, hwnd: HWND) -> Result<Self> {
        let apartment = ComApartment::enter()?;
        let device = Device::open(luid)?;
        let cell = Cell::try_new(device, |device| -> Result<State<'_>> {
            Ok(State {
                presenter: Presenter::new(device, hwnd)?,
                decoder: direct::Backend::displayable(device),
            })
        })?;
        Ok(Self {
            cell,
            sequence: 0,
            _apartment: apartment,
        })
    }
    pub fn adapter(&self) -> &str {
        &self.cell.borrow_owner().name
    }
    pub fn feed(
        &mut self,
        bytes: &[u8],
        visible: bool,
        target: impl Fn(Layout) -> Result<[i32; 4]>,
    ) -> Result<Output> {
        let stamp = self.sequence;
        self.sequence = self.sequence.wrapping_add(1);
        self.cell.with_dependent_mut(|device, state| {
            let fed = state.decoder.feed(bytes, stamp)?;
            ensure!(
                fed != Fed::FormatChanged,
                "displayable decoder format changed"
            );
            let mut output = Output::default();
            if fed != Fed::Picture {
                return Ok(output);
            }
            let layout = state.decoder.layout()?;
            output.layout = Some(layout);
            while state.decoder.take_leased(|raw, slice, _| {
                ensure!(slice == 0, "array slice cannot be presented directly");
                let raw = raw.cast();
                // SAFETY: original DPB allocation, held through take_leased.
                let texture = unsafe { ID3D11Texture2D::from_raw_borrowed(&raw) }
                    .context("missing decoder texture")?;
                let owner = unsafe { texture.GetDevice()? };
                ensure!(owner == device.native, "cross-device presentation refused");
                output.decoded += 1;
                if !visible {
                    return Ok(None);
                }
                let lease = state.presenter.present(texture, layout, target(layout)?)?;
                output.presented += 1;
                Ok(Some(lease))
            })? {}
            // Submit decode work even when hidden; never wait for scanout.
            unsafe {
                device.context.Flush();
            }
            Ok(output)
        })
    }
}
