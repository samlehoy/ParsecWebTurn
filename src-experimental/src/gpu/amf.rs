use super::*;
use lowlat_drivers::amf::{
    Amf, Codec, LOW_LATENCY_DECODE, Layout, REORDER_LOW_LATENCY, REORDER_MODE, Submitted,
};

pub(super) fn trial(device: &Device, clip: Clip, units: &[&[u8]], fps: u64) -> Result<Value> {
    let runtime = Amf::load()?;
    let context = runtime.context(device)?;
    let decoder = runtime.decoder(
        &context,
        if clip == Clip::H264 {
            Codec::H264
        } else {
            Codec::Hevc
        },
    )?;
    let low_latency_present = decoder.has(LOW_LATENCY_DECODE);
    let low_latency_accepted = decoder.set_bool(LOW_LATENCY_DECODE, true).is_ok();
    let reorder_present = decoder.has(REORDER_MODE);
    let reorder_accepted = decoder.set_int(REORDER_MODE, REORDER_LOW_LATENCY).is_ok();
    // Documented defaults made explicit; refuse a path that would add a
    // surface copy or distribute decode to another GPU.
    decoder.set_bool("SurfaceCopy", false)?;
    decoder.set_bool("EnableDecoderSmartAccessVideo", false)?;
    decoder.init(Layout::Nv12, 1280, 720)?;
    let mut buffer = context.buffer(units.iter().map(|u| u.len()).max().context("empty clip")?)?;
    let mut samples = Samples::new();
    let origin = Instant::now();
    for (index, unit) in units.iter().enumerate() {
        pace(origin, index, fps);
        buffer.fill(unit)?;
        buffer.set_pts(index as i64 + 1);
        samples.submit(index);
        let submitted = decoder.submit(&buffer)?;
        ensure!(
            matches!(submitted, Submitted::Taken | Submitted::MoreInput),
            "AMF needs unsupported retry: {submitted:?}"
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        // Poll asynchronous output at once, without a frame/timer-sized wait.
        loop {
            if let Some(surface) = decoder.query()? {
                let raw = surface.texture();
                ensure!(!raw.is_null(), "AMF has no native D3D11 texture");
                // SAFETY: surface owns the borrowed texture until after output.
                let texture =
                    unsafe { ID3D11Texture2D::from_raw_borrowed(&raw) }.context("AMF texture")?;
                samples.output(surface.pts(), index, texture, 0, device, clip)?;
                break;
            }
            ensure!(
                Instant::now() < deadline,
                "AMF held output or failed to progress"
            );
            std::thread::yield_now();
        }
    }
    let mut result = samples.finish(units.len())?;
    result["low_latency_property_present"] = json!(low_latency_present);
    result["low_latency_request_accepted"] = json!(low_latency_accepted);
    result["reorder_property_present"] = json!(reorder_present);
    result["reorder_request_accepted"] = json!(reorder_accepted);
    Ok(result)
}
