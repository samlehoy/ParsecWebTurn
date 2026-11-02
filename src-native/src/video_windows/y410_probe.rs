//! Opt-in offline Y410 diagnosis. Only synthetic pixels are uploaded/read back.
//! No network, account, input injection, live decoder or production capability override.
use super::*;
use serde_json::{json, Value};
const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;

fn expected(x: u32, y: u32) -> [f64; 3] {
    if y < HEIGHT / 2 {
        [f64::from(x.min(1023)) / 1023.; 3]
    } else if x.is_multiple_of(2) {
        [0.75, 0.25, 0.25]
    } else {
        [0.25, 0.75, 0.25]
    }
}
fn packed(rgb: [f64; 3], full: bool) -> u32 {
    let [r, g, b] = rgb;
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let uv_scale = if full { 1023. } else { 896. };
    let luma = if full { y * 1023. } else { 64. + y * 876. };
    let quantize = |v: f64| v.round().clamp(0., 1023.) as u32;
    quantize(512. + (b - y) / 1.8556 * uv_scale)
        | (quantize(luma) << 10)
        | (quantize(512. + (r - y) / 1.5748 * uv_scale) << 20)
        | (3 << 30)
}
fn failure(error: Failure) -> Value {
    json!({"stage":error.stage,"hresult":error.hresult})
}
unsafe fn conversion(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    full: bool,
    explicit: bool,
    report: &mut Value,
) -> Result<()> {
    let video: ID3D11VideoDevice = api("probe-video-device", device.cast())?;
    let vc: ID3D11VideoContext = api("probe-video-context", context.cast())?;
    let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
        InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
        InputWidth: WIDTH,
        InputHeight: HEIGHT,
        OutputWidth: WIDTH,
        OutputHeight: HEIGHT,
        InputFrameRate: DXGI_RATIONAL {
            Numerator: 60,
            Denominator: 1,
        },
        OutputFrameRate: DXGI_RATIONAL {
            Numerator: 60,
            Denominator: 1,
        },
        Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
    };
    let enumeration = api(
        "probe-enumerator",
        video.CreateVideoProcessorEnumerator(&desc),
    )?;
    report["input_support"] = enumeration
        .CheckVideoProcessorFormat(DXGI_FORMAT_Y410)
        .map(|flags| json!(flags & D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_INPUT.0 as u32 != 0))
        .unwrap_or(Value::Null);
    report["output_support"] = enumeration
        .CheckVideoProcessorFormat(DXGI_FORMAT_R10G10B10A2_UNORM)
        .map(|flags| json!(flags & D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_OUTPUT.0 as u32 != 0))
        .unwrap_or(Value::Null);
    let color = crate::video_color::processor_color(Some(1), Some(if full { 1 } else { 2 }));
    report["conversion_query"] = match enumeration.cast::<ID3D11VideoProcessorEnumerator1>() {
        Ok(e) => match e.CheckVideoProcessorFormatConversion(
            DXGI_FORMAT_Y410,
            rgb10_input_color(color),
            DXGI_FORMAT_R10G10B10A2_UNORM,
            DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709,
        ) {
            Ok(supported) => json!({"supported":supported.as_bool()}),
            Err(e) => json!({"hresult":format!("0x{:08X}",e.code().0 as u32)}),
        },
        Err(e) => json!({"interface_hresult":format!("0x{:08X}",e.code().0 as u32)}),
    };
    let processor = api(
        "probe-processor",
        video.CreateVideoProcessor(&enumeration, 0),
    )?;
    let pixels: Vec<u32> = (0..HEIGHT)
        .flat_map(|y| (0..WIDTH).map(move |x| packed(expected(x, y), full)))
        .collect();
    let input_desc = D3D11_TEXTURE2D_DESC {
        Width: WIDTH,
        Height: HEIGHT,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_Y410,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_DECODER.0 as u32,
        ..Default::default()
    };
    let initial = D3D11_SUBRESOURCE_DATA {
        pSysMem: pixels.as_ptr().cast(),
        SysMemPitch: WIDTH * 4,
        SysMemSlicePitch: 0,
    };
    let mut input = None;
    api(
        "probe-input-texture",
        device.CreateTexture2D(&input_desc, Some(&initial), Some(&mut input)),
    )?;
    let input = input.ok_or_else(|| unavailable("probe-input-null"))?;
    let mut output = None;
    let output_desc = D3D11_TEXTURE2D_DESC {
        Format: DXGI_FORMAT_R10G10B10A2_UNORM,
        BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
        ..input_desc
    };
    api(
        "probe-output-texture",
        device.CreateTexture2D(&output_desc, None, Some(&mut output)),
    )?;
    let output = output.ok_or_else(|| unavailable("probe-output-null"))?;
    let mut input_view = None;
    api(
        "probe-input-view",
        video.CreateVideoProcessorInputView(
            &input,
            &enumeration,
            &D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
                ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_VPIV {
                        MipSlice: 0,
                        ArraySlice: 0,
                    },
                },
                ..Default::default()
            },
            Some(&mut input_view),
        ),
    )?;
    let mut output_view = None;
    api(
        "probe-output-view",
        video.CreateVideoProcessorOutputView(
            &output,
            &enumeration,
            &D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
                },
            },
            Some(&mut output_view),
        ),
    )?;
    let input_view = input_view.ok_or_else(|| unavailable("probe-input-view-null"))?;
    let output_view = output_view.ok_or_else(|| unavailable("probe-output-view-null"))?;
    vc.VideoProcessorSetStreamAutoProcessingMode(&processor, 0, false);
    vc.VideoProcessorSetStreamFrameFormat(&processor, 0, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE);
    vc.VideoProcessorSetStreamAlpha(&processor, 0, true, 1.);
    let rect = RECT {
        left: 0,
        top: 0,
        right: WIDTH as i32,
        bottom: HEIGHT as i32,
    };
    vc.VideoProcessorSetStreamSourceRect(&processor, 0, true, Some(&rect));
    vc.VideoProcessorSetStreamDestRect(&processor, 0, true, Some(&rect));
    vc.VideoProcessorSetOutputTargetRect(&processor, true, Some(&rect));
    if explicit {
        let vc1: ID3D11VideoContext1 = api("probe-context1", vc.cast())?;
        vc1.VideoProcessorSetStreamColorSpace1(&processor, 0, rgb10_input_color(color));
        vc1.VideoProcessorSetOutputColorSpace1(&processor, DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709);
    } else {
        vc.VideoProcessorSetStreamColorSpace(
            &processor,
            0,
            &D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: color },
        );
        vc.VideoProcessorSetOutputColorSpace(
            &processor,
            &D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: 0 },
        );
    }
    let mut stream = D3D11_VIDEO_PROCESSOR_STREAM {
        Enable: true.into(),
        pInputSurface: ManuallyDrop::new(Some(input_view)),
        ..Default::default()
    };
    let result = vc.VideoProcessorBlt(&processor, &output_view, 0, std::slice::from_ref(&stream));
    ManuallyDrop::drop(&mut stream.pInputSurface);
    api("probe-blt", result)?;
    // Offline readback only. This module is absent from the production executable.
    let mut staging = None;
    api(
        "probe-staging",
        device.CreateTexture2D(
            &D3D11_TEXTURE2D_DESC {
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                ..output_desc
            },
            None,
            Some(&mut staging),
        ),
    )?;
    let staging = staging.ok_or_else(|| unavailable("probe-staging-null"))?;
    context.CopyResource(&staging, &output);
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    api(
        "probe-map",
        context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)),
    )?;
    if mapped.pData.is_null() || mapped.RowPitch < WIDTH * 4 {
        context.Unmap(&staging, 0);
        return Err(unavailable("probe-map-layout"));
    }
    let mut levels = std::collections::BTreeSet::new();
    let mut gray_error = 0.;
    let mut color_error = 0.;
    for (y, error) in [
        (HEIGHT / 4, &mut gray_error),
        (HEIGHT * 3 / 4, &mut color_error),
    ] {
        let row = mapped
            .pData
            .cast::<u8>()
            .add((y * mapped.RowPitch) as usize);
        for x in 0..1024 {
            let word = std::ptr::read_unaligned(row.add(x as usize * 4).cast::<u32>());
            if y == HEIGHT / 4 {
                levels.insert(word & 1023);
            }
            for (i, want) in expected(x, y).iter().enumerate() {
                let actual = ((word >> (i * 10)) & 1023) as f64;
                *error = f64::max(*error, (actual - want * 1023.).abs());
            }
        }
    }
    context.Unmap(&staging, 0);
    report["blt_succeeded"] = true.into();
    report["gray_levels"] = levels.len().into();
    report["gray_max_error_10bit"] = gray_error.into();
    report["chroma_max_error_10bit"] = color_error.into();
    report["pixels_match"] = json!(gray_error <= 8. && color_error <= 8.);
    report["more_than_8bit_levels"] = json!(levels.len() > 256);
    Ok(())
}

#[test]
#[ignore = "Offline GPU diagnostic: run explicitly on the affected Intel machine"]
fn compare_y410_conversion_paths() {
    let mut reports = Vec::new();
    for adapter in crate::gpu_windows::enumerate().unwrap() {
        let preference = adapter.preference;
        let mut gpu = json!({"adapter":preference.name,"vendor_id":preference.vendor_id,"device_id":preference.device_id,"cases":[]});
        match unsafe { create_device(Some(&preference)) } {
            Err(error) => gpu["device_error"] = failure(error),
            Ok((device, context)) => {
                for full in [true, false] {
                    for explicit in [false, true] {
                        let mut report = json!({"range":if full {"full"} else {"limited"},"color_api":if explicit {"explicit"} else {"legacy"},"blt_succeeded":false});
                        if let Err(error) =
                            unsafe { conversion(&device, &context, full, explicit, &mut report) }
                        {
                            report["error"] = failure(error);
                        }
                        gpu["cases"].as_array_mut().unwrap().push(report);
                    }
                }
            }
        }
        reports.push(gpu);
    }
    let report = json!({"synthetic_only":true,"live_frames_read_back":false,"input_format":"Y410","output_format":"R10G10B10A2_UNORM","width":WIDTH,"height":HEIGHT,"adapters":reports});
    let path = std::env::var_os("EVOPARSEQ_Y410_REPORT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "y410-probe.json".into());
    std::fs::write(&path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    println!("Synthetic Y410 report: {}", path.display());
}

#[test]
fn synthetic_source_preserves_full_precision_and_per_pixel_chroma() {
    let levels: std::collections::BTreeSet<_> = (0..1024)
        .map(|x| (packed(expected(x, 0), true) >> 10) & 1023)
        .collect();
    assert_eq!(levels.len(), 1024);
    assert_eq!(packed([0.; 3], false) >> 10 & 1023, 64);
    assert_eq!(packed([1.; 3], false) >> 10 & 1023, 940);
    assert_ne!(
        packed(expected(0, HEIGHT - 1), true),
        packed(expected(1, HEIGHT - 1), true)
    );
}
