//! Isolated synthetic presentation smoke test; no account/network/readback.
#[cfg(not(windows))]
fn main() {}
#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    use anyhow::{Context as _, ensure};
    use lowlat_drivers::d3d11::D3d11;
    use parsec_gpu_video::display::Session;
    use std::time::{Duration, Instant};
    use windows::{
        Win32::{
            Foundation::*,
            Graphics::Gdi::*,
            System::{Com::*, LibraryLoader::*},
            UI::WindowsAndMessaging::*,
        },
        core::w,
    };
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        let result = (|| -> anyhow::Result<()> {
            let adapter = D3d11::load()?
                .adapters()?
                .into_iter()
                .find(|a| a.vendor == 0x10de && a.decodes_here())
                .context("NVIDIA GPU missing")?;
            let instance = GetModuleHandleW(None)?;
            let class = w!("ParsecOriginalSurfaceProbe");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(DefWindowProcW),
                hInstance: instance.into(),
                lpszClassName: class,
                hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as _),
                ..Default::default()
            };
            ensure!(RegisterClassW(&wc) != 0, "window class registration");
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class,
                w!("Synthetic NVIDIA NV12 presentation test"),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                960,
                600,
                None,
                None,
                Some(instance.into()),
                None,
            )?;
            let test = (|| -> anyhow::Result<()> {
                let mut decoder = Session::open(adapter.luid.value(), hwnd)?;
                println!("Adapter: {}", decoder.adapter());
                let mut bytes: &[u8] =
                    include_bytes!("../../src-experimental/fixtures/synthetic-720p-hevc.bin");
                let mut frames = 0;
                let mut presented = 0;
                let origin = Instant::now();
                while !bytes.is_empty() {
                    ensure!(bytes.len() >= 4, "fixture header");
                    let len = u32::from_le_bytes(bytes[..4].try_into()?) as usize;
                    bytes = &bytes[4..];
                    ensure!(len > 0 && len <= bytes.len(), "fixture unit");
                    let mut msg = MSG::default();
                    while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                    let mut client = RECT::default();
                    GetClientRect(hwnd, &mut client)?;
                    let output = decoder.feed(&bytes[..len], true, |layout| {
                        let (cw, ch) = (
                            (client.right - client.left) as f64,
                            (client.bottom - client.top) as f64,
                        );
                        let scale = (cw / layout.width as f64).min(ch / layout.height as f64);
                        let (w, h) = (
                            (layout.width as f64 * scale).round() as i32,
                            (layout.height as f64 * scale).round() as i32,
                        );
                        Ok([
                            (cw as i32 - w) / 2,
                            (ch as i32 - h) / 2,
                            (cw as i32 + w) / 2,
                            (ch as i32 + h) / 2,
                        ])
                    })?;
                    presented += output.presented;
                    frames += 1;
                    bytes = &bytes[len..];
                    std::thread::sleep(
                        (origin + Duration::from_nanos(frames as u64 * 1_000_000_000 / 60))
                            .saturating_duration_since(Instant::now()),
                    );
                }
                ensure!(
                    frames == 120 && presented == 120,
                    "not all synthetic frames submitted"
                );
                println!(
                    "PASS: {presented} original NV12 frames submitted; no application conversion/copy or CPU readback. Actual scanout mode not verified."
                );
                Ok(())
            })();
            DestroyWindow(hwnd)?;
            UnregisterClassW(class, Some(instance.into()))?;
            test
        })();
        CoUninitialize();
        result
    }
}
