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
mod direct;
mod mf;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Clip {
    H264,
    Hevc,
}
impl Clip {
    fn name(self) -> &'static str {
        match self {
            Self::H264 => "H.264",
            Self::Hevc => "HEVC Main",
        }
    }
    fn format(self) -> DXGI_FORMAT {
        DXGI_FORMAT_NV12
    }
    fn units(self) -> Result<Vec<&'static [u8]>> {
        let mut bytes: &[u8] = match self {
            Self::H264 => include_bytes!("../fixtures/synthetic-720p-h264.bin"),
            Self::Hevc => include_bytes!("../fixtures/synthetic-720p-hevc.bin"),
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
                "output is not the requested GPU-only NV12 layout"
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
            // Exercise the same single conversion/output write as playback,
            // directly from the decoded texture. No pixel readback, split,
            // intermediate decoded texture or CopyResource/CopySubresource.
            let native_raw = device.device().cast();
            let native = ID3D11Device::from_raw_borrowed(&native_raw).context("D3D11 device")?;
            let mut target = None;
            native.CreateTexture2D(
                &D3D11_TEXTURE2D_DESC {
                    Width: 1280,
                    Height: 720,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
                    ..Default::default()
                },
                None,
                Some(&mut target),
            )?;
            let target = target.context("processor output texture")?;
            let mut output = None;
            video.CreateVideoProcessorOutputView(
                &target,
                &enumeration,
                &D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                    ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                    Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                        Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
                    },
                },
                Some(&mut output),
            )?;
            let processor = video.CreateVideoProcessor(&enumeration, 0)?;
            let raw_context = device.video_context().cast();
            let context =
                ID3D11VideoContext::from_raw_borrowed(&raw_context).context("video context")?;
            context.VideoProcessorSetStreamAutoProcessingMode(&processor, 0, false);
            // All fixtures are 1280x720. Decoder arrays can have padded coded
            // height (e.g. HEVC aligned to 128); consume the same visible area
            // for all backends instead of scaling their padding into the image.
            let visible = windows::Win32::Foundation::RECT {
                left: 0,
                top: 0,
                right: 1280,
                bottom: 720,
            };
            context.VideoProcessorSetOutputTargetRect(&processor, true, Some(&visible));
            context.VideoProcessorSetStreamSourceRect(&processor, 0, true, Some(&visible));
            context.VideoProcessorSetStreamDestRect(&processor, 0, true, Some(&visible));
            let mut stream = D3D11_VIDEO_PROCESSOR_STREAM {
                Enable: true.into(),
                pInputSurface: std::mem::ManuallyDrop::new(view),
                ..Default::default()
            };
            let conversion = context.VideoProcessorBlt(
                &processor,
                output.as_ref().context("processor output view")?,
                0,
                std::slice::from_ref(&stream),
            );
            drop(std::mem::ManuallyDrop::take(&mut stream.pInputSurface));
            conversion?;
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
            "direct_conversion_verified":true,"conversion_output_writes_per_frame":1,
            "runtime_internal_copies":"unknown","presentation_verified":false}),
        )
    }
}

fn pace(origin: Instant, index: usize, fps: u64) {
    let deadline = origin + Duration::from_nanos(index as u64 * 1_000_000_000 / fps);
    std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
}

pub fn probe() -> Result<Value> {
    let integrated_only = std::env::args().any(|a| a == "--integrated-only");
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
        for clip in if integrated_only {
            Vec::new()
        } else {
            vec![Clip::H264, Clip::Hevc]
        } {
            let units = clip.units()?;
            for fps in [30, 60] {
                for round in 0..3 {
                    // Alternate order so startup/thermal drift affects both.
                    for selected in if round % 2 == 0 { [0, 1, 2] } else { [2, 1, 0] } {
                        if selected == 1 && adapter.vendor != 0x1002 {
                            continue;
                        }
                        let backend = match selected {
                            1 => "AMF",
                            2 => "Direct D3D11",
                            _ => "Media Foundation (current low latency requests)",
                        };
                        eprintln!("  {} {backend} {fps} fps round {round}", clip.name());
                        let result = match selected {
                            1 => amf::trial(&device, clip, &units, fps),
                            2 => direct::trial(&device, clip, &units, fps),
                            _ => mf::trial(&device, clip, &units, fps),
                        };
                        trials.push(json!({"codec":clip.name(),"backend":backend,"fps":fps,"round":round,
                            "result":match result { Ok(v)=>v, Err(e)=>json!({"refused":format!("{e:#}")}) }}));
                    }
                }
            }
        }
        if let Some(mode) = parsec_gpu_video::preferred(adapter.vendor, true) {
            let units = Clip::Hevc.units()?;
            let result = (|| -> Result<Value> {
                let mut decoder =
                    parsec_gpu_video::Decoder::open(mode, adapter.luid.value(), (1280, 720))?;
                let name = decoder.name();
                let mut samples = Samples::new();
                let origin = Instant::now();
                for (index, unit) in units.iter().enumerate() {
                    pace(origin, index, 60);
                    samples.submit(index);
                    decoder.feed(unit, |picture| {
                        samples.output(
                            picture.sequence as i64 + 1,
                            index,
                            picture.texture,
                            picture.slice,
                            picture.device,
                            Clip::Hevc,
                        )
                    })?;
                }
                let mut report = samples.finish(units.len())?;
                report["actual_backend"] = json!(name);
                Ok(report)
            })();
            trials.push(json!({"codec":"HEVC Main","backend":"Integrated GPU decoder","fps":60,"round":0,"result":match result {Ok(v)=>v,Err(e)=>json!({"refused":format!("{e:#}")})}}));
        }
        adapters.push(json!({"adapter":adapter.description,"luid":adapter.luid.to_string(),"driver":adapter.driver,"trials":trials}));
    }
    Ok(
        json!({"scope":"isolated GPU-only decode, same synthetic 720p clips; no live changes",
        "adapters":adapters,
        "p50_definition":"upper middle","p95_definition":"nearest rank","includes_network_or_present":false,
        "warning":"Output availability is not GPU completion. Completion includes offline input-view validation. No live performance or zero-copy claim."}),
    )
}

/// Existing GPU-only MF/direct paths, without the live client's HEVC profile
/// preflight. An actual fixture decode decides this isolated trial's result.
pub fn intel_hevc_baseline(luid: u64) -> Result<Value> {
    let _runtime = Runtime::start()?;
    let d3d = D3d11::load()?;
    let adapter = d3d
        .adapters()?
        .into_iter()
        .find(|a| a.luid.value() == luid)
        .context("requested adapter missing")?;
    let device = d3d.open(adapter.luid)?;
    ensure!(device.protect(), "D3D11 multithread protection unavailable");
    let clip = Clip::Hevc;
    let units = clip.units()?;
    let mut trials = Vec::new();
    for direct in [false, true] {
        let result = if direct {
            direct::trial(&device, clip, &units, 60)
        } else {
            mf::trial(&device, clip, &units, 60)
        };
        trials.push(json!({"path":if direct {"direct_d3d11"} else {"synchronous_mft"},
            "result":match result {Ok(value)=>value, Err(error)=>json!({"failed":format!("{error:#}")})}}));
    }
    Ok(json!(trials))
}
