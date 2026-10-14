//! Intel capability gate: only current VPL on a caller-owned D3D11 device.
//! No legacy runtime, CPU plane fallback, split/export or staging textures.
use anyhow::{Result, anyhow, ensure};
use lowlat_drivers::{
    d3d11::D3d11,
    vpl::{Runtime, Vpl},
};
use serde_json::{Value, json};
use windows::{
    Win32::Graphics::{
        Direct3D11::{D3D11_DECODER_PROFILE_HEVC_VLD_MAIN, ID3D11VideoDevice},
        Dxgi::Common::DXGI_FORMAT_NV12,
    },
    core::Interface,
};
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
            let raw_video = device.video().cast();
            // SAFETY: the Device owns the borrowed video interface for this scope.
            let video = unsafe { ID3D11VideoDevice::from_raw_borrowed(&raw_video) }
                .ok_or_else(|| anyhow!("D3D11 video interface unavailable"))?;
            let d3d11_hevc = match unsafe {
                video
                    .CheckVideoDecoderFormat(&D3D11_DECODER_PROFILE_HEVC_VLD_MAIN, DXGI_FORMAT_NV12)
            } {
                Ok(supported) => json!({"supported":supported.as_bool()}),
                Err(error) => json!({"unknown":error.to_string()}),
            };
            let session = vpl.session(&device, index)?;
            let version = session.version()?;
            let caps = lowlat_decode::vpl::caps(&session, Runtime::Current);
            let mut h264_trials = Vec::new();
            for round in 0..5 {
                let result = match fixture::decode(&session, &device) {
                    Ok(mut value) => {
                        value["round"] = json!(round);
                        value
                    }
                    Err(error) => json!({"round":round,"refused":format!("{error:#}")}),
                };
                let failed = result.get("refused").is_some();
                h264_trials.push(result);
                if failed {
                    break;
                }
            }
            let hevc_fixture = match fixture::hevc(&session, &device) {
                Ok(value) => value,
                Err(error) => json!({"refused":format!("{error:#}")}),
            };
            Ok(
                json!({"vpl_version":[version.0,version.1], "h264":caps.h264,
                "hevc_8bit":caps.hevc, "device_fences":device.has_fences(),
                "d3d11_hevc_main_nv12":d3d11_hevc,
                "output_contract":"caller D3D11 device, video memory",
                "h264_fixture":h264_trials.first(),
                "h264_trials":h264_trials,
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
