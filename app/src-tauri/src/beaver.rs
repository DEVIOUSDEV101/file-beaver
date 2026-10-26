//! The floating beaver: a small always-on-top widget you can drag around and drop files
//! onto. Dropping on it opens the ring centred on the beaver, no Shift needed. It's a
//! per-pixel-alpha window, so only the beaver's own pixels take clicks and drops; around
//! it, everything passes through to the windows underneath.
//!
//! It sits on the desktop like a gadget, behind every other window, and rises on top only
//! while a file is being dragged (the trigger thread calls [`raise`]), so it never covers
//! your work but is there to drop onto.
//!
//! Lives on the trigger thread (which has OLE set up and a message loop). The app talks
//! to it with [`set_visible`]; it reports back through `Hooks::on_beaver`.

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, Ordering::SeqCst};
use std::sync::OnceLock;

use image::imageops::FilterType;
use image::RgbaImage;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Ole::*;
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::*;

/// Widget edge in logical px, and its gap from the screen corner by default.
const SIZE_LOGICAL: f64 = 76.0;
const MARGIN_LOGICAL: f64 = 28.0;
/// The beaver is drawn a little smaller at rest and full size while a file hovers over it.
const REST: f32 = 0.88;
const PERKED: f32 = 1.0;

const WM_APP_VISIBLE: u32 = WM_APP + 20;
const WM_APP_RAISE: u32 = WM_APP + 21;
const ID_SETTINGS: usize = 1;
const ID_HIDE: usize = 2;

/// What the beaver tells the app.
pub enum BeaverEvent {
    /// Dragged to a new place (top-left, physical px).
    Moved(i32, i32),
    /// Hidden from its right-click menu.
    Hidden,
    OpenSettings,
}

static HWND_RAW: AtomicIsize = AtomicIsize::new(0);
static WANT_VISIBLE: AtomicBool = AtomicBool::new(true);
static RAISED: AtomicBool = AtomicBool::new(false);
static SIZE_PX: AtomicI32 = AtomicI32::new(76);
static IMAGE: OnceLock<RgbaImage> = OnceLock::new();

fn hwnd() -> HWND {
    HWND(HWND_RAW.load(SeqCst) as *mut _)
}

fn emit(e: BeaverEvent) {
    (crate::trigger::hooks().on_beaver)(e)
}

/// Shows or hides the beaver. Safe from any thread, before or after it exists.
pub fn set_visible(on: bool) {
    WANT_VISIBLE.store(on, SeqCst);
    if HWND_RAW.load(SeqCst) != 0 {
        unsafe {
            let _ = PostMessageW(hwnd(), WM_APP_VISIBLE, WPARAM(on as usize), LPARAM(0));
        }
    }
}

/// Brings the beaver above every window (a file drag is under way) or sends it back down
/// to the desktop. Safe from any thread.
pub(crate) fn raise(on: bool) {
    if RAISED.swap(on, SeqCst) != on && HWND_RAW.load(SeqCst) != 0 {
        unsafe {
            let _ = PostMessageW(hwnd(), WM_APP_RAISE, WPARAM(on as usize), LPARAM(0));
        }
    }
}

/// Down to desktop level: owned by the desktop window (Windows keeps an owned window just
/// above its owner, and Show desktop leaves it alone), below every normal window.
unsafe fn sink(h: HWND) {
    unsafe {
        let _ = SetWindowPos(h, HWND_NOTOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        if let Ok(desktop) = FindWindowW(w!("Progman"), None) {
            SetWindowLongPtrW(h, GWLP_HWNDPARENT, desktop.0 as isize);
        }
        let _ = SetWindowPos(h, HWND_BOTTOM, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

/// The beaver's screen rect (left, top, right, bottom) while it's showing, so cards can
/// keep clear of it. Safe from any thread.
pub(crate) fn visible_rect() -> Option<(i32, i32, i32, i32)> {
    if HWND_RAW.load(SeqCst) == 0 || !unsafe { IsWindowVisible(hwnd()) }.as_bool() {
        return None;
    }
    let mut r = RECT::default();
    unsafe { GetWindowRect(hwnd(), &mut r) }.ok()?;
    Some((r.left, r.top, r.right, r.bottom))
}

fn image() -> &'static RgbaImage {
    IMAGE.get_or_init(|| {
        image::load_from_memory_with_format(include_bytes!("../icons/beaver-widget.png"), image::ImageFormat::Png)
            .expect("bundled beaver image")
            .to_rgba8()
    })
}

/// Widget size in physical px for the monitor containing `pt`.
fn size_at(pt: POINT) -> i32 {
    unsafe {
        let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        (SIZE_LOGICAL * dx as f64 / 96.0).round() as i32
    }
}

/// Draws the beaver at `scale` of the window into the layered window.
unsafe fn paint(scale: f32) {
    unsafe {
        let size = SIZE_PX.load(SeqCst);
        let inner = ((size as f32 * scale).round() as u32).max(1);
        let small = image::imageops::resize(image(), inner, inner, FilterType::Lanczos3);
        let off = (size as u32 - inner) / 2;
        // Premultiplied BGRA, top-down, as UpdateLayeredWindow expects.
        let mut px = vec![0u8; (size * size * 4) as usize];
        for (x, y, p) in small.enumerate_pixels() {
            let a = p[3] as u32;
            let i = (((y + off) * size as u32 + x + off) * 4) as usize;
            px[i] = (p[2] as u32 * a / 255) as u8;
            px[i + 1] = (p[1] as u32 * a / 255) as u8;
            px[i + 2] = (p[0] as u32 * a / 255) as u8;
            px[i + 3] = a as u8;
        }
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(screen);
        let mut bi = BITMAPINFO::default();
        bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = size;
        bi.bmiHeader.biHeight = -size;
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        if let Ok(dib) = CreateDIBSection(mem, &bi, DIB_RGB_COLORS, &mut bits, None, 0) {
            std::ptr::copy_nonoverlapping(px.as_ptr(), bits as *mut u8, px.len());
            let old = SelectObject(mem, dib);
            let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
            let sz = SIZE { cx: size, cy: size };
            let src = POINT { x: 0, y: 0 };
            let _ = UpdateLayeredWindow(hwnd(), screen, None, Some(&sz), mem, Some(&src), COLORREF(0), Some(&blend), ULW_ALPHA);
            SelectObject(mem, old);
            let _ = DeleteObject(dib);
        }
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
    }
}

/// Re-fits the widget to the DPI of the monitor it's now on.
unsafe fn refit() {
    unsafe {
        let mut r = RECT::default();
        let _ = GetWindowRect(hwnd(), &mut r);
        let size = size_at(POINT { x: (r.left + r.right) / 2, y: (r.top + r.bottom) / 2 });
        if size != SIZE_PX.swap(size, SeqCst) {
            let _ = SetWindowPos(hwnd(), None, 0, 0, size, size, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
            paint(REST);
        }
    }
}

unsafe fn context_menu(x: i32, y: i32) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let _ = AppendMenuW(menu, MF_STRING, ID_SETTINGS, w!("Settings…"));
        let _ = AppendMenuW(menu, MF_STRING, ID_HIDE, w!("Hide beaver"));
        // Required so the menu closes when you click elsewhere.
        let _ = SetForegroundWindow(hwnd());
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, x, y, 0, hwnd(), None).0 as usize;
        let _ = DestroyMenu(menu);
        match cmd {
            ID_SETTINGS => emit(BeaverEvent::OpenSettings),
            ID_HIDE => {
                set_visible(false);
                emit(BeaverEvent::Hidden);
            }
            _ => {}
        }
    }
}

unsafe extern "system" fn wndproc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::crash::guarded("beaver window", || unsafe { wndproc_body(h, msg, wp, lp) }).unwrap_or(LRESULT(0))
}

unsafe fn wndproc_body(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            // Grab anywhere on the beaver to move it.
            WM_NCHITTEST => LRESULT(HTCAPTION as isize),
            WM_EXITSIZEMOVE => {
                refit();
                let mut r = RECT::default();
                let _ = GetWindowRect(h, &mut r);
                emit(BeaverEvent::Moved(r.left, r.top));
                LRESULT(0)
            }
            WM_NCLBUTTONDBLCLK => {
                emit(BeaverEvent::OpenSettings);
                LRESULT(0)
            }
            // With HTCAPTION, right-clicks arrive as non-client clicks; show our menu
            // instead of the system one.
            WM_NCRBUTTONUP | WM_CONTEXTMENU => {
                let (x, y) = ((lp.0 & 0xFFFF) as i16 as i32, ((lp.0 >> 16) & 0xFFFF) as i16 as i32);
                context_menu(x, y);
                LRESULT(0)
            }
            WM_NCRBUTTONDOWN => LRESULT(0),
            WM_APP_RAISE => {
                if wp.0 != 0 {
                    SetWindowLongPtrW(h, GWLP_HWNDPARENT, 0);
                    let _ = SetWindowPos(h, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                } else {
                    sink(h);
                }
                LRESULT(0)
            }
            WM_APP_VISIBLE => {
                let _ = ShowWindow(h, if wp.0 != 0 { SW_SHOWNOACTIVATE } else { SW_HIDE });
                LRESULT(0)
            }
            _ => DefWindowProcW(h, msg, wp, lp),
        }
    }
}

#[implement(IDropTarget)]
struct BeaverDrop {
    perked: Cell<bool>,
}

impl BeaverDrop_Impl {
    fn perk(&self, on: bool) {
        if self.perked.replace(on) != on {
            unsafe { paint(if on { PERKED } else { REST }) };
        }
    }
}

/// The beaver itself never accepts the drop: it opens the ring, whose drop target sits
/// on top and takes the drag from there.
impl IDropTarget_Impl for BeaverDrop_Impl {
    fn DragEnter(&self, obj: Option<&IDataObject>, _k: MODIFIERKEYS_FLAGS, _pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        crate::crash::guarded("beaver drag enter", || {
            let files = obj.map(|o| unsafe { o.QueryGetData(&crate::trigger::hdrop_format()) == S_OK }).unwrap_or(false);
            log::debug!("beaver drag enter, files={files}");
            if files {
                self.perk(true);
                let mut r = RECT::default();
                unsafe {
                    let _ = GetWindowRect(hwnd(), &mut r);
                }
                crate::trigger::open_ring_at((r.left + r.right) / 2, (r.top + r.bottom) / 2);
            }
        });
        unsafe { *effect = DROPEFFECT_NONE };
        Ok(())
    }

    fn DragOver(&self, _k: MODIFIERKEYS_FLAGS, _pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe { *effect = DROPEFFECT_NONE };
        Ok(())
    }

    fn DragLeave(&self) -> Result<()> {
        crate::crash::guarded("beaver drag leave", || self.perk(false));
        Ok(())
    }

    fn Drop(&self, _obj: Option<&IDataObject>, _k: MODIFIERKEYS_FLAGS, _pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        crate::crash::guarded("beaver drop", || self.perk(false));
        unsafe { *effect = DROPEFFECT_NONE };
        Ok(())
    }
}

/// Creates the beaver on the calling (trigger) thread. `start` is where it was last left.
pub(crate) unsafe fn create(hinst: HMODULE, start: Option<(i32, i32)>) -> Result<()> {
    unsafe {
        let class = w!("FileBeaverWidget");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_HAND)?,
            style: CS_DBLCLKS,
            ..Default::default()
        };
        RegisterClassW(&wc);

        // Last position, if it's still on a connected monitor; else the main screen's
        // bottom-right corner.
        let on_screen = |(x, y): (i32, i32)| !MonitorFromPoint(POINT { x: x + 10, y: y + 10 }, MONITOR_DEFAULTTONULL).is_invalid();
        let (x, y, size) = match start.filter(|p| on_screen(*p)) {
            Some((x, y)) => (x, y, size_at(POINT { x, y })),
            None => {
                let mut work = RECT::default();
                let _ = SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut work as *mut _ as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
                let size = size_at(POINT { x: work.right - 1, y: work.bottom - 1 });
                let margin = (MARGIN_LOGICAL * size as f64 / SIZE_LOGICAL) as i32;
                (work.right - size - margin, work.bottom - size - margin, size)
            }
        };
        SIZE_PX.store(size, SeqCst);
        let h = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class,
            w!("File Beaver"),
            WS_POPUP,
            x,
            y,
            size,
            size,
            None,
            None,
            hinst,
            None,
        )?;
        HWND_RAW.store(h.0 as isize, SeqCst);
        sink(h);
        paint(REST);
        let target: IDropTarget = BeaverDrop { perked: Cell::new(false) }.into();
        RegisterDragDrop(h, &target)?;
        if WANT_VISIBLE.load(SeqCst) {
            let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
        }
        log::info!("beaver ready at ({x}, {y}), {size}px");
        Ok(())
    }
}
