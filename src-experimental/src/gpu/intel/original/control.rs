//! Independent GPU clear control, not a fallback or a decoded-frame operation.
use super::*;

pub(super) fn run(device: &DisplayDevice, hwnd: HWND) -> Result<Value> {
    unsafe {
        SetWindowTextW(hwnd, w!("Presentation control - solid MAGENTA expected"))?;
        let desc = D3D11_TEXTURE2D_DESC {
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
            BindFlags: (D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE).0 as u32,
            MiscFlags: (D3D11_RESOURCE_MISC_SHARED | D3D11_RESOURCE_MISC_SHARED_NTHANDLE).0 as u32,
            ..Default::default()
        };
        let mut texture = None;
        device
            .native
            .CreateTexture2D(&desc, None, Some(&mut texture))?;
        let texture = texture.context("RGB control texture")?;
        let mut view = None;
        device
            .native
            .CreateRenderTargetView(&texture, None, Some(&mut view))?;
        let context = device.native.GetImmediateContext()?;
        context.ClearRenderTargetView(
            view.as_ref().context("RGB control view")?,
            &[1.0, 0.0, 1.0, 1.0],
        );
        // Independent synthetic GPU write, not an extra pass on any VPL frame.
        context.Flush();
        let mut presenter = Presenter::new(device, hwnd)?;
        let mut trace = statistics::Trace::new(&presenter.manager)?;
        presenter.composition.WaitForCommitCompletion()?;
        pump();
        let mut client = RECT::default();
        GetClientRect(hwnd, &mut client)?;
        let _lease = presenter.present(
            &texture,
            Layout {
                left: 0,
                top: 0,
                width: 1280,
                height: 720,
                full_range: true,
            },
            (client.right as u32, client.bottom as u32),
            [0, 0, client.right, client.bottom],
        )?;
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            pump();
            trace.collect(&presenter.manager);
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok(
            json!({"expected":"solid magenta","submitted":1,"presentation_feedback":trace.report(),
            "source":"independent GPU ClearRenderTargetView","decoded_frame_operation":false,
            "cpu_pixel_readback":false,"format":desc.Format.0,"misc_flags":desc.MiscFlags}),
        )
    }
}
