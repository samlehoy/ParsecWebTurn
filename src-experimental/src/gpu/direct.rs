use lowlat_core::video::Codec;
use lowlat_decode::Fed;
use lowlat_drivers::d3d11::Device;
use parsec_gpu_video::direct::Backend;

pub(super) fn trial(
    device: &Device,
    clip: super::Clip,
    units: &[&[u8]],
    fps: u64,
) -> anyhow::Result<serde_json::Value> {
    use windows::core::Interface;
    let mut decoder = Backend::new(
        device,
        if clip == super::Clip::H264 {
            Codec::H264
        } else {
            Codec::H265
        },
    );
    let mut samples = super::Samples::new();
    let origin = std::time::Instant::now();
    for (index, unit) in units.iter().enumerate() {
        super::pace(origin, index, fps);
        samples.submit(index);
        anyhow::ensure!(
            decoder.feed(unit, index)? != Fed::FormatChanged,
            "unexpected D3D11 format change"
        );
        while decoder.take(|raw, slice, sequence| {
            let raw = raw.cast();
            // SAFETY: the DPB output slot remains leased until this callback ends.
            let texture = unsafe {
                windows::Win32::Graphics::Direct3D11::ID3D11Texture2D::from_raw_borrowed(&raw)
            }
            .ok_or_else(|| anyhow::anyhow!("missing direct texture"))?;
            samples.output(sequence as i64 + 1, index, texture, slice, device, clip)
        })? {}
    }
    decoder.drain();
    while decoder.take(|raw, slice, sequence| {
        let raw = raw.cast();
        // SAFETY: as above; no decoded frame is copied/exported.
        let texture = unsafe {
            windows::Win32::Graphics::Direct3D11::ID3D11Texture2D::from_raw_borrowed(&raw)
        }
        .ok_or_else(|| anyhow::anyhow!("missing direct texture"))?;
        samples.output(
            sequence as i64 + 1,
            units.len() - 1,
            texture,
            slice,
            device,
            clip,
        )
    })? {}
    samples.finish(units.len())
}
