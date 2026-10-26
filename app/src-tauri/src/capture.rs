//! Small images for the ring, as PNG data URLs:
//! - the patch of screen behind the ring (the UI blurs it for the frosted-glass look,
//!   which works on Windows 10 and 11 and on a round shape, unlike DWM acrylic),
//! - the Explorer thumbnail of the dragged file for the centre hub.

use base64::Engine;
use windows::core::*;
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::{CoInitializeEx, IBindCtx, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK};

/// Backdrop is blurred anyway, so a small capture is plenty and keeps the event tiny.
const BACKDROP_PX: i32 = 96;

fn encode_png(w: u32, h: u32, rgba: &[u8]) -> Option<String> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        enc.write_header().ok()?.write_image_data(rgba).ok()?;
    }
    Some(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(out)))
}

/// Reads a bitmap (not selected into any DC) as top-down RGBA.
unsafe fn bitmap_rgba(hbm: HBITMAP, w: i32, h: i32, keep_alpha: bool) -> Option<Vec<u8>> {
    unsafe {
        let mut bi = BITMAPINFO::default();
        bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = w;
        bi.bmiHeader.biHeight = -h; // top-down
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB.0;
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let screen = GetDC(None);
        let lines = GetDIBits(screen, hbm, 0, h as u32, Some(buf.as_mut_ptr() as *mut _), &mut bi, DIB_RGB_COLORS);
        ReleaseDC(None, screen);
        if lines == 0 {
            return None;
        }
        for px in buf.chunks_exact_mut(4) {
            px.swap(0, 2); // BGRA -> RGBA
            if !keep_alpha {
                px[3] = 255;
            } else if px[3] > 0 && px[3] < 255 {
                // shell thumbnails are premultiplied
                let a = px[3] as u32;
                for c in &mut px[..3] {
                    *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
                }
            }
        }
        Some(buf)
    }
}

/// Captures the screen square at (x, y) of `size` physical pixels, downscaled.
pub fn backdrop(x: i32, y: i32, size: i32) -> Option<String> {
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(screen);
        let bmp = CreateCompatibleBitmap(screen, BACKDROP_PX, BACKDROP_PX);
        let old = SelectObject(mem, bmp);
        SetStretchBltMode(mem, HALFTONE);
        let ok = StretchBlt(mem, 0, 0, BACKDROP_PX, BACKDROP_PX, screen, x, y, size, size, SRCCOPY).as_bool();
        SelectObject(mem, old);
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        let rgba = if ok { bitmap_rgba(bmp, BACKDROP_PX, BACKDROP_PX, false) } else { None };
        let _ = DeleteObject(bmp);
        encode_png(BACKDROP_PX as u32, BACKDROP_PX as u32, &rgba?)
    }
}

/// Explorer's thumbnail (or icon) for `path`. Call from a worker thread.
pub fn thumbnail(path: &str, size: i32) -> Option<String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let factory: IShellItemImageFactory = SHCreateItemFromParsingName(&HSTRING::from(path), None::<&IBindCtx>).ok()?;
        let hbm = factory.GetImage(SIZE { cx: size, cy: size }, SIIGBF_BIGGERSIZEOK).ok()?;
        let mut info = BITMAP::default();
        GetObjectW(hbm, std::mem::size_of::<BITMAP>() as i32, Some(&mut info as *mut _ as *mut _));
        let rgba = bitmap_rgba(hbm, info.bmWidth, info.bmHeight, true);
        let _ = DeleteObject(hbm);
        encode_png(info.bmWidth as u32, info.bmHeight as u32, &rgba?)
    }
}
