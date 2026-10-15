//! Test-only current VPL HEVC backend. Original NV12 on the renderer device.
//! No plane splitting, export, decoded-frame copy, readback or CPU fallback.
use super::*;
use lowlat_drivers::{
    d3d11::Fence,
    ffi::vpl::*,
    vpl::{Decoded, Picture as Held, Runtime, Session, Vpl},
};
use windows::Win32::Graphics::{Direct3D11::*, Dxgi::Common::DXGI_FORMAT_NV12};

struct Owner {
    device: DeviceOwner,
    runtime: Vpl,
    index: u32,
}
type Context<'a> = Session<'a>;
self_cell!(
    struct ContextCell {
        owner: Owner,
        #[covariant]
        dependent: Context,
    }
);
struct State<'a> {
    session: &'a Session<'a>,
    pending: Vec<(Held<'a>, u64)>,
    fence: Fence,
    signaled: u64,
    layout: Option<Layout>,
    size: (u32, u32),
    sets: Vec<u8>,
    initialized: bool,
}
impl Drop for State<'_> {
    fn drop(&mut self) {
        // Teardown only: retain decoder leases while the renderer may read
        // them. Live decoding never waits on this completion fence.
        let deadline = Instant::now() + Duration::from_secs(1);
        while self.fence.completed() < self.signaled && Instant::now() < deadline {
            std::thread::yield_now();
        }
        self.pending.clear();
        self.session.close_decoder();
    }
}
self_cell!(
    struct Cell {
        owner: ContextCell,
        #[covariant]
        dependent: State,
    }
);
pub(super) struct Backend(Cell);
impl Backend {
    pub(super) fn open(device: DeviceOwner, size: (u32, u32)) -> Result<Self> {
        ensure!(
            device.device.has_fences(),
            "Intel VPL requires GPU completion fences"
        );
        let runtime = Vpl::for_adapter(&device.device.adapter)?;
        ensure!(
            runtime.runtime() == Runtime::Current,
            "current GPU VPL runtime required"
        );
        let index = device
            ._runtime
            .plain_index(device.device.adapter.luid)?
            .context("Intel DXGI index")?;
        let context = ContextCell::try_new(
            Owner {
                device,
                runtime,
                index,
            },
            |o| o.runtime.session(&o.device.device, o.index),
        )?;
        let cell = Cell::try_new(context, |c| -> Result<State<'_>> {
            Ok(State {
                session: c.borrow_dependent(),
                pending: Vec::new(),
                fence: c.borrow_owner().device.device.fence(0)?,
                signaled: 0,
                layout: None,
                size,
                sets: Vec::new(),
                initialized: false,
            })
        })?;
        Ok(Self(cell))
    }
    pub(super) fn device(&self) -> &Device {
        &self.0.borrow_owner().borrow_owner().device.device
    }
    pub(super) fn feed(
        &mut self,
        bytes: &[u8],
        sequence: usize,
        mut consume: impl FnMut(Picture<'_>) -> Result<()>,
    ) -> Result<usize> {
        self.0.with_dependent_mut(|context, state| {
            let device = &context.borrow_owner().device.device;
            let completed = state.fence.completed();
            state.pending.retain(|(_, value)| *value > completed);
            ensure!(
                state.pending.len() < 4,
                "Intel GPU renderer fell behind; refusing to grow the queue"
            );
            let mut picture = false;
            for unit in nal::Units::new(bytes) {
                let Some(&first) = unit.bytes.first() else {
                    continue;
                };
                let kind = (first >> 1) & 63;
                picture |= kind <= 31;
                if kind == 33 {
                    let sps = hevc::sps::parse(unit.bytes.get(2..).context("truncated HEVC SPS")?)
                        .map_err(|e| anyhow::anyhow!("HEVC SPS: {e:?}"))?;
                    ensure!(
                        sps.max_num_reorder_pics == 0,
                        "Intel low latency requires a non-reordered stream"
                    );
                    ensure!(
                        (sps.width, sps.height) == state.size,
                        "Intel HEVC geometry changed"
                    );
                    state.layout = Some(Layout::hevc(&sps)?);
                }
            }
            if !state.initialized && !picture {
                ensure!(
                    state.sets.len() + bytes.len() <= 1024 * 1024,
                    "Intel parameter sets too large"
                );
                state.sets.extend_from_slice(bytes);
                return Ok(0);
            }
            let combined;
            let input = if !state.initialized && !state.sets.is_empty() {
                combined = [state.sets.as_slice(), bytes].concat();
                combined.as_slice()
            } else {
                bytes
            };
            if !state.initialized {
                let mut param = state
                    .session
                    .header(input, MFX_CODEC_HEVC as u32)?
                    .context("Intel HEVC header unavailable")?;
                state.layout.context("Intel requires actual SPS metadata")?;
                param.IOPattern = MFX_IOPATTERN_OUT_VIDEO_MEMORY as u16;
                param.AsyncDepth = 1;
                // The stream is checked not to reorder; decoded order therefore
                // matches presentation order without an added frame buffer.
                unsafe {
                    param
                        .__bindgen_anon_1
                        .mfx
                        .__bindgen_anon_1
                        .__bindgen_anon_2
                        .DecodedOrder = 1;
                    state.session.query(&mut param)?;
                    ensure!(param.AsyncDepth == 1, "Intel refused AsyncDepth=1");
                    state.session.init(&mut param)?;
                }
                state.initialized = true;
            }
            let mut bits: mfxBitstream = unsafe { std::mem::zeroed() };
            bits.Data = input.as_ptr().cast_mut();
            bits.DataLength = input.len() as u32;
            bits.MaxLength = bits.DataLength;
            bits.DataFlag = MFX_BITSTREAM_COMPLETE_FRAME as u16;
            bits.TimeStamp = sequence as u64 + 1;
            let deadline = Instant::now() + Duration::from_millis(20);
            let mut outputs = 0;
            let session = state.session;
            loop {
                ensure!(Instant::now() < deadline,
                    "Intel VPL input availability timeout (remaining={}, outputs={}, picture={picture})",
                    bits.DataLength, outputs);
                match unsafe { session.decode(&mut bits, std::ptr::null_mut()) }? {
                    Decoded::Picture(held) => {
                        let raw = held.texture();
                        let texture = unsafe { ID3D11Texture2D::from_raw_borrowed(&raw) }
                            .context("Intel native texture missing")?;
                        let mut desc = D3D11_TEXTURE2D_DESC::default();
                        unsafe {
                            texture.GetDesc(&mut desc);
                        }
                        ensure!(
                            desc.Format == DXGI_FORMAT_NV12
                                && desc.CPUAccessFlags == 0
                                && desc.ArraySize == 1,
                            "Intel output is not a single GPU NV12 texture"
                        );
                        ensure!(
                            unsafe { texture.GetDevice()? }.as_raw() == device.device().cast(),
                            "Intel decoded on another device"
                        );
                        ensure!(
                            held.timestamp() > 0 && held.timestamp() <= sequence as u64 + 1,
                            "Intel output timestamp mismatch"
                        );
                        // Current VPL queues decode on this caller's protected
                        // D3D11 immediate context. Queue the existing processor
                        // read on that same context, then fence its completion.
                        // Retain the VPL lease until that read has completed.
                        let result = consume(Picture {
                            device,
                            sequence: (held.timestamp() - 1) as usize,
                            texture,
                            slice: 0,
                            layout: state.layout.context("Intel layout")?,
                        });
                        state.signaled += 1;
                        device.signal(&state.fence, state.signaled)?;
                        state.pending.push((held, state.signaled));
                        result?;
                        outputs += 1;
                        if bits.DataLength == 0 {
                            break;
                        }
                    }
                    Decoded::MoreData => break,
                    Decoded::Warning => {
                        // A positive status without a surface is not failure.
                        // Separate parameter packets (common on the WASM path)
                        // can be fully consumed this way. Do not retry an empty
                        // input until our own watchdog falsely reports a stall.
                        if bits.DataLength == 0 { break; }
                        std::thread::yield_now();
                    }
                    Decoded::Busy => std::thread::yield_now(),
                    _ => {
                        anyhow::bail!("Intel VPL requires unsupported surfaces or a format change")
                    }
                }
            }
            state.sets.clear();
            Ok(outputs)
        })
    }
}
