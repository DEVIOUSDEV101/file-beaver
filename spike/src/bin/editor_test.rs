//! End-to-end check of the Phase 5 editors against the running app:
//! drag a file onto a Tools petal, wait for the editor, edit with the mouse, Ctrl+S,
//! then inspect the saved file's pixels.
//!
//! usage: editor_test <file> <tool: crop|add_bg|edit|annotate|redact> <out_dir>
//!        editor_test --make-pdf <image> <out.pdf>

use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant};

use image::GenericImageView;
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
        let (vx, vy) = (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN));
        let (vw, vh) = (GetSystemMetrics(SM_CXVIRTUALSCREEN), GetSystemMetrics(SM_CYVIRTUALSCREEN));
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: (x - vx) * 65535 / (vw - 1),
                    dy: (y - vy) * 65535 / (vh - 1),
                    dwFlags: flags | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_MOVE | MOUSEEVENTF_VIRTUALDESK,
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

fn tap(vk: VIRTUAL_KEY) {
    key(vk, true);
    sleep(Duration::from_millis(60));
    key(vk, false);
}

fn glide(from: (i32, i32), to: (i32, i32)) {
    for i in 1..=20 {
        mouse(MOUSE_EVENT_FLAGS(0), from.0 + (to.0 - from.0) * i / 20, from.1 + (to.1 - from.1) * i / 20);
        sleep(Duration::from_millis(20));
    }
}

fn drag(from: (i32, i32), to: (i32, i32)) {
    mouse(MOUSE_EVENT_FLAGS(0), from.0, from.1);
    sleep(Duration::from_millis(80));
    mouse(MOUSEEVENTF_LEFTDOWN, from.0, from.1);
    sleep(Duration::from_millis(80));
    glide(from, to);
    sleep(Duration::from_millis(80));
    mouse(MOUSEEVENTF_LEFTUP, to.0, to.1);
    sleep(Duration::from_millis(200));
}

fn click(at: (i32, i32)) {
    mouse(MOUSE_EVENT_FLAGS(0), at.0, at.1);
    sleep(Duration::from_millis(80));
    mouse(MOUSEEVENTF_LEFTDOWN, at.0, at.1);
    sleep(Duration::from_millis(60));
    mouse(MOUSEEVENTF_LEFTUP, at.0, at.1);
    sleep(Duration::from_millis(250));
}

fn polar(c: (i32, i32), r: f64, deg: f64) -> (i32, i32) {
    let a = deg.to_radians();
    (c.0 + (r * a.sin()) as i32, c.1 - (r * a.cos()) as i32)
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

fn screenshot(path: &Path, x: i32, y: i32, w: i32, h: i32) {
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(screen);
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(mem, bmp);
        let _ = BitBlt(mem, 0, 0, w, h, screen, x, y, SRCCOPY | CAPTUREBLT);
        SelectObject(mem, old);
        let mut bi = BITMAPINFO::default();
        bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = w;
        bi.bmiHeader.biHeight = -h;
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        let mut buf = vec![0u8; (w * h * 4) as usize];
        GetDIBits(mem, bmp, 0, h as u32, Some(buf.as_mut_ptr() as *mut _), &mut bi, DIB_RGB_COLORS);
        let _ = DeleteObject(bmp);
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        for px in buf.chunks_exact_mut(4) {
            px.swap(0, 2);
            px[3] = 255;
        }
        image::RgbaImage::from_raw(w as u32, h as u32, buf).unwrap().save(path).unwrap();
        println!("screenshot {}", path.display());
    }
}

unsafe extern "system" fn pad_proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(h, m, w, l) }
}

fn tool_info(tool: &str) -> (usize, &'static str, &'static str) {
    // (index on the 7-petal Tools ring, window title, output suffix)
    match tool {
        "edit" => (2, "Edit", "Edited"),
        "annotate" => (3, "Annotate", "Annotated"),
        "add_bg" => (4, "Add Background", "With BG"),
        "crop" => (5, "Crop", "Cropped"),
        "redact" => (6, "Redact", "Redacted"),
        _ => panic!("unknown tool {tool}"),
    }
}

/// Drags `file` onto the tool's petal (tap Shift twice: Convert ring, then Tools ring).
fn drop_on_tool(file: &str, index: usize) -> Result<()> {
    unsafe {
        let hinst = GetModuleHandleW(None)?;
        let wc = WNDCLASSW { lpfnWndProc: Some(pad_proc), hInstance: hinst.into(), lpszClassName: w!("EditorTestPad"), hbrBackground: HBRUSH(GetStockObject(WHITE_BRUSH).0), ..Default::default() };
        RegisterClassW(&wc);
        let pad = CreateWindowExW(WS_EX_TOPMOST, w!("EditorTestPad"), w!("editor test"), WS_POPUP | WS_VISIBLE, 200, 150, 1500, 1300, None, None, hinst, None)?;
        let _ = SetForegroundWindow(pad);
        pump(300);
        let scale = GetDpiForWindow(pad) as f64 / 96.0;
        let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(file), None::<&IBindCtx>)?;
        let data: IDataObject = item.BindToHandler(None, &BHID_DataObject)?;
        let source: IDropSource = Source.into();
        let start = (900, 800);
        let center = (start.0 + 30, start.1);
        mouse(MOUSE_EVENT_FLAGS(0), start.0, start.1);
        pump(100);
        mouse(MOUSEEVENTF_LEFTDOWN, start.0, start.1);
        pump(100);
        let r = 0.665 * 180.0 * scale;
        std::thread::spawn(move || {
            sleep(Duration::from_millis(200));
            glide(start, center);
            sleep(Duration::from_millis(200));
            tap(VK_SHIFT);
            sleep(Duration::from_millis(400));
            tap(VK_SHIFT);
            sleep(Duration::from_millis(400));
            let t = polar(center, r, 360.0 * index as f64 / 7.0);
            glide(center, t);
            sleep(Duration::from_millis(400));
            mouse(MOUSEEVENTF_LEFTUP, t.0, t.1);
        });
        let mut effect = DROPEFFECT::default();
        let _ = DoDragDrop(&data, &source, DROPEFFECT_COPY | DROPEFFECT_MOVE, &mut effect);
        println!("drop effect={} (1=COPY)", effect.0);
        pump(200);
        let _ = DestroyWindow(pad);
        Ok(())
    }
}

/// Natural size of what the editor shows: the upright image, or page 1 at 150 dpi.
fn shown_size(file: &Path) -> (u32, u32) {
    if file.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
        let img = shiftdrop_core::pdf::open(file).unwrap().render(0, 150.0).unwrap();
        (img.width(), img.height())
    } else {
        let d = shiftdrop_core::decode::load(file).unwrap();
        d.image.dimensions()
    }
}

fn newest_with(dir: &Path, suffix: &str, after: std::time::SystemTime) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().contains(suffix)))
        .filter(|p| std::fs::metadata(p).and_then(|m| m.modified()).is_ok_and(|t| t >= after))
        .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
}

/// The saved result as an image (PDFs rendered at 96 dpi).
fn load_result(p: &Path) -> image::DynamicImage {
    if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
        shiftdrop_core::pdf::open(p).unwrap().render(0, 96.0).unwrap()
    } else {
        image::open(p).unwrap()
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    unsafe {
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        OleInitialize(None)?;
    }
    shiftdrop_core::wic::init_thread();
    if args[1] == "--make-pdf" {
        let img = image::open(&args[2]).unwrap();
        let page = shiftdrop_core::pdf::Page { image: shiftdrop_core::pdf::PageImage::Pixels(img), dpi: 96.0 };
        std::fs::write(&args[3], shiftdrop_core::pdf::write_images(&[page]).unwrap()).unwrap();
        println!("made {}", args[3]);
        return Ok(());
    }
    let file = PathBuf::from(&args[1]);
    let tool = args[2].as_str();
    let out = PathBuf::from(&args[3]);
    std::fs::create_dir_all(&out).unwrap();
    let (index, title, suffix) = tool_info(tool);
    let (iw, ih) = shown_size(&file);
    let started = std::time::SystemTime::now();

    drop_on_tool(file.to_str().unwrap(), index)?;

    // Find the editor window.
    let name = file.file_name().unwrap().to_string_lossy();
    let wtitle = HSTRING::from(format!("{title} · {name}"));
    let t0 = Instant::now();
    let hwnd = loop {
        if let Ok(h) = unsafe { FindWindowW(None, &wtitle) } {
            if unsafe { IsWindowVisible(h) }.as_bool() {
                break h;
            }
        }
        if t0.elapsed() > Duration::from_secs(8) {
            println!("FAIL: editor window '{wtitle}' never appeared");
            std::process::exit(1);
        }
        sleep(Duration::from_millis(100));
    };
    println!("editor opened after {:.1}s", t0.elapsed().as_secs_f32());
    unsafe {
        let _ = SetForegroundWindow(hwnd);
    }
    sleep(Duration::from_millis(300));

    // Where the canvas is: same layout maths as Stage in src/editor/ui.tsx.
    let (cx0, cy0, cw, ch, scale) = unsafe {
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        let mut p = POINT::default();
        let _ = ClientToScreen(hwnd, &mut p);
        (p.x, p.y, rc.right, rc.bottom, GetDpiForWindow(hwnd) as f64 / 96.0)
    };
    let (w, h) = (cw as f64 / scale, ch as f64 / scale);
    let (sw, sh) = (w - 264.0, h - 48.0 - 56.0);
    let k = ((sw - 56.0) / iw as f64).min((sh - 56.0) / ih as f64).min(3.0);
    let (dw, dh) = ((iw as f64 * k).floor(), (ih as f64 * k).floor());
    let (left, top) = ((sw - dw) / 2.0, 48.0 + (sh - dh) / 2.0);
    let at = |nx: f64, ny: f64| (cx0 + ((left + nx * dw) * scale).round() as i32, cy0 + ((top + ny * dh) * scale).round() as i32);
    let panel = |x: f64, y: f64| (cx0 + ((w - 264.0 + x) * scale) as i32, cy0 + (y * scale) as i32);

    // Wait until the image is actually drawn (the stage shows "Loading…" until then).
    let t2 = Instant::now();
    let stage_bg = (0xeb, 0xe3, 0xda);
    loop {
        let (x, y) = at(0.5, 0.5);
        let c = unsafe { GetPixel(GetDC(None), x, y) }.0;
        let rgb = ((c & 0xff) as i32, ((c >> 8) & 0xff) as i32, ((c >> 16) & 0xff) as i32);
        let is_bg = (rgb.0 - stage_bg.0).abs() < 6 && (rgb.1 - stage_bg.1).abs() < 6 && (rgb.2 - stage_bg.2).abs() < 6;
        if !is_bg && t2.elapsed() > Duration::from_millis(150) {
            break;
        }
        if t2.elapsed() > Duration::from_secs(10) {
            println!("FAIL: image never appeared in the editor");
            std::process::exit(1);
        }
        sleep(Duration::from_millis(30));
    }
    println!("image visible {} ms after the window appeared", (t0.elapsed() - Duration::from_millis(0)).as_millis());
    sleep(Duration::from_millis(250));

    match tool {
        "crop" => drag(at(1.0, 1.0), at(0.6, 0.5)),
        "annotate" => drag(at(0.2, 0.2), at(0.7, 0.6)),
        "redact" => drag(at(0.1, 0.1), at(0.5, 0.5)),
        // "Rotate right": second button in the first panel section.
        "edit" => click(panel(16.0 + 34.0 + 6.0 + 17.0, 48.0 + 6.0 + 12.0 + 15.0 + 9.0 + 17.0)),
        _ => {}
    }
    sleep(Duration::from_millis(500));
    screenshot(&out.join(format!("{tool}-editor.png")), cx0, cy0, cw, ch);

    key(VK_CONTROL, true);
    tap(VIRTUAL_KEY(b'S' as u16));
    key(VK_CONTROL, false);

    let dir = file.parent().unwrap();
    let t1 = Instant::now();
    let saved = loop {
        if let Some(p) = newest_with(dir, suffix, started) {
            sleep(Duration::from_millis(300));
            break p;
        }
        if t1.elapsed() > Duration::from_secs(20) {
            println!("FAIL: nothing saved with '{suffix}' in {}", dir.display());
            std::process::exit(1);
        }
        sleep(Duration::from_millis(100));
    };
    println!("saved {} after {:.1}s", saved.display(), t1.elapsed().as_secs_f32());
    println!("editor still visible: {}", unsafe { IsWindowVisible(hwnd) }.as_bool());

    let img = load_result(&saved);
    let (ow, oh) = img.dimensions();
    let is_pdf = file.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
    // Source size in the result's units (PDF results are rendered at 96 dpi).
    let (sw0, sh0) = if is_pdf { ((iw as f64 * 96.0 / 150.0).round() as u32, (ih as f64 * 96.0 / 150.0).round() as u32) } else { (iw, ih) };
    let px = |nx: f64, ny: f64| img.get_pixel(((nx * ow as f64) as u32).min(ow - 1), ((ny * oh as f64) as u32).min(oh - 1));
    let close = |a: u32, b: f64| (a as f64 - b).abs() <= b * 0.03 + 2.0;
    let ok = match tool {
        "crop" => {
            println!("source {sw0}x{sh0} -> {ow}x{oh} (expect ~60% x 50%)");
            close(ow, sw0 as f64 * 0.6) && close(oh, sh0 as f64 * 0.5)
        }
        "add_bg" => {
            let pad = 0.08 * sw0.max(sh0) as f64;
            println!("source {sw0}x{sh0} -> {ow}x{oh} (expect +{pad:.0}px padding each side)");
            close(ow, sw0 as f64 + 2.0 * pad) && close(oh, sh0 as f64 + 2.0 * pad)
        }
        "edit" => {
            println!("source {sw0}x{sh0} -> {ow}x{oh} (expect rotated: {sh0}x{sw0})");
            ow == sh0 && oh == sw0
        }
        "annotate" => {
            let p = px(0.45, 0.4);
            println!("pixel on the arrow: {p:?} (expect red)");
            p[0] > 170 && p[1] < 110 && p[2] < 110
        }
        "redact" => {
            let p = px(0.3, 0.3);
            let q = px(0.8, 0.8);
            println!("inside box: {p:?} (expect black), outside: {q:?}");
            p[0] < 25 && p[1] < 25 && p[2] < 25 && (q[0] as u32 + q[1] as u32 + q[2] as u32) > 75
        }
        _ => false,
    };
    println!("{} {tool}", if ok { "PASS" } else { "FAIL" });
    Ok(())
}
