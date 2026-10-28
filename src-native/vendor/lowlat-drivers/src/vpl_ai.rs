//! Optional VPL 2.17 AI Super Resolution on the caller's D3D11 device.
//! Public Intel C API only. No CPU mapping, software runtime or ordinary-scaler fallback.
use super::*;
use crate::ffi::vpl::*;

type VppQuery =
    unsafe extern "C" fn(mfxSession, *mut mfxVideoParam, *mut mfxVideoParam) -> mfxStatus;
type VppInit = unsafe extern "C" fn(mfxSession, *mut mfxVideoParam) -> mfxStatus;
type GetSurface = unsafe extern "C" fn(mfxSession, *mut *mut mfxFrameSurface1) -> mfxStatus;
type Run = unsafe extern "C" fn(
    mfxSession,
    *mut mfxFrameSurface1,
    *mut mfxFrameSurface1,
    *mut c_void,
    *mut mfxSyncPoint,
) -> mfxStatus;
type VppClose = unsafe extern "C" fn(mfxSession) -> mfxStatus;

// Public mfxExtVPPAISuperResolution, stable Intel VPL v2.17.0 headers.
// This backwards-compatible filter first appeared in API 2.13.
#[repr(C)]
struct AiFilter {
    header: mfxExtBuffer,
    mode: u32,
    reserved1: [u32; 16],
    reserved2: [mfxHDL; 4],
}
const AI_FILTER: u32 = u32::from_le_bytes(*b"VASR");
const _: () = assert!(size_of::<AiFilter>() == 112);
const _: () = assert!(core::mem::offset_of!(AiFilter, reserved2) == 80);
fn strict(status: mfxStatus) -> Result<()> {
    if status == 0 {
        Ok(())
    } else {
        Err(Error::Status(status))
    }
}
fn operation<T>(name: &'static str, api: Option<(u16, u16)>, result: Result<T>) -> Result<T> {
    result.map_err(|error| match error {
        Error::Status(status) => Error::Ai {
            operation: name,
            status,
            api,
            support: None,
        },
        other => other,
    })
}
/// Query-only diagnosis after rejection. Never initializes or processes frames.
/// No CPU-memory request, native-resource allocation or ordinary-scaler fallback.
unsafe fn support_queries(query: VppQuery, session: mfxSession, base: mfxVideoParam) -> [i32; 4] {
    unsafe {
        let one = |mut param: mfxVideoParam, nv12: bool, ai: bool, double: bool| {
            if nv12 {
                let vpp = &mut param.__bindgen_anon_1.vpp;
                for info in [&mut vpp.In, &mut vpp.Out] {
                    info.FourCC = MFX_FOURCC_NV12 as u32;
                    info.ChromaFormat = MFX_CHROMAFORMAT_YUV420 as u16;
                }
            }
            if double {
                let input = param
                    .__bindgen_anon_1
                    .vpp
                    .In
                    .__bindgen_anon_1
                    .__bindgen_anon_1;
                let Some(width) = input.CropW.checked_mul(2) else {
                    return MFX_ERR_UNSUPPORTED;
                };
                let Some(height) = input.CropH.checked_mul(2) else {
                    return MFX_ERR_UNSUPPORTED;
                };
                let Ok(mut out) = info((u32::from(width), u32::from(height))) else {
                    return MFX_ERR_UNSUPPORTED;
                };
                out.FourCC = MFX_FOURCC_NV12 as u32;
                out.ChromaFormat = MFX_CHROMAFORMAT_YUV420 as u16;
                param.__bindgen_anon_1.vpp.Out = out;
            }
            let mut filter_in = filter();
            let mut filter_out = filter();
            let mut extensions = [&raw mut filter_in.header];
            let mut outputs = [&raw mut filter_out.header];
            param.NumExtParam = u16::from(ai);
            param.ExtParam = if ai {
                extensions.as_mut_ptr()
            } else {
                core::ptr::null_mut()
            };
            let mut result = param;
            result.ExtParam = if ai {
                outputs.as_mut_ptr()
            } else {
                core::ptr::null_mut()
            };
            query(session, &raw mut param, &raw mut result)
        };
        [
            one(base, false, false, false),
            one(base, true, false, false),
            one(base, true, true, false),
            one(base, true, true, true),
        ]
    }
}
fn filter() -> AiFilter {
    AiFilter {
        header: mfxExtBuffer {
            BufferId: AI_FILTER,
            BufferSz: size_of::<AiFilter>() as u32,
        },
        mode: 1,
        reserved1: [0; 16],
        reserved2: [core::ptr::null_mut(); 4],
    }
}
fn info(size: (u32, u32)) -> Result<mfxFrameInfo> {
    let (w, h) = size;
    if w == 0 || h == 0 || w > 8192 || h > 8192 {
        return Err(Error::Status(MFX_ERR_UNSUPPORTED));
    }
    let mut i: mfxFrameInfo = zeroed();
    i.FourCC = MFX_FOURCC_RGB4 as u32;
    i.BitDepthLuma = 8;
    i.BitDepthChroma = 8;
    i.ChromaFormat = MFX_CHROMAFORMAT_YUV444 as u16;
    i.PicStruct = MFX_PICSTRUCT_PROGRESSIVE as u16;
    i.FrameRateExtN = 60;
    i.FrameRateExtD = 1;
    i.__bindgen_anon_1.__bindgen_anon_1 = mfxFrameInfo__bindgen_ty_1__bindgen_ty_1 {
        Width: w.next_multiple_of(16) as u16,
        Height: h.next_multiple_of(16) as u16,
        CropW: w as u16,
        CropH: h as u16,
        CropX: 0,
        CropY: 0,
    };
    Ok(i)
}
fn same_info(a: mfxFrameInfo, b: mfxFrameInfo) -> bool {
    let a_size = unsafe { a.__bindgen_anon_1.__bindgen_anon_1 };
    let b_size = unsafe { b.__bindgen_anon_1.__bindgen_anon_1 };
    a.FourCC == b.FourCC
        && a.BitDepthLuma == b.BitDepthLuma
        && a.BitDepthChroma == b.BitDepthChroma
        && a.PicStruct == b.PicStruct
        && a.ChromaFormat == b.ChromaFormat
        && (
            a_size.Width,
            a_size.Height,
            a_size.CropX,
            a_size.CropY,
            a_size.CropW,
            a_size.CropH,
        ) == (
            b_size.Width,
            b_size.Height,
            b_size.CropX,
            b_size.CropY,
            b_size.CropW,
            b_size.CropH,
        )
}

struct Surface(NonNull<mfxFrameSurface1>);
impl Surface {
    unsafe fn acquire(get: GetSurface, session: mfxSession) -> Result<Self> {
        let mut out = core::ptr::null_mut();
        // A runtime allocation with one application reference, released on Drop.
        strict(unsafe { get(session, &raw mut out) })?;
        NonNull::new(out).map(Self).ok_or(Error::Unavailable)
    }
    unsafe fn texture(&self, device: *mut c_void) -> Result<*mut c_void> {
        unsafe {
            let s = self.0.as_ptr();
            let i = (*s)
                .__bindgen_anon_1
                .FrameInterface
                .as_ref()
                .ok_or(Error::Unavailable)?;
            let mut handle = core::ptr::null_mut();
            let mut kind = 0;
            strict(i.GetDeviceHandle.ok_or(Error::MissingSymbol)?(
                s,
                &raw mut handle,
                &raw mut kind,
            ))?;
            if kind != MFX_HANDLE_D3D11_DEVICE || handle != device {
                return Err(Error::Status(MFX_ERR_UNSUPPORTED));
            }
            let mut resource = 0;
            strict(i.GetNativeHandle.ok_or(Error::MissingSymbol)?(
                s,
                &raw mut handle,
                &raw mut resource,
            ))?;
            if resource != MFX_RESOURCE_DX11_TEXTURE || handle.is_null() {
                return Err(Error::Status(MFX_ERR_UNSUPPORTED));
            }
            Ok(handle)
        }
    }
}
impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            if let Some(i) = (*self.0.as_ptr()).__bindgen_anon_1.FrameInterface.as_ref()
                && let Some(release) = i.Release
            {
                release(self.0.as_ptr());
            }
        }
    }
}

/// Exactly one input and output surface, kept until the session closes.
/// Calls and D3D11 texture use belong to the renderer's owner thread.
pub struct Scaler {
    input: Option<Surface>,
    output: Option<Surface>,
    raw: NonNull<_mfxSession>,
    run: Run,
    close: VppClose,
    // Retain the DLL until every session/surface has been released.
    runtime: Vpl,
    api: Option<(u16, u16)>,
}
impl fmt::Debug for Scaler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("IntelAiScaler")
    }
}
impl Scaler {
    /// # Safety
    /// `device` is the live, multithread-protected ID3D11Device of `index` and
    /// outlives this scaler. Serialize all GPU calls on that device's context.
    pub unsafe fn new(
        runtime: Vpl,
        device: *mut c_void,
        index: u32,
        source: (u32, u32),
        target: (u32, u32),
        viewport: [u32; 4],
    ) -> Result<Self> {
        unsafe {
            if runtime.runtime() != Runtime::Current || device.is_null() {
                return operation(
                    "current GPU VPL runtime and native device required",
                    None,
                    Err(Error::Status(MFX_ERR_UNSUPPORTED)),
                );
            }
            let query: VppQuery = symbol(&runtime._library, c"MFXVideoVPP_Query")?;
            let init_vpp: VppInit = symbol(&runtime._library, c"MFXVideoVPP_Init")?;
            let get_param: VppInit = symbol(&runtime._library, c"MFXVideoVPP_GetVideoParam")?;
            let get_in: GetSurface = symbol(&runtime._library, c"MFXMemory_GetSurfaceForVPP")?;
            let get_out: GetSurface = symbol(&runtime._library, c"MFXMemory_GetSurfaceForVPPOut")?;
            let run = symbol(&runtime._library, c"MFXVideoVPP_RunFrameVPPAsync")?;
            let close = symbol(&runtime._library, c"MFXVideoVPP_Close")?;
            let initialize = runtime.initialize.ok_or(Error::MissingSymbol)?;
            let mut init: mfxInitializationParam = zeroed();
            init.AccelerationMode = MFX_ACCEL_MODE_VIA_D3D11;
            init.VendorImplID = index;
            let mut thread: mfxExtThreadsParam = zeroed();
            thread.Header.BufferId = MFX_EXTBUFF_THREADS_PARAM as u32;
            thread.Header.BufferSz = size_of::<mfxExtThreadsParam>() as u32;
            thread.NumThread = 1;
            let mut ext = [&raw mut thread.Header];
            init.NumExtParam = 1;
            init.ExtParam = ext.as_mut_ptr();
            let mut raw = core::ptr::null_mut();
            let initialize_status = initialize(init, &raw mut raw);
            if raw.is_null() {
                return operation(
                    "MFXInitialize (no session)",
                    None,
                    Err(Error::Status(initialize_status)),
                );
            }
            let mut this = Self {
                input: None,
                output: None,
                raw: NonNull::new(raw).ok_or(Error::Unavailable)?,
                run,
                close,
                runtime,
                api: None,
            };
            operation("MFXInitialize", None, strict(initialize_status))?;
            operation(
                "MFXVideoCORE_SetHandle",
                None,
                strict((this.runtime.set_handle)(
                    raw,
                    MFX_HANDLE_D3D11_DEVICE,
                    device,
                )),
            )?;
            let mut version: mfxVersion = zeroed();
            operation(
                "MFXQueryVersion",
                None,
                strict((this.runtime.query_version)(raw, &raw mut version)),
            )?;
            let v = version.__bindgen_anon_1;
            this.api = Some((v.Major, v.Minor));
            let api = this.api;
            if (v.Major, v.Minor) < (2, 13) {
                return operation(
                    "AI requires runtime API >= 2.13",
                    api,
                    Err(Error::Status(MFX_ERR_UNSUPPORTED)),
                );
            }
            let mut ai = filter();
            let mut ai_out = filter();
            let mut in_ext = [&raw mut ai.header];
            let mut out_ext = [&raw mut ai_out.header];
            let mut param: mfxVideoParam = zeroed();
            param.AsyncDepth = 1;
            param.IOPattern =
                (MFX_IOPATTERN_IN_VIDEO_MEMORY | MFX_IOPATTERN_OUT_VIDEO_MEMORY) as u16;
            param.__bindgen_anon_1.vpp.In = info(source)?;
            param.__bindgen_anon_1.vpp.Out = info(target)?;
            let [left, top, right, bottom] = viewport;
            if left >= right || top >= bottom || right > target.0 || bottom > target.1 {
                return Err(Error::Status(MFX_ERR_UNSUPPORTED));
            }
            let crop = &mut param
                .__bindgen_anon_1
                .vpp
                .Out
                .__bindgen_anon_1
                .__bindgen_anon_1;
            crop.CropX = left as u16;
            crop.CropY = top as u16;
            crop.CropW = (right - left) as u16;
            crop.CropH = (bottom - top) as u16;
            param.NumExtParam = 1;
            param.ExtParam = in_ext.as_mut_ptr();
            let mut out = param;
            out.ExtParam = out_ext.as_mut_ptr();
            // Warnings (filter skipped, partial acceleration, altered parameters)
            // are rejection, never a claim that ordinary scaling is AI.
            let requested = param;
            let query_status = query(raw, &raw mut param, &raw mut out);
            if query_status != 0 {
                return Err(Error::Ai {
                    operation: "MFXVideoVPP_Query (AI, BGRA8, GPU memory)",
                    status: query_status,
                    api,
                    support: Some(support_queries(query, raw, requested)),
                });
            }
            if ai_out.mode != 1
                || ai_out.header.BufferId != AI_FILTER
                || out.IOPattern != param.IOPattern
                || out.AsyncDepth != 1
                || !same_info(out.__bindgen_anon_1.vpp.In, param.__bindgen_anon_1.vpp.In)
                || !same_info(out.__bindgen_anon_1.vpp.Out, param.__bindgen_anon_1.vpp.Out)
            {
                return operation(
                    "AI Query changed required parameters",
                    api,
                    Err(Error::Status(MFX_ERR_UNSUPPORTED)),
                );
            }
            operation(
                "MFXVideoVPP_Init (AI)",
                api,
                strict(init_vpp(raw, &raw mut param)),
            )?;
            ai_out.mode = 0;
            operation(
                "MFXVideoVPP_GetVideoParam (AI)",
                api,
                strict(get_param(raw, &raw mut out)),
            )?;
            if ai_out.mode != 1
                || out.IOPattern != param.IOPattern
                || out.AsyncDepth != 1
                || !same_info(out.__bindgen_anon_1.vpp.In, param.__bindgen_anon_1.vpp.In)
                || !same_info(out.__bindgen_anon_1.vpp.Out, param.__bindgen_anon_1.vpp.Out)
            {
                return operation(
                    "active AI parameters were not confirmed",
                    api,
                    Err(Error::Status(MFX_ERR_UNSUPPORTED)),
                );
            }
            this.input = Some(operation(
                "MFXMemory_GetSurfaceForVPP",
                api,
                Surface::acquire(get_in, raw),
            )?);
            this.output = Some(operation(
                "MFXMemory_GetSurfaceForVPPOut",
                api,
                Surface::acquire(get_out, raw),
            )?);
            this.input_texture(device)?;
            this.output_texture(device)?;
            Ok(this)
        }
    }
    /// Borrowed texture, valid until this scaler drops. No AddRef/CPU mapping.
    /// # Safety
    /// `device` must be the same live device passed to new.
    pub unsafe fn input_texture(&self, device: *mut c_void) -> Result<*mut c_void> {
        unsafe {
            operation(
                "input native D3D11 surface/device",
                self.api,
                self.input
                    .as_ref()
                    .ok_or(Error::Unavailable)?
                    .texture(device),
            )
        }
    }
    /// # Safety
    /// Same as input_texture; synchronize processing before reading pixels.
    pub unsafe fn output_texture(&self, device: *mut c_void) -> Result<*mut c_void> {
        unsafe {
            operation(
                "output native D3D11 surface/device",
                self.api,
                self.output
                    .as_ref()
                    .ok_or(Error::Unavailable)?
                    .texture(device),
            )
        }
    }
    /// Process precisely one frame. Bound completion wait; never enqueue history
    /// or generate frames. Busy/warnings/timeouts disable the optional effect.
    /// # Safety
    /// Finish submitting writes to input on the shared immediate context first;
    /// submit output reads on that same context before calling again.
    pub unsafe fn evaluate(&self) -> Result<()> {
        unsafe {
            let input = self.input.as_ref().ok_or(Error::Unavailable)?.0.as_ptr();
            let output = self.output.as_ref().ok_or(Error::Unavailable)?.0.as_ptr();
            if (*input).Data.Locked != 0 || (*output).Data.Locked != 0 {
                return Err(Error::Status(MFX_WRN_DEVICE_BUSY));
            }
            let mut sync = core::ptr::null_mut();
            operation(
                "MFXVideoVPP_RunFrameVPPAsync",
                self.api,
                strict((self.run)(
                    self.raw.as_ptr(),
                    input,
                    output,
                    core::ptr::null_mut(),
                    &raw mut sync,
                )),
            )?;
            if sync.is_null() {
                return Err(Error::Unavailable);
            }
            operation(
                "MFXVideoCORE_SyncOperation (100 ms limit)",
                self.api,
                strict((self.runtime.sync_operation)(self.raw.as_ptr(), sync, 100)),
            )
        }
    }
}
impl Drop for Scaler {
    fn drop(&mut self) {
        unsafe {
            // Close drains the native operation before releasing its surfaces.
            // Owner-thread teardown stays under the existing video watchdog.
            (self.close)(self.raw.as_ptr());
            self.input.take();
            self.output.take();
            (self.runtime.close)(self.raw.as_ptr());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn query_only_matrix_distinguishes_plain_scaling_format_and_ratio() {
        unsafe extern "C" fn query(
            _session: mfxSession,
            param: *mut mfxVideoParam,
            _out: *mut mfxVideoParam,
        ) -> mfxStatus {
            unsafe {
                let p = &*param;
                if p.IOPattern
                    != (MFX_IOPATTERN_IN_VIDEO_MEMORY | MFX_IOPATTERN_OUT_VIDEO_MEMORY) as u16
                {
                    return -999;
                }
                if p.NumExtParam == 0 {
                    return if p.ExtParam.is_null() { 0 } else { -999 };
                }
                let vpp = p.__bindgen_anon_1.vpp;
                let input = vpp.In.__bindgen_anon_1.__bindgen_anon_1;
                let output = vpp.Out.__bindgen_anon_1.__bindgen_anon_1;
                if vpp.In.FourCC == MFX_FOURCC_NV12 as u32 && output.CropW == input.CropW * 2 {
                    0
                } else {
                    -3
                }
            }
        }
        let mut base: mfxVideoParam = zeroed();
        base.IOPattern = (MFX_IOPATTERN_IN_VIDEO_MEMORY | MFX_IOPATTERN_OUT_VIDEO_MEMORY) as u16;
        base.AsyncDepth = 1;
        base.__bindgen_anon_1.vpp.In = info((1280, 800)).unwrap();
        base.__bindgen_anon_1.vpp.Out = info((1920, 1200)).unwrap();
        let result = unsafe { support_queries(query, core::ptr::null_mut(), base) };
        assert_eq!(result, [0, 0, -3, 0]);
        assert_eq!(
            unsafe { base.__bindgen_anon_1.vpp.In.FourCC },
            MFX_FOURCC_RGB4 as u32
        );
    }
    #[test]
    fn unsupported_diagnostics_distinguish_api_age_from_ai_configuration() {
        let old: Result<()> = operation(
            "AI requires runtime API >= 2.13",
            Some((2, 12)),
            Err(Error::Status(-3)),
        );
        let config: Result<()> = operation(
            "MFXVideoVPP_Query (AI, BGRA8, GPU memory)",
            Some((2, 17)),
            Err(Error::Status(-3)),
        );
        assert!(old.unwrap_err().to_string().contains("runtime API 2.12"));
        let config = config.unwrap_err().to_string();
        assert!(config.contains("MFXVideoVPP_Query"));
        assert!(config.contains("MFX_ERR_UNSUPPORTED"));
        assert!(config.contains("runtime API 2.17"));
        assert!(operation("query", Some((2, 17)), strict(0)).is_ok());
    }
    #[test]
    fn ai_accepts_only_exact_success_and_keeps_video_memory_geometry() {
        assert!(strict(0).is_ok());
        for status in [-3, MFX_WRN_DEVICE_BUSY, 3, 4, 10] {
            assert!(strict(status).is_err());
        }
        let frame = info((1920, 1080)).unwrap();
        let size = unsafe { frame.__bindgen_anon_1.__bindgen_anon_1 };
        assert_eq!(
            (size.Width, size.Height, size.CropW, size.CropH),
            (1920, 1088, 1920, 1080)
        );
        assert!(info((0, 1080)).is_err());
        assert!(info((8193, 1080)).is_err());
        assert_eq!(filter().header.BufferId, AI_FILTER);
        assert!(same_info(frame, frame));
        assert!(!same_info(frame, info((1280, 720)).unwrap()));
    }
}
