//! Optional documented Intel VPL AI scaler, separate from all decoder backends.
use anyhow::{Context, Result, ensure};
use lowlat_drivers::{
    d3d11::D3d11,
    vpl::{Vpl, ai::Scaler},
};
use windows::{
    Win32::Graphics::{
        Direct3D10::ID3D10Multithread,
        Direct3D11::*,
        Dxgi::{Common::*, IDXGIDevice},
    },
    core::Interface,
};

struct Resources {
    scaler: Scaler,
    input: ID3D11Texture2D,
    output: ID3D11Texture2D,
    clear: ID3D11RenderTargetView,
    key: ((u32, u32), (u32, u32), [u32; 4]),
}
pub struct IntelAiVsr {
    resources: Option<Resources>,
    device: ID3D11Device,
    evaluated: bool,
}
impl IntelAiVsr {
    pub unsafe fn new(device: &ID3D11Device) -> Result<Self> {
        let desc = unsafe { device.cast::<IDXGIDevice>()?.GetAdapter()?.GetDesc()? };
        ensure!(
            desc.VendorId == 0x8086,
            "Intel GPU required for VPL AI Super Resolution"
        );
        let lock: ID3D10Multithread = device.cast()?;
        unsafe {
            lock.SetMultithreadProtected(true);
        }
        ensure!(
            unsafe { lock.GetMultithreadProtected().as_bool() },
            "Intel VPL requires protected D3D11 context"
        );
        Ok(Self {
            resources: None,
            device: device.clone(),
            evaluated: false,
        })
    }
    pub fn warming(&self) -> bool {
        !self.evaluated
    }
    pub fn needs_setup(&self, source: (u32, u32), target: (u32, u32), viewport: [i32; 4]) -> bool {
        let key = (source, target, viewport.map(|v| v as u32));
        self.resources.as_ref().is_none_or(|r| r.key != key)
    }
    pub unsafe fn prepare(
        &mut self,
        source: (u32, u32),
        target: (u32, u32),
        viewport: [i32; 4],
        format: DXGI_FORMAT,
    ) -> Result<(ID3D11Texture2D, ID3D11Texture2D)> {
        ensure!(
            format == DXGI_FORMAT_B8G8R8A8_UNORM,
            "Intel AI upscaling currently supports 8-bit SDR output only"
        );
        ensure!(
            viewport.iter().all(|&v| v >= 0),
            "Invalid Intel AI viewport"
        );
        let viewport = viewport.map(|v| v as u32);
        let key = (source, target, viewport);
        if self.resources.as_ref().is_none_or(|r| r.key != key) {
            // Native VPP close and resource retirement happen on this thread.
            self.resources = None;
            self.evaluated = false;
            let device = &self.device;
            let desc = unsafe { device.cast::<IDXGIDevice>()?.GetAdapter()?.GetDesc()? };
            let luid = ((desc.AdapterLuid.HighPart as u32 as u64) << 32)
                | u64::from(desc.AdapterLuid.LowPart);
            let d3d = D3d11::load()?;
            let adapter = d3d
                .adapters()?
                .into_iter()
                .find(|a| a.luid.value() == luid)
                .context("Intel adapter unavailable")?;
            let index = d3d
                .plain_index(adapter.luid)?
                .context("Intel adapter index unavailable")?;
            let runtime = Vpl::for_adapter(&adapter)?;
            let scaler =
                unsafe { Scaler::new(runtime, device.as_raw(), index, source, target, viewport)? };
            let input = unsafe {
                ID3D11Texture2D::from_raw_borrowed(&scaler.input_texture(device.as_raw())?)
                    .context("Intel AI input missing")?
                    .clone()
            };
            let output = unsafe {
                ID3D11Texture2D::from_raw_borrowed(&scaler.output_texture(device.as_raw())?)
                    .context("Intel AI output missing")?
                    .clone()
            };
            for (texture, size) in [(&input, source), (&output, target)] {
                let mut td = D3D11_TEXTURE2D_DESC::default();
                unsafe {
                    texture.GetDesc(&mut td);
                }
                ensure!(
                    td.Format == format
                        && td.ArraySize == 1
                        && td.MipLevels == 1
                        && td.Width >= size.0
                        && td.Height >= size.1
                        && td.Usage == D3D11_USAGE_DEFAULT
                        && td.CPUAccessFlags == 0,
                    "Intel AI runtime did not provide compatible native GPU textures"
                );
                let actual = unsafe { texture.GetDevice()? };
                ensure!(
                    actual.as_raw() == device.as_raw(),
                    "Intel AI texture is on a different D3D11 device"
                );
            }
            let mut clear = None;
            unsafe {
                device.CreateRenderTargetView(&output, None, Some(&mut clear))?;
            }
            self.resources = Some(Resources {
                scaler,
                input,
                output,
                clear: clear.context("Intel AI output view missing")?,
                key,
            });
        }
        let r = self.resources.as_ref().unwrap();
        Ok((r.input.clone(), r.output.clone()))
    }
    pub unsafe fn evaluate(&mut self, context: &ID3D11DeviceContext) -> Result<()> {
        let r = self
            .resources
            .as_ref()
            .context("Intel AI surfaces missing")?;
        unsafe {
            context.ClearRenderTargetView(&r.clear, &[0.0, 0.0, 0.0, 1.0]);
            // Flush submits GPU writes, not a CPU pixel transfer or completion wait.
            context.Flush();
            r.scaler.evaluate()?;
        }
        self.evaluated = true;
        Ok(())
    }
}
