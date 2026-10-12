//! Opt-in CPU GUI rasterizer and per-pixel-alpha Win32 layer.
//! Consumes captured GUI vertices/atlases only. Never reads decoded video.
use crate::overlay::Frame;
#[cfg(test)]
use std::sync::Arc;
use windows_sys::Win32::{Foundation::*, Graphics::Gdi::*, UI::WindowsAndMessaging::*};

pub struct Surface {
    pub size: (u32, u32),
    pub bgra: Vec<u8>,
}
#[derive(Clone, Copy)]
struct Vertex {
    p: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
}
fn edge(a: [f32; 2], b: [f32; 2], p: [f32; 2]) -> f32 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}
fn inclusive(a: [f32; 2], b: [f32; 2]) -> bool {
    b[1] < a[1] || (b[1] == a[1] && b[0] > a[0])
}
pub fn rasterize(frame: &Frame) -> Option<Surface> {
    let (w, h) = frame.size;
    let len = (w as usize).checked_mul(h as usize)?.checked_mul(4)?;
    if w == 0 || h == 0 || len > 64 * 1024 * 1024 {
        return None;
    }
    let mut bgra = vec![0u8; len];
    for batch in &frame.batches {
        let tex = &batch.texture;
        if tex.width == 0
            || tex.height == 0
            || tex.rgba.len() != tex.width as usize * tex.height as usize * 4
        {
            return None;
        }
        for triangle in batch.vertices.chunks_exact(60) {
            let mut v = [Vertex {
                p: [0.; 2],
                uv: [0.; 2],
                color: [0.; 4],
            }; 3];
            for (n, bytes) in triangle.chunks_exact(20).enumerate() {
                let f = |i| f32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
                v[n] = Vertex {
                    p: [(f(0) + 1.) * w as f32 * 0.5, (1. - f(4)) * h as f32 * 0.5],
                    uv: [f(8), f(12)],
                    color: std::array::from_fn(|i| bytes[16 + i] as f32 / 255.),
                };
            }
            if v.iter()
                .any(|v| v.p.iter().chain(v.uv.iter()).any(|f| !f.is_finite()))
            {
                continue;
            }
            let mut area = edge(v[0].p, v[1].p, v[2].p);
            if area == 0. {
                continue;
            }
            if area < 0. {
                v.swap(1, 2);
                area = -area;
            }
            let min_x = v
                .iter()
                .map(|v| v.p[0])
                .fold(f32::INFINITY, f32::min)
                .floor() as i32;
            let min_y = v
                .iter()
                .map(|v| v.p[1])
                .fold(f32::INFINITY, f32::min)
                .floor() as i32;
            let max_x = v
                .iter()
                .map(|v| v.p[0])
                .fold(f32::NEG_INFINITY, f32::max)
                .ceil() as i32;
            let max_y = v
                .iter()
                .map(|v| v.p[1])
                .fold(f32::NEG_INFINITY, f32::max)
                .ceil() as i32;
            let [left, top, right, bottom] = batch.clip;
            for y in min_y.max(top).max(0)..max_y.min(bottom).min(h as i32) {
                for x in min_x.max(left).max(0)..max_x.min(right).min(w as i32) {
                    let p = [x as f32 + 0.5, y as f32 + 0.5];
                    let edges = [
                        edge(v[1].p, v[2].p, p),
                        edge(v[2].p, v[0].p, p),
                        edge(v[0].p, v[1].p, p),
                    ];
                    if (0..3).any(|i| {
                        edges[i] < 0.
                            || (edges[i] == 0. && !inclusive(v[(i + 1) % 3].p, v[(i + 2) % 3].p))
                    }) {
                        continue;
                    }
                    let weights = edges.map(|e| e / area);
                    let uv: [f32; 2] =
                        std::array::from_fn(|i| (0..3).map(|n| v[n].uv[i] * weights[n]).sum());
                    let color: [f32; 4] =
                        std::array::from_fn(|i| (0..3).map(|n| v[n].color[i] * weights[n]).sum());
                    let tx = (uv[0] * tex.width as f32 - 0.5).clamp(0., (tex.width - 1) as f32);
                    let ty = (uv[1] * tex.height as f32 - 0.5).clamp(0., (tex.height - 1) as f32);
                    let (x0, y0) = (tx.floor() as u32, ty.floor() as u32);
                    let (x1, y1) = ((x0 + 1).min(tex.width - 1), (y0 + 1).min(tex.height - 1));
                    let (fx, fy) = (tx - x0 as f32, ty - y0 as f32);
                    let sample: [f32; 4] = std::array::from_fn(|i| {
                        let at =
                            |x, y| tex.rgba[((y * tex.width + x) * 4) as usize + i] as f32 / 255.;
                        ((at(x0, y0) * (1. - fx) + at(x1, y0) * fx) * (1. - fy)
                            + (at(x0, y1) * (1. - fx) + at(x1, y1) * fx) * fy)
                            * color[i]
                    });
                    let a = sample[3].clamp(0., 1.);
                    let dst = &mut bgra[((y as u32 * w + x as u32) * 4) as usize..][..4];
                    for i in 0..3 {
                        dst[i] = (sample[2 - i] * a * 255. + dst[i] as f32 * (1. - a))
                            .round()
                            .clamp(0., 255.) as u8;
                    }
                    dst[3] = (a * 255. + dst[3] as f32 * (1. - a))
                        .round()
                        .clamp(0., 255.) as u8;
                }
            }
        }
    }
    Some(Surface {
        size: frame.size,
        bgra,
    })
}

#[derive(Default)]
pub struct Presenter {
    hwnd: usize,
}
impl Presenter {
    pub unsafe fn hide(&self) {
        if self.hwnd != 0 {
            ShowWindow(self.hwnd as HWND, SW_HIDE);
        }
    }
    pub unsafe fn destroy(&mut self) {
        if self.hwnd != 0 {
            DestroyWindow(self.hwnd as HWND);
            self.hwnd = 0;
        }
    }
    pub unsafe fn show(&mut self, parent: HWND, surface: &Surface) -> bool {
        if self.hwnd == 0 {
            let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                class.as_ptr(),
                std::ptr::null(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                parent,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            );
            if hwnd.is_null() {
                return false;
            }
            self.hwnd = hwnd as usize;
        }
        let (w, h) = surface.size;
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w as i32,
                biHeight: -(h as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            },
            ..std::mem::zeroed()
        };
        let dc = CreateCompatibleDC(std::ptr::null_mut());
        if dc.is_null() {
            return false;
        }
        let mut bits = std::ptr::null_mut();
        let bitmap = CreateDIBSection(
            dc,
            &info,
            DIB_RGB_COLORS,
            &mut bits,
            std::ptr::null_mut(),
            0,
        );
        if bitmap.is_null() {
            DeleteDC(dc);
            return false;
        }
        std::ptr::copy_nonoverlapping(surface.bgra.as_ptr(), bits.cast(), surface.bgra.len());
        let old = SelectObject(dc, bitmap);
        let mut position = POINT::default();
        ClientToScreen(parent, &mut position);
        let size = SIZE {
            cx: w as i32,
            cy: h as i32,
        };
        let origin = POINT::default();
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let ok = UpdateLayeredWindow(
            self.hwnd as HWND,
            std::ptr::null_mut(),
            &position,
            &size,
            dc,
            &origin,
            0,
            &blend,
            ULW_ALPHA,
        ) != 0;
        SelectObject(dc, old);
        DeleteObject(bitmap);
        DeleteDC(dc);
        if ok {
            ShowWindow(self.hwnd as HWND, SW_SHOWNA);
        } else {
            self.hide();
        }
        ok
    }
}

#[derive(Default)]
pub struct Pending {
    pub surface: Option<Surface>,
    pub posted: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn triangle_edges_blend_once_and_scissor_preserves_transparency() {
        let mut vertices = Vec::new();
        for (x, y) in [
            (-1., 1.),
            (1., 1.),
            (1., -1.),
            (-1., 1.),
            (1., -1.),
            (-1., -1.),
        ] {
            for f in [x, y, 0.5f32, 0.5] {
                vertices.extend_from_slice(&f.to_le_bytes());
            }
            vertices.extend_from_slice(&[255, 255, 255, 255]);
        }
        let texture = Arc::new(crate::overlay::Texture {
            version: 1,
            width: 3,
            height: 1,
            rgba: [255, 0, 0, 128].repeat(3),
        });
        let frame = Frame {
            size: (8, 8),
            batches: vec![crate::overlay::Batch {
                vertices,
                texture,
                clip: [1, 1, 7, 7],
            }],
        };
        let surface = rasterize(&frame).unwrap();
        for y in 0..8 {
            for x in 0..8 {
                let pixel = &surface.bgra[(y * 8 + x) * 4..][..4];
                assert_eq!(
                    pixel,
                    if (1..7).contains(&x) && (1..7).contains(&y) {
                        &[0, 0, 128, 128]
                    } else {
                        &[0, 0, 0, 0]
                    }
                );
            }
        }
    }
}
