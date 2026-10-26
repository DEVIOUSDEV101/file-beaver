//! Phase 1 spike: detect "file being dragged + Shift held" anywhere on Windows,
//! show a radial overlay under the cursor, and receive the dropped file paths
//! as a real OLE drop target. The source file is never moved (we force COPY).

use std::f64::consts::PI;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, Ordering::SeqCst};

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::*;
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_SHIFT};
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::*;

const SIZE: i32 = 320;
const DEAD_ZONE: f64 = 45.0;
const LABELS: [&str; 8] = [
    "Convert", "Compress", "Metadata", "Edit", "Annotate", "Add BG", "Crop", "Redact",
];

const WM_APP_SHOW: u32 = WM_APP + 1;
const WM_APP_HIDE: u32 = WM_APP + 2;
const TIMER_VERIFY: usize = 1;
const TIMER_HIDE: usize = 2;

static OVERLAY: AtomicIsize = AtomicIsize::new(0);
static BUTTON_DOWN: AtomicBool = AtomicBool::new(false);
static SHOWN: AtomicBool = AtomicBool::new(false);
/// Set once the ring has been dismissed during this press, so it doesn't re-open mid-drag.
static DISMISSED: AtomicBool = AtomicBool::new(false);
static FILE_DRAG: AtomicBool = AtomicBool::new(false);
static DOWN_X: AtomicI32 = AtomicI32::new(0);
static DOWN_Y: AtomicI32 = AtomicI32::new(0);
static WIN_X: AtomicI32 = AtomicI32::new(0);
static WIN_Y: AtomicI32 = AtomicI32::new(0);
static HOVER: AtomicI32 = AtomicI32::new(-1);

fn log(msg: &str) {
    println!("{msg}");
    let path = std::env::current_exe().unwrap().with_file_name("shiftdrop-spike.log");
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{msg}");
    }
}

fn overlay() -> HWND {
    HWND(OVERLAY.load(SeqCst) as *mut _)
}

fn shift_down() -> bool {
    unsafe { (GetAsyncKeyState(VK_SHIFT.0 as i32) as u16 & 0x8000) != 0 }
}

/// Which wedge a screen point is over: 0..8 clockwise from the top, -1 = centre/outside.
fn wedge_at(x: i32, y: i32) -> i32 {
    let cx = (WIN_X.load(SeqCst) + SIZE / 2) as f64;
    let cy = (WIN_Y.load(SeqCst) + SIZE / 2) as f64;
    let (dx, dy) = (x as f64 - cx, y as f64 - cy);
    let r = (dx * dx + dy * dy).sqrt();
    if r < DEAD_ZONE || r > SIZE as f64 / 2.0 {
        return -1;
    }
    // angle 0 = straight up, increasing clockwise
    let mut a = dx.atan2(-dy);
    if a < 0.0 {
        a += 2.0 * PI;
    }
    (((a + PI / 8.0) / (PI / 4.0)) as i32) % 8
}

fn set_hover(w: i32) {
    if HOVER.swap(w, SeqCst) != w {
        unsafe {
            let _ = InvalidateRect(overlay(), None, true);
        }
    }
}

// ---------- global mouse hook ----------

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let info = &*(lparam.0 as *const MSLLHOOKSTRUCT);
        let (x, y) = (info.pt.x, info.pt.y);
        match wparam.0 as u32 {
            WM_LBUTTONDOWN => {
                BUTTON_DOWN.store(true, SeqCst);
                DISMISSED.store(false, SeqCst);
                DOWN_X.store(x, SeqCst);
                DOWN_Y.store(y, SeqCst);
            }
            WM_LBUTTONUP => {
                BUTTON_DOWN.store(false, SeqCst);
                if SHOWN.load(SeqCst) {
                    // give OLE time to deliver Drop before we hide
                    let _ = PostMessageW(overlay(), WM_APP_HIDE, WPARAM(250), LPARAM(0));
                }
            }
            WM_MOUSEMOVE if BUTTON_DOWN.load(SeqCst) => {
                let moved = (x - DOWN_X.load(SeqCst)).abs() > GetSystemMetrics(SM_CXDRAG)
                    || (y - DOWN_Y.load(SeqCst)).abs() > GetSystemMetrics(SM_CYDRAG);
                let shift = shift_down();
                if moved && shift && !SHOWN.load(SeqCst) && !DISMISSED.load(SeqCst) {
                    SHOWN.store(true, SeqCst);
                    let _ = PostMessageW(overlay(), WM_APP_SHOW, WPARAM(x as usize), LPARAM(y as isize));
                } else if SHOWN.load(SeqCst) && !shift {
                    SHOWN.store(false, SeqCst);
                    let _ = PostMessageW(overlay(), WM_APP_HIDE, WPARAM(0), LPARAM(0));
                }
            }
            _ => {}
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

// ---------- OLE drop target ----------

#[implement(IDropTarget)]
struct DropTarget;

fn force_copy(effect: *mut DROPEFFECT) {
    unsafe {
        if !effect.is_null() {
            // Shift normally means MOVE in Explorer; we only ever accept COPY.
            *effect = if (*effect).0 & DROPEFFECT_COPY.0 != 0 { DROPEFFECT_COPY } else { DROPEFFECT_NONE };
        }
    }
}

fn read_paths(obj: &IDataObject) -> Option<Vec<String>> {
    unsafe {
        let fmt = FORMATETC {
            cfFormat: CF_HDROP.0,
            ptd: std::ptr::null_mut(),
            dwAspect: DVASPECT_CONTENT.0,
            lindex: -1,
            tymed: TYMED_HGLOBAL.0 as u32,
        };
        let mut medium = obj.GetData(&fmt).ok()?;
        let hdrop = HDROP(medium.u.hGlobal.0);
        let count = DragQueryFileW(hdrop, u32::MAX, None);
        let mut paths = Vec::new();
        for i in 0..count {
            let mut buf = vec![0u16; 32768];
            let n = DragQueryFileW(hdrop, i, Some(&mut buf)) as usize;
            paths.push(String::from_utf16_lossy(&buf[..n]));
        }
        ReleaseStgMedium(&mut medium);
        Some(paths)
    }
}

impl IDropTarget_Impl for DropTarget_Impl {
    fn DragEnter(&self, obj: Option<&IDataObject>, _k: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let has_files = obj
            .map(|o| unsafe {
                let fmt = FORMATETC { cfFormat: CF_HDROP.0, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 };
                o.QueryGetData(&fmt) == S_OK
            })
            .unwrap_or(false);
        if !has_files {
            log("DragEnter: not a file drag -> hiding");
            unsafe { *effect = DROPEFFECT_NONE; let _ = PostMessageW(overlay(), WM_APP_HIDE, WPARAM(0), LPARAM(0)); }
            return Ok(());
        }
        FILE_DRAG.store(true, SeqCst);
        log("DragEnter: file drag confirmed");
        set_hover(wedge_at(pt.x, pt.y));
        force_copy(effect);
        Ok(())
    }

    fn DragOver(&self, _k: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let w = wedge_at(pt.x, pt.y);
        set_hover(w);
        if w < 0 { unsafe { *effect = DROPEFFECT_NONE } } else { force_copy(effect) }
        Ok(())
    }

    fn DragLeave(&self) -> Result<()> {
        set_hover(-1);
        Ok(())
    }

    fn Drop(&self, obj: Option<&IDataObject>, _k: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let w = wedge_at(pt.x, pt.y);
        if w < 0 {
            log("DROP cancelled (centre or outside ring)");
            unsafe { *effect = DROPEFFECT_NONE }
        } else {
            let paths = obj.and_then(read_paths).unwrap_or_default();
            log(&format!("DROP action={} files={:?}", LABELS[w as usize], paths));
            force_copy(effect);
        }
        unsafe { let _ = PostMessageW(overlay(), WM_APP_HIDE, WPARAM(0), LPARAM(0)); }
        Ok(())
    }
}

// ---------- overlay window ----------

fn hide() {
    unsafe {
        let _ = ShowWindow(overlay(), SW_HIDE);
    }
    SHOWN.store(false, SeqCst);
    DISMISSED.store(true, SeqCst);
    FILE_DRAG.store(false, SeqCst);
    HOVER.store(-1, SeqCst);
}

unsafe fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    let c = SIZE / 2;
    let r = c - 2;
    let base = CreateSolidBrush(COLORREF(0x004a8cf0)); // BGR: warm orange
    let hot = CreateSolidBrush(COLORREF(0x002050c0));
    let pen = CreatePen(PS_SOLID, 2, COLORREF(0x00ffffff));
    SelectObject(hdc, pen);
    let hover = HOVER.load(SeqCst);
    for i in 0..8 {
        SelectObject(hdc, if i == hover { hot } else { base });
        // wedge i spans [i*45-22.5, i*45+22.5] degrees clockwise from top
        let a1 = (i as f64 * 45.0 - 22.5) * PI / 180.0;
        let a2 = (i as f64 * 45.0 + 22.5) * PI / 180.0;
        let p = |a: f64| ((c as f64 + r as f64 * a.sin()) as i32, (c as f64 - r as f64 * a.cos()) as i32);
        let (x1, y1) = p(a2); // Pie draws counter-clockwise from start to end
        let (x2, y2) = p(a1);
        let _ = Pie(hdc, 2, 2, SIZE - 2, SIZE - 2, x1, y1, x2, y2);
    }
    SelectObject(hdc, base);
    let _ = Ellipse(hdc, c - DEAD_ZONE as i32, c - DEAD_ZONE as i32, c + DEAD_ZONE as i32, c + DEAD_ZONE as i32);
    SetBkMode(hdc, TRANSPARENT);
    SetTextColor(hdc, COLORREF(0x00ffffff));
    for (i, label) in LABELS.iter().enumerate() {
        let a = i as f64 * 45.0 * PI / 180.0;
        let lr = (r as f64 + DEAD_ZONE) / 2.0;
        let (x, y) = ((c as f64 + lr * a.sin()) as i32, (c as f64 - lr * a.cos()) as i32);
        let mut text: Vec<u16> = label.encode_utf16().collect();
        let mut rc = RECT { left: x - 40, top: y - 10, right: x + 40, bottom: y + 10 };
        DrawTextW(hdc, &mut text, &mut rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    }
    let _ = DeleteObject(base);
    let _ = DeleteObject(hot);
    let _ = DeleteObject(pen);
    let _ = EndPaint(hwnd, &ps);
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_APP_SHOW => {
            let (x, y) = (wparam.0 as i32 - SIZE / 2, lparam.0 as i32 - SIZE / 2);
            WIN_X.store(x, SeqCst);
            WIN_Y.store(y, SeqCst);
            HOVER.store(-1, SeqCst);
            FILE_DRAG.store(false, SeqCst);
            let _ = SetWindowPos(hwnd, HWND_TOPMOST, x, y, SIZE, SIZE, SWP_NOACTIVATE | SWP_SHOWWINDOW);
            let _ = InvalidateRect(hwnd, None, true);
            // If Windows doesn't confirm a real file drag soon, this wasn't one.
            SetTimer(hwnd, TIMER_VERIFY, 600, None);
            log(&format!("ring shown at ({}, {})", wparam.0 as i32, lparam.0 as i32));
            LRESULT(0)
        }
        WM_APP_HIDE => {
            if wparam.0 == 0 { hide() } else { SetTimer(hwnd, TIMER_HIDE, wparam.0 as u32, None); }
            LRESULT(0)
        }
        WM_TIMER => {
            let _ = KillTimer(hwnd, wparam.0);
            if wparam.0 == TIMER_VERIFY && !FILE_DRAG.load(SeqCst) {
                log("no OLE file drag arrived -> hiding (text/window drag)");
                hide();
            } else if wparam.0 == TIMER_HIDE {
                hide();
            }
            LRESULT(0)
        }
        WM_PAINT => { paint(hwnd); LRESULT(0) }
        WM_DESTROY => { PostQuitMessage(0); LRESULT(0) }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn main() -> Result<()> {
    unsafe {
        // Hook coordinates are physical pixels; window placement must match on scaled displays.
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
        OleInitialize(None)?;
        let hinst = GetModuleHandleW(None)?;
        let class = w!("ShiftDropSpikeOverlay");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class, w!("ShiftDrop"), WS_POPUP,
            0, 0, SIZE, SIZE, None, None, hinst, None,
        )?;
        OVERLAY.store(hwnd.0 as isize, SeqCst);
        SetLayeredWindowAttributes(hwnd, COLORREF(0), 230, LWA_ALPHA)?;
        let _ = SetWindowRgn(hwnd, CreateEllipticRgn(0, 0, SIZE, SIZE), false);

        let target: IDropTarget = DropTarget.into();
        RegisterDragDrop(hwnd, &target)?;
        let _hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), hinst, 0)?;

        log("ShiftDrop spike running. Drag a file and hold Shift. Ctrl+C to quit.");
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).into() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        Ok(())
    }
}
