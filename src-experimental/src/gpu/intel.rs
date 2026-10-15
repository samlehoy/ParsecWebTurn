//! Offline Intel HEVC comparison. No live backend selection is changed.
use super::*;
use lowlat_drivers::{
    ffi::vpl::*,
    vpl::{Decoded, Runtime as VplRuntime, Session, Vpl},
};
mod original;

fn vpl_trial(
    session: &Session<'_>,
    device: &Device,
    clip: Clip,
    units: &[&[u8]],
    fps: u64,
) -> Result<Value> {
    let mut param = session
        .header(
            units[0],
            if clip == Clip::H264 {
                MFX_CODEC_AVC as u32
            } else {
                MFX_CODEC_HEVC as u32
            },
        )?
        .context("codec header")?;
    param.IOPattern = MFX_IOPATTERN_OUT_VIDEO_MEMORY as u16;
    param.AsyncDepth = 1;
    unsafe {
        session.query(&mut param)?;
        session.init(&mut param)?;
    }
    let result = (|| -> Result<Value> {
        let mut samples = Samples::new();
        let origin = Instant::now();
        for (index, unit) in units.iter().enumerate() {
            pace(origin, index, fps);
            samples.submit(index);
            let mut bits: mfxBitstream = unsafe { std::mem::zeroed() };
            bits.Data = unit.as_ptr().cast_mut();
            bits.DataLength = unit.len() as u32;
            bits.MaxLength = bits.DataLength;
            bits.DataFlag = MFX_BITSTREAM_COMPLETE_FRAME as u16;
            bits.TimeStamp = index as u64 + 1;
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                ensure!(Instant::now() < deadline, "VPL decode stalled");
                match unsafe { session.decode(&mut bits, std::ptr::null_mut()) }? {
                    Decoded::Picture(picture) => {
                        // Synchronize before handing the surface to a non-VPL component.
                        // This offline wait is included in the measured output endpoint.
                        while !session.sync(&picture, 0)? {
                            ensure!(Instant::now() < deadline, "VPL completion timeout");
                            std::thread::yield_now();
                        }
                        let raw = picture.texture();
                        let texture = unsafe { ID3D11Texture2D::from_raw_borrowed(&raw) }
                            .context("VPL native texture missing")?;
                        // Current VPL internal allocation returns individual textures.
                        // Refuse arrays rather than guessing an undocumented slice.
                        let mut desc = D3D11_TEXTURE2D_DESC::default();
                        unsafe {
                            texture.GetDesc(&mut desc);
                        }
                        ensure!(
                            desc.ArraySize == 1,
                            "VPL array surface needs a documented slice"
                        );
                        samples.output(
                            picture.timestamp() as i64,
                            index,
                            texture,
                            0,
                            device,
                            clip,
                        )?;
                        if bits.DataLength == 0 {
                            break;
                        }
                    }
                    Decoded::Busy | Decoded::Warning => std::thread::yield_now(),
                    Decoded::MoreData => break,
                    _ => anyhow::bail!("VPL requires unsupported surface or format change"),
                }
            }
        }
        let mut report = samples.finish_buffered(units.len())?;
        report["async_depth"] = json!(param.AsyncDepth);
        report["vpl_sync_before_external_use"] = json!(true);
        Ok(report)
    })();
    session.close_decoder();
    result
}

pub fn compare() -> Result<Value> {
    let _runtime = Runtime::start()?;
    let d3d = D3d11::load()?;
    let units = Clip::Hevc.units()?;
    let mut adapters = Vec::new();
    for adapter in d3d
        .adapters()?
        .into_iter()
        .filter(|a| a.vendor == 0x8086 && a.decodes_here())
    {
        eprintln!("Intel HEVC: {}", adapter.description);
        let device = d3d.open(adapter.luid)?;
        ensure!(device.protect(), "D3D11 protection required");
        let index = d3d
            .plain_index(adapter.luid)?
            .context("DXGI adapter index")?;
        let vpl = Vpl::for_adapter(&adapter);
        let mut trials = Vec::new();
        for clip in [Clip::H264, Clip::Hevc] {
            let units = clip.units()?;
            for fps in [30, 60] {
                for round in 0..5 {
                    for backend in if round % 2 == 0 { [0, 1] } else { [1, 0] } {
                        eprintln!(
                            "  {} {} / {fps} fps / round {}",
                            clip.name(),
                            if backend == 0 { "MF" } else { "VPL" },
                            round + 1
                        );
                        let result = if backend == 0 {
                            mf::trial(&device, clip, &units, fps)
                        } else {
                            (|| -> Result<Value> {
                                let vpl = vpl.as_ref().map_err(|e| anyhow::anyhow!("{e}"))?;
                                ensure!(
                                    vpl.runtime() == VplRuntime::Current,
                                    "current GPU VPL runtime required"
                                );
                                let session = vpl.session(&device, index)?;
                                vpl_trial(&session, &device, clip, &units, fps)
                            })()
                        };
                        trials.push(json!({"codec":clip.name(),"backend":if backend == 0 {"MF + video processor"} else {"VPL + video processor"},
                        "fps":fps,"round":round+1,"result":match result {Ok(v)=>v,Err(e)=>json!({"refused":format!("{e:#}")})}}));
                    }
                }
            }
        }
        // The original-surface path has a different endpoint: presentation
        // submission is not scanout and is not ranked against GPU completion.
        let original = (|| -> Result<Value> {
            let vpl = vpl.as_ref().map_err(|e| anyhow::anyhow!("{e}"))?;
            ensure!(
                vpl.runtime() == VplRuntime::Current,
                "current GPU VPL runtime required"
            );
            original::trial(&device, index, &units)
        })();
        trials.push(json!({"codec":"HEVC Main","backend":"VPL original NV12 / Windows Presentation API","fps":60,
            "result":match original {Ok(v)=>v,Err(e)=>json!({"refused":format!("{e:#}"),"copy_fallback":false})}}));
        adapters.push(json!({"adapter":adapter.description,"driver":adapter.driver,"luid":adapter.luid.to_string(),"trials":trials}));
    }
    ensure!(!adapters.is_empty(), "no Intel GPU found");
    Ok(
        json!({"fixtures":["synthetic-720p-h264.bin","synthetic-720p-hevc.bin"],"frames_per_trial":120,"warmup_frames":16,
        "adapters":adapters,"live_client_modified":false,"cpu_pixel_readback":false,
        "warning":"Synthetic offline waits. MF/VPL completion includes the same video processor output write and validation. Original-surface submission is a different endpoint; scanout and internal driver copies are unknown. GPU textures alone do not prove hardware codec execution."}),
    )
}
