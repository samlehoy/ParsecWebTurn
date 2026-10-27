//! Optional RTX Video SDK 1.1 D3D11 adapter. No CPU pixel access.
//! The public C ABI is bound directly; no NVIDIA sample implementation is copied.
use windows::Win32::Graphics::{Direct3D11::*, Dxgi::Common::*};

pub struct Surfaces {
    pub input: ID3D11Texture2D,
    pub output: ID3D11Texture2D,
    clear: ID3D11RenderTargetView,
    key: (u32, u32, u32, u32, DXGI_FORMAT),
}

impl Surfaces {
    unsafe fn new(
        device: &ID3D11Device,
        source: (u32, u32),
        target: (u32, u32),
        format: DXGI_FORMAT,
    ) -> anyhow::Result<Self> {
        let texture = |size: (u32, u32), flags: u32| -> windows::core::Result<ID3D11Texture2D> {
            let mut resource = None;
            device.CreateTexture2D(
                &D3D11_TEXTURE2D_DESC {
                    Width: size.0,
                    Height: size.1,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: format,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: flags,
                    ..Default::default()
                },
                None,
                Some(&mut resource),
            )?;
            resource.ok_or_else(windows::core::Error::from_thread)
        };
        let input = texture(
            source,
            (D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE).0 as u32,
        )?;
        let output = texture(
            target,
            (D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_UNORDERED_ACCESS).0
                as u32,
        )?;
        let mut clear = None;
        device.CreateRenderTargetView(&output, None, Some(&mut clear))?;
        Ok(Self {
            input,
            output,
            clear: clear.ok_or_else(windows::core::Error::from_thread)?,
            key: (source.0, source.1, target.0, target.1, format),
        })
    }
}

pub struct Vsr {
    #[cfg(nvidia_vsr_sdk)]
    sdk: sdk::Instance,
    surfaces: Option<Surfaces>,
    quality: u8,
    evaluated: bool,
}
impl Vsr {
    pub unsafe fn new(
        device: &ID3D11Device,
        quality: u8,
        data: &std::path::Path,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!((1..=4).contains(&quality), "Invalid RTX VSR quality");
        #[cfg(nvidia_vsr_sdk)]
        {
            Ok(Self {
                sdk: sdk::Instance::new(device, data)?,
                surfaces: None,
                quality,
                evaluated: false,
            })
        }
        #[cfg(not(nvidia_vsr_sdk))]
        {
            let _ = (device, data);
            anyhow::bail!("RTX Video SDK was not included in this build")
        }
    }
    /// One source-sized conversion target and one UAV output, reused across frames.
    /// The caller skips VSR when the viewport does not upscale in both dimensions.
    pub unsafe fn prepare(
        &mut self,
        device: &ID3D11Device,
        source: (u32, u32),
        target: (u32, u32),
        format: DXGI_FORMAT,
    ) -> anyhow::Result<&Surfaces> {
        let key = (source.0, source.1, target.0, target.1, format);
        if self.surfaces.as_ref().is_none_or(|s| s.key != key) {
            self.surfaces = Some(Surfaces::new(device, source, target, format)?);
        }
        Ok(self.surfaces.as_ref().unwrap())
    }
    pub fn warming(&self) -> bool {
        !self.evaluated
    }
    pub unsafe fn evaluate(
        &mut self,
        context: &ID3D11DeviceContext,
        target: [i32; 4],
    ) -> anyhow::Result<()> {
        let surfaces = self
            .surfaces
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("VSR surfaces missing"))?;
        context.ClearRenderTargetView(&surfaces.clear, &[0.0, 0.0, 0.0, 1.0]);
        #[cfg(nvidia_vsr_sdk)]
        {
            let result = self.sdk.evaluate(context, surfaces, target, self.quality);
            self.evaluated = result.is_ok();
            result
        }
        #[cfg(not(nvidia_vsr_sdk))]
        {
            let _ = (target, self.quality);
            anyhow::bail!("RTX Video SDK unavailable")
        }
    }
}

#[cfg(nvidia_vsr_sdk)]
mod sdk {
    use super::*;
    use std::{
        ffi::{c_char, c_void},
        ptr::null_mut,
    };
    use windows::core::Interface;
    use windows::Win32::Graphics::Direct3D10::ID3D10Multithread;
    struct ContextLock(ID3D10Multithread);
    impl ContextLock {
        unsafe fn enter(device: &ID3D11Device) -> anyhow::Result<Self> {
            let lock: ID3D10Multithread = device.cast()?;
            lock.Enter();
            Ok(Self(lock))
        }
    }
    impl Drop for ContextLock {
        fn drop(&mut self) {
            unsafe {
                self.0.Leave();
            }
        }
    }

    // Layout and signatures from RTX Video SDK 1.1 public nvsdk_ngx*.h C API.
    #[repr(C)]
    struct Paths {
        paths: *const *const u16,
        count: u32,
    }
    #[repr(C)]
    struct Logging {
        callback: *const c_void,
        level: u32,
        other_sinks_disabled: bool,
    }
    #[repr(C)]
    struct Common {
        paths: Paths,
        internal: *mut c_void,
        logging: Logging,
    }
    extern "C" {
        fn NVSDK_NGX_D3D11_Init(
            app: u64,
            data: *const u16,
            device: *mut c_void,
            common: *const Common,
            version: u32,
        ) -> u32;
        fn NVSDK_NGX_D3D11_Shutdown1(device: *mut c_void) -> u32;
        fn NVSDK_NGX_D3D11_GetCapabilityParameters(parameters: *mut *mut c_void) -> u32;
        fn NVSDK_NGX_D3D11_DestroyParameters(parameters: *mut c_void) -> u32;
        fn NVSDK_NGX_D3D11_CreateFeature(
            context: *mut c_void,
            feature: u32,
            parameters: *mut c_void,
            handle: *mut *mut c_void,
        ) -> u32;
        fn NVSDK_NGX_D3D11_ReleaseFeature(handle: *mut c_void) -> u32;
        fn NVSDK_NGX_D3D11_EvaluateFeature_C(
            context: *mut c_void,
            handle: *const c_void,
            parameters: *mut c_void,
            callback: *const c_void,
        ) -> u32;
        fn NVSDK_NGX_Parameter_GetI(
            parameters: *mut c_void,
            name: *const c_char,
            value: *mut i32,
        ) -> u32;
        fn NVSDK_NGX_Parameter_SetUI(parameters: *mut c_void, name: *const c_char, value: u32);
        fn NVSDK_NGX_Parameter_SetD3d11Resource(
            parameters: *mut c_void,
            name: *const c_char,
            value: *mut c_void,
        );
    }
    fn check(result: u32) -> anyhow::Result<()> {
        anyhow::ensure!(result == 1, "RTX Video SDK status 0x{result:08X}");
        Ok(())
    }
    pub struct Instance {
        device: ID3D11Device,
        parameters: *mut c_void,
        handle: *mut c_void,
    }
    impl Instance {
        pub unsafe fn new(device: &ID3D11Device, data: &std::path::Path) -> anyhow::Result<Self> {
            let root = crate::vsr_runtime::validated_directory(data)?;
            use std::os::windows::ffi::OsStrExt;
            let path: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
            let paths = [path.as_ptr()];
            let common = Common {
                paths: Paths {
                    paths: paths.as_ptr(),
                    count: 1,
                },
                internal: null_mut(),
                logging: Logging {
                    callback: std::ptr::null(),
                    level: 0,
                    other_sinks_disabled: false,
                },
            };
            let _lock = ContextLock::enter(device)?;
            check(NVSDK_NGX_D3D11_Init(
                0,
                path.as_ptr(),
                device.as_raw(),
                &common,
                0x15,
            ))?;
            let mut instance = Self {
                device: device.clone(),
                parameters: null_mut(),
                handle: null_mut(),
            };
            check(NVSDK_NGX_D3D11_GetCapabilityParameters(
                &mut instance.parameters,
            ))?;
            let mut available = 0;
            check(NVSDK_NGX_Parameter_GetI(
                instance.parameters,
                c"VSR.Available".as_ptr(),
                &mut available,
            ))?;
            anyhow::ensure!(available != 0, "RTX VSR is unsupported by this GPU/driver");
            let context = device.GetImmediateContext()?;
            check(NVSDK_NGX_D3D11_CreateFeature(
                context.as_raw(),
                16,
                instance.parameters,
                &mut instance.handle,
            ))?;
            Ok(instance)
        }
        pub unsafe fn evaluate(
            &self,
            context: &ID3D11DeviceContext,
            s: &Surfaces,
            rect: [i32; 4],
            quality: u8,
        ) -> anyhow::Result<()> {
            let _lock = ContextLock::enter(&self.device)?;
            NVSDK_NGX_Parameter_SetD3d11Resource(
                self.parameters,
                c"Input1".as_ptr(),
                s.input.as_raw(),
            );
            NVSDK_NGX_Parameter_SetD3d11Resource(
                self.parameters,
                c"Output".as_ptr(),
                s.output.as_raw(),
            );
            for (name, value) in [
                (c"Rect.X", 0),
                (c"Rect.Y", 0),
                (c"Rect.W", s.key.0),
                (c"Rect.H", s.key.1),
                (c"OutRect.X", rect[0] as u32),
                (c"OutRect.Y", rect[1] as u32),
                (c"OutRect.W", (rect[2] - rect[0]) as u32),
                (c"OutRect.H", (rect[3] - rect[1]) as u32),
                (c"VSR.QualityLevel", u32::from(quality)),
            ] {
                NVSDK_NGX_Parameter_SetUI(self.parameters, name.as_ptr(), value);
            }
            check(NVSDK_NGX_D3D11_EvaluateFeature_C(
                context.as_raw(),
                self.handle,
                self.parameters,
                std::ptr::null(),
            ))
        }
    }
    impl Drop for Instance {
        fn drop(&mut self) {
            unsafe {
                let _lock = ContextLock::enter(&self.device).ok();
                if !self.handle.is_null() {
                    NVSDK_NGX_D3D11_ReleaseFeature(self.handle);
                }
                if !self.parameters.is_null() {
                    NVSDK_NGX_D3D11_DestroyParameters(self.parameters);
                }
                NVSDK_NGX_D3D11_Shutdown1(self.device.as_raw());
            }
        }
    }
}
