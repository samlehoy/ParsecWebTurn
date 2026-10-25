//! Intel VPL HEVC backend. Original NV12 on the renderer device.
//! No plane splitting, export, decoded-frame copy, readback or CPU fallback.
use super::*;
use lowlat_drivers::{
    d3d11::Fence,
    ffi::vpl::*,
    vpl::{Decoded, Picture as Held, Runtime, Session, Vpl},
};
use windows::Win32::Graphics::{Direct3D11::*, Dxgi::Common::DXGI_FORMAT_NV12};

// Bound only renderer reads, never decode input or codec reference pictures.
const MAX_RENDERER_READS: usize = 2;
fn renderer_has_room<T>(pending: &mut Vec<(T, u64)>, completed: u64) -> bool {
    pending.retain(|(_, value)| *value > completed);
    pending.len() < MAX_RENDERER_READS
}

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
            renderer_has_room(&mut state.pending, state.fence.completed());
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
                        outputs += 1;
                        if !renderer_has_room(&mut state.pending, state.fence.completed()) {
                            // No renderer read is submitted for this output.
                            // Release only our external VPL reference; the codec
                            // retains its own decode/reference-picture ownership.
                            // Keep decoding even during a startup/network burst.
                            drop(held);
                            if bits.DataLength == 0 {
                                break;
                            }
                            continue;
                        }
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
                        if bits.DataLength == 0 {
                            break;
                        }
                    }
                    Decoded::MoreData => break,
                    Decoded::Warning => {
                        // A positive status without a surface is not failure.
                        // Separate parameter packets
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, rc::Rc};

    struct Lease(Rc<Cell<usize>>);
    impl Drop for Lease {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    #[test]
    fn burst_releases_unpresented_outputs_but_keeps_inflight_reads_alive() {
        let released = Rc::new(Cell::new(0));
        let mut pending = Vec::new();
        for sequence in 1..=120 {
            let output = Lease(released.clone());
            if renderer_has_room(&mut pending, 0) {
                pending.push((output, sequence));
            }
        }
        // A stalled renderer cannot make output leases grow with the burst.
        // Only unpresented outputs have been released, with no fence wait.
        assert_eq!(pending.len(), 2);
        assert_eq!(released.get(), 118);
        assert!(!renderer_has_room(&mut pending, 0));
        assert!(renderer_has_room(&mut pending, 1));
        assert_eq!(released.get(), 119);
        assert_eq!(pending[0].1, 2);
        // Rendering resumes as soon as a slot retires, even if another read
        // remains in flight. That read must still retain its original lease.
        pending.push((Lease(released.clone()), 3));
        assert!(!renderer_has_room(&mut pending, 1));
        assert!(renderer_has_room(&mut pending, 2));
        assert_eq!(released.get(), 120);
        assert_eq!(pending[0].1, 3);
        assert!(renderer_has_room(&mut pending, 3));
        assert!(pending.is_empty());
        assert_eq!(released.get(), 121);
    }
}
