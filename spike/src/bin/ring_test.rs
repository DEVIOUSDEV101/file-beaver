//! Automated check for the Phase 3 ring, run against the real app:
//! Explorer-identical file drag -> tap Shift (Convert ring) -> hover a format -> screenshot
//! -> tap Shift (Tools ring) -> hover Crop -> screenshot -> drop. Then a Shift-drag that is
//! not a file drag, which must auto-hide.
//!
//! usage: ring_test <file> <out_dir> <drop: crop|png|hub>

use std::path::PathBuf;
use std::thread::sleep;
use std::time::Duration;

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::*;
use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

#[implement(IDropSource)]
struct Source;

impl IDropSource_Impl for Source_Impl {
    fn QueryContinueDrag(&self, esc: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
        if esc.as_bool() { DRAGDROP_S_CANCEL } else if keys.0 & MK_LBUTTON.0 == 0 { DRAGDROP_S_DROP } else { S_OK }
    }
    fn GiveFeedback(&self, _e: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

fn mouse(flags: MOUSE_EVENT_FLAGS, x: i32, y: i32) {
    unsafe {
        let (sw, sh) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: x * 65535 / (sw - 1),
                    dy: y * 65535 / (sh - 1),
                    dwFlags: flags | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_MOVE,
                    ..Default::default()
                },
            },
        };
        SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
}

fn key(vk: VIRTUAL_KEY, down: bool) {
    unsafe {
        let input = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT { wVk: vk, dwFlags: if down { KEYBD_EVENT_FLAGS(0) } else { KEYEVENTF_KEYUP }, ..Default::default() },
            },
        };
        SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
}

fn tap_shift() {
    key(VK_SHIFT, true);
    sleep(Duration::from_millis(60));
    key(VK_SHIFT, false);
}

fn glide(from: (i32, i32), to: (i32, i32)) {
    for i in 1..=20 {
        mouse(MOUSE_EVENT_FLAGS(0), from.0 + (to.0 - from.0) * i / 20, from.1 + (to.1 - from.1) * i / 20);
        sleep(Duration::from_millis(20));
    }
}

/// Point at `deg` clockwise from the top, `r` physical px from `c`.
fn polar(c: (i32, i32), r: f64, deg: f64) -> (i32, i32) {
    let a = deg.to_radians();
    (c.0 + (r * a.sin()) as i32, c.1 - (r * a.cos()) as i32)
}

fn screenshot(path: &PathBuf, cx: i32, cy: i32, half: i32) {
    unsafe {
        let size = half * 2;
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(screen);
        let bmp = CreateCompatibleBitmap(screen, size, size);
        let old = SelectObject(mem, bmp);
        let _ = BitBlt(mem, 0, 0, size, size, screen, cx - half, cy - half, SRCCOPY | CAPTUREBLT);
        SelectObject(mem, old);
        let mut bi = BITMAPINFO::default();
        bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = size;
        bi.bmiHeader.biHeight = -size;
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        let mut buf = vec![0u8; (size * size * 4) as usize];
        GetDIBits(mem, bmp, 0, size as u32, Some(buf.as_mut_ptr() as *mut _), &mut bi, DIB_RGB_COLORS);
        let _ = DeleteObject(bmp);
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        for px in buf.chunks_exact_mut(4) {
            px.swap(0, 2);
            px[3] = 255;
        }
        let f = std::fs::File::create(path).unwrap();
        let mut enc = png::Encoder::new(f, size as u32, size as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.write_header().unwrap().write_image_data(&buf).unwrap();
        println!("screenshot {}", path.display());
    }
}

/// Warm gradient like the reference's wallpaper, so the frosted backdrop has something to show.
unsafe extern "system" fn pad_proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if m == WM_PAINT {
            let mut ps = PAINTSTRUCT::default();
            let dc = BeginPaint(h, &mut ps);
            let mut rc = RECT::default();
            let _ = GetClientRect(h, &mut rc);
            let bands = 60;
            for i in 0..bands {
                let t = i as f32 / (bands - 1) as f32;
                let (r, g, b) = (240.0 - 10.0 * t, 170.0 - 90.0 * t, 60.0 + 70.0 * t);
                let brush = CreateSolidBrush(COLORREF((b as u32) << 16 | (g as u32) << 8 | r as u32));
                let band = RECT {
                    left: rc.left,
                    right: rc.right,
                    top: rc.top + (rc.bottom - rc.top) * i / bands,
                    bottom: rc.top + (rc.bottom - rc.top) * (i + 1) / bands,
                };
                FillRect(dc, &band, brush);
                let _ = DeleteObject(brush);
            }
            let _ = EndPaint(h, &ps);
            return LRESULT(0);
        }
        DefWindowProcW(h, m, w, l)
    }
}

fn pump(ms: u64) {
    for _ in 0..(ms / 10) {
        unsafe {
            let mut m = MSG::default();
            while PeekMessageW(&mut m, None, 0, 0, PM_REMOVE).into() {
                let _ = TranslateMessage(&m);
                DispatchMessageW(&m);
            }
        }
        sleep(Duration::from_millis(10));
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let file = args[1].clone();
    let out = PathBuf::from(&args[2]);
    let drop_on = args.get(3).cloned().unwrap_or("crop".into());
    std::fs::create_dir_all(&out).unwrap();
    unsafe {
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
        OleInitialize(None)?;
        let hinst = GetModuleHandleW(None)?;
        let wc = WNDCLASSW { lpfnWndProc: Some(pad_proc), hInstance: hinst.into(), lpszClassName: w!("RingTestPad"), ..Default::default() };
        RegisterClassW(&wc);
        let pad = CreateWindowExW(WS_EX_TOPMOST, w!("RingTestPad"), w!("ring test"), WS_POPUP | WS_VISIBLE, 200, 150, 1500, 1300, None, None, hinst, None)?;
        let _ = SetForegroundWindow(pad);
        pump(400);
        let scale = GetDpiForWindow(pad) as f64 / 96.0;
        let r_hover = 0.665 * 180.0 * scale; // middle of a petal
        println!("scale {scale}");

        let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(file.as_str()), None::<&IBindCtx>)?;
        let data: IDataObject = item.BindToHandler(None, &BHID_DataObject)?;
        let source: IDropSource = Source.into();

        let start = (900, 800);
        let center = (start.0 + 30, start.1);
        let half = (200.0 * scale) as i32;
        mouse(MOUSE_EVENT_FLAGS(0), start.0, start.1);
        pump(100);
        mouse(MOUSEEVENTF_LEFTDOWN, start.0, start.1);
        pump(100);
        let out2 = out.clone();
        let drop_on2 = drop_on.clone();
        std::thread::spawn(move || {
            sleep(Duration::from_millis(200));
            glide(start, center); // drag begins; ring not shown yet
            sleep(Duration::from_millis(200));
            tap_shift(); // Convert ring
            sleep(Duration::from_millis(400));
            let png = polar(center, r_hover, 0.0); // PNG is at the top for a JPG
            glide(center, png);
            sleep(Duration::from_millis(500));
            screenshot(&out2.join("1-convert-hover-png.png"), center.0, center.1, half);
            tap_shift(); // Tools ring
            sleep(Duration::from_millis(400));
            let crop = polar(center, r_hover, 360.0 * 5.0 / 7.0); // Crop is index 5 of 7
            glide(png, crop);
            sleep(Duration::from_millis(500));
            screenshot(&out2.join("2-tools-hover-crop.png"), center.0, center.1, half);
            let target = match drop_on2.as_str() {
                // Tools ring: Compress is at the top, Metadata next clockwise.
                "compress" | "metadata" => {
                    let deg = if drop_on2 == "compress" { 0.0 } else { 360.0 / 7.0 };
                    let t = polar(center, r_hover, deg);
                    glide(crop, t);
                    sleep(Duration::from_millis(300));
                    t
                }
                "png" => {
                    tap_shift(); // back to Convert
                    sleep(Duration::from_millis(300));
                    glide(crop, png);
                    sleep(Duration::from_millis(300));
                    png
                }
                "hub" => {
                    glide(crop, center);
                    sleep(Duration::from_millis(300));
                    center
                }
                _ => crop,
            };
            mouse(MOUSEEVENTF_LEFTUP, target.0, target.1);
        });
        let mut effect = DROPEFFECT::default();
        let hr = DoDragDrop(&data, &source, DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK, &mut effect);
        println!("TEST1 DoDragDrop hr={hr:?} effect={} (1=COPY, 0=NONE)", effect.0);
        println!("TEST1 source still exists: {}", std::path::Path::new(&file).exists());
        // The progress card appears below-right of the drop point.
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let card = (pt.x + (190.0 * scale) as i32, pt.y + (100.0 * scale) as i32);
        pump(250);
        screenshot(&out.join("4-card-early.png"), card.0, card.1, (210.0 * scale) as i32);
        pump(1500);
        screenshot(&out.join("5-card-later.png"), card.0, card.1, (210.0 * scale) as i32);
        pump(600);

        // Shift-drag that is NOT a file drag: ring may flash, must hide by itself.
        mouse(MOUSEEVENTF_LEFTDOWN, start.0, start.1);
        pump(50);
        glide(start, center);
        tap_shift();
        pump(1300);
        screenshot(&out.join("3-non-file-drag-after-1.3s.png"), center.0, center.1, half);
        mouse(MOUSEEVENTF_LEFTUP, center.0, center.1);
        pump(300);
        let _ = DestroyWindow(pad);
    }
    Ok(())
}
