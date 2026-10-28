//! Optional GPU postprocessing. The normal/zero-copy video paths stay separate.
use windows::{
    core::Interface,
    Win32::Graphics::{
        Direct3D11::*,
        Dxgi::{Common::*, IDXGIDevice},
    },
};

pub enum Effect {
    Nvidia(crate::nvidia_vsr::Vsr),
    #[cfg(feature = "vendor-video")]
    Intel(evoparseq_video::intel_vsr::IntelAiVsr),
}
impl Effect {
    pub unsafe fn new(device: &ID3D11Device, quality: u8) -> anyhow::Result<Self> {
        let vendor = device
            .cast::<IDXGIDevice>()?
            .GetAdapter()?
            .GetDesc()?
            .VendorId;
        match vendor {
            0x10de => Ok(Self::Nvidia(crate::nvidia_vsr::Vsr::new(device, quality)?)),
            #[cfg(feature = "vendor-video")]
            0x8086 => Ok(Self::Intel(evoparseq_video::intel_vsr::IntelAiVsr::new(
                device,
            )?)),
            _ => anyhow::bail!("AI upscaling is unavailable for the selected GPU"),
        }
    }
    pub fn vendor(&self) -> &'static str {
        match self {
            Self::Nvidia(_) => "NVIDIA",
            #[cfg(feature = "vendor-video")]
            Self::Intel(_) => "Intel",
        }
    }
    pub fn format(&self) -> DXGI_FORMAT {
        match self {
            Self::Nvidia(_) => DXGI_FORMAT_R8G8B8A8_UNORM,
            #[cfg(feature = "vendor-video")]
            Self::Intel(_) => DXGI_FORMAT_B8G8R8A8_UNORM,
        }
    }
    pub fn warming(&self) -> bool {
        match self {
            Self::Nvidia(v) => v.warming(),
            #[cfg(feature = "vendor-video")]
            Self::Intel(v) => v.warming(),
        }
    }
    pub fn needs_setup(&self, source: (u32, u32), target: (u32, u32), viewport: [i32; 4]) -> bool {
        match self {
            Self::Nvidia(v) => v.warming(),
            #[cfg(feature = "vendor-video")]
            Self::Intel(v) => v.needs_setup(source, target, viewport),
        }
    }
    pub unsafe fn prepare(
        &mut self,
        device: &ID3D11Device,
        source: (u32, u32),
        target: (u32, u32),
        viewport: [i32; 4],
        format: DXGI_FORMAT,
    ) -> anyhow::Result<(ID3D11Texture2D, ID3D11Texture2D)> {
        match self {
            Self::Nvidia(v) => {
                let s = v.prepare(device, source, target, format)?;
                Ok((s.input.clone(), s.output.clone()))
            }
            #[cfg(feature = "vendor-video")]
            Self::Intel(v) => v.prepare(source, target, viewport, format),
        }
    }
    pub unsafe fn evaluate(
        &mut self,
        context: &ID3D11DeviceContext,
        target: [i32; 4],
    ) -> anyhow::Result<()> {
        match self {
            Self::Nvidia(v) => v.evaluate(context, target),
            #[cfg(feature = "vendor-video")]
            Self::Intel(v) => v.evaluate(context),
        }
    }
    pub unsafe fn copy_output(
        &self,
        context: &ID3D11DeviceContext,
        back: &ID3D11Texture2D,
        output: &ID3D11Texture2D,
        size: (u32, u32),
    ) {
        match self {
            Self::Nvidia(_) => context.CopyResource(back, output),
            #[cfg(feature = "vendor-video")]
            Self::Intel(_) => context.CopySubresourceRegion(
                back,
                0,
                0,
                0,
                0,
                output,
                0,
                Some(&D3D11_BOX {
                    left: 0,
                    top: 0,
                    front: 0,
                    right: size.0,
                    bottom: size.1,
                    back: 1,
                }),
            ),
        }
    }
}
