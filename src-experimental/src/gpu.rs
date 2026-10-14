//! Offline vendor comparison. No decoder exporter, pixel map or live client.
use anyhow::{Context, Result, ensure};
use lowlat_drivers::d3d11::{D3d11, Device};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Graphics::{Direct3D11::*, Dxgi::Common::*},
        Media::MediaFoundation::*,
        System::Com::*,
    },
    core::Interface,
};
mod amf;
mod mf;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Clip {
    H264,
    Hevc,
    Main10,
}
impl Clip {
    fn name(self) -> &'static str {
        match self {
            Self::H264 => "H.264",
            Self::Hevc => "HEVC Main",
            Self::Main10 => "HEVC Main 10",
        }
    }
    fn format(self) -> DXGI_FORMAT {
        if self == Self::Main10 {
            DXGI_FORMAT_P010
        } else {
            DXGI_FORMAT_NV12
        }
    }
    fn units(self) -> Result<Vec<&'static [u8]>> {
        let mut bytes: &[u8] = match self {
            Self::H264 => include_bytes!("../fixtures/synthetic-720p-h264.bin"),
            Self::Hevc => include_bytes!("../fixtures/synthetic-720p-hevc.bin"),
            Self::Main10 => include_bytes!("../fixtures/synthetic-720p-hevc10.bin"),
        };
        let mut units = Vec::new();
        while !bytes.is_empty() {
            ensure!(bytes.len() >= 4, "truncated fixture length");
            let len = u32::from_le_bytes(bytes[..4].try_into()?) as usize;
            bytes = &bytes[4..];
            ensure!(len > 0 && len <= bytes.len(), "invalid fixture access unit");
            units.push(&bytes[..len]);
            bytes = &bytes[len..];
        }
        ensure!(units.len() == 120, "fixture frame count changed");
        Ok(units)
    }
}

struct Runtime;
impl Runtime {
    fn start() -> Result<Self> {
        // SAFETY: this standalone process initializes and owns COM/MF here.
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
            if let Err(error) = MFStartup(MF_VERSION, MFSTARTUP_FULL) {
                CoUninitialize();
                return Err(error.into());
            }
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

struct Samples {
    pending: BTreeMap<i64, (usize, Instant)>,
    available: Vec<u64>,
    completed: Vec<u64>,
    outputs: usize,
    views: usize,
    max_lag: usize,
}
impl Samples {
    fn new() -> Self {
        Self {
            pending: BTreeMap::new(),
            available: Vec::new(),
            completed: Vec::new(),
            outputs: 0,
            views: 0,
            max_lag: 0,
        }
    }
    fn submit(&mut self, index: usize) {
        self.pending
            .insert(index as i64 + 1, (index, Instant::now()));
    }
    fn output(
        &mut self,
        stamp: i64,
        latest: usize,
        texture: &ID3D11Texture2D,
        slice: u32,
        device: &Device,
        clip: Clip,
    ) -> Result<()> {
        let ready = Instant::now();
        let (index, start) = self.pending.remove(&stamp).context("uncorrelated output")?;
        // SAFETY: the decoder's owner retains this borrowed texture throughout.
        unsafe {
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            texture.GetDesc(&mut desc);
            ensure!(
                desc.Format == clip.format()
                    && desc.CPUAccessFlags == 0
                    && desc.Usage == D3D11_USAGE_DEFAULT,
                "output is not the requested GPU-only NV12/P010 layout"
            );
            ensure!(
                desc.MipLevels == 1 && slice < desc.ArraySize,
                "invalid decoded texture slice"
            );
            let owner = texture.GetDevice()?;
            ensure!(
                owner.as_raw() == device.device().cast(),
                "cross-device output refused"
            );
            // Check that the existing video processor can consume this exact
            // surface. No split, intermediate texture or copy is introduced.
            let raw = device.video().cast();
            let video = ID3D11VideoDevice::from_raw_borrowed(&raw).context("video device")?;
            let enumeration =
                video.CreateVideoProcessorEnumerator(&D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
                    InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                    InputFrameRate: DXGI_RATIONAL {
                        Numerator: 60,
                        Denominator: 1,
                    },
                    InputWidth: desc.Width,
                    InputHeight: desc.Height,
                    OutputFrameRate: DXGI_RATIONAL {
                        Numerator: 60,
                        Denominator: 1,
                    },
                    OutputWidth: 1280,
                    OutputHeight: 720,
                    Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
                })?;
            let view_desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
                ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_VPIV {
                        MipSlice: 0,
                        ArraySlice: slice,
                    },
                },
                ..Default::default()
            };
            let mut view = None;
            video.CreateVideoProcessorInputView(
                texture,
                &enumeration,
                &view_desc,
                Some(&mut view),
            )?;
            ensure!(view.is_some(), "missing processor input view");
        }
        self.views += 1;
        self.outputs += 1;
        self.max_lag = self.max_lag.max(latest.saturating_sub(index));
        if index >= 16 {
            self.available
                .push(ready.duration_since(start).as_micros() as u64);
        }
        if device.has_fences() {
            // Offline only: hold the decoder surface until this device's work
            // completes. Never put this per-picture CPU wait in live playback.
            let fence = device.fence(0)?;
            device.signal(&fence, 1)?;
            while fence.completed() < 1 {
                ensure!(
                    start.elapsed() < Duration::from_secs(2),
                    "GPU completion timed out"
                );
                std::thread::yield_now();
            }
            if index >= 16 {
                self.completed.push(start.elapsed().as_micros() as u64);
            }
        }
        Ok(())
    }
    fn finish(mut self, count: usize) -> Result<Value> {
        ensure!(
            self.outputs == count && self.pending.is_empty(),
            "incomplete decode: {}/{count}",
            self.outputs
        );
        fn percentiles(v: &mut [u64]) -> Value {
            v.sort_unstable();
            if v.is_empty() {
                return Value::Null;
            }
            json!({"p50":v[v.len()/2],"p95":v[(v.len()*95).div_ceil(100)-1],"max":v[v.len()-1]})
        }
        Ok(
            json!({"frames":self.outputs,"direct_processor_input_views":self.views,
            "submission_to_output_us":percentiles(&mut self.available),"submission_to_completion_after_view_validation_us":percentiles(&mut self.completed),
            "max_later_inputs_before_output":self.max_lag,"warmup_frames":16,
            "application_decoded_frame_copies":0,"cpu_pixel_readback":false,"same_d3d11_device":true,
            "runtime_internal_copies":"unknown","presentation_verified":false}),
        )
    }
}

fn pace(origin: Instant, index: usize, fps: u64) {
    let deadline = origin + Duration::from_nanos(index as u64 * 1_000_000_000 / fps);
    std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
}

fn nvidia_capabilities() -> Result<Value> {
    use lowlat_drivers::{cuda::Cuda, cuvid::Cuvid};
    let cuda = Cuda::load()?;
    let cuvid = Cuvid::load()?;
    let mut devices = Vec::new();
    for ordinal in 0..cuda.device_count()? {
        let device = cuda.device(ordinal)?;
        let context = cuda.retain_primary(&device)?;
        context.make_current()?;
        let caps = lowlat_decode::nvdec::caps(&cuvid);
        let mut name = [0; 96];
        let len = cuda.device_name(&device, &mut name)?;
        devices.push(json!({"device":String::from_utf8_lossy(&name[..len]),"pci":device.address().as_str(),
            "h264_decoder_created":caps.h264,"hevc_decoder_created":caps.hevc,"hevc_main10_decoder_created":caps.hevc_10,
            "async_surface_api":cuvid.asynchronous(),"decode_measured":false,
            "integration_refused":"Pinned NVDEC D3D11 export copies decoded planes; no equivalent direct texture handoff verified"}));
        context.release_current()?;
    }
    Ok(json!(devices))
}

pub fn probe() -> Result<Value> {
    let _runtime = Runtime::start()?;
    let d3d = D3d11::load()?;
    let mut adapters = Vec::new();
    for adapter in d3d
        .adapters()?
        .into_iter()
        .filter(|a| a.decodes_here() && matches!(a.vendor, 0x1002 | 0x10de))
    {
        eprintln!("Testing {}", adapter.description);
        let device = d3d.open(adapter.luid)?;
        ensure!(device.protect(), "D3D11 multithread protection required");
        let mut trials = Vec::new();
        for clip in [Clip::H264, Clip::Hevc, Clip::Main10] {
            let units = clip.units()?;
            for fps in [30, 60] {
                for round in 0..3 {
                    // Alternate order so startup/thermal drift affects both.
                    for vendor in if round % 2 == 0 {
                        [false, true]
                    } else {
                        [true, false]
                    } {
                        if vendor && adapter.vendor != 0x1002 {
                            continue;
                        }
                        let backend = if vendor {
                            "AMF"
                        } else {
                            "Media Foundation (current low latency requests)"
                        };
                        eprintln!("  {} {backend} {fps} fps round {round}", clip.name());
                        let result = if vendor {
                            amf::trial(&device, clip, &units, fps)
                        } else {
                            mf::trial(&device, clip, &units, fps)
                        };
                        trials.push(json!({"codec":clip.name(),"backend":backend,"fps":fps,"round":round,
                            "result":match result { Ok(v)=>v, Err(e)=>json!({"refused":format!("{e:#}")}) }}));
                    }
                }
            }
        }
        adapters.push(json!({"adapter":adapter.description,"luid":adapter.luid.to_string(),"driver":adapter.driver,"trials":trials}));
    }
    Ok(
        json!({"scope":"isolated GPU-only decode, same synthetic 720p clips; no live changes",
        "adapters":adapters,"nvdec":match nvidia_capabilities() {Ok(v)=>v,Err(e)=>json!({"refused":format!("{e:#}")})},
        "p50_definition":"upper middle","p95_definition":"nearest rank","includes_network_or_present":false,
        "warning":"Output availability is not GPU completion. Completion includes offline input-view validation. No live performance or zero-copy claim."}),
    )
}
