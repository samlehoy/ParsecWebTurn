//! Offline decode on the original NV12 surface: no export, split or readback.
use anyhow::{Context, Result, anyhow, ensure};
use lowlat_drivers::{
    d3d11::Device,
    ffi::vpl::*,
    vpl::{Decoded, Session},
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use windows::{
    Win32::Graphics::{Direct3D11::*, Dxgi::Common::DXGI_FORMAT_NV12},
    core::Interface,
};

pub fn decode(session: &Session<'_>, device: &Device) -> Result<Value> {
    const FIXTURE: &[u8] = include_bytes!("../../../src-native/fixtures/synthetic-1920x1080.h264");
    let mut starts: Vec<usize> = (0..FIXTURE.len().saturating_sub(4))
        .filter(|&i| FIXTURE[i..].starts_with(&[0, 0, 0, 1]) && FIXTURE[i + 4] & 31 == 9)
        .collect();
    ensure!(starts.len() == 8, "H.264 fixture access unit count changed");
    starts.push(FIXTURE.len());
    let units: Vec<&[u8]> = (0..80)
        .map(|sequence| {
            let index = sequence % 8;
            &FIXTURE[starts[index]..starts[index + 1]]
        })
        .collect();
    decode_units(session, device, MFX_CODEC_AVC as u32, &units)
}

pub fn hevc(session: &Session<'_>, device: &Device) -> Result<Value> {
    let fixture = include_bytes!("../../fixtures/synthetic-720p-hevc.bin");
    let mut remaining: &[u8] = fixture;
    let mut units = Vec::new();
    while !remaining.is_empty() {
        ensure!(remaining.len() >= 4, "truncated HEVC fixture prefix");
        let len = u32::from_le_bytes(remaining[..4].try_into()?) as usize;
        remaining = &remaining[4..];
        ensure!(len <= remaining.len(), "truncated HEVC fixture unit");
        units.push(&remaining[..len]);
        remaining = &remaining[len..];
    }
    ensure!(units.len() == 120, "HEVC fixture count changed");
    decode_units(session, device, MFX_CODEC_HEVC as u32, &units)
}

fn decode_units(
    session: &Session<'_>,
    device: &Device,
    codec: u32,
    units: &[&[u8]],
) -> Result<Value> {
    let mut param = session
        .header(units[0], codec)
        .context("VPL DecodeHeader")?
        .ok_or_else(|| anyhow!("fixture has no header"))?;
    param.IOPattern = MFX_IOPATTERN_OUT_VIDEO_MEMORY as u16;
    param.AsyncDepth = 0;
    // SAFETY: header initialized the decoder union; no extension buffers.
    unsafe {
        param
            .__bindgen_anon_1
            .mfx
            .__bindgen_anon_1
            .__bindgen_anon_2
            .DecodedOrder = 1;
        session.init(&mut param).context("VPL DecodeInit")?;
    }
    let result = (|| -> Result<Value> {
        let mut outputs = 0;
        let mut output_us = Vec::new();
        let mut completed_us = Vec::new();
        let mut warnings = 0;
        let mut pending = BTreeMap::new();
        let origin = Instant::now();
        for (sequence, unit) in units.iter().enumerate() {
            let deadline = origin + Duration::from_nanos(sequence as u64 * 16_666_667);
            std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
            // SAFETY: zeroed public bitstream is valid. The compressed bytes
            // remain live until decode returns; the runtime does not write them.
            let mut bits: mfxBitstream = unsafe { std::mem::zeroed() };
            bits.Data = unit.as_ptr().cast_mut();
            bits.DataLength = unit.len() as u32;
            bits.MaxLength = bits.DataLength;
            bits.DataFlag = MFX_BITSTREAM_COMPLETE_FRAME as u16;
            bits.TimeStamp = sequence as u64 + 1;
            let submitted = Instant::now();
            pending.insert(bits.TimeStamp, submitted);
            loop {
                ensure!(
                    submitted.elapsed() < Duration::from_secs(2),
                    "Intel decode did not make progress"
                );
                // SAFETY: input outlives the call; current VPL allocates GPU
                // output, so the caller-owned work surface must be null.
                match unsafe { session.decode(&mut bits, std::ptr::null_mut()) }
                    .context("VPL DecodeFrameAsync")?
                {
                    Decoded::Picture(picture) => {
                        let output_ready = Instant::now();
                        let output_submitted = pending
                            .remove(&picture.timestamp())
                            .ok_or_else(|| anyhow!("uncorrelated VPL output timestamp"))?;
                        output_us
                            .push(output_ready.duration_since(output_submitted).as_micros() as u64);
                        let raw = picture.texture();
                        ensure!(!raw.is_null(), "Intel output has no native GPU texture");
                        // SAFETY: Picture keeps the native texture alive. This
                        // is a borrowed COM reference, never released by us.
                        let texture = unsafe { ID3D11Texture2D::from_raw_borrowed(&raw) }
                            .ok_or_else(|| anyhow!("invalid native texture"))?;
                        let mut desc = D3D11_TEXTURE2D_DESC::default();
                        unsafe { texture.GetDesc(&mut desc) };
                        ensure!(
                            desc.Format == DXGI_FORMAT_NV12 && desc.CPUAccessFlags == 0,
                            "Intel output is not GPU-only NV12"
                        );
                        let owner = unsafe { texture.GetDevice()? };
                        ensure!(
                            owner.as_raw() == device.device().cast(),
                            "Intel decoded on a different device"
                        );
                        // Offline only. Retain the surface until completion;
                        // do not use a synchronous per-picture wait in playback.
                        while !session.sync(&picture, 0).context("VPL SyncOperation")? {
                            ensure!(
                                submitted.elapsed() < Duration::from_secs(2),
                                "Intel GPU completion timed out"
                            );
                            std::thread::yield_now();
                        }
                        completed_us.push(output_submitted.elapsed().as_micros() as u64);
                        outputs += 1;
                        // A picture need not consume the entire access unit.
                        // Keep feeding its remainder as the SDK backend does.
                        if bits.DataLength == 0 {
                            break;
                        }
                    }
                    Decoded::Busy => std::thread::yield_now(),
                    Decoded::MoreData => break,
                    Decoded::Warning => {
                        // The SDK reports positive statuses without an output
                        // surface here. They are not fatal errors. Respect its
                        // consumption of input and retry only the remaining bytes.
                        // A driver that makes no progress still hits the deadline.
                        warnings += 1;
                        if bits.DataLength == 0 {
                            break;
                        }
                        std::thread::yield_now();
                    }
                    Decoded::MoreSurface => {
                        anyhow::bail!("Intel unexpectedly requires a caller surface")
                    }
                    Decoded::Incompatible => anyhow::bail!("Intel fixture format changed"),
                }
            }
        }
        ensure!(
            outputs == units.len(),
            "Intel held or lost fixture frames: {outputs}/{}",
            units.len()
        );
        Ok(
            json!({"frames":outputs,"native_nv12_texture":true,"same_d3d11_device":true,
            "submission_to_output_us":output_us,"submission_to_gpu_completion_us":completed_us,
            "nonfatal_warnings":warnings,
            "input_pacing_fps":60,"warmup_frames":16,"output_timestamp_correlated":true,
            "application_decoded_frame_copies":0,"cpu_readback":false,
            "includes_presentation":false,"runtime_internal_copies":"unknown"}),
        )
    })();
    session.close_decoder();
    result
}
