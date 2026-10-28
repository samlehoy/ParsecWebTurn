//! Optional GPU video backends. No transport, pixels on CPU or exporter.
#![cfg(windows)]
pub mod direct;
pub mod display;
mod intel;
pub mod intel_vsr;
use anyhow::{Context as _, Result, ensure};
use lowlat_decode::{Fed, hevc, nal};
use lowlat_drivers::{
    amf,
    d3d11::{D3d11, Device, Luid},
};
use self_cell::self_cell;
use std::time::{Duration, Instant};
use windows::{Win32::Graphics::Direct3D11::ID3D11Device, core::Interface};

// AMF can accept Init even when the engine cannot decode the coded dimensions.
// Ask D3D11 on that same device once, before AMF allocates its live decoder.
// No surfaces, compressed input or GPU commands are submitted by this probe.
fn verify_amf_size(device: &ID3D11Device, coded: (u32, u32), bit_depth: u8) -> Result<()> {
    use windows::Win32::Graphics::{Direct3D11::*, Dxgi::Common::*};
    let (profile, format) = match bit_depth {
        8 => (D3D11_DECODER_PROFILE_HEVC_VLD_MAIN, DXGI_FORMAT_NV12),
        10 => (D3D11_DECODER_PROFILE_HEVC_VLD_MAIN10, DXGI_FORMAT_P010),
        _ => anyhow::bail!("unsupported AMF depth"),
    };
    let desc = D3D11_VIDEO_DECODER_DESC {
        Guid: profile,
        SampleWidth: coded.0,
        SampleHeight: coded.1,
        OutputFormat: format,
    };
    let check = || -> Result<()> {
        let video: ID3D11VideoDevice = device.cast().context("D3D11 video device")?;
        // SAFETY: the device and all descriptors live through each synchronous
        // call. The temporary decoder is released before AMF initialization.
        unsafe {
            ensure!(
                video
                    .CheckVideoDecoderFormat(&desc.Guid, desc.OutputFormat)?
                    .as_bool(),
                "D3D11 decoder output format unavailable"
            );
            let count = video.GetVideoDecoderConfigCount(&desc)?;
            let mut last_error = None;
            for index in 0..count {
                let mut config = D3D11_VIDEO_DECODER_CONFIG::default();
                let result = video
                    .GetVideoDecoderConfig(&desc, index, &mut config)
                    .and_then(|()| video.CreateVideoDecoder(&desc, &config));
                match result {
                    Ok(decoder) => {
                        drop(decoder);
                        return Ok(());
                    }
                    Err(error) => last_error = Some(error),
                }
            }
            if let Some(error) = last_error {
                return Err(error.into());
            }
            anyhow::bail!("D3D11 offers no decoder configuration for the coded size")
        }
    };
    check().with_context(|| {
        format!(
            "AMF HEVC hardware-size check: {}x{}, {bit_depth}-bit",
            coded.0, coded.1
        )
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    AmdAmf,
    DirectD3d11,
    IntelVpl,
}
pub fn preferred(vendor: u32, hevc: bool) -> Option<Mode> {
    if !hevc {
        return None;
    }
    match vendor {
        0x1002 => Some(Mode::AmdAmf),
        0x10de => Some(Mode::DirectD3d11),
        0x8086 => Some(Mode::IntelVpl),
        _ => None,
    }
}
pub fn original_surface_vendor_allowed(vendor: u32) -> bool {
    matches!(vendor, 0x10de | 0x8086 | 0x1002)
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
                && !s.is_range_extended()
                && s.bit_depth_luma_minus8 == 0
                && s.bit_depth_chroma_minus8 == 0,
            "unsupported HEVC color format"
        );
        Self::hevc_direct(s)
    }
    pub fn hevc_amf(s: &hevc::sps::Sps, bit_depth: u8) -> Result<Self> {
        ensure!(
            s.chroma_format_idc == 1
                && !s.separate_colour_plane
                && !s.is_range_extended()
                && matches!(bit_depth, 8 | 10)
                && s.bit_depth_luma_minus8 == bit_depth - 8
                && s.bit_depth_chroma_minus8 == s.bit_depth_luma_minus8,
            "unsupported or changed AMF color format"
        );
        Self::geometry(s)
    }
    pub fn hevc_direct(s: &hevc::sps::Sps) -> Result<Self> {
        ensure!(
            matches!(s.chroma_format_idc, 1 | 3)
                && !s.separate_colour_plane
                && (s.bit_depth_luma_minus8 == 0
                    || (s.chroma_format_idc == 3 && s.bit_depth_luma_minus8 == 2))
                && s.bit_depth_chroma_minus8 == s.bit_depth_luma_minus8,
            "unsupported HEVC color format"
        );
        Self::geometry(s)
    }
    fn geometry(s: &hevc::sps::Sps) -> Result<Self> {
        let (width, height) = s.visible();
        let crop = s.conformance_window.unwrap_or([0; 4]);
        ensure!(
            width > 0 && height > 0 && width <= 4096 && height <= 4096,
            "unsupported video geometry"
        );
        Ok(Self {
            left: crop[0] * if s.chroma_format_idc == 3 { 1 } else { 2 },
            top: crop[2] * if s.chroma_format_idc == 3 { 1 } else { 2 },
            width,
            height,
            full_range: s.video_full_range,
        })
    }
}
pub struct Picture<'a> {
    pub device: &'a Device,
    pub sequence: usize,
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
type DirectBackend<'a> = direct::Backend<'a>;
self_cell!(
    struct DirectCell {
        owner: DeviceOwner,
        #[covariant]
        dependent: DirectBackend,
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
    coded_size: (u32, u32),
    visible_size: (u32, u32),
    bit_depth: u8,
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
    Intel(intel::Backend),
}
pub struct Decoder {
    inner: Inner,
    sequence: usize,
}
// AMF 1.4.31 needs the displayed size at Init for cropped streams. Other
// backends retain their coded-size contract. Validate coded limits either way.
fn decoder_init_size(mode: Mode, coded: (u32, u32), visible: (u32, u32)) -> Result<(u32, u32)> {
    ensure!(
        coded.0 > 0 && coded.1 > 0 && coded.0 <= 4096 && coded.1 <= 4096,
        "unsupported decoder size"
    );
    if mode == Mode::AmdAmf {
        ensure!(
            visible.0 > 0 && visible.1 > 0 && visible.0 <= coded.0 && visible.1 <= coded.1,
            "unsupported AMF visible size"
        );
        Ok(visible)
    } else {
        Ok(coded)
    }
}
impl Decoder {
    pub fn open(
        mode: Mode,
        luid: u64,
        size: (u32, u32),
        visible_size: (u32, u32),
        bit_depth: u8,
    ) -> Result<Self> {
        let init_size = decoder_init_size(mode, size, visible_size)?;
        let device = DeviceOwner::open(luid)?;
        let inner = match mode {
            Mode::IntelVpl => Inner::Intel(intel::Backend::open(device, size)?),
            Mode::DirectD3d11 => Inner::Direct(DirectCell::new(device, |owner| {
                direct::Backend::new(&owner.device)
            })),
            Mode::AmdAmf => {
                let format = match bit_depth {
                    8 => amf::Layout::Nv12,
                    10 => amf::Layout::P010,
                    _ => anyhow::bail!("unsupported AMF depth"),
                };
                let raw = device.device.device().cast();
                // SAFETY: borrow the very device AMF will use; no second adapter.
                let native = unsafe { ID3D11Device::from_raw_borrowed(&raw) }
                    .context("missing AMF D3D11 device")?;
                verify_amf_size(native, size, bit_depth)?;
                let owner = AmfOwner {
                    device,
                    runtime: amf::Amf::load()?,
                };
                let context = ContextCell::try_new(owner, |o| o.runtime.context(&o.device.device))?;
                let state = AmfCell::try_new(context, |c| -> Result<AmfState<'_>> {
                    let decoder = c.borrow_owner().runtime.decoder_with_layout(
                        c.borrow_dependent(),
                        amf::Codec::Hevc,
                        format,
                    )?;
                    decoder.set_int(amf::REORDER_MODE, amf::REORDER_LOW_LATENCY)?;
                    ensure!(
                        decoder.has(amf::LOW_LATENCY_DECODE),
                        "AMF low latency property unavailable"
                    );
                    decoder.set_bool(amf::LOW_LATENCY_DECODE, true)?;
                    decoder.set_bool("SurfaceCopy", false)?;
                    decoder.set_bool("EnableDecoderSmartAccessVideo", false)?;
                    decoder.init(format, init_size.0, init_size.1)?;
                    let buffer = c.borrow_dependent().buffer(8 * 1024 * 1024)?;
                    Ok(AmfState {
                        decoder,
                        buffer,
                        layout: None,
                        coded_size: size,
                        visible_size: init_size,
                        bit_depth,
                        sequence: 0,
                    })
                })?;
                Inner::Amd(state)
            }
        };
        Ok(Self { inner, sequence: 0 })
    }
    /// Reset only the direct decoder's reference/surface pool on its owner thread.
    /// The D3D11 device is unchanged, so attached rendering/effect state stays valid.
    pub fn reset_direct(&mut self) -> bool {
        if let Inner::Direct(cell) = &mut self.inner {
            cell.with_dependent_mut(|owner, backend| {
                *backend = direct::Backend::new(&owner.device)
            });
            self.sequence = 0;
            true
        } else {
            false
        }
    }
    fn device(&self) -> &Device {
        match &self.inner {
            Inner::Direct(c) => &c.borrow_owner().device,
            Inner::Amd(c) => &c.borrow_owner().borrow_owner().device.device,
            Inner::Intel(c) => c.device(),
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
            Inner::Intel(_) => "Intel VPL HEVC",
        }
    }
    pub fn low_latency_request_accepted(&self) -> bool {
        matches!(self.inner, Inner::Amd(_) | Inner::Intel(_))
    }
    /// Returns all decoded outputs. Under renderer backpressure an output may
    /// be released without invoking `consume`; encoded input is still decoded.
    pub fn feed(
        &mut self,
        bytes: &[u8],
        mut consume: impl FnMut(Picture<'_>) -> Result<()>,
    ) -> Result<usize> {
        let sequence = self.sequence;
        self.sequence = self.sequence.wrapping_add(1);
        match &mut self.inner {
                        Inner::Intel(backend) => backend.feed(bytes, sequence, consume),
            Inner::Direct(cell)=>cell.with_dependent_mut(|owner,decoder| {
                let fed=decoder.feed(bytes,sequence)?;
                ensure!(fed!=Fed::FormatChanged,"direct decoder format changed");
                if fed!=Fed::Picture {return Ok(0);}
                let layout=decoder.layout()?;
                let mut count=0;
                while decoder.take(|raw,slice,sequence| {
                    let raw=raw.cast();
                    // SAFETY: the DPB lease is held until consume queues the
                    // existing processor read on the very same device.
                    let texture=unsafe {windows::Win32::Graphics::Direct3D11::ID3D11Texture2D::from_raw_borrowed(&raw)}.context("missing direct decoder texture")?;
                    consume(Picture {device:&owner.device,sequence,texture,slice,layout})?;count+=1;Ok(())
                })? {}
                Ok(count)
            }),
            Inner::Amd(cell)=>cell.with_dependent_mut(|context,state| {
                let mut picture=false;
                for unit in nal::Units::new(bytes) {
                    let Some(&first)=unit.bytes.first() else {continue;};
                    let kind=(first>>1)&63;
                    picture |= kind<=31;
                    if kind==33 {
                        let sps=hevc::sps::parse(unit.bytes.get(2..).context("truncated HEVC SPS")?).map_err(|error|anyhow::anyhow!("HEVC SPS: {error:?}"))?;
                        ensure!(sps.max_num_reorder_pics==0,"AMF low latency requires a non-reordered stream");
                        let layout=Layout::hevc_amf(&sps,state.bit_depth)?;
                        ensure!((sps.width,sps.height)==state.coded_size && (layout.width,layout.height)==state.visible_size,"AMF geometry changed");
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
                        consume(Picture {device:&context.borrow_owner().device.device,sequence:(surface.pts()-1) as usize,texture,slice:0,layout})?;return Ok(1);
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
    fn amf_size_probe_rejects_software_hevc_without_an_amf_runtime() {
        use windows::Win32::Graphics::{Direct3D::D3D_DRIVER_TYPE_WARP, Direct3D11::*};
        let mut device = None;
        // SAFETY: WARP is an isolated software device; no live session/GPU state.
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_WARP,
                windows::Win32::Foundation::HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                None,
            )
            .unwrap();
        }
        let device = device.unwrap();
        for depth in [8, 10] {
            let error = verify_amf_size(&device, (1920, 1088), depth).unwrap_err();
            let message = format!("{error:#}");
            assert!(message.contains("AMF HEVC hardware-size check: 1920x1088"));
            assert!(message.contains(&format!("{depth}-bit")));
        }
        assert!(verify_amf_size(&device, (1920, 1088), 12).is_err());
    }
    #[test]
    fn cropped_amf_init_uses_visible_size_but_other_backends_keep_coded_size() {
        assert_eq!(
            decoder_init_size(Mode::AmdAmf, (1920, 1088), (1920, 1080)).unwrap(),
            (1920, 1080)
        );
        assert_eq!(
            decoder_init_size(Mode::AmdAmf, (1920, 1080), (1920, 1080)).unwrap(),
            (1920, 1080)
        );
        for mode in [Mode::IntelVpl, Mode::DirectD3d11] {
            assert_eq!(
                decoder_init_size(mode, (1920, 1088), (1920, 1080)).unwrap(),
                (1920, 1088)
            );
        }
        for (coded, visible) in [
            ((4097, 1088), (1920, 1080)),
            ((1920, 1088), (0, 1080)),
            ((1920, 1088), (1922, 1080)),
            ((1920, 1088), (1920, 1090)),
        ] {
            assert!(decoder_init_size(Mode::AmdAmf, coded, visible).is_err());
        }
        let bytes = include_bytes!("../fixtures/hevc-crop.sps");
        let sps = hevc::sps::parse(&bytes[5..]).unwrap();
        let layout = Layout::hevc(&sps).unwrap();
        assert_eq!((sps.width, sps.height), (152, 152));
        assert_eq!((layout.width, layout.height), (150, 146));
        assert_eq!(
            decoder_init_size(Mode::AmdAmf, (sps.width, sps.height), sps.visible()).unwrap(),
            (150, 146)
        );
    }
    #[test]
    fn only_measured_hevc_gpu_routes_are_selected() {
        assert_eq!(preferred(0x1002, true), Some(Mode::AmdAmf));
        assert_eq!(preferred(0x10de, true), Some(Mode::DirectD3d11));
        for vendor in [0x1002, 0x10de, 0x8086, 0] {
            assert_eq!(preferred(vendor, false), None);
        }
        assert_eq!(preferred(0x8086, true), Some(Mode::IntelVpl));
        assert!(original_surface_vendor_allowed(0x10de));
        assert!(original_surface_vendor_allowed(0x1002));
        assert!(original_surface_vendor_allowed(0x8086));
        assert!(!original_surface_vendor_allowed(0));
    }
    #[test]
    fn amf_main10_keeps_geometry_and_rejects_depth_or_chroma_changes() {
        let bytes = include_bytes!("../fixtures/hevc-crop.sps");
        let mut sps = hevc::sps::parse(&bytes[5..]).unwrap();
        assert!(Layout::hevc_amf(&sps, 8).is_ok());
        assert!(Layout::hevc_amf(&sps, 10).is_err());
        sps.bit_depth_luma_minus8 = 2;
        sps.bit_depth_chroma_minus8 = 2;
        let layout = Layout::hevc_amf(&sps, 10).unwrap();
        assert_eq!((layout.width, layout.height), (150, 146));
        assert!(Layout::hevc(&sps).is_err()); // Intel's existing 8-bit contract.
        assert!(Layout::hevc_amf(&sps, 8).is_err());
        assert!(Layout::hevc_amf(&sps, 12).is_err());
        sps.bit_depth_chroma_minus8 = 0;
        assert!(Layout::hevc_amf(&sps, 10).is_err());
        sps.bit_depth_chroma_minus8 = 2;
        sps.chroma_format_idc = 3;
        assert!(Layout::hevc_amf(&sps, 10).is_err());
    }
}
