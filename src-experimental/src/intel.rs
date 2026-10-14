//! Intel capability gate: only current VPL on a caller-owned D3D11 device.
//! No legacy runtime, CPU plane fallback, split/export or staging textures.
use anyhow::{Result, anyhow, ensure};
use lowlat_drivers::{
    d3d11::D3d11,
    vpl::{Runtime, Vpl},
};
use serde_json::{Value, json};
mod fixture;

pub fn probe() -> Result<Value> {
    let d3d = D3d11::load()?;
    let mut results = Vec::new();
    for adapter in d3d
        .adapters()?
        .into_iter()
        .filter(|a| a.vendor == 0x8086 && a.decodes_here())
    {
        let result = (|| -> Result<Value> {
            let vpl = Vpl::for_adapter(&adapter)?;
            ensure!(
                vpl.runtime() == Runtime::Current,
                "legacy Intel runtime refused: requires CPU planes"
            );
            let index = d3d
                .plain_index(adapter.luid)?
                .ok_or_else(|| anyhow!("Intel DXGI index unavailable"))?;
            let device = d3d.open(adapter.luid)?;
            ensure!(device.protect(), "D3D11 multithread protection unavailable");
            let session = vpl.session(&device, index)?;
            let version = session.version()?;
            let caps = lowlat_decode::vpl::caps(&session, Runtime::Current);
            let fixture = match fixture::decode(&session, &device) {
                Ok(value) => value,
                Err(error) => json!({"refused":error.to_string()}),
            };
            let hevc_fixture = match fixture::hevc(&session, &device) {
                Ok(value) => value,
                Err(error) => json!({"refused":error.to_string()}),
            };
            Ok(
                json!({"vpl_version":[version.0,version.1], "h264":caps.h264,
                "hevc_8bit":caps.hevc, "device_fences":device.has_fences(),
                "output_contract":"caller D3D11 device, video memory",
                "h264_fixture":fixture,
                "hevc_fixture":hevc_fixture,
                "decode_performance_verified":false, "renderer_integration_verified":false}),
            )
        })();
        results.push(
            json!({"gpu":adapter.description, "luid":adapter.luid.to_string(),
            "driver":adapter.driver, "result":match result {
                Ok(value) => value, Err(e) => json!({"refused":e.to_string()})
            }}),
        );
    }
    Ok(json!({"intel_adapters":results,"cpu_video_fallback":false,
        "live_client_modified":false,"note":"Capability only; not proof of latency or copy count"}))
}
