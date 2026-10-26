//! Scripted product demo, recorded from the real app: a clean "desktop" stage (the user's
//! wallpaper, the demo files down the left, the beaver in the middle), then
//! PDF -> JPG -> Crop -> Add BG -> Compress, then Annotate and Redact on a screenshot and
//! Edit and Metadata on a WEBP: every step by dragging the file onto the beaver.
//! The stage covers the main screen, so only the stage and File Beaver's own windows end
//! up in the recording.
//!
//! usage: demo <desk_dir> <assets_dir> <out_dir> [--no-record] [--until <scene>] [--no-clear]
//!   desk_dir   holds the starting files (see START); everything else in it is deleted first
//!   assets_dir wallpaper.png, pdf_thumb.png
//!   out_dir    raw.mkv (the recording) and events.txt (seconds since the sync marker)

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering::SeqCst};
use std::sync::{Mutex, OnceLock};
use std::thread::sleep;
use std::time::{Duration, Instant};

use image::RgbaImage;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::*;
use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

// ---------- layout (logical px; the stage multiplies by the screen's scale) ----------

const ICON_BOX: f64 = 80.0;
const ICON_STEP: f64 = 136.0;
const ICON_COL_STEP: f64 = 150.0;
const ICONS_PER_COL: usize = 5;
/// The files on the desk at the start, in this order.
const START: [&str; 4] = ["Launch plan.pdf", "unnamed.webp", "Screenshot.png", "images (7).jpg"];
const COL_X: f64 = 70.0;
const COL_Y: f64 = 40.0;
const LABEL_W: f64 = 140.0;
/// The beaver's top-left during the demo: dead centre of a 1440x810 (logical) screen.
const BEAVER_AT: (f64, f64) = (720.0 - 38.0, 405.0 - 38.0);

// ---------- shared state ----------

struct Icon {
    path: PathBuf,
    thumb: RgbaImage,
}

static ICONS: Mutex<Vec<Icon>> = Mutex::new(Vec::new());
static SELECTED: AtomicIsize = AtomicIsize::new(-1);
static MARKER: AtomicBool = AtomicBool::new(false);
static STAGE: AtomicIsize = AtomicIsize::new(0);
static SCALE: OnceLock<f64> = OnceLock::new();
static DESK: OnceLock<PathBuf> = OnceLock::new();
static ASSETS: OnceLock<PathBuf> = OnceLock::new();
static WALL: OnceLock<RgbaImage> = OnceLock::new();
static EVENTS: Mutex<Vec<(f64, String)>> = Mutex::new(Vec::new());
static T0: OnceLock<Instant> = OnceLock::new();

const TIMER_SCAN: usize = 1;

fn scale() -> f64 {
    *SCALE.get().unwrap()
}

fn px(v: f64) -> i32 {
    (v * scale()).round() as i32
}

fn event(what: &str) {
    let t = T0.get().map(|t| t.elapsed().as_secs_f64()).unwrap_or(0.0);
    println!("[{t:6.2}] {what}");
    EVENTS.lock().unwrap().push((t, what.to_string()));
}

/// Icon slot i: (thumb box, label rect), physical px.
fn slot(i: usize) -> (RECT, RECT) {
    let (x, y) = (COL_X + ICON_COL_STEP * (i / ICONS_PER_COL) as f64, COL_Y + ICON_STEP * (i % ICONS_PER_COL) as f64);
    let boxr = RECT { left: px(x), top: px(y), right: px(x + ICON_BOX), bottom: px(y + ICON_BOX) };
    let cx = x + ICON_BOX / 2.0;
    let label = RECT { left: px(cx - LABEL_W / 2.0), top: px(y + ICON_BOX + 8.0), right: px(cx + LABEL_W / 2.0), bottom: px(y + ICON_BOX + 46.0) };
    (boxr, label)
}

fn center(r: RECT) -> (i32, i32) {
    ((r.left + r.right) / 2, (r.top + r.bottom) / 2)
}

/// Centre of the icon whose name ends with `suffix` (e.g. "Cropped.jpg").
fn icon_center(suffix: &str) -> Option<(i32, i32)> {
    let icons = ICONS.lock().unwrap();
    let i = icons.iter().position(|ic| ic.path.file_name().unwrap().to_string_lossy().ends_with(suffix))?;
    Some(center(slot(i).0))
}

fn thumbnail(path: &Path) -> Option<RgbaImage> {
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    let img = if ext == "pdf" { image::open(ASSETS.get().unwrap().join("pdf_thumb.png")).ok()? } else { image::open(path).ok()? };
    let b = px(ICON_BOX) as u32;
    Some(img.resize(b, b, image::imageops::FilterType::Triangle).to_rgba8())
}

/// Picks up files that appeared in the desk folder, oldest first.
fn scan() -> bool {
    let Ok(rd) = std::fs::read_dir(DESK.get().unwrap()) else { return false };
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = rd
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| (e.metadata().and_then(|m| m.created()).unwrap_or(std::time::UNIX_EPOCH), e.path()))
        .collect();
    let rank = |p: &PathBuf| START.iter().position(|n| p.file_name().is_some_and(|f| f == *n)).unwrap_or(START.len());
    found.sort_by(|a, b| (rank(&a.1), a.0).cmp(&(rank(&b.1), b.0)));
    let mut icons = ICONS.lock().unwrap();
    let mut changed = false;
    for (_, p) in found {
        if icons.iter().any(|ic| ic.path == p) {
            continue;
        }
        // Skip files still being written (they won't decode yet); next scan gets them.
        if let Some(thumb) = thumbnail(&p) {
            icons.push(Icon { path: p, thumb });
            changed = true;
        }
    }
    changed
}

// ---------- painting ----------

fn blit_rgba(dc: HDC, img: &RgbaImage, x: i32, y: i32) {
    let (w, h) = img.dimensions();
    let mut bgra: Vec<u8> = Vec::with_capacity((w * h * 4) as usize);
    for p in img.pixels() {
        bgra.extend_from_slice(&[p[2], p[1], p[0], 255]);
    }
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            biHeight: -(h as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    unsafe {
        SetDIBitsToDevice(dc, x, y, w, h, 0, 0, 0, h, bgra.as_ptr() as *const _, &bmi, DIB_RGB_COLORS);
    }
}

fn paint(h: HWND) {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let dc = BeginPaint(h, &mut ps);
        let mut rc = RECT::default();
        let _ = GetClientRect(h, &mut rc);
        let mem = CreateCompatibleDC(dc);
        let bmp = CreateCompatibleBitmap(dc, rc.right, rc.bottom);
        let old = SelectObject(mem, bmp);

        blit_rgba(mem, WALL.get().unwrap(), 0, 0);
        let font = CreateFontW(px(13.0), 0, 0, 0, 600, 0, 0, 0, DEFAULT_CHARSET.0 as u32, OUT_DEFAULT_PRECIS.0 as u32, CLIP_DEFAULT_PRECIS.0 as u32, CLEARTYPE_QUALITY.0 as u32, 0, w!("Segoe UI"));
        let oldf = SelectObject(mem, font);
        SetBkMode(mem, TRANSPARENT);
        let sel = SELECTED.load(SeqCst);
        let blue = CreateSolidBrush(COLORREF(0x00d86324));
        let frame = CreateSolidBrush(COLORREF(0x00ffffff));
        for (i, ic) in ICONS.lock().unwrap().iter().enumerate() {
            let (boxr, label) = slot(i);
            let (tw, th) = ic.thumb.dimensions();
            let x = boxr.left + (boxr.right - boxr.left - tw as i32) / 2;
            let y = boxr.bottom - th as i32;
            // White page edge, like a document icon.
            let edge = px(2.0);
            let fr = RECT { left: x - edge, top: y - edge, right: x + tw as i32 + edge, bottom: y + th as i32 + edge };
            FillRect(mem, &fr, frame);
            blit_rgba(mem, &ic.thumb, x, y);

            let name: Vec<u16> = ic.path.file_name().unwrap().to_string_lossy().encode_utf16().collect();
            let mut measure = label;
            let mut text = name.clone();
            DrawTextW(mem, &mut text, &mut measure, DT_CENTER | DT_WORDBREAK | DT_CALCRECT | DT_NOPREFIX);
            let tw = measure.right - measure.left;
            let mut lr = RECT { left: (label.left + label.right - tw) / 2, top: label.top, right: (label.left + label.right + tw) / 2, bottom: measure.bottom };
            if i as isize == sel {
                let pad = px(5.0);
                let old_brush = SelectObject(mem, blue);
                let old_pen = SelectObject(mem, GetStockObject(NULL_PEN));
                let _ = RoundRect(mem, lr.left - pad, lr.top - px(1.0), lr.right + pad, lr.bottom + px(2.0), px(8.0), px(8.0));
                SelectObject(mem, old_pen);
                SelectObject(mem, old_brush);
            } else {
                // Soft shadow under white text.
                SetTextColor(mem, COLORREF(0x00203020));
                let mut sh = RECT { left: lr.left + px(1.0), top: lr.top + px(1.0), right: lr.right + px(1.0), bottom: lr.bottom + px(1.0) };
                let mut t = name.clone();
                DrawTextW(mem, &mut t, &mut sh, DT_CENTER | DT_WORDBREAK | DT_NOPREFIX);
            }
            SetTextColor(mem, COLORREF(0x00ffffff));
            let mut t = name.clone();
            DrawTextW(mem, &mut t, &mut lr, DT_CENTER | DT_WORDBREAK | DT_NOPREFIX);
        }
        if MARKER.load(SeqCst) {
            let m = CreateSolidBrush(COLORREF(0x00ff00ff));
            FillRect(mem, &RECT { left: 0, top: 0, right: 48, bottom: 48 }, m);
            let _ = DeleteObject(m);
        }
        SelectObject(mem, oldf);
        let _ = DeleteObject(font);
        let _ = DeleteObject(blue);
        let _ = DeleteObject(frame);
        let _ = BitBlt(dc, 0, 0, rc.right, rc.bottom, mem, 0, 0, SRCCOPY);
        SelectObject(mem, old);
        let _ = DeleteObject(bmp);
        let _ = DeleteDC(mem);
        let _ = EndPaint(h, &ps);
    }
}

fn repaint() {
    unsafe {
        let _ = InvalidateRect(HWND(STAGE.load(SeqCst) as *mut _), None, false);
    }
}

// ---------- drag source (the stage acts like Explorer's desktop) ----------

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

fn start_drag(h: HWND, x: i32, y: i32) {
    let hit = {
        let icons = ICONS.lock().unwrap();
        (0..icons.len()).find(|i| {
            let (b, l) = slot(*i);
            (x >= b.left && x < b.right && y >= b.top && y < b.bottom) || (x >= l.left && x < l.right && y >= l.top && y < l.bottom)
        }).map(|i| (i, icons[i].path.clone()))
    };
    let Some((i, path)) = hit else {
        SELECTED.store(-1, SeqCst);
        repaint();
        return;
    };
    SELECTED.store(i as isize, SeqCst);
    repaint();
    unsafe {
        let _ = UpdateWindow(h);
        let item: IShellItem = match SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None::<&IBindCtx>) {
            Ok(i) => i,
            Err(_) => return,
        };
        let Ok(data) = item.BindToHandler::<_, IDataObject>(None, &BHID_DataObject) else { return };
        let source: IDropSource = Source.into();
        let _ = SHDoDragDrop(h, &data, &source, DROPEFFECT_COPY | DROPEFFECT_LINK);
    }
}

unsafe extern "system" fn stage_proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        match m {
            WM_PAINT => {
                paint(h);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_TIMER if w.0 == TIMER_SCAN => {
                if scan() {
                    repaint();
                }
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let (x, y) = ((l.0 & 0xffff) as i16 as i32, ((l.0 >> 16) & 0xffff) as i16 as i32);
                start_drag(h, x, y);
                LRESULT(0)
            }
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            _ => DefWindowProcW(h, m, w, l),
        }
    }
}

// ---------- input ----------

fn mouse(flags: MOUSE_EVENT_FLAGS, x: i32, y: i32) {
    unsafe {
        let (vx, vy) = (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN));
        let (vw, vh) = (GetSystemMetrics(SM_CXVIRTUALSCREEN), GetSystemMetrics(SM_CYVIRTUALSCREEN));
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: ((x - vx) as i64 * 65535 / (vw - 1) as i64) as i32,
                    dy: ((y - vy) as i64 * 65535 / (vh - 1) as i64) as i32,
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
            Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, dwFlags: if down { KEYBD_EVENT_FLAGS(0) } else { KEYEVENTF_KEYUP }, ..Default::default() } },
        };
        SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
}

fn cursor() -> (i32, i32) {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    (p.x, p.y)
}

/// Eased mouse move from where the cursor is, ~120 steps a second.
fn glide(to: (i32, i32), ms: u64) {
    let from = cursor();
    let steps = (ms / 8).max(1);
    for i in 1..=steps {
        let t = i as f64 / steps as f64;
        let e = if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 };
        let x = from.0 as f64 + (to.0 - from.0) as f64 * e;
        let y = from.1 as f64 + (to.1 - from.1) as f64 * e;
        mouse(MOUSE_EVENT_FLAGS(0), x.round() as i32, y.round() as i32);
        sleep(Duration::from_millis(8));
    }
}

fn wait(ms: u64) {
    sleep(Duration::from_millis(ms));
}

fn click(at: (i32, i32)) {
    glide(at, 450);
    wait(120);
    mouse(MOUSEEVENTF_LEFTDOWN, at.0, at.1);
    wait(70);
    mouse(MOUSEEVENTF_LEFTUP, at.0, at.1);
}

fn visible_window(title: &str, timeout_ms: u64) -> Option<HWND> {
    let t = Instant::now();
    loop {
        if let Ok(h) = unsafe { FindWindowW(None, &HSTRING::from(title)) } {
            if unsafe { IsWindowVisible(h) }.as_bool() {
                return Some(h);
            }
        }
        if t.elapsed() > Duration::from_millis(timeout_ms) {
            return None;
        }
        wait(25);
    }
}

fn rect_of(h: HWND) -> RECT {
    let mut r = RECT::default();
    unsafe {
        let _ = GetWindowRect(h, &mut r);
    }
    r
}

/// Petal i of n on the open ring (clockwise from the top).
fn petal(i: usize, n: usize) -> Option<(i32, i32)> {
    let ring = visible_window("File Beaver menu", 2000)?;
    let rr = rect_of(ring);
    let (cx, cy) = center(rr);
    let r = (rr.right - rr.left) as f64 * 180.0 / 384.0 * 0.665;
    let a = i as f64 * std::f64::consts::TAU / n as f64;
    Some((cx + (r * a.sin()).round() as i32, cy - (r * a.cos()).round() as i32))
}

// ---------- UI Automation: find editor controls by name ----------

struct Uia(IUIAutomation);

impl Uia {
    fn new() -> Self {
        unsafe { Uia(CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).unwrap()) }
    }

    /// Bounding rect of the first element in `win` whose name starts with `name`
    /// (and, if given, has this control type). Retries while the web page builds its tree.
    fn find(&self, win: HWND, name: &str, kind: Option<UIA_CONTROLTYPE_ID>) -> Option<RECT> {
        let t = Instant::now();
        loop {
            unsafe {
                if let Ok(root) = self.0.ElementFromHandle(win) {
                    if let Ok(all) = root.FindAll(TreeScope_Descendants, &self.0.CreateTrueCondition().unwrap()) {
                        for i in 0..all.Length().unwrap_or(0) {
                            let Ok(e) = all.GetElement(i) else { continue };
                            let n = e.CurrentName().map(|b| b.to_string()).unwrap_or_default();
                            let k = e.CurrentControlType().unwrap_or_default();
                            if n.starts_with(name) && kind.is_none_or(|want| want == k) {
                                if let Ok(r) = e.CurrentBoundingRectangle() {
                                    if r.right > r.left {
                                        return Some(r);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if t.elapsed() > Duration::from_secs(6) {
                println!("FAIL: no control named '{name}'");
                return None;
            }
            wait(150);
        }
    }
}

/// Drags a range slider's thumb from fraction `from` to `to` of its track.
fn slide(r: RECT, from: f64, to: f64) {
    let thumb = (r.bottom - r.top).min(px(16.0));
    let x = |f: f64| r.left + thumb / 2 + ((r.right - r.left - thumb) as f64 * f).round() as i32;
    let y = (r.top + r.bottom) / 2;
    glide((x(from), y), 500);
    wait(100);
    mouse(MOUSEEVENTF_LEFTDOWN, x(from), y);
    wait(80);
    glide((x(to), y), 650);
    wait(80);
    mouse(MOUSEEVENTF_LEFTUP, x(to), y);
}

// ---------- scenes ----------

fn beaver_center() -> Option<(i32, i32)> {
    unsafe { FindWindowW(w!("FileBeaverWidget"), None) }.ok().map(|b| center(rect_of(b)))
}

/// Picks up `file`, carries it onto the beaver (the ring opens around him), then slides
/// onto petal i of n and lets go.
fn beaver_drop(file: (i32, i32), i: usize, n: usize, label: &str) -> Option<()> {
    glide(file, 550);
    wait(180);
    mouse(MOUSEEVENTF_LEFTDOWN, file.0, file.1);
    wait(100);
    glide(beaver_center()?, 800);
    event("on beaver");
    wait(500);
    let p = petal(i, n)?;
    event(&format!("hover {label}"));
    glide(p, 450);
    wait(350);
    event(&format!("drop {label}"));
    mouse(MOUSEEVENTF_LEFTUP, p.0, p.1);
    Some(())
}

/// Picks up `file`, carries it to `spot`, taps Shift (the ring opens under the cursor),
/// then slides onto petal i of n and lets go.
fn shift_drop(file: (i32, i32), spot: (i32, i32), i: usize, n: usize, label: &str) -> Option<()> {
    glide(file, 550);
    wait(180);
    mouse(MOUSEEVENTF_LEFTDOWN, file.0, file.1);
    wait(100);
    glide(spot, 700);
    wait(250);
    event("shift");
    key(VK_SHIFT, true);
    wait(110);
    key(VK_SHIFT, false);
    wait(500);
    let p = petal(i, n)?;
    event(&format!("hover {label}"));
    glide(p, 450);
    wait(350);
    event(&format!("drop {label}"));
    mouse(MOUSEEVENTF_LEFTUP, p.0, p.1);
    Some(())
}

/// The editor's canvas maths (src/editor/ui.tsx Stage): where image point (nx, ny) is.
fn editor_point(win: HWND, img: (u32, u32), nx: f64, ny: f64) -> (i32, i32) {
    unsafe {
        let mut rc = RECT::default();
        let _ = GetClientRect(win, &mut rc);
        let mut p = POINT::default();
        let _ = ClientToScreen(win, &mut p);
        let s = GetDpiForWindow(win) as f64 / 96.0;
        let (w, h) = (rc.right as f64 / s, rc.bottom as f64 / s);
        let (sw, sh) = (w - 264.0, h - 48.0 - 56.0);
        let k = ((sw - 56.0) / img.0 as f64).min((sh - 56.0) / img.1 as f64).min(3.0);
        let (dw, dh) = ((img.0 as f64 * k).floor(), (img.1 as f64 * k).floor());
        let (left, top) = ((sw - dw) / 2.0, 48.0 + (sh - dh) / 2.0);
        (p.x + ((left + nx * dw) * s).round() as i32, p.y + ((top + ny * dh) * s).round() as i32)
    }
}

/// Square editor in the middle of the screen, so it fills a square reel frame.
fn square_up(win: HWND) {
    unsafe {
        let mut work = RECT::default();
        let _ = SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut work as *mut _ as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
        let side = (work.bottom - work.top) - px(16.0);
        let x = (work.left + work.right - side) / 2;
        let y = work.top + px(8.0);
        let _ = SetWindowPos(win, None, x, y, side, side, SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

fn run_script(from: u32, until: u32) -> Option<()> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let uia = Uia::new();
    // Sync marker for the edit: a magenta square in the corner for a moment.
    wait(800);
    MARKER.store(true, SeqCst);
    repaint();
    T0.set(Instant::now()).ok();
    event("marker");
    wait(300);
    MARKER.store(false, SeqCst);
    repaint();
    mouse(MOUSE_EVENT_FLAGS(0), px(1100.0), px(700.0));
    wait(1000);

    if from <= 1 {
        // 1. Convert: PDF -> JPG (Convert ring, JPG at the top of 8).
        event("scene convert");
        beaver_drop(icon_center(".pdf")?, 0, 8, "jpg")?;
        wait_for_name("Launch plan.jpg", 8000)?;
        event("jpg appeared");
        deselect(900);
        if until == 1 {
            return Some(());
        }
    }

    if from <= 2 {
        // 2. The meme screenshot -> Crop (6th of 7): keep just the painting.
        event("scene crop");
        beaver_drop(icon_center("unnamed.webp")?, 5, 7, "crop")?;
        let win = open_editor("Crop · unnamed.webp")?;
        let size = (924, 2000);
        drag_on(editor_point(win, size, 0.0, 0.0), editor_point(win, size, 0.05, 0.287), 800);
        drag_on(editor_point(win, size, 1.0, 1.0), editor_point(win, size, 0.95, 0.628), 900);
        event("cropped");
        wait(300);
        click(center(uia.find(win, "Save", Some(UIA_ButtonControlTypeId))?));
        wait_for_name("Cropped", 8000)?;
        event("crop saved");
        deselect(700);
        if until == 2 {
            return Some(());
        }
    }

    if from <= 3 {
        // 3. Cropped -> Add BG (5th of 7): white, portrait 4:5, square corners: meme layout.
        event("scene add bg");
        let cropped_name = name_containing("Cropped")?;
        beaver_drop(wait_for_name("Cropped", 1000)?, 4, 7, "add bg")?;
        let win = open_editor(&format!("Add Background · {cropped_name}"))?;
        click(center(uia.find(win, "#ffffff", None)?));
        wait(250);
        click(center(uia.find(win, "4:5", Some(UIA_ButtonControlTypeId))?));
        wait(250);
        let rad = uia.find(win, "Corner radius", Some(UIA_SliderControlTypeId))?;
        slide(rad, 0.03 / 0.2, 0.0);
        let sh = uia.find(win, "Shadow", Some(UIA_SliderControlTypeId))?;
        slide(sh, 0.5, 0.0);
        event("bg set");
        wait(300);
        click(center(uia.find(win, "Save with Background", Some(UIA_ButtonControlTypeId))?));
        wait_for_name("With BG", 8000)?;
        event("bg saved");
        deselect(700);
        if until == 3 {
            return Some(());
        }
    }

    // 4. With BG -> Annotate (4th of 7): the caption, in black text across the top.
    event("scene annotate");
    let bg_name = name_containing("With BG")?;
    beaver_drop(wait_for_name("With BG", 1000)?, 3, 7, "annotate")?;
    let win = open_editor(&format!("Annotate · {bg_name}"))?;
    let size = image::image_dimensions(DESK.get().unwrap().join(&bg_name)).ok()?;
    click(center(uia.find(win, "Text", Some(UIA_ButtonControlTypeId))?));
    click(center(uia.find(win, "#111111", None)?));
    click(center(uia.find(win, "M", Some(UIA_ButtonControlTypeId))?));
    focus(win)?;
    click(editor_point(win, size, 0.075, 0.07));
    wait(300);
    // The text box must be there and focused, or the typing would go elsewhere.
    uia.find(win, "", Some(UIA_EditControlTypeId))?;
    if unsafe { GetForegroundWindow() } != win {
        println!("FAIL: editor lost focus before typing");
        return None;
    }
    type_text("How it feels showing someone");
    key(VK_SHIFT, true);
    key(VK_RETURN, true);
    key(VK_RETURN, false);
    key(VK_SHIFT, false);
    type_text("File Beaver for the first time");
    key(VK_RETURN, true);
    key(VK_RETURN, false);
    event("caption");
    wait(600);
    click(center(uia.find(win, "Save", Some(UIA_ButtonControlTypeId))?));
    wait_for_name("Annotated", 8000)?;
    event("annotated");
    deselect(700);
    if until == 4 {
        return Some(());
    }

    // 5. The finished meme -> let go on the beaver, click Compress (1st of 7).
    event("scene compress");
    let meme = wait_for_name("Annotated", 1000)?;
    glide(meme, 550);
    wait(180);
    mouse(MOUSEEVENTF_LEFTDOWN, meme.0, meme.1);
    wait(100);
    let b = beaver_center()?;
    glide(b, 900);
    event("on beaver");
    wait(500);
    mouse(MOUSEEVENTF_LEFTUP, b.0, b.1);
    event("dropped on beaver");
    wait(700);
    click(petal(0, 7)?);
    event("compress");
    wait_for_name("Compressed", 8000)?;
    deselect(800);
    if until == 5 {
        return Some(());
    }

    // 6. Screenshot -> Redact (7th of 7): black out the address, email and phone.
    event("scene redact");
    beaver_drop(icon_center("Screenshot.png")?, 6, 7, "redact")?;
    let win = open_editor("Redact · Screenshot.png")?;
    let size = (1400, 900);
    drag_on(editor_point(win, size, 0.285, 0.575), editor_point(win, size, 0.525, 0.64), 800);
    event("redacted box");
    wait(400);
    click(center(uia.find(win, "Save Redacted", Some(UIA_ButtonControlTypeId))?));
    wait_for_name("Redacted", 8000)?;
    event("redacted");
    deselect(700);

    // 7. A JPG -> Edit (3rd of 7), the other way in: hold the file anywhere and tap Shift.
    // Brighter and punchier (it's a grey image, so saturation wouldn't show).
    event("scene edit");
    let photo = icon_center("images (7).jpg")?;
    shift_drop(photo, (px(560.0), px(470.0)), 2, 7, "edit")?;
    let win = open_editor("Edit · images (7).jpg")?;
    let bri = uia.find(win, "Brightness", Some(UIA_SliderControlTypeId))?;
    slide(bri, 0.5, 0.72);
    let con = uia.find(win, "Contrast", Some(UIA_SliderControlTypeId))?;
    slide(con, 0.5, 0.76);
    event("adjusted");
    wait(400);
    click(center(uia.find(win, "Save", Some(UIA_ButtonControlTypeId))?));
    wait_for_name("Edited", 8000)?;
    event("edited");
    deselect(700);

    // 8. The same JPG -> Metadata (2nd of 7): strips the phone + GPS, quick card.
    event("scene metadata");
    beaver_drop(photo, 1, 7, "metadata")?;
    wait_for_name("Clean", 8000)?;
    event("metadata done");
    deselect(0);
    glide((px(1100.0), px(700.0)), 600);
    wait(1800);
    event("end");
    Some(())
}

fn deselect(then_ms: u64) {
    SELECTED.store(-1, SeqCst);
    repaint();
    wait(then_ms);
}

/// Newest file on the desk whose name contains `part`.
fn name_containing(part: &str) -> Option<String> {
    ICONS.lock().unwrap().iter().rev().map(|ic| ic.path.file_name().unwrap().to_string_lossy().into_owned()).find(|n| n.contains(part))
}

/// Waits for, then returns the centre of, the newest icon whose name contains `part`.
fn wait_for_name(part: &str, timeout_ms: u64) -> Option<(i32, i32)> {
    let t = Instant::now();
    loop {
        {
            let icons = ICONS.lock().unwrap();
            if let Some(i) = icons.iter().rposition(|ic| ic.path.file_name().unwrap().to_string_lossy().contains(part)) {
                return Some(center(slot(i).0));
            }
        }
        if t.elapsed() > Duration::from_millis(timeout_ms) {
            println!("FAIL: no file containing '{part}' appeared");
            return None;
        }
        wait(50);
    }
}

/// Brings an editor to the front (an Alt tap lets this process take the foreground).
fn focus(win: HWND) -> Option<()> {
    for _ in 0..5 {
        key(VK_MENU, true);
        key(VK_MENU, false);
        unsafe {
            let _ = SetForegroundWindow(win);
        }
        wait(150);
        if unsafe { GetForegroundWindow() } == win {
            return Some(());
        }
    }
    println!("FAIL: couldn't bring the editor to the front");
    None
}

/// Types like a person: one character at a time.
fn type_text(text: &str) {
    for ch in text.encode_utf16() {
        for up in [false, true] {
            let flags = if up { KEYEVENTF_UNICODE | KEYEVENTF_KEYUP } else { KEYEVENTF_UNICODE };
            let input = INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wScan: ch, dwFlags: flags, ..Default::default() } } };
            unsafe {
                SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
            }
        }
        wait(38);
    }
}

/// Waits for an editor window, squares it up and gives the image a moment to draw.
fn open_editor(title: &str) -> Option<HWND> {
    let win = visible_window(title, 8000)?;
    square_up(win);
    event(&format!("editor {title}"));
    wait(1100);
    Some(win)
}

fn drag_on(from: (i32, i32), to: (i32, i32), ms: u64) {
    glide(from, 450);
    wait(120);
    mouse(MOUSEEVENTF_LEFTDOWN, from.0, from.1);
    wait(80);
    glide(to, ms);
    wait(80);
    mouse(MOUSEEVENTF_LEFTUP, to.0, to.1);
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let desk = PathBuf::from(&args[1]);
    let assets = PathBuf::from(&args[2]);
    let out = PathBuf::from(&args[3]);
    let record = !args.iter().any(|a| a == "--no-record");
    let clear = !args.iter().any(|a| a == "--no-clear");
    let from: u32 = args.iter().position(|a| a == "--from").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(1);
    let until: u32 = args.iter().position(|a| a == "--until").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(9);
    std::fs::create_dir_all(&out).unwrap();
    // Fresh desk: only the starting files (unless resuming part-way with --from).
    for e in std::fs::read_dir(&desk).unwrap().flatten() {
        if from == 1 && !START.iter().any(|n| e.file_name() == *n) {
            let _ = std::fs::remove_file(e.path());
        }
    }
    DESK.set(desk).ok();
    ASSETS.set(assets.clone()).ok();

    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        OleInitialize(None)?;
        let (sw, sh) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
        let mon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        SCALE.set(dx as f64 / 96.0).ok();
        let wall = image::open(assets.join("wallpaper.png")).unwrap().resize_exact(sw as u32, sh as u32, image::imageops::FilterType::Triangle).to_rgba8();
        WALL.set(wall).ok();
        scan();

        let hinst = GetModuleHandleW(None)?;
        let wc = WNDCLASSW { lpfnWndProc: Some(stage_proc), hInstance: hinst.into(), lpszClassName: w!("FileBeaverDemoStage"), hCursor: LoadCursorW(None, IDC_ARROW)?, ..Default::default() };
        RegisterClassW(&wc);
        // Owned by the desktop window, like the beaver: Show desktop (Win+D) leaves both,
        // and the beaver stays on top of the stage.
        let desktop = FindWindowW(w!("Progman"), None).ok();
        let stage = CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE, w!("FileBeaverDemoStage"), w!("File Beaver demo"), WS_POPUP, 0, 0, sw, sh, desktop.unwrap_or_default(), None, hinst, None)?;
        STAGE.store(stage.0 as isize, SeqCst);
        let beaver = FindWindowW(w!("FileBeaverWidget"), None).ok();
        let beaver_was = beaver.map(rect_of);
        if let Some(b) = beaver {
            let _ = SetWindowPos(b, None, px(BEAVER_AT.0), px(BEAVER_AT.1), 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
            let _ = SetWindowPos(stage, b, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
        let _ = ShowWindow(stage, SW_SHOWNOACTIVATE);
        SetTimer(stage, TIMER_SCAN, 150, None);
        // A leftover notice card would sit in the shot.
        if let Ok(card) = FindWindowW(None, w!("File Beaver progress")) {
            let _ = ShowWindow(card, SW_HIDE);
        }

        if clear {
            // Win+D: everything else off the main screen for the take (pressed again after).
            key(VK_LWIN, true);
            key(VIRTUAL_KEY(b'D' as u16), true);
            key(VIRTUAL_KEY(b'D' as u16), false);
            key(VK_LWIN, false);
        }

        let mut ffmpeg = None;
        if record {
            wait(1200);
            ffmpeg = std::process::Command::new("ffmpeg")
                .args(["-hide_banner", "-loglevel", "warning", "-y", "-f", "lavfi", "-i", "ddagrab=output_idx=0:framerate=30:draw_mouse=1,hwmap=derive_device=qsv,format=qsv", "-c:v", "h264_qsv", "-global_quality", "18"])
                .arg(out.join("raw.mkv"))
                .stdin(std::process::Stdio::piped())
                .spawn()
                .ok();
        }

        let done = std::sync::Arc::new(AtomicBool::new(false));
        let d2 = done.clone();
        let script = std::thread::spawn(move || {
            let ok = run_script(from, until).is_some();
            d2.store(true, SeqCst);
            ok
        });
        let mut m = MSG::default();
        while !done.load(SeqCst) {
            while PeekMessageW(&mut m, None, 0, 0, PM_REMOVE).into() {
                let _ = TranslateMessage(&m);
                DispatchMessageW(&m);
            }
            sleep(Duration::from_millis(5));
        }
        let ok = script.join().unwrap_or(false);
        if let Some(mut f) = ffmpeg {
            use std::io::Write;
            if let Some(stdin) = f.stdin.as_mut() {
                let _ = stdin.write_all(b"q");
            }
            let _ = f.wait();
        }
        let log: String = EVENTS.lock().unwrap().iter().map(|(t, e)| format!("{t:.3}\t{e}\n")).collect();
        std::fs::write(out.join("events.txt"), log).unwrap();

        let _ = DestroyWindow(stage);
        if let (Some(b), Some(r)) = (beaver, beaver_was) {
            let _ = SetWindowPos(b, None, r.left, r.top, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
        if clear {
            key(VK_LWIN, true);
            key(VIRTUAL_KEY(b'D' as u16), true);
            key(VIRTUAL_KEY(b'D' as u16), false);
            key(VK_LWIN, false);
        }
        println!("{}", if ok { "DEMO OK" } else { "DEMO FAILED" });
    }
    Ok(())
}
