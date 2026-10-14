//! Optional 8-bit GPU video backends. No transport, pixels on CPU or exporter.
#![cfg(windows)]
pub mod direct;
use anyhow::{Context as _, Result, ensure};
use lowlat_core::video::Codec;
use lowlat_decode::{Fed, hevc, nal};
use lowlat_drivers::{
    amf,
    d3d11::{D3d11, Device, Luid},
};
use self_cell::self_cell;
use std::time::{Duration, Instant};
use windows::{Win32::Graphics::Direct3D11::ID3D11Device, core::Interface};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    AmdAmf,
    DirectD3d11,
}
pub fn preferred(vendor: u32, hevc: bool) -> Option<Mode> {
    if !hevc {
        return None;
    }
    match vendor {
        0x1002 => Some(Mode::AmdAmf),
        0x10de => Some(Mode::DirectD3d11),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub left: u32,
    pub top: u32,
    pub width: u32,
    pub height: u32,
    pub full_range: bool,
}
impl Layout {
    pub fn hevc(s: &hevc::sps::Sps) -> Result<Self> {
        ensure!(
            s.chroma_format_idc == 1
                && s.bit_depth_luma_minus8 == 0
                && s.bit_depth_chroma_minus8 == 0
                && !s.is_range_extended(),
            "unsupported HEVC color format"
        );
        let (width, height) = s.visible();
        let crop = s.conformance_window.unwrap_or([0; 4]);
        ensure!(
            width > 0 && height > 0 && width <= 4096 && height <= 4096,
            "unsupported video geometry"
        );
        Ok(Self {
            left: crop[0] * 2,
            top: crop[2] * 2,
            width,
            height,
            full_range: s.video_full_range,
        })
    }
}
pub struct Picture<'a> {
    pub texture: &'a windows::Win32::Graphics::Direct3D11::ID3D11Texture2D,
    pub slice: u32,
    pub layout: Layout,
}

struct DeviceOwner {
    device: Device,
    _runtime: D3d11,
}
impl DeviceOwner {
    fn open(luid: u64) -> Result<Self> {
        let runtime = D3d11::load()?;
        let device = runtime.open(Luid {
            high: (luid >> 32) as i32,
            low: luid as u32,
        })?;
        ensure!(device.protect(), "D3D11 multithread protection unavailable");
        Ok(Self {
            device,
            _runtime: runtime,
        })
    }
}
self_cell!(
    struct DirectCell {
        owner: DeviceOwner,
        #[covariant]
        dependent: direct::Backend,
    }
);
struct AmfOwner {
    device: DeviceOwner,
    runtime: amf::Amf,
}
type AmfContext<'a> = amf::Context<'a>;
self_cell!(
    struct ContextCell {
        owner: AmfOwner,
        #[covariant]
        dependent: AmfContext,
    }
);
struct AmfState<'a> {
    decoder: amf::Component<'a>,
    buffer: amf::Buffer<'a>,
    layout: Option<Layout>,
    size: (u32, u32),
    sequence: i64,
}
self_cell!(
    struct AmfCell {
        owner: ContextCell,
        #[covariant]
        dependent: AmfState,
    }
);
enum Inner {
    Direct(DirectCell),
    Amd(AmfCell),
}
pub struct Decoder {
    inner: Inner,
    sequence: usize,
}
impl Decoder {
    pub fn open(mode: Mode, luid: u64, size: (u32, u32)) -> Result<Self> {
        ensure!(
            size.0 > 0 && size.1 > 0 && size.0 <= 4096 && size.1 <= 4096,
            "unsupported decoder size"
        );
        let device = DeviceOwner::open(luid)?;
        let inner = match mode {
            Mode::DirectD3d11 => Inner::Direct(DirectCell::new(device, |owner| {
                direct::Backend::new(&owner.device, Codec::H265)
            })),
            Mode::AmdAmf => {
                let owner = AmfOwner {
                    device,
                    runtime: amf::Amf::load()?,
                };
                let context = ContextCell::try_new(owner, |o| o.runtime.context(&o.device.device))?;
                let state = AmfCell::try_new(context, |c| -> Result<AmfState<'_>> {
                    let decoder = c
                        .borrow_owner()
                        .runtime
                        .decoder(c.borrow_dependent(), amf::Codec::Hevc)?;
                    decoder.set_int(amf::REORDER_MODE, amf::REORDER_LOW_LATENCY)?;
                    ensure!(
                        decoder.has(amf::LOW_LATENCY_DECODE),
                        "AMF low latency property unavailable"
                    );
                    decoder.set_bool(amf::LOW_LATENCY_DECODE, true)?;
                    decoder.set_bool("SurfaceCopy", false)?;
                    decoder.set_bool("EnableDecoderSmartAccessVideo", false)?;
                    decoder.init(amf::Layout::Nv12, size.0, size.1)?;
                    let buffer = c.borrow_dependent().buffer(8 * 1024 * 1024)?;
                    Ok(AmfState {
                        decoder,
                        buffer,
                        layout: None,
                        size,
                        sequence: 0,
                    })
                })?;
                Inner::Amd(state)
            }
        };
        Ok(Self { inner, sequence: 0 })
    }
    fn device(&self) -> &Device {
        match &self.inner {
            Inner::Direct(c) => &c.borrow_owner().device,
            Inner::Amd(c) => &c.borrow_owner().borrow_owner().device.device,
        }
    }
    pub fn native_device(&self) -> Result<ID3D11Device> {
        let raw = self.device().device().cast();
        // SAFETY: clone a COM reference; the decoder owns its native device.
        Ok(unsafe { ID3D11Device::from_raw_borrowed(&raw) }
            .context("missing D3D11 device")?
            .clone())
    }
    pub fn name(&self) -> &'static str {
        match &self.inner {
            Inner::Direct(_) => "Direct D3D11 HEVC",
            Inner::Amd(_) => "AMD AMF HEVC",
        }
    }
    pub fn low_latency_request_accepted(&self) -> bool {
        matches!(self.inner, Inner::Amd(_))
    }
    pub fn feed(
        &mut self,
        bytes: &[u8],
        mut consume: impl FnMut(Picture<'_>) -> Result<()>,
    ) -> Result<usize> {
        let sequence = self.sequence;
        self.sequence = self.sequence.wrapping_add(1);
        match &mut self.inner {
            Inner::Direct(cell)=>cell.with_dependent_mut(|_,decoder| {
                let fed=decoder.feed(bytes,sequence)?;
                ensure!(fed!=Fed::FormatChanged,"direct decoder format changed");
                if fed!=Fed::Picture {return Ok(0);}
                let layout=decoder.layout()?;
                let mut count=0;
                while decoder.take(|raw,slice,_| {
                    let raw=raw.cast();
                    // SAFETY: the DPB lease is held until consume queues the
                    // existing processor read on the very same device.
                    let texture=unsafe {windows::Win32::Graphics::Direct3D11::ID3D11Texture2D::from_raw_borrowed(&raw)}.context("missing direct decoder texture")?;
                    consume(Picture {texture,slice,layout})?;count+=1;Ok(())
                })? {}
                Ok(count)
            }),
            Inner::Amd(cell)=>cell.with_dependent_mut(|_,state| {
                let mut picture=false;
                for unit in nal::Units::new(bytes) {
                    let Some(&first)=unit.bytes.first() else {continue;};
                    let kind=(first>>1)&63;
                    picture |= kind<=31;
                    if kind==33 {
                        let sps=hevc::sps::parse(unit.bytes.get(2..).context("truncated HEVC SPS")?)?;
                        ensure!(sps.max_num_reorder_pics==0,"AMF low latency requires a non-reordered stream");
                        let layout=Layout::hevc(&sps)?;
                        ensure!((sps.width,sps.height)==state.size,"AMF geometry changed");
                        state.layout=Some(layout);
                    }
                }
                state.buffer.fill(bytes)?;state.sequence+=1;state.buffer.set_pts(state.sequence);
                ensure!(matches!(state.decoder.submit(&state.buffer)?,amf::Submitted::Taken|amf::Submitted::MoreInput),"AMF input refused");
                if !picture {return Ok(0);}
                let layout=state.layout.context("AMF requires actual SPS metadata")?;
                let until=Instant::now()+Duration::from_millis(20);
                loop {
                    if let Some(surface)=state.decoder.query()? {
                        ensure!(surface.pts()==state.sequence,"AMF output order mismatch");
                        let raw=surface.texture();
                        // SAFETY: the AMF surface lives through consume; no
                        // CPU wait for GPU completion, readback or export.
                        let texture=unsafe {windows::Win32::Graphics::Direct3D11::ID3D11Texture2D::from_raw_borrowed(&raw)}.context("missing AMF texture")?;
                        consume(Picture {texture,slice:0,layout})?;return Ok(1);
                    }
                    ensure!(Instant::now()<until,"AMF output availability timeout");
                    std::thread::yield_now();
                }
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_measured_hevc_gpu_routes_are_selected() {
        assert_eq!(preferred(0x1002, true), Some(Mode::AmdAmf));
        assert_eq!(preferred(0x10de, true), Some(Mode::DirectD3d11));
        for vendor in [0x1002, 0x10de, 0x8086, 0] {
            assert_eq!(preferred(vendor, false), None);
        }
        assert_eq!(preferred(0x8086, true), None);
    }
}
