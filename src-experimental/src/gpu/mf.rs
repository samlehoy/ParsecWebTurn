use super::*;
use std::mem::ManuallyDrop;
use windows::{
    Win32::{Foundation::VARIANT_TRUE, System::Variant::*},
    core::GUID,
};

unsafe fn decoder(clip: Clip) -> Result<IMFTransform> {
    // SAFETY: COM/MF are initialized and the returned transform owns its references.
    unsafe {
        if clip == Clip::H264 {
            return Ok(CoCreateInstance(
                &CMSH264DecoderMFT,
                None,
                CLSCTX_INPROC_SERVER,
            )?);
        }
        let input = MFT_REGISTER_TYPE_INFO {
            guidMajorType: MFMediaType_Video,
            guidSubtype: MFVideoFormat_HEVC,
        };
        let output = MFT_REGISTER_TYPE_INFO {
            guidMajorType: MFMediaType_Video,
            guidSubtype: subtype(clip),
        };
        let mut list = std::ptr::null_mut();
        let mut count = 0;
        MFTEnumEx(
            MFT_CATEGORY_VIDEO_DECODER,
            MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_LOCALMFT | MFT_ENUM_FLAG_SORTANDFILTER,
            Some(&input),
            Some(&output),
            &mut list,
            &mut count,
        )?;
        ensure!(!list.is_null(), "no synchronous HEVC transform");
        let mut found = None;
        for index in 0..count {
            let activation = std::ptr::read(list.add(index as usize));
            if found.is_none()
                && let Some(activation) = &activation
                && let Ok(transform) = activation.ActivateObject::<IMFTransform>()
                && transform
                    .GetAttributes()
                    .and_then(|a| a.GetUINT32(&MF_SA_D3D11_AWARE))
                    .is_ok_and(|v| v != 0)
            {
                found = Some(transform);
            }
            drop(activation);
        }
        CoTaskMemFree(Some(list.cast()));
        found.context("no synchronous D3D11-aware HEVC transform")
    }
}
fn subtype(clip: Clip) -> GUID {
    if clip == Clip::Main10 {
        MFVideoFormat_P010
    } else {
        MFVideoFormat_NV12
    }
}
unsafe fn output_type(transform: &IMFTransform, clip: Clip) -> Result<()> {
    unsafe {
        for index in 0..64 {
            let media = transform.GetOutputAvailableType(0, index)?;
            if media.GetGUID(&MF_MT_SUBTYPE)? == subtype(clip) {
                transform.SetOutputType(0, &media, 0)?;
                let flags = transform.GetOutputStreamInfo(0)?.dwFlags;
                ensure!(
                    flags
                        & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0
                            | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0)
                            as u32
                        != 0,
                    "CPU output buffer refused"
                );
                return Ok(());
            }
        }
    }
    anyhow::bail!("required NV12/P010 output unavailable")
}

unsafe fn collect(
    transform: &IMFTransform,
    samples: &mut Samples,
    latest: usize,
    device: &Device,
    clip: Clip,
) -> Result<()> {
    unsafe {
        for _ in 0..128 {
            let mut output = MFT_OUTPUT_DATA_BUFFER {
                dwStreamID: 0,
                pSample: ManuallyDrop::new(None),
                dwStatus: 0,
                pEvents: ManuallyDrop::new(None),
            };
            let mut status = 0;
            let result = transform.ProcessOutput(0, std::slice::from_mut(&mut output), &mut status);
            let sample = ManuallyDrop::take(&mut output.pSample);
            drop(ManuallyDrop::take(&mut output.pEvents));
            match result {
                Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(()),
                Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                    output_type(transform, clip)?;
                    continue;
                }
                result => result?,
            }
            let sample = sample.context("MF output sample missing")?;
            let stamp = sample.GetSampleTime()?;
            let dxgi: IMFDXGIBuffer = sample
                .GetBufferByIndex(0)?
                .cast()
                .context("CPU output refused")?;
            let mut raw = std::ptr::null_mut();
            dxgi.GetResource(&ID3D11Texture2D::IID, &mut raw)?;
            ensure!(!raw.is_null(), "MF texture missing");
            let texture = ID3D11Texture2D::from_raw(raw);
            samples.output(
                stamp,
                latest,
                &texture,
                dxgi.GetSubresourceIndex()?,
                device,
                clip,
            )?;
        }
    }
    anyhow::bail!("unbounded MF output loop")
}

pub(super) fn trial(device: &Device, clip: Clip, units: &[&[u8]], fps: u64) -> Result<Value> {
    // SAFETY: this probe owns COM/MF, the selected device and all sample lifetimes.
    unsafe {
        let raw = device.device().cast();
        let native_device = ID3D11Device::from_raw_borrowed(&raw).context("D3D11 device")?;
        let mut token = 0;
        let mut manager = None;
        MFCreateDXGIDeviceManager(&mut token, &mut manager)?;
        let manager = manager.context("DXGI manager")?;
        manager.ResetDevice(native_device, token)?;
        let decoder = decoder(clip)?;
        let attrs = decoder.GetAttributes()?;
        ensure!(
            attrs.GetUINT32(&MF_SA_D3D11_AWARE)? != 0,
            "MF transform not D3D11 aware"
        );
        let value = VARIANT {
            Anonymous: VARIANT_0 {
                Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                    vt: if clip == Clip::H264 { VT_UI4 } else { VT_BOOL },
                    Anonymous: if clip == Clip::H264 {
                        VARIANT_0_0_0 { ulVal: 1 }
                    } else {
                        VARIANT_0_0_0 {
                            boolVal: VARIANT_TRUE,
                        }
                    },
                    ..Default::default()
                }),
            },
        };
        let api_accepted = decoder
            .cast::<ICodecAPI>()
            .is_ok_and(|c| c.SetValue(&CODECAPI_AVLowLatencyMode, &value).is_ok());
        let accepted =
            api_accepted || (clip != Clip::H264 && attrs.SetUINT32(&MF_LOW_LATENCY, 1).is_ok());
        decoder.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, manager.as_raw() as usize)?;
        let input = MFCreateMediaType()?;
        input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        input.SetGUID(
            &MF_MT_SUBTYPE,
            &if clip == Clip::H264 {
                MFVideoFormat_H264
            } else {
                MFVideoFormat_HEVC_ES
            },
        )?;
        if clip != Clip::H264 {
            input.SetUINT32(
                &MF_MT_MPEG2_PROFILE,
                if clip == Clip::Main10 { 2 } else { 1 },
            )?;
            input.SetUINT64(&MF_MT_FRAME_SIZE, (1280u64 << 32) | 720)?;
        }
        decoder.SetInputType(0, &input, 0)?;
        // HEVC can reject a placeholder output until parameter sets arrive.
        let initial_type = output_type(&decoder, clip);
        if clip == Clip::H264 {
            initial_type?;
        }
        decoder.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
        decoder.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
        let mut samples = Samples::new();
        let origin = Instant::now();
        for (index, bytes) in units.iter().enumerate() {
            pace(origin, index, fps);
            let buffer = MFCreateMemoryBuffer(bytes.len() as u32)?;
            let mut ptr = std::ptr::null_mut();
            buffer.Lock(&mut ptr, None, None)?;
            if ptr.is_null() {
                let _ = buffer.Unlock();
                anyhow::bail!("compressed buffer missing");
            }
            // Compressed input only; decoded pixels never touch host memory.
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
            buffer.Unlock()?;
            buffer.SetCurrentLength(bytes.len() as u32)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buffer)?;
            sample.SetSampleTime(index as i64 + 1)?;
            samples.submit(index);
            let result = decoder.ProcessInput(0, &sample, 0);
            if result
                .as_ref()
                .err()
                .is_some_and(|e| e.code() == MF_E_NOTACCEPTING)
            {
                collect(&decoder, &mut samples, index, device, clip)?;
                decoder.ProcessInput(0, &sample, 0)?;
            } else {
                result?;
            }
            collect(&decoder, &mut samples, index, device, clip)?;
            // Match AMF's immediate output polling. NEED_MORE_INPUT may be
            // transient even without a new input: do not accidentally measure
            // our next 30/60 Hz submission tick as the decoder's delay.
            let next = origin + Duration::from_nanos((index as u64 + 1) * 1_000_000_000 / fps);
            while samples.outputs <= index && Instant::now() < next {
                collect(&decoder, &mut samples, index, device, clip)?;
                std::thread::yield_now();
            }
        }
        let held = samples.pending.len();
        decoder.ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, 0)?;
        collect(&decoder, &mut samples, units.len() - 1, device, clip)?;
        let mut result = samples.finish(units.len())?;
        result["low_latency_request_accepted"] = json!(accepted);
        result["held_before_drain"] = json!(held);
        Ok(result)
    }
}
