//! Offline H.264 MFT output-availability comparison; never maps decoded pixels.
#[cfg(windows)]
mod probe {
    use anyhow::{ensure, Context, Result};
    use serde_json::{json, Value};
    use std::{
        collections::BTreeMap,
        mem::ManuallyDrop,
        time::{Duration, Instant},
    };
    use windows::{
        core::Interface,
        Win32::{
            Foundation::HMODULE,
            Graphics::{Direct3D::*, Direct3D10::ID3D10Multithread, Direct3D11::*},
            Media::MediaFoundation::*,
            System::{Com::*, Variant::*},
        },
    };

    const FIXTURE: &[u8] =
        include_bytes!("../../../../src-native/fixtures/synthetic-1920x1080.h264");
    const WARMUP: usize = 16;
    const FRAMES: usize = 80;

    struct Runtime;
    impl Runtime {
        unsafe fn start() -> Result<Self> {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
            if let Err(error) = MFStartup(MF_VERSION, MFSTARTUP_FULL) {
                CoUninitialize();
                return Err(error.into());
            }
            Ok(Self)
        }
    }
    impl Drop for Runtime {
        fn drop(&mut self) {
            unsafe {
                let _ = MFShutdown();
                CoUninitialize();
            }
        }
    }

    unsafe fn output_type(decoder: &IMFTransform) -> Result<()> {
        for index in 0..32 {
            let media = decoder.GetOutputAvailableType(0, index)?;
            if media.GetGUID(&MF_MT_SUBTYPE)? == MFVideoFormat_NV12 {
                decoder.SetOutputType(0, &media, 0)?;
                let flags = decoder.GetOutputStreamInfo(0)?.dwFlags;
                ensure!(
                    flags
                        & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0
                            | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0)
                            as u32
                        != 0,
                    "decoder requires a caller-owned output buffer"
                );
                return Ok(());
            }
        }
        anyhow::bail!("no NV12 output type");
    }

    #[derive(Default)]
    struct Measurements {
        pending: BTreeMap<i64, Instant>,
        delay_us: Vec<u64>,
        lag_frames: Vec<usize>,
        decoded: usize,
        decoder_surfaces: usize,
    }
    impl Measurements {
        unsafe fn collect(&mut self, decoder: &IMFTransform, latest: usize) -> Result<()> {
            for _ in 0..128 {
                let mut out = MFT_OUTPUT_DATA_BUFFER {
                    dwStreamID: 0,
                    pSample: ManuallyDrop::new(None),
                    dwStatus: 0,
                    pEvents: ManuallyDrop::new(None),
                };
                let mut status = 0;
                let result = decoder.ProcessOutput(0, std::slice::from_mut(&mut out), &mut status);
                let sample = ManuallyDrop::take(&mut out.pSample);
                drop(ManuallyDrop::take(&mut out.pEvents));
                match result {
                    Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(()),
                    Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                        output_type(decoder)?;
                        continue;
                    }
                    result => result?,
                }
                let sample = sample.context("missing output sample")?;
                let stamp = sample.GetSampleTime()?;
                let submitted = self
                    .pending
                    .remove(&stamp)
                    .context("uncorrelated decoder output timestamp")?;
                let index = usize::try_from(stamp / 166_667)?;
                let dxgi: IMFDXGIBuffer = sample.GetBufferByIndex(0)?.cast()?;
                let mut raw = std::ptr::null_mut();
                dxgi.GetResource(&ID3D11Texture2D::IID, &mut raw)?;
                ensure!(!raw.is_null(), "missing GPU output texture");
                let texture = ID3D11Texture2D::from_raw(raw);
                let mut desc = D3D11_TEXTURE2D_DESC::default();
                texture.GetDesc(&mut desc);
                self.decoder_surfaces +=
                    usize::from(desc.BindFlags & D3D11_BIND_DECODER.0 as u32 != 0);
                self.decoded += 1;
                if index >= WARMUP {
                    self.delay_us.push(submitted.elapsed().as_micros() as u64);
                    self.lag_frames.push(latest.saturating_sub(index));
                }
            }
            anyhow::bail!("unbounded decoder output loop");
        }
    }

    unsafe fn trial(low_latency: bool, round: usize, frames: &[&[u8]]) -> Result<Value> {
        let mut device = None;
        let mut context = None;
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_VIDEO_SUPPORT | D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&[D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )?;
        let device = device.context("missing D3D11 device")?;
        let dxgi: windows::Win32::Graphics::Dxgi::IDXGIDevice = device.cast()?;
        let desc = dxgi.GetAdapter()?.GetDesc()?;
        let name_length = desc
            .Description
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(desc.Description.len());
        let adapter = String::from_utf16_lossy(&desc.Description[..name_length]);
        let _video: ID3D11VideoDevice =
            device.cast().context("D3D11 video interface unavailable")?;
        let context = context.context("missing D3D11 context")?;
        let multithread: ID3D10Multithread = context.cast()?;
        let _ = multithread.SetMultithreadProtected(true);
        let mut token = 0;
        let mut manager = None;
        MFCreateDXGIDeviceManager(&mut token, &mut manager)?;
        let manager = manager.context("missing device manager")?;
        manager.ResetDevice(&device, token)?;
        let decoder: IMFTransform =
            CoCreateInstance(&CMSH264DecoderMFT, None, CLSCTX_INPROC_SERVER)?;
        ensure!(
            decoder.GetAttributes()?.GetUINT32(&MF_SA_D3D11_AWARE)? != 0,
            "decoder is not D3D11 aware"
        );
        let codec: ICodecAPI = decoder.cast()?;
        let value = VARIANT {
            Anonymous: VARIANT_0 {
                Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                    vt: VT_UI4,
                    Anonymous: VARIANT_0_0_0 {
                        ulVal: u32::from(low_latency),
                    },
                    ..Default::default()
                }),
            },
        };
        let accepted = codec.SetValue(&CODECAPI_AVLowLatencyMode, &value).is_ok();
        decoder.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, manager.as_raw() as usize)?;
        let input = MFCreateMediaType()?;
        input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        input.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
        decoder.SetInputType(0, &input, 0)?;
        output_type(&decoder)?;
        decoder.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
        decoder.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
        let mut samples = Measurements::default();
        let origin = Instant::now();
        for index in 0..FRAMES {
            let deadline = origin + Duration::from_nanos(index as u64 * 16_666_667);
            std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
            let bytes = frames[index % frames.len()];
            let buffer = MFCreateMemoryBuffer(bytes.len() as u32)?;
            let mut ptr = std::ptr::null_mut();
            buffer.Lock(&mut ptr, None, None)?;
            if ptr.is_null() {
                let _ = buffer.Unlock();
                anyhow::bail!("missing compressed input memory");
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
            buffer.Unlock()?;
            buffer.SetCurrentLength(bytes.len() as u32)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buffer)?;
            let stamp = index as i64 * 166_667;
            sample.SetSampleTime(stamp)?;
            if index.is_multiple_of(frames.len()) {
                sample.SetUINT32(&MFSampleExtension_CleanPoint, 1)?;
            }
            samples.pending.insert(stamp, Instant::now());
            let result = decoder.ProcessInput(0, &sample, 0);
            if result
                .as_ref()
                .err()
                .is_some_and(|e| e.code() == MF_E_NOTACCEPTING)
            {
                samples.collect(&decoder, index)?;
                decoder.ProcessInput(0, &sample, 0)?;
            } else {
                result?;
            }
            samples.collect(&decoder, index)?;
        }
        let held_before_drain = samples.pending.len();
        decoder.ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, 0)?;
        samples.collect(&decoder, FRAMES - 1)?;
        ensure!(
            samples.decoded == FRAMES && samples.pending.is_empty(),
            "incomplete synthetic decode"
        );
        samples.delay_us.sort_unstable();
        samples.lag_frames.sort_unstable();
        let n = samples.delay_us.len();
        Ok(
            json!({"round":round,"adapter":adapter,"low_latency_requested":low_latency,"request_accepted":accepted,
            "frames_decoded":samples.decoded,"decoder_surface_outputs":samples.decoder_surfaces,
            "held_before_drain":held_before_drain,"measured_frames":n,
            "output_available_p50_us":samples.delay_us[n/2],"output_available_p95_us":samples.delay_us[(n-1)*95/100],
            "output_available_max_us":samples.delay_us[n-1],"max_later_inputs_before_output":samples.lag_frames[n-1]}),
        )
    }

    pub fn run() -> Result<()> {
        let _runtime = unsafe { Runtime::start()? };
        let mut starts: Vec<usize> = (0..FIXTURE.len().saturating_sub(4))
            .filter(|&i| FIXTURE[i..].starts_with(&[0, 0, 0, 1]) && FIXTURE[i + 4] & 31 == 9)
            .collect();
        ensure!(starts.len() == 8, "synthetic access unit count changed");
        starts.push(FIXTURE.len());
        let frames: Vec<&[u8]> = starts
            .windows(2)
            .map(|pair| &FIXTURE[pair[0]..pair[1]])
            .collect();
        let mut trials = Vec::new();
        for round in 0..5 {
            for enabled in if round % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            } {
                trials.push(unsafe { trial(enabled, round, &frames)? });
            }
        }
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({"scope":"offline-h264-mft-d3d11-output-availability",
            "fixture":"synthetic-1920x1080.h264, no B frames", "includes_network_queue_present_or_gpu_completion":false,
            "decoded_pixel_readback":false,"live_client_changed":false,"trials":trials})
            )?
        );
        Ok(())
    }
}

fn main() -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        probe::run()
    }
    #[cfg(not(windows))]
    {
        anyhow::bail!("Windows D3D11/Media Foundation hardware is required")
    }
}
