//! GPU-only D3D11 decoder. Adapted from lowlat d3d11.rs at 5f8db568.
//! MIT Copyright (c) 2026 Nguyen Duy; see ../LICENSE-lowlat.
//! Only 8-bit 4:2:0: original decoder array slice, no split/export/staging/map.
use core::{ffi::c_void, fmt};
use lowlat_core::video::Codec;
use lowlat_decode::h264::dpb::{Parity, Structure};
use lowlat_decode::h264::sps::{ZIGZAG_4X4, ZIGZAG_8X8};
use lowlat_decode::hevc::sps::{DIAG_4X4, DIAG_8X8};
use lowlat_decode::{Fed, h264, hevc};
use lowlat_drivers::{
    d3d11::{Com, Device, Error as RuntimeError},
    ffi::{
        d3d11::*,
        d3d11_guids::{D3D11_DECODER_PROFILE_H264_VLD_NOFGT, D3D11_DECODER_PROFILE_HEVC_VLD_MAIN},
    },
    vcall,
};
use std::rc::Rc;
/// The decoder only needs these documented device operations. The normal
/// lowlat device stays unchanged; displayable output uses its own flagged device.
pub trait DeviceAccess {
    fn video(&self) -> *mut ID3D11VideoDevice;
    fn video_context(&self) -> *mut ID3D11VideoContext;
    fn texture(
        &self,
        desc: &D3D11_TEXTURE2D_DESC,
    ) -> core::result::Result<Com<ID3D11Texture2D>, RuntimeError>;
}
impl DeviceAccess for Device {
    fn video(&self) -> *mut ID3D11VideoDevice {
        self.video()
    }
    fn video_context(&self) -> *mut ID3D11VideoContext {
        self.video_context()
    }
    fn texture(
        &self,
        desc: &D3D11_TEXTURE2D_DESC,
    ) -> core::result::Result<Com<ID3D11Texture2D>, RuntimeError> {
        self.texture(desc)
    }
}
/// Presentation retains a decoder surface until it can be safely written again.
pub trait SurfaceLease {
    fn available(&self) -> bool;
}
struct SurfaceLeases(Vec<Option<Rc<dyn SurfaceLease>>>);
impl SurfaceLeases {
    fn new() -> Self {
        Self((0..SURFACES).map(|_| None).collect())
    }
    fn retire(&mut self, mut released: impl FnMut(usize)) {
        for (slot, lease) in self.0.iter_mut().enumerate() {
            if lease.as_ref().is_some_and(|l| l.available()) {
                *lease = None;
                released(slot);
            }
        }
    }
    fn pinned(&self, slot: usize) -> bool {
        self.0[slot].is_some()
    }
    fn writable(&self, slot: usize) -> bool {
        self.0[slot].as_ref().is_none_or(|lease| lease.available())
    }
}
const SURFACES: usize = h264::dpb::MAX_FRAMES;
const MAX_SLICES: usize = h264::MAX_SLICES;
const START_CODE: [u8; 3] = [0, 0, 1];
const BITSTREAM_ALIGN: usize = 128;
const SHORT_H264: u32 = 2;
const SHORT_HEVC: u32 = 1;
const NO_ENTRY: u8 = 0xff;
/// A result code as the system spells it, in hex.
const fn hresult(code: u32) -> HRESULT {
    HRESULT::from_ne_bytes(code.to_ne_bytes())
}

/// The device's answer that the surface asked for is still being read and
/// cannot be written yet; asked again after a short sleep, a bounded number
/// of times.
const E_PENDING: HRESULT = hresult(0x8000_000A_u32);
const PENDING_TRIES: u32 = 50;
/// The device is gone: removed, hung, or reset.
const DEVICE_LOST: [HRESULT; 3] = [
    hresult(0x887A_0005_u32),
    hresult(0x887A_0006_u32),
    hresult(0x887A_0007_u32),
];

/// Why a decoder could not be built or a picture could not be decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The system's libraries, the device, or the device lost.
    Runtime(RuntimeError),
    /// The device decodes no profile this stream needs, or not in the
    /// short slice form.
    NoProfile,
    /// A call failed, with its result.
    Status(i32),
    /// A picture larger than the surfaces or the buffers.
    TooLarge,
    SurfaceBusy,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Runtime(e) => write!(f, "{e}"),
            Self::NoProfile => f.write_str("device decodes no profile this stream needs"),
            Self::Status(s) => write!(f, "system video call returned 0x{s:08x}"),
            Self::SurfaceBusy => f.write_str("presentation still owns decoder surface"),
            Self::TooLarge => f.write_str("picture larger than the surfaces"),
        }
    }
}

impl std::error::Error for Error {}

impl From<RuntimeError> for Error {
    fn from(e: RuntimeError) -> Self {
        Self::Runtime(e)
    }
}

pub(crate) type Result<T> = core::result::Result<T, Error>;

/// A failed result as an error: the device lost is the runtime's, which
/// no fresh decoder on it survives.
pub(crate) fn check(hr: HRESULT) -> Result<()> {
    if hr >= 0 {
        Ok(())
    } else if DEVICE_LOST.contains(&hr) {
        Err(Error::Runtime(RuntimeError::Status(hr)))
    } else {
        Err(Error::Status(hr))
    }
}

// SAFETY: every structure this is used for is plain data the device's
// headers define, whose all-zero value is valid and is where a fill starts.
fn zeroed<T>() -> T {
    unsafe { core::mem::zeroed() }
}

/// The bytes of a staged structure, as the device's buffer takes them.
fn bytes_of<T>(value: &T) -> &[u8] {
    // SAFETY: a plain-data structure read as the bytes it is made of, for
    // as long as it is borrowed.
    unsafe { core::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Shape {
    codec: Codec,
}
impl Shape {
    fn base(codec: Codec) -> Self {
        Self { codec }
    }
    fn profile(self) -> &'static GUID {
        match self.codec {
            Codec::H264 => &D3D11_DECODER_PROFILE_H264_VLD_NOFGT,
            Codec::H265 => &D3D11_DECODER_PROFILE_HEVC_VLD_MAIN,
        }
    }
    fn surface(self) -> DXGI_FORMAT {
        DXGI_FORMAT_NV12
    }
    fn short_slices(self) -> u32 {
        if self.codec == Codec::H264 {
            SHORT_H264
        } else {
            SHORT_HEVC
        }
    }
    fn alignment(self) -> u32 {
        if self.codec == Codec::H264 { 16 } else { 128 }
    }
}
fn offers(device: &dyn DeviceAccess, shape: Shape) -> bool {
    let mut supported: BOOL = 0;
    // SAFETY: the device and the profile/output pointers remain live.
    let hr = unsafe {
        vcall!(
            device.video(),
            CheckVideoDecoderFormat,
            shape.profile(),
            shape.surface(),
            &raw mut supported
        )
    };
    hr.is_some_and(|hr| hr >= 0) && supported != 0
}

fn create_decoder(
    device: &dyn DeviceAccess,
    shape: Shape,
    width: u32,
    height: u32,
) -> Result<(Com<ID3D11VideoDecoder>, D3D11_VIDEO_DECODER_CONFIG)> {
    if !offers(device, shape) {
        return Err(Error::NoProfile);
    }
    let desc = D3D11_VIDEO_DECODER_DESC {
        Guid: *shape.profile(),
        SampleWidth: width,
        SampleHeight: height,
        OutputFormat: shape.surface(),
    };
    let mut count = 0u32;
    // SAFETY: a live video device; the description and output are live.
    let hr = unsafe {
        vcall!(
            device.video(),
            GetVideoDecoderConfigCount,
            &raw const desc,
            &raw mut count
        )
    }
    .ok_or(Error::NoProfile)?;
    check(hr).map_err(|_| Error::NoProfile)?;
    for index in 0..count {
        let mut config: D3D11_VIDEO_DECODER_CONFIG = zeroed();
        // SAFETY: as above.
        let hr = unsafe {
            vcall!(
                device.video(),
                GetVideoDecoderConfig,
                &raw const desc,
                index,
                &raw mut config
            )
        }
        .ok_or(Error::NoProfile)?;
        if hr < 0 || config.ConfigBitstreamRaw != shape.short_slices() {
            continue;
        }
        let mut decoder: *mut ID3D11VideoDecoder = core::ptr::null_mut();
        // SAFETY: as above; the configuration is the device's own.
        let hr = unsafe {
            vcall!(
                device.video(),
                CreateVideoDecoder,
                &raw const desc,
                &raw const config,
                &raw mut decoder
            )
        }
        .ok_or(Error::NoProfile)?;
        check(hr).map_err(|e| match e {
            Error::Runtime(_) => e,
            _ => Error::NoProfile,
        })?;
        // SAFETY: a decoder whose reference the call handed over.
        let decoder = unsafe { Com::from_raw(decoder) }.ok_or(Error::NoProfile)?;
        return Ok((decoder, config));
    }
    Err(Error::NoProfile)
}

struct Staging {
    h264_picture: DXVA_PicParams_H264,
    h264_matrix: DXVA_Qmatrix_H264,
    h264_slices: [DXVA_Slice_H264_Short; MAX_SLICES],
    // HEVC Main 8-bit picture parameters.
    hevc_picture: DXVA_PicParams_HEVC,
    hevc_matrix: DXVA_Qmatrix_HEVC,
    hevc_slices: [DXVA_Slice_HEVC_Short; MAX_SLICES],
}

struct Built {
    shape: Shape,
    coded: (u32, u32),
    decoder: Com<ID3D11VideoDecoder>,
    surfaces: Vec<Com<ID3D11Texture2D>>,
    leases: SurfaceLeases,
    views: Vec<Com<ID3D11VideoDecoderOutputView>>,
}
pub struct Backend<'a> {
    device: &'a dyn DeviceAccess,
    ceiling: (u32, u32),
    codec: Codec,
    built: Option<Built>,
    h264: Box<h264::Stream>,
    hevc: Box<hevc::Stream>,
    staging: Box<Staging>,
    ranges: [(usize, usize); MAX_SLICES],
    report: u32,
    sequence: usize,
    stamps: [Option<usize>; SURFACES],
    displayable: bool,
}
impl<'a> Backend<'a> {
    pub fn new(device: &'a dyn DeviceAccess, codec: Codec) -> Self {
        Self {
            device,
            ceiling: (4096, 4096),
            codec,
            built: None,
            h264: Box::new(h264::Stream::new()),
            hevc: Box::new(hevc::Stream::new()),
            staging: Box::new(Staging {
                h264_picture: zeroed(),
                h264_matrix: zeroed(),
                h264_slices: [zeroed(); MAX_SLICES],
                hevc_picture: zeroed(),
                hevc_matrix: zeroed(),
                hevc_slices: [zeroed(); MAX_SLICES],
            }),
            ranges: [(0, 0); MAX_SLICES],
            report: 0,
            sequence: 0,
            stamps: [None; SURFACES],
            displayable: false,
        }
    }
    pub fn displayable(device: &'a dyn DeviceAccess) -> Self {
        let mut decoder = Self::new(device, Codec::H265);
        decoder.displayable = true;
        decoder
    }
    pub fn feed(&mut self, bytes: &[u8], sequence: usize) -> Result<Fed> {
        self.sequence = sequence;
        if let Some(built) = &mut self.built {
            built.leases.retire(|slot| match self.codec {
                Codec::H264 => self.h264.dpb.taken(slot),
                Codec::H265 => self.hevc.dpb.taken(slot),
            });
        }
        match self.codec {
            Codec::H264 => self.decode_h264(bytes),
            Codec::H265 => self.decode_hevc(bytes),
        }
    }
    pub fn layout(&self) -> anyhow::Result<crate::Layout> {
        match self.codec {
            Codec::H264 => {
                let s = self.h264.active_sps().ok_or(Error::NoProfile)?;
                let (width, height) = s.visible();
                let crop = s.crop.unwrap_or([0; 4]);
                Ok(crate::Layout {
                    left: crop[0] * 2,
                    top: crop[2] * 2 * if s.frame_mbs_only { 1 } else { 2 },
                    width,
                    height,
                    full_range: s.vui.video_full_range,
                })
            }
            Codec::H265 => crate::Layout::hevc(self.hevc.active_sps().ok_or(Error::NoProfile)?),
        }
    }
    pub fn drain(&mut self) {
        self.h264.drain();
        self.hevc.drain();
    }
    pub fn take(
        &mut self,
        consume: impl FnOnce(*mut ID3D11Texture2D, u32, usize) -> anyhow::Result<()>,
    ) -> anyhow::Result<bool> {
        self.take_leased(|texture, slice, stamp| consume(texture, slice, stamp).map(|_| None))
    }
    pub fn take_leased(
        &mut self,
        consume: impl FnOnce(
            *mut ID3D11Texture2D,
            u32,
            usize,
        ) -> anyhow::Result<Option<Rc<dyn SurfaceLease>>>,
    ) -> anyhow::Result<bool> {
        let slot = match self.codec {
            Codec::H264 => self.h264.next_output().map(|o| o.slot),
            Codec::H265 => self.hevc.next_output().map(|o| o.slot),
        };
        let Some(slot) = slot else { return Ok(false) };
        let result = (|| {
            let built = self.built.as_mut().ok_or(Error::NoProfile)?;
            let stamp = self
                .stamps
                .get_mut(slot)
                .and_then(Option::take)
                .ok_or(Error::Status(-1))?;
            let (texture, slice) = if self.displayable {
                (&built.surfaces[slot], 0)
            } else {
                (&built.surfaces[0], slot as u32)
            };
            built.leases.0[slot] = consume(texture.as_ptr(), slice, stamp)?;
            Ok::<_, anyhow::Error>(())
        })();
        if self.built.as_ref().is_none_or(|b| !b.leases.pinned(slot)) {
            match self.codec {
                Codec::H264 => self.h264.dpb.taken(slot),
                Codec::H265 => self.hevc.dpb.taken(slot),
            };
        }
        result?;
        Ok(true)
    }
    fn ensure(&mut self, shape: Shape, coded: (u32, u32)) -> Result<bool> {
        if let Some(built) = &self.built {
            return Ok(built.shape == shape && built.coded == coded);
        }
        if coded.0 > self.ceiling.0 || coded.1 > self.ceiling.1 {
            return Err(Error::TooLarge);
        }
        let (decoder, config) = create_decoder(self.device, shape, coded.0, coded.1)?;
        // Bit 14 advertises support/recommendation for independent decode
        // textures. Never force an unsupported allocation or add a copy.
        if self.displayable && config.ConfigDecoderSpecific & (1 << 14) == 0 {
            return Err(Error::NoProfile);
        }
        let desc = D3D11_TEXTURE2D_DESC {
            Width: coded.0.next_multiple_of(shape.alignment()),
            Height: coded.1.next_multiple_of(shape.alignment()),
            MipLevels: 1,
            ArraySize: if self.displayable { 1 } else { SURFACES as u32 },
            Format: DXGI_FORMAT_NV12,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_DECODER as u32,
            CPUAccessFlags: 0,
            MiscFlags: if self.displayable {
                use windows::Win32::Graphics::Direct3D11::{
                    D3D11_RESOURCE_MISC_SHARED, D3D11_RESOURCE_MISC_SHARED_DISPLAYABLE,
                    D3D11_RESOURCE_MISC_SHARED_EXCLUSIVE_WRITER,
                    D3D11_RESOURCE_MISC_SHARED_NTHANDLE,
                };
                (D3D11_RESOURCE_MISC_SHARED.0
                    | D3D11_RESOURCE_MISC_SHARED_NTHANDLE.0
                    | D3D11_RESOURCE_MISC_SHARED_DISPLAYABLE.0
                    | D3D11_RESOURCE_MISC_SHARED_EXCLUSIVE_WRITER.0) as u32
            } else {
                0
            },
        };
        let mut surfaces = Vec::new();
        let mut views = Vec::with_capacity(SURFACES);
        for slice in 0..SURFACES {
            if slice == 0 || self.displayable {
                surfaces.push(self.device.texture(&desc)?);
            }
            let index = if self.displayable { slice } else { 0 };
            views.push(self.view(
                shape,
                &surfaces[index],
                if self.displayable { 0 } else { slice },
            )?);
        }
        self.built = Some(Built {
            shape,
            coded,
            decoder,
            surfaces,
            leases: SurfaceLeases::new(),
            views,
        });
        Ok(true)
    }
    fn view(
        &self,
        shape: Shape,
        surfaces: &Com<ID3D11Texture2D>,
        slice: usize,
    ) -> Result<Com<ID3D11VideoDecoderOutputView>> {
        let desc = D3D11_VIDEO_DECODER_OUTPUT_VIEW_DESC {
            DecodeProfile: *shape.profile(),
            ViewDimension: D3D11_VDOV_DIMENSION_TEXTURE2D,
            __bindgen_anon_1: D3D11_VIDEO_DECODER_OUTPUT_VIEW_DESC__bindgen_ty_1 {
                Texture2D: D3D11_TEX2D_VDOV {
                    ArraySlice: u32::try_from(slice).map_err(|_| Error::TooLarge)?,
                },
            },
        };
        let mut view: *mut ID3D11VideoDecoderOutputView = core::ptr::null_mut();
        // SAFETY: a live video device and texture; the output is live.
        let hr = unsafe {
            vcall!(
                self.device.video(),
                CreateVideoDecoderOutputView,
                surfaces.as_ptr().cast::<ID3D11Resource>(),
                &raw const desc,
                &raw mut view
            )
        }
        .ok_or(Error::NoProfile)?;
        check(hr)?;
        // SAFETY: a view whose reference the call handed over.
        unsafe { Com::from_raw(view) }.ok_or(Error::NoProfile)
    }

    fn next_report(&mut self) -> u32 {
        self.report = self.report.wrapping_add(1).max(1);
        self.report
    }

    fn decode_h264(&mut self, unit: &[u8]) -> Result<Fed> {
        match self.h264.read(unit).map_err(|_| Error::Status(-1))? {
            h264::Read::Nothing => return Ok(self.pending_fed_h264()),
            h264::Read::FormatChanged => return Ok(Fed::FormatChanged),
            h264::Read::Picture => {}
        }
        let job = self.h264.job().ok_or(Error::Status(-1))?;
        // The one profile here is eight-bit 4:2:0; the rest of what the
        // readers admit has none.
        if job.sps.chroma_format_idc != 1 || job.sps.bit_depth_luma_minus8 != 0 {
            self.h264.abandon();
            return Err(Error::NoProfile);
        }
        let coded = (job.sps.coded_width(), job.sps.coded_height());
        if !self.ensure(Shape::base(Codec::H264), coded)? {
            self.h264.abandon();
            return Ok(Fed::FormatChanged);
        }
        let submitted = self
            .stage_h264()
            .and_then(|slices| self.submit(unit, slices));
        if let Err(e) = submitted {
            self.h264.abandon();
            return Err(e);
        }
        self.h264.finish().map_err(|_| Error::Status(-1))?;
        Ok(self.pending_fed_h264())
    }

    fn pending_fed_h264(&self) -> Fed {
        if self.h264.dpb.has_output() {
            Fed::Picture
        } else {
            Fed::NeedMoreData
        }
    }

    /// Fill the picture parameters and the matrix from the H.264 job, and
    /// note where its slices lie; returns the slice count.
    fn stage_h264(&mut self) -> Result<usize> {
        let report = self.next_report();
        let job = self.h264.job().ok_or(Error::Status(-1))?;
        let sps = &job.sps;
        let pps = &job.pps;
        let current = &job.current;
        let first = job
            .slices
            .items
            .first()
            .copied()
            .flatten()
            .ok_or(Error::Status(-1))?;
        let intra_only = job
            .slices
            .as_slice()
            .iter()
            .flatten()
            .all(|s| s.header.slice_type.is_intra());

        // The references, and which fields of each are referenced.
        let mut refs = [entry_h264(None, false); 16];
        let mut orders = [[0i32; 2]; 16];
        let mut frame_nums = [0u16; 16];
        let mut used = 0u32;
        for (i, r) in job.references.as_slice().iter().take(16).enumerate() {
            let Some(r) = r else {
                continue;
            };
            let (top, bottom) = match r.parity {
                None => (true, true),
                Some(Parity::Top) => (true, false),
                Some(Parity::Bottom) => (false, true),
            };
            if let Some(e) = refs.get_mut(i) {
                *e = entry_h264(Some(r.slot), r.long_term);
            }
            if let Some(o) = orders.get_mut(i) {
                *o = [
                    if top { r.top_poc } else { 0 },
                    if bottom { r.bottom_poc } else { 0 },
                ];
            }
            if let Some(n) = frame_nums.get_mut(i) {
                // The frame number of a short-term reference, the long-term
                // index of a long-term one.
                *n = u16::try_from(r.frame_idx).unwrap_or(0);
            }
            used |= u32::from(top) << (2 * i) | u32::from(bottom) << (2 * i + 1);
        }
        let (top_order, bottom_order) = match current.structure {
            Structure::Frame => (current.top_poc, current.bottom_poc),
            Structure::Field(Parity::Top) => (current.top_poc, 0),
            Structure::Field(Parity::Bottom) => (0, current.bottom_poc),
        };

        let mut bits: _DXVA_PicParams_H264__bindgen_ty_1__bindgen_ty_1 = zeroed();
        bits.set_field_pic_flag(u16::from(first.header.field_pic));
        bits.set_MbaffFrameFlag(u16::from(
            sps.mb_adaptive_frame_field && !first.header.field_pic,
        ));
        bits.set_residual_colour_transform_flag(u16::from(sps.separate_colour_plane));
        bits.set_sp_for_switch_flag(0);
        bits.set_chroma_format_idc(u16::from(sps.chroma_format_idc));
        bits.set_RefPicFlag(u16::from(first.header.nal_ref_idc != 0));
        bits.set_constrained_intra_pred_flag(u16::from(pps.constrained_intra_pred));
        bits.set_weighted_pred_flag(u16::from(pps.weighted_pred));
        bits.set_weighted_bipred_idc(u16::from(pps.weighted_bipred_idc));
        bits.set_MbsConsecutiveFlag(1);
        bits.set_frame_mbs_only_flag(u16::from(sps.frame_mbs_only));
        bits.set_transform_8x8_mode_flag(u16::from(pps.transform_8x8_mode));
        bits.set_MinLumaBipredSize8x8Flag(u16::from(sps.level_idc >= 31));
        bits.set_IntraPicFlag(u16::from(intra_only));

        let p = &mut self.staging.h264_picture;
        *p = zeroed();
        p.wFrameWidthInMbsMinus1 = sps.pic_width_in_mbs_minus1;
        p.wFrameHeightInMbsMinus1 = u16::try_from(sps.frame_height_in_mbs().saturating_sub(1))
            .map_err(|_| Error::TooLarge)?;
        p.CurrPic = entry_h264(
            Some(current.slot),
            current.structure == Structure::Field(Parity::Bottom),
        );
        p.num_ref_frames = sps.max_num_ref_frames;
        p.__bindgen_anon_1 = _DXVA_PicParams_H264__bindgen_ty_1 {
            __bindgen_anon_1: bits,
        };
        p.bit_depth_luma_minus8 = sps.bit_depth_luma_minus8;
        p.bit_depth_chroma_minus8 = sps.bit_depth_chroma_minus8;
        // What every decoder since the interface's first revision expects;
        // one older mode needs another value, and no device here has it.
        p.Reserved16Bits = 3;
        p.StatusReportFeedbackNumber = report;
        p.RefFrameList = refs;
        p.CurrFieldOrderCnt = [top_order, bottom_order];
        p.FieldOrderCntList = orders;
        p.pic_init_qs_minus26 = pps.pic_init_qs_minus26;
        p.chroma_qp_index_offset = pps.chroma_qp_index_offset;
        p.second_chroma_qp_index_offset = pps.second_chroma_qp_index_offset;
        p.ContinuationFlag = 1;
        p.pic_init_qp_minus26 = pps.pic_init_qp_minus26;
        p.num_ref_idx_l0_active_minus1 = pps.num_ref_idx_l0_default_active_minus1;
        p.num_ref_idx_l1_active_minus1 = pps.num_ref_idx_l1_default_active_minus1;
        p.FrameNumList = frame_nums;
        p.UsedForReferenceFlags = used;
        p.NonExistingFrameFlags = 0;
        p.frame_num = u16::try_from(current.frame_num).unwrap_or(0);
        p.log2_max_frame_num_minus4 = sps.log2_max_frame_num_minus4;
        p.pic_order_cnt_type = sps.pic_order_cnt_type;
        p.log2_max_pic_order_cnt_lsb_minus4 = sps.log2_max_pic_order_cnt_lsb_minus4;
        p.delta_pic_order_always_zero_flag = u8::from(sps.delta_pic_order_always_zero);
        p.direct_8x8_inference_flag = u8::from(sps.direct_8x8_inference);
        p.entropy_coding_mode_flag = u8::from(pps.entropy_coding_mode);
        p.pic_order_present_flag = u8::from(pps.bottom_field_pic_order_in_frame_present);
        p.num_slice_groups_minus1 = 0;
        p.slice_group_map_type = 0;
        p.deblocking_filter_control_present_flag = u8::from(pps.deblocking_filter_control_present);
        p.redundant_pic_cnt_present_flag = u8::from(pps.redundant_pic_cnt_present);
        p.slice_group_change_rate_minus1 = 0;

        let mut list_4x4 = [[0u8; 16]; 6];
        for (dst, src) in list_4x4.iter_mut().zip(pps.scaling.list_4x4.iter()) {
            coded_order(dst, src, &ZIGZAG_4X4);
        }
        let mut list_8x8 = [[0u8; 64]; 2];
        for (dst, src) in list_8x8.iter_mut().zip(pps.scaling.list_8x8.iter()) {
            coded_order(dst, src, &ZIGZAG_8X8);
        }
        let m = &mut self.staging.h264_matrix;
        m.bScalingLists4x4 = list_4x4;
        m.bScalingLists8x8 = list_8x8;

        let slices = job.slices.len.min(MAX_SLICES);
        for (i, slice) in job.slices.as_slice().iter().flatten().enumerate() {
            if let Some(r) = self.ranges.get_mut(i) {
                *r = (slice.offset, slice.len);
            }
        }
        Ok(slices)
    }

    fn decode_hevc(&mut self, unit: &[u8]) -> Result<Fed> {
        match self.hevc.read(unit).map_err(|_| Error::Status(-1))? {
            hevc::Read::Nothing => return Ok(self.pending_fed_hevc()),
            hevc::Read::FormatChanged => return Ok(Fed::FormatChanged),
            hevc::Read::Picture => {}
        }
        let job = self.hevc.job().ok_or(Error::Status(-1))?;
        let shape = Shape::base(Codec::H265);
        if job.sps.chroma_format_idc != 1
            || job.sps.bit_depth_luma_minus8 != 0
            || job.sps.is_range_extended()
        {
            self.hevc.abandon();
            return Err(Error::NoProfile);
        }
        let coded = (job.sps.width, job.sps.height);
        if !self.ensure(shape, coded)? {
            self.hevc.abandon();
            return Ok(Fed::FormatChanged);
        }
        let submitted = self
            .stage_hevc(shape)
            .and_then(|slices| self.submit(unit, slices));
        if let Err(e) = submitted {
            self.hevc.abandon();
            return Err(e);
        }
        self.hevc.finish().map_err(|_| Error::Status(-1))?;
        Ok(self.pending_fed_hevc())
    }

    fn pending_fed_hevc(&self) -> Fed {
        if self.hevc.dpb.has_output() {
            Fed::Picture
        } else {
            Fed::NeedMoreData
        }
    }

    /// Fill the picture parameters and the matrix from the HEVC job, and
    /// note where its slices lie; returns the slice count.
    fn stage_hevc(&mut self, _shape: Shape) -> Result<usize> {
        let report = self.next_report();
        let job = self.hevc.job().ok_or(Error::Status(-1))?;
        let sps = &job.sps;
        let pps = &job.pps;
        let current = &job.current;
        let first = job
            .slices
            .items
            .first()
            .copied()
            .flatten()
            .ok_or(Error::Status(-1))?;
        let intra_only = job
            .slices
            .as_slice()
            .iter()
            .flatten()
            .all(|s| s.header.slice_type.is_intra());

        // Every reference the buffer holds, then which of them each set
        // names, as indices into that list.
        let mut refs = [entry_hevc(None, false); 15];
        let mut orders = [0i32; 15];
        let mut before = [NO_ENTRY; 8];
        let mut after = [NO_ENTRY; 8];
        let mut long = [NO_ENTRY; 8];
        let (mut nb, mut na, mut nl) = (0usize, 0usize, 0usize);
        for (i, r) in job.references.as_slice().iter().take(15).enumerate() {
            let Some(r) = r else {
                continue;
            };
            if let Some(e) = refs.get_mut(i) {
                *e = entry_hevc(Some(r.slot), r.long_term);
            }
            if let Some(o) = orders.get_mut(i) {
                *o = r.poc;
            }
            let index = u8::try_from(i).unwrap_or(NO_ENTRY);
            let (set, n) = match r.set {
                hevc::dpb::Set::StCurrBefore => (&mut before, &mut nb),
                hevc::dpb::Set::StCurrAfter => (&mut after, &mut na),
                hevc::dpb::Set::LtCurr => (&mut long, &mut nl),
                hevc::dpb::Set::Foll => continue,
            };
            if let Some(s) = set.get_mut(*n) {
                *s = index;
                *n += 1;
            }
        }

        let mut format: _DXVA_PicParams_HEVC__bindgen_ty_1__bindgen_ty_1 = zeroed();
        format.set_chroma_format_idc(u16::from(sps.chroma_format_idc));
        format.set_separate_colour_plane_flag(u16::from(sps.separate_colour_plane));
        format.set_bit_depth_luma_minus8(u16::from(sps.bit_depth_luma_minus8));
        format.set_bit_depth_chroma_minus8(u16::from(sps.bit_depth_chroma_minus8));
        format.set_log2_max_pic_order_cnt_lsb_minus4(u16::from(
            sps.log2_max_pic_order_cnt_lsb_minus4,
        ));
        // Both hints left clear, which every device takes as "not known".
        format.set_NoPicReorderingFlag(0);
        format.set_NoBiPredFlag(0);

        let mut tools: _DXVA_PicParams_HEVC__bindgen_ty_2__bindgen_ty_1 = zeroed();
        tools.set_scaling_list_enabled_flag(u32::from(sps.scaling_list_enabled));
        tools.set_amp_enabled_flag(u32::from(sps.amp_enabled));
        tools
            .set_sample_adaptive_offset_enabled_flag(u32::from(sps.sample_adaptive_offset_enabled));
        tools.set_pcm_enabled_flag(u32::from(sps.pcm_enabled));
        if sps.pcm_enabled {
            tools.set_pcm_sample_bit_depth_luma_minus1(u32::from(
                sps.pcm_sample_bit_depth_luma_minus1,
            ));
            tools.set_pcm_sample_bit_depth_chroma_minus1(u32::from(
                sps.pcm_sample_bit_depth_chroma_minus1,
            ));
            tools.set_log2_min_pcm_luma_coding_block_size_minus3(u32::from(
                sps.log2_min_pcm_luma_coding_block_size_minus3,
            ));
            tools.set_log2_diff_max_min_pcm_luma_coding_block_size(u32::from(
                sps.log2_diff_max_min_pcm_luma_coding_block_size,
            ));
        }
        tools.set_pcm_loop_filter_disabled_flag(u32::from(sps.pcm_loop_filter_disabled));
        tools.set_long_term_ref_pics_present_flag(u32::from(sps.long_term_ref_pics_present));
        tools.set_sps_temporal_mvp_enabled_flag(u32::from(sps.temporal_mvp_enabled));
        tools
            .set_strong_intra_smoothing_enabled_flag(u32::from(sps.strong_intra_smoothing_enabled));
        tools.set_dependent_slice_segments_enabled_flag(u32::from(
            pps.dependent_slice_segments_enabled,
        ));
        tools.set_output_flag_present_flag(u32::from(pps.output_flag_present));
        tools.set_num_extra_slice_header_bits(u32::from(pps.num_extra_slice_header_bits));
        tools.set_sign_data_hiding_enabled_flag(u32::from(pps.sign_data_hiding_enabled));
        tools.set_cabac_init_present_flag(u32::from(pps.cabac_init_present));

        let mut props: _DXVA_PicParams_HEVC__bindgen_ty_3__bindgen_ty_1 = zeroed();
        props.set_constrained_intra_pred_flag(u32::from(pps.constrained_intra_pred));
        props.set_transform_skip_enabled_flag(u32::from(pps.transform_skip_enabled));
        props.set_cu_qp_delta_enabled_flag(u32::from(pps.cu_qp_delta_enabled));
        props.set_pps_slice_chroma_qp_offsets_present_flag(u32::from(
            pps.slice_chroma_qp_offsets_present,
        ));
        props.set_weighted_pred_flag(u32::from(pps.weighted_pred));
        props.set_weighted_bipred_flag(u32::from(pps.weighted_bipred));
        props.set_transquant_bypass_enabled_flag(u32::from(pps.transquant_bypass_enabled));
        props.set_tiles_enabled_flag(u32::from(pps.tiles_enabled));
        props.set_entropy_coding_sync_enabled_flag(u32::from(pps.entropy_coding_sync_enabled));
        props.set_uniform_spacing_flag(u32::from(pps.uniform_spacing));
        props.set_loop_filter_across_tiles_enabled_flag(u32::from(
            pps.tiles_enabled && pps.loop_filter_across_tiles_enabled,
        ));
        props.set_pps_loop_filter_across_slices_enabled_flag(u32::from(
            pps.loop_filter_across_slices_enabled,
        ));
        props.set_deblocking_filter_override_enabled_flag(u32::from(
            pps.deblocking_filter_override_enabled,
        ));
        props.set_pps_deblocking_filter_disabled_flag(u32::from(pps.disable_deblocking_filter));
        props.set_lists_modification_present_flag(u32::from(pps.lists_modification_present));
        props.set_slice_segment_header_extension_present_flag(u32::from(
            pps.slice_segment_header_extension_present,
        ));
        props.set_IrapPicFlag(u32::from(first.header.is_irap()));
        props.set_IdrPicFlag(u32::from(first.header.is_idr()));
        props.set_IntraPicFlag(u32::from(intra_only));

        // Explicit tile sizes only; under uniform spacing the device derives
        // them, as the standard does.
        let mut columns = [0u16; 19];
        let mut rows = [0u16; 21];
        if pps.tiles_enabled && !pps.uniform_spacing {
            for (dst, src) in columns.iter_mut().zip(pps.column_width_minus1.iter()) {
                *dst = *src;
            }
            for (dst, src) in rows.iter_mut().zip(pps.row_height_minus1.iter()) {
                *dst = *src;
            }
        }

        let min_cb = u32::from(sps.log2_min_luma_coding_block_size_minus3) + 3;
        let mut base: DXVA_PicParams_HEVC = zeroed();
        base.PicWidthInMinCbsY = u16::try_from(sps.width >> min_cb).map_err(|_| Error::TooLarge)?;
        base.PicHeightInMinCbsY =
            u16::try_from(sps.height >> min_cb).map_err(|_| Error::TooLarge)?;
        base.__bindgen_anon_1 = _DXVA_PicParams_HEVC__bindgen_ty_1 {
            __bindgen_anon_1: format,
        };
        base.CurrPic = entry_hevc(Some(current.slot), false);
        base.sps_max_dec_pic_buffering_minus1 = sps.max_dec_pic_buffering_minus1;
        base.log2_min_luma_coding_block_size_minus3 = sps.log2_min_luma_coding_block_size_minus3;
        base.log2_diff_max_min_luma_coding_block_size =
            sps.log2_diff_max_min_luma_coding_block_size;
        base.log2_min_transform_block_size_minus2 = sps.log2_min_luma_transform_block_size_minus2;
        base.log2_diff_max_min_transform_block_size =
            sps.log2_diff_max_min_luma_transform_block_size;
        base.max_transform_hierarchy_depth_inter = sps.max_transform_hierarchy_depth_inter;
        base.max_transform_hierarchy_depth_intra = sps.max_transform_hierarchy_depth_intra;
        base.num_short_term_ref_pic_sets = sps.num_short_term_ref_pic_sets;
        base.num_long_term_ref_pics_sps = sps.num_long_term_ref_pics_sps;
        base.num_ref_idx_l0_default_active_minus1 = pps.num_ref_idx_l0_default_active_minus1;
        base.num_ref_idx_l1_default_active_minus1 = pps.num_ref_idx_l1_default_active_minus1;
        base.init_qp_minus26 = pps.init_qp_minus26;
        // The slice's own reference set's size, when it codes one rather
        // than naming one of the sequence's.
        if !first.header.st_rps_from_sps {
            base.ucNumDeltaPocsOfRefRpsIdx = first.header.st_rps.predicted_from_deltas;
            base.wNumBitsForShortTermRPSInSlice =
                u16::try_from(first.header.st_rps_bits).unwrap_or(u16::MAX);
        }
        base.__bindgen_anon_2 = _DXVA_PicParams_HEVC__bindgen_ty_2 {
            __bindgen_anon_1: tools,
        };
        base.__bindgen_anon_3 = _DXVA_PicParams_HEVC__bindgen_ty_3 {
            __bindgen_anon_1: props,
        };
        base.pps_cb_qp_offset = pps.cb_qp_offset;
        base.pps_cr_qp_offset = pps.cr_qp_offset;
        if pps.tiles_enabled {
            base.num_tile_columns_minus1 = pps.num_tile_columns_minus1;
            base.num_tile_rows_minus1 = pps.num_tile_rows_minus1;
        }
        base.column_width_minus1 = columns;
        base.row_height_minus1 = rows;
        base.diff_cu_qp_delta_depth = pps.diff_cu_qp_delta_depth;
        base.pps_beta_offset_div2 = pps.beta_offset_div2;
        base.pps_tc_offset_div2 = pps.tc_offset_div2;
        base.log2_parallel_merge_level_minus2 = pps.log2_parallel_merge_level_minus2;
        base.CurrPicOrderCntVal = current.poc;
        base.RefPicList = refs;
        base.PicOrderCntValList = orders;
        base.RefPicSetStCurrBefore = before;
        base.RefPicSetStCurrAfter = after;
        base.RefPicSetLtCurr = long;
        base.StatusReportFeedbackNumber = report;

        self.staging.hevc_picture = base;

        let s = &pps.scaling;
        let mut lists0 = [[0u8; 16]; 6];
        let mut lists1 = [[0u8; 64]; 6];
        let mut lists2 = [[0u8; 64]; 6];
        let mut lists3 = [[0u8; 64]; 2];
        for (dst, src) in lists0.iter_mut().zip(s.list_4x4.iter()) {
            coded_order(dst, src, &DIAG_4X4);
        }
        for (dst, src) in lists1.iter_mut().zip(s.list_8x8.iter()) {
            coded_order(dst, src, &DIAG_8X8);
        }
        for (dst, src) in lists2.iter_mut().zip(s.list_16x16.iter()) {
            coded_order(dst, src, &DIAG_8X8);
        }
        for (dst, src) in lists3.iter_mut().zip(s.list_32x32.iter()) {
            coded_order(dst, src, &DIAG_8X8);
        }
        let m = &mut self.staging.hevc_matrix;
        m.ucScalingLists0 = lists0;
        m.ucScalingLists1 = lists1;
        m.ucScalingLists2 = lists2;
        m.ucScalingLists3 = lists3;
        m.ucScalingListDCCoefSizeID2 = s.dc_16x16;
        m.ucScalingListDCCoefSizeID3 = s.dc_32x32;

        let slices = job.slices.len.min(MAX_SLICES);
        for (i, slice) in job.slices.as_slice().iter().flatten().enumerate() {
            if let Some(r) = self.ranges.get_mut(i) {
                *r = (slice.offset, slice.len);
            }
        }
        Ok(slices)
    }

    /// Hand the staged picture to the device: begin on the current slot's
    /// surface, fill and submit the buffers, end.
    fn submit(&mut self, unit: &[u8], slices: usize) -> Result<()> {
        let slot = match self.codec {
            Codec::H264 => self.h264.job().map(|j| j.current.slot),
            Codec::H265 => self.hevc.job().map(|j| j.current.slot),
        }
        .ok_or(Error::Status(-1))?;
        let built = self.built.as_ref().ok_or(Error::NoProfile)?;
        if !built.leases.writable(slot) {
            // No sleep and no overwrite while scanout/presentation owns it.
            return Err(Error::SurfaceBusy);
        }
        let decoder = built.decoder.as_ptr();
        let view = built.views.get(slot).ok_or(Error::TooLarge)?.as_ptr();
        let context = self.device.video_context();
        let mut tries = 0;
        loop {
            // SAFETY: a live video context, decoder and view.
            let hr = unsafe {
                vcall!(
                    context,
                    DecoderBeginFrame,
                    decoder,
                    view,
                    0,
                    core::ptr::null()
                )
            }
            .ok_or(Error::NoProfile)?;
            if hr == E_PENDING && tries < PENDING_TRIES {
                tries += 1;
                std::thread::sleep(core::time::Duration::from_millis(2));
                continue;
            }
            check(hr)?;
            break;
        }
        let filled = self.fill(unit, slices, decoder);
        let submitted = filled.and_then(|descs| {
            let (descs, count) = descs;
            // SAFETY: as above; the descriptions are live for the call.
            let hr = unsafe {
                vcall!(
                    context,
                    SubmitDecoderBuffers,
                    decoder,
                    count,
                    descs.as_ptr()
                )
            }
            .ok_or(Error::NoProfile)?;
            check(hr)
        });
        // Ended whatever happened, so the next picture can begin.
        // SAFETY: as above.
        let ended = unsafe { vcall!(context, DecoderEndFrame, decoder) }.ok_or(Error::NoProfile);
        submitted?;
        check(ended?)?;
        *self.stamps.get_mut(slot).ok_or(Error::TooLarge)? = Some(self.sequence);
        Ok(())
    }

    /// Copy the picture's parameters, its matrix, its slices' places and
    /// the bitstream into the device's buffers; returns the descriptions
    /// the submission takes.
    fn fill(
        &mut self,
        unit: &[u8],
        slices: usize,
        decoder: *mut ID3D11VideoDecoder,
    ) -> Result<([D3D11_VIDEO_DECODER_BUFFER_DESC; 4], u32)> {
        // The bitstream first: it says where each slice lies.
        let (length, macroblocks) = self.fill_bitstream(unit, slices, decoder)?;
        let mut descs: [D3D11_VIDEO_DECODER_BUFFER_DESC; 4] = zeroed();
        let mut count = 0usize;
        let mut add = |kind: D3D11_VIDEO_DECODER_BUFFER_TYPE, size: usize, mbs: u32| {
            if let Some(d) = descs.get_mut(count) {
                d.BufferType = kind;
                d.DataSize = u32::try_from(size).unwrap_or(0);
                d.NumMBsInBuffer = mbs;
                count += 1;
            }
        };
        match self.codec {
            Codec::H264 => {
                let s = &self.staging;
                self.put(
                    decoder,
                    D3D11_VIDEO_DECODER_BUFFER_PICTURE_PARAMETERS,
                    bytes_of(&s.h264_picture),
                )?;
                self.put(
                    decoder,
                    D3D11_VIDEO_DECODER_BUFFER_INVERSE_QUANTIZATION_MATRIX,
                    bytes_of(&s.h264_matrix),
                )?;
                let control = s.h264_slices.get(..slices).ok_or(Error::TooLarge)?;
                self.put(
                    decoder,
                    D3D11_VIDEO_DECODER_BUFFER_SLICE_CONTROL,
                    slice_bytes(control),
                )?;
                add(
                    D3D11_VIDEO_DECODER_BUFFER_PICTURE_PARAMETERS,
                    size_of::<DXVA_PicParams_H264>(),
                    0,
                );
                add(
                    D3D11_VIDEO_DECODER_BUFFER_INVERSE_QUANTIZATION_MATRIX,
                    size_of::<DXVA_Qmatrix_H264>(),
                    0,
                );
                add(
                    D3D11_VIDEO_DECODER_BUFFER_SLICE_CONTROL,
                    size_of_val(control),
                    macroblocks,
                );
            }
            Codec::H265 => {
                let s = &self.staging;
                let picture = bytes_of(&s.hevc_picture);
                self.put(
                    decoder,
                    D3D11_VIDEO_DECODER_BUFFER_PICTURE_PARAMETERS,
                    picture,
                )?;
                add(
                    D3D11_VIDEO_DECODER_BUFFER_PICTURE_PARAMETERS,
                    picture.len(),
                    0,
                );
                let scaling = self.hevc.job().is_some_and(|j| j.sps.scaling_list_enabled);
                if scaling {
                    self.put(
                        decoder,
                        D3D11_VIDEO_DECODER_BUFFER_INVERSE_QUANTIZATION_MATRIX,
                        bytes_of(&s.hevc_matrix),
                    )?;
                    add(
                        D3D11_VIDEO_DECODER_BUFFER_INVERSE_QUANTIZATION_MATRIX,
                        size_of::<DXVA_Qmatrix_HEVC>(),
                        0,
                    );
                }
                let control = s.hevc_slices.get(..slices).ok_or(Error::TooLarge)?;
                self.put(
                    decoder,
                    D3D11_VIDEO_DECODER_BUFFER_SLICE_CONTROL,
                    slice_bytes(control),
                )?;
                add(
                    D3D11_VIDEO_DECODER_BUFFER_SLICE_CONTROL,
                    size_of_val(control),
                    0,
                );
            }
        }
        add(D3D11_VIDEO_DECODER_BUFFER_BITSTREAM, length, macroblocks);
        Ok((descs, u32::try_from(count).unwrap_or(0)))
    }

    /// Write each slice, a start code ahead of it, into the device's
    /// bitstream buffer and record its place in the slice control; the
    /// whole zero-padded to the boundary the device reads in. Returns the
    /// length written and the picture's macroblocks, which the first codec's
    /// descriptions carry.
    fn fill_bitstream(
        &mut self,
        unit: &[u8],
        slices: usize,
        decoder: *mut ID3D11VideoDecoder,
    ) -> Result<(usize, u32)> {
        let context = self.device.video_context();
        let mut size = 0u32;
        let mut data: *mut c_void = core::ptr::null_mut();
        // SAFETY: a live video context and decoder; the outputs are live.
        let hr = unsafe {
            vcall!(
                context,
                GetDecoderBuffer,
                decoder,
                D3D11_VIDEO_DECODER_BUFFER_BITSTREAM,
                &raw mut size,
                &raw mut data
            )
        }
        .ok_or(Error::NoProfile)?;
        check(hr)?;
        if data.is_null() {
            return Err(Error::Status(-1));
        }
        // SAFETY: the device lent `size` writable bytes at `data` until the
        // release below.
        let buffer = unsafe {
            core::slice::from_raw_parts_mut(data.cast::<u8>(), usize::try_from(size).unwrap_or(0))
        };
        let written = self.write_bitstream(unit, slices, buffer);
        // SAFETY: as above; released once.
        let released = unsafe {
            vcall!(
                context,
                ReleaseDecoderBuffer,
                decoder,
                D3D11_VIDEO_DECODER_BUFFER_BITSTREAM
            )
        }
        .ok_or(Error::NoProfile);
        let length = written?;
        check(released?)?;
        let macroblocks = match self.codec {
            Codec::H264 => self.h264.job().map_or(0, |j| {
                (u32::from(j.sps.pic_width_in_mbs_minus1) + 1) * j.sps.frame_height_in_mbs()
            }),
            Codec::H265 => 0,
        };
        Ok((length, macroblocks))
    }

    fn write_bitstream(&mut self, unit: &[u8], slices: usize, buffer: &mut [u8]) -> Result<usize> {
        let mut at = 0usize;
        for i in 0..slices {
            let &(offset, len) = self.ranges.get(i).ok_or(Error::TooLarge)?;
            let data = unit.get(offset..offset + len).ok_or(Error::Status(-1))?;
            let end = at + START_CODE.len() + data.len();
            let room = buffer.get_mut(at..end).ok_or(Error::TooLarge)?;
            let (code, body) = room.split_at_mut(START_CODE.len());
            code.copy_from_slice(&START_CODE);
            body.copy_from_slice(data);
            let location = u32::try_from(at).map_err(|_| Error::TooLarge)?;
            let bytes = u32::try_from(end - at).map_err(|_| Error::TooLarge)?;
            match self.codec {
                Codec::H264 => {
                    let s = self.staging.h264_slices.get_mut(i).ok_or(Error::TooLarge)?;
                    *s = DXVA_Slice_H264_Short {
                        BSNALunitDataLocation: location,
                        SliceBytesInBuffer: bytes,
                        wBadSliceChopping: 0,
                    };
                }
                Codec::H265 => {
                    let s = self.staging.hevc_slices.get_mut(i).ok_or(Error::TooLarge)?;
                    *s = DXVA_Slice_HEVC_Short {
                        BSNALunitDataLocation: location,
                        SliceBytesInBuffer: bytes,
                        wBadSliceChopping: 0,
                    };
                }
            }
            at = end;
        }
        // Zeros to the boundary, counted as the last slice's.
        let padded = at.next_multiple_of(BITSTREAM_ALIGN).min(buffer.len());
        buffer.get_mut(at..padded).ok_or(Error::TooLarge)?.fill(0);
        let pad = u32::try_from(padded - at).map_err(|_| Error::TooLarge)?;
        if let Some(last) = slices.checked_sub(1) {
            match self.codec {
                Codec::H264 => {
                    if let Some(s) = self.staging.h264_slices.get_mut(last) {
                        s.SliceBytesInBuffer += pad;
                    }
                }
                Codec::H265 => {
                    if let Some(s) = self.staging.hevc_slices.get_mut(last) {
                        s.SliceBytesInBuffer += pad;
                    }
                }
            }
        }
        Ok(padded)
    }

    /// Copy `bytes` into the device's buffer of `kind`.
    fn put(
        &self,
        decoder: *mut ID3D11VideoDecoder,
        kind: D3D11_VIDEO_DECODER_BUFFER_TYPE,
        bytes: &[u8],
    ) -> Result<()> {
        let context = self.device.video_context();
        let mut size = 0u32;
        let mut data: *mut c_void = core::ptr::null_mut();
        // SAFETY: a live video context and decoder; the outputs are live.
        let hr = unsafe {
            vcall!(
                context,
                GetDecoderBuffer,
                decoder,
                kind,
                &raw mut size,
                &raw mut data
            )
        }
        .ok_or(Error::NoProfile)?;
        check(hr)?;
        let fits = !data.is_null() && usize::try_from(size).is_ok_and(|s| s >= bytes.len());
        if fits {
            // SAFETY: the device lent at least `bytes.len()` writable bytes
            // at `data` until the release below; the two do not overlap.
            unsafe {
                core::ptr::copy_nonoverlapping(bytes.as_ptr(), data.cast::<u8>(), bytes.len());
            }
        }
        // SAFETY: as above; released once.
        let hr = unsafe { vcall!(context, ReleaseDecoderBuffer, decoder, kind) }
            .ok_or(Error::NoProfile)?;
        if !fits {
            return Err(Error::TooLarge);
        }
        check(hr)
    }
}
fn entry_h264(slot: Option<usize>, flag: bool) -> DXVA_PicEntry_H264 {
    match slot.and_then(|s| u8::try_from(s).ok()) {
        Some(index) => {
            let mut bits: _DXVA_PicEntry_H264__bindgen_ty_1__bindgen_ty_1 = zeroed();
            bits.set_Index7Bits(index);
            bits.set_AssociatedFlag(u8::from(flag));
            DXVA_PicEntry_H264 {
                __bindgen_anon_1: _DXVA_PicEntry_H264__bindgen_ty_1 {
                    __bindgen_anon_1: bits,
                },
            }
        }
        None => DXVA_PicEntry_H264 {
            __bindgen_anon_1: _DXVA_PicEntry_H264__bindgen_ty_1 {
                bPicEntry: NO_ENTRY,
            },
        },
    }
}

/// The second codec's entries have the same layout as the first's.
fn entry_hevc(slot: Option<usize>, long_term: bool) -> DXVA_PicEntry_HEVC {
    let entry = entry_h264(slot, long_term);
    // SAFETY: both are one byte of the same bit layout; either view of it
    // is valid.
    unsafe { core::mem::transmute::<DXVA_PicEntry_H264, DXVA_PicEntry_HEVC>(entry) }
}

/// A raster list read out in the coded order `scan` names: the `i`th coded
/// value is the one at the raster place `scan[i]`.
fn coded_order(dst: &mut [u8], raster: &[u8], scan: &[usize]) {
    for (d, &at) in dst.iter_mut().zip(scan.iter()) {
        if let Some(&v) = raster.get(at) {
            *d = v;
        }
    }
}

fn slice_bytes<T>(slices: &[T]) -> &[u8] {
    // SAFETY: plain-data structures read as the bytes they are made of,
    // for as long as they are borrowed.
    unsafe { core::slice::from_raw_parts(slices.as_ptr().cast::<u8>(), size_of_val(slices)) }
}

#[cfg(test)]
mod presentation_ownership_tests {
    use super::*;
    use std::cell::Cell;
    struct Lease(Cell<bool>);
    impl SurfaceLease for Lease {
        fn available(&self) -> bool {
            self.0.get()
        }
    }
    #[test]
    fn held_scanout_surface_is_not_reused_until_presentation_releases_it() {
        let mut leases = SurfaceLeases::new();
        let lease = Rc::new(Lease(Cell::new(false)));
        leases.0[2] = Some(lease.clone());
        let mut released = Vec::new();
        leases.retire(|slot| released.push(slot));
        assert!(leases.pinned(2));
        assert!(!leases.writable(2));
        assert!(leases.writable(3));
        assert!(released.is_empty());
        lease.0.set(true);
        leases.retire(|slot| released.push(slot));
        assert!(!leases.pinned(2));
        assert!(leases.writable(2));
        leases.retire(|slot| released.push(slot));
        assert_eq!(released, vec![2]);
    }
    #[test]
    fn decoder_reset_cannot_bypass_the_presentation_write_guard() {
        let mut leases = SurfaceLeases::new();
        leases.0[0] = Some(Rc::new(Lease(Cell::new(false))));
        // The parser may clear its own pending-output flags at an IDR. This
        // independent ownership guard must still refuse the physical write.
        let mut dpb = hevc::dpb::Dpb::new();
        dpb.clear();
        assert!(!leases.writable(0));
    }
}
