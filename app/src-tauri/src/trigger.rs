//! Global trigger and drop handling for the ring.
//!
//! - Low-level mouse + keyboard hooks spot a drag in progress and the trigger key being
//!   tapped (Shift by default). First tap opens the ring under the cursor; further taps flip
//!   between the Convert and Tools rings. The ring stays open after the key is released.
//! - Two windows make up the ring: the transparent web window "ring" draws it, and an
//!   invisible native window exactly on top is the OLE drop target. Windows hands the drag
//!   to the native window, which hit-tests here and tells the web window what to highlight.
//! - Drops are always accepted as COPY, so the source file is never moved.

use std::cell::RefCell;
use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, AtomicU8, AtomicUsize, Ordering::SeqCst};
use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::*;
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::capture;
use shiftdrop_core::geometry::wedge_index;
use shiftdrop_core::formats::Caps;
use shiftdrop_core::menu::{self, Menu};

// Geometry in logical pixels. Must match src/Ring.tsx (WIN, R, HUB).
/// Window edge; a little larger than the ring so its shadow isn't clipped.
const WIN_LOGICAL: f64 = 384.0;
const RING_R_LOGICAL: f64 = 180.0;
/// Petals shown before Windows has told us which files are being dragged.
const PLACEHOLDER_SEGMENTS: usize = 8;

const WM_APP_SHOW: u32 = WM_APP + 1;
const WM_APP_HIDE: u32 = WM_APP + 2;
const WM_APP_FLIP: u32 = WM_APP + 3;
const TIMER_VERIFY: usize = 1;
const TIMER_HIDE: usize = 2;
const VERIFY_MS: u32 = 800;

/// Which key combination opens the ring.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum TriggerKey {
    Shift = 0,
    CtrlShift = 1,
    Alt = 2,
}

/// Everything the web ring needs to know, sent as the "ring" event.
#[derive(Serialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RingEvent {
    Show { backdrop: Option<String> },
    Menu { menu: Menu, page: usize },
    Page { page: usize },
    Hover { index: i32 },
    Thumb { src: String },
    Chosen { index: i32 },
    Hide,
}

pub struct Hooks {
    /// HWND of the web window that draws the ring.
    pub ring_hwnd: isize,
    /// Which HEIC/AVIF codecs this PC has; unavailable formats are greyed out.
    pub caps: Caps,
    pub on_ring: Box<dyn Fn(RingEvent) + Send + Sync>,
    /// (segment id such as "tool:crop", dropped paths)
    pub on_drop: Box<dyn Fn(String, Vec<String>) + Send + Sync>,
}

static HOOKS: OnceLock<Hooks> = OnceLock::new();
static ENABLED: AtomicBool = AtomicBool::new(true);
static KEY: AtomicU8 = AtomicU8::new(TriggerKey::Shift as u8);

static OVERLAY: AtomicIsize = AtomicIsize::new(0);
static BUTTON_DOWN: AtomicBool = AtomicBool::new(false);
/// Button held and moved past the system drag threshold.
static DRAGGING: AtomicBool = AtomicBool::new(false);
/// Ring visible or about to be.
static SHOWN: AtomicBool = AtomicBool::new(false);
/// Windows confirmed a file drag over the ring.
static FILE_DRAG: AtomicBool = AtomicBool::new(false);
static DOWN_X: AtomicI32 = AtomicI32::new(0);
static DOWN_Y: AtomicI32 = AtomicI32::new(0);
static CENTER_X: AtomicI32 = AtomicI32::new(0);
static CENTER_Y: AtomicI32 = AtomicI32::new(0);
/// Ring radius in physical pixels for the monitor it is on.
static RADIUS: AtomicI32 = AtomicI32::new(180);
static HOVER: AtomicI32 = AtomicI32::new(-1);
static PAGE: AtomicUsize = AtomicUsize::new(0);
static MENU: Mutex<Option<Menu>> = Mutex::new(None);

static SHIFT_HELD: AtomicBool = AtomicBool::new(false);
static CTRL_HELD: AtomicBool = AtomicBool::new(false);
static ALT_HELD: AtomicBool = AtomicBool::new(false);

pub fn set_enabled(on: bool) {
    ENABLED.store(on, SeqCst);
    if !on && SHOWN.load(SeqCst) {
        post(WM_APP_HIDE, 0, 0);
    }
}

pub fn set_key(key: TriggerKey) {
    KEY.store(key as u8, SeqCst);
}

/// Makes an overlay web window (ring, progress card) a non-activating tool window:
/// no focus steal, no taskbar button, no Alt-Tab entry.
pub fn prepare_overlay_window(hwnd: isize) {
    unsafe {
        let h = HWND(hwnd as *mut _);
        let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
        SetWindowLongPtrW(h, GWL_EXSTYLE, ex | (WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST).0 as isize);
    }
}

/// Shows a window on top without activating it, so the user's app keeps focus.
pub fn show_no_activate(hwnd: isize) {
    unsafe {
        let h = HWND(hwnd as *mut _);
        let _ = SetWindowPos(h, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW);
    }
}

/// Current cursor position in physical pixels.
pub fn cursor_pos() -> (i32, i32) {
    let mut pt = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut pt);
    }
    (pt.x, pt.y)
}

/// Starts the hooks + drop window on a dedicated thread. Call once.
pub fn start(hooks: Hooks) {
    if HOOKS.set(hooks).is_err() {
        return;
    }
    std::thread::Builder::new()
        .name("shiftdrop-trigger".into())
        .spawn(|| {
            if let Err(e) = unsafe { run() } {
                log::error!("trigger thread failed: {e:?}");
            }
        })
        .expect("spawn trigger thread");
}

fn hooks() -> &'static Hooks {
    HOOKS.get().expect("trigger started")
}

fn emit(e: RingEvent) {
    (hooks().on_ring)(e)
}

fn overlay() -> HWND {
    HWND(OVERLAY.load(SeqCst) as *mut _)
}

fn ring_window() -> HWND {
    HWND(hooks().ring_hwnd as *mut _)
}

fn post(msg: u32, w: usize, l: isize) {
    unsafe {
        let _ = PostMessageW(overlay(), msg, WPARAM(w), LPARAM(l));
    }
}

fn trigger_held() -> bool {
    let (s, c, a) = (SHIFT_HELD.load(SeqCst), CTRL_HELD.load(SeqCst), ALT_HELD.load(SeqCst));
    match KEY.load(SeqCst) {
        1 => c && s,
        2 => a,
        _ => s,
    }
}

// ---------- geometry ----------

/// The menu lock, usable even if a previous holder panicked.
fn menu_lock() -> std::sync::MutexGuard<'static, Option<Menu>> {
    MENU.lock().unwrap_or_else(|e| e.into_inner())
}

fn segment_count() -> usize {
    match &*menu_lock() {
        Some(m) => m.pages[PAGE.load(SeqCst)].len(),
        None => PLACEHOLDER_SEGMENTS,
    }
}

fn hit(x: i32, y: i32) -> i32 {
    let dx = (x - CENTER_X.load(SeqCst)) as f64;
    let dy = (y - CENTER_Y.load(SeqCst)) as f64;
    wedge_index(dx, dy, RADIUS.load(SeqCst) as f64, segment_count())
}

/// The enabled segment at `index` on the current page: (id, label).
fn segment(index: i32) -> Option<(String, String)> {
    if index < 0 {
        return None;
    }
    let menu = menu_lock();
    let s = menu.as_ref()?.pages[PAGE.load(SeqCst)].get(index as usize)?;
    s.enabled.then(|| (s.id.clone(), s.label.clone()))
}

fn set_hover(w: i32) {
    if HOVER.swap(w, SeqCst) != w {
        emit(RingEvent::Hover { index: w });
    }
}

// ---------- hooks ----------

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    crate::crash::guarded("mouse hook", || unsafe { mouse_hook_body(code, wparam, lparam) });
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe fn mouse_hook_body(code: i32, wparam: WPARAM, lparam: LPARAM) {
    if code >= 0 && ENABLED.load(SeqCst) {
        let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        let (x, y) = (info.pt.x, info.pt.y);
        match wparam.0 as u32 {
            WM_LBUTTONDOWN => {
                BUTTON_DOWN.store(true, SeqCst);
                DRAGGING.store(false, SeqCst);
                DOWN_X.store(x, SeqCst);
                DOWN_Y.store(y, SeqCst);
                // Re-read the real modifier state each press: if Windows ever skipped a
                // key-up for this hook, Shift would otherwise stay "held" and taps be ignored.
                let held = |vk: VIRTUAL_KEY| unsafe { (GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000) != 0 };
                SHIFT_HELD.store(held(VK_SHIFT), SeqCst);
                CTRL_HELD.store(held(VK_CONTROL), SeqCst);
                ALT_HELD.store(held(VK_MENU), SeqCst);
            }
            WM_LBUTTONUP => {
                BUTTON_DOWN.store(false, SeqCst);
                DRAGGING.store(false, SeqCst);
                if SHOWN.load(SeqCst) {
                    // give OLE time to deliver Drop before we hide
                    post(WM_APP_HIDE, 250, 0);
                }
            }
            WM_MOUSEMOVE if BUTTON_DOWN.load(SeqCst) && !DRAGGING.load(SeqCst) => {
                let crossed = unsafe {
                    (x - DOWN_X.load(SeqCst)).abs() > GetSystemMetrics(SM_CXDRAG)
                        || (y - DOWN_Y.load(SeqCst)).abs() > GetSystemMetrics(SM_CYDRAG)
                };
                if crossed {
                    DRAGGING.store(true, SeqCst);
                    // Key already held when the drag began counts as a tap.
                    if trigger_held() && !SHOWN.swap(true, SeqCst) {
                        post(WM_APP_SHOW, x as usize, y as isize);
                    }
                }
            }
            _ => {}
        }
    }
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    crate::crash::guarded("keyboard hook", || unsafe { keyboard_hook_body(code, wparam, lparam) });
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe fn keyboard_hook_body(code: i32, wparam: WPARAM, lparam: LPARAM) {
    if code >= 0 {
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let pressed = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
        let before = trigger_held();
        let vk = VIRTUAL_KEY(info.vkCode as u16);
        match vk {
            VK_SHIFT | VK_LSHIFT | VK_RSHIFT => SHIFT_HELD.store(pressed, SeqCst),
            VK_CONTROL | VK_LCONTROL | VK_RCONTROL => CTRL_HELD.store(pressed, SeqCst),
            VK_MENU | VK_LMENU | VK_RMENU => ALT_HELD.store(pressed, SeqCst),
            _ => {}
        }
        // Key-repeat keeps `before` true, so only a fresh press counts.
        if ENABLED.load(SeqCst) && !before && trigger_held() && DRAGGING.load(SeqCst) {
            if SHOWN.swap(true, SeqCst) {
                post(WM_APP_FLIP, 0, 0);
            } else {
                let mut pt = POINT::default();
                let _ = unsafe { GetCursorPos(&mut pt) };
                post(WM_APP_SHOW, pt.x as usize, pt.y as isize);
            }
        }
    }
}

// ---------- OLE drop target ----------

#[implement(IDropTarget)]
struct DropTarget {
    /// Draws Explorer's drag image (the file riding the cursor) over our window.
    helper: Option<IDropTargetHelper>,
    data: RefCell<Option<IDataObject>>,
    /// Label currently shown next to the drag image ("+ Crop"), to avoid redundant updates.
    described: RefCell<Option<String>>,
}

fn hdrop_format() -> FORMATETC {
    FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}

fn read_paths(obj: &IDataObject) -> Option<Vec<String>> {
    unsafe {
        let mut medium = obj.GetData(&hdrop_format()).ok()?;
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

/// Sets the caption Windows shows beside the drag image; None restores the source's own.
fn describe(obj: &IDataObject, label: Option<&str>) {
    unsafe {
        let cf = RegisterClipboardFormatW(w!("DropDescription"));
        let fmt = FORMATETC { cfFormat: cf as u16, ..hdrop_format() };
        let mut desc = DROPDESCRIPTION::default();
        match label {
            Some(text) => {
                desc.r#type = DROPIMAGE_COPY;
                // DROPDESCRIPTION is packed: fill a local copy, then assign the whole array.
                let mut msg = [0u16; 260];
                for (dst, src) in msg.iter_mut().zip(text.encode_utf16().take(259)) {
                    *dst = src;
                }
                desc.szMessage = msg;
            }
            None => desc.r#type = DROPIMAGE_INVALID,
        }
        let Ok(hg) = GlobalAlloc(GMEM_MOVEABLE, std::mem::size_of::<DROPDESCRIPTION>()) else { return };
        let p = GlobalLock(hg) as *mut DROPDESCRIPTION;
        if p.is_null() {
            return;
        }
        *p = desc;
        let _ = GlobalUnlock(hg);
        let medium = STGMEDIUM {
            tymed: TYMED_HGLOBAL.0 as u32,
            u: STGMEDIUM_0 { hGlobal: hg },
            pUnkForRelease: ManuallyDrop::new(None),
        };
        let _ = obj.SetData(&fmt, &medium, true);
    }
}

impl DropTarget_Impl {
    fn update_description(&self, label: Option<String>) {
        if *self.described.borrow() == label {
            return;
        }
        if let Some(obj) = self.data.borrow().as_ref() {
            describe(obj, label.as_deref());
        }
        *self.described.borrow_mut() = label;
    }

    /// Hover + effect for a point; COPY only over an enabled segment.
    fn track(&self, pt: &POINTL, effect: *mut DROPEFFECT) {
        let w = hit(pt.x, pt.y);
        set_hover(w);
        let seg = segment(w);
        unsafe {
            if !effect.is_null() {
                let copy_allowed = (*effect).0 & DROPEFFECT_COPY.0 != 0;
                *effect = if seg.is_some() && copy_allowed { DROPEFFECT_COPY } else { DROPEFFECT_NONE };
            }
        }
        self.update_description(seg.map(|(_, label)| label));
    }
}

impl DropTarget_Impl {
    fn drag_enter(&self, obj: Option<&IDataObject>, _k: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let paths = obj.and_then(read_paths).unwrap_or_default();
        if paths.is_empty() {
            log::debug!("DragEnter: not a file drag, hiding");
            unsafe { *effect = DROPEFFECT_NONE };
            post(WM_APP_HIDE, 0, 0);
            return Ok(());
        }
        FILE_DRAG.store(true, SeqCst);
        *self.data.borrow_mut() = obj.cloned();
        *self.described.borrow_mut() = None;

        let m = menu::build(&paths, &hooks().caps);
        let page = m.start_page;
        PAGE.store(page, SeqCst);
        *menu_lock() = Some(m.clone());
        HOVER.store(-2, SeqCst); // force a hover event after the menu arrives
        emit(RingEvent::Menu { menu: m, page });

        let first = paths[0].clone();
        std::thread::spawn(move || {
            if let Some(src) = capture::thumbnail(&first, 192) {
                if SHOWN.load(SeqCst) {
                    emit(RingEvent::Thumb { src });
                }
            }
        });

        self.track(pt, effect);
        if let (Some(h), Some(o)) = (&self.helper, obj) {
            unsafe { let _ = h.DragEnter(overlay(), o, &POINT { x: pt.x, y: pt.y }, *effect); }
        }
        Ok(())
    }

    fn drag_over(&self, _k: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        self.track(pt, effect);
        if let Some(h) = &self.helper {
            unsafe { let _ = h.DragOver(&POINT { x: pt.x, y: pt.y }, *effect); }
        }
        Ok(())
    }

    fn drag_leave(&self) -> Result<()> {
        self.update_description(None);
        if let Some(h) = &self.helper {
            unsafe { let _ = h.DragLeave(); }
        }
        *self.data.borrow_mut() = None;
        // Dragging off the ring closes it; tap the key again to reopen.
        post(WM_APP_HIDE, 0, 0);
        Ok(())
    }

    fn drop(&self, obj: Option<&IDataObject>, _k: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let w = hit(pt.x, pt.y);
        let chosen = segment(w);
        unsafe {
            let copy_allowed = (*effect).0 & DROPEFFECT_COPY.0 != 0;
            *effect = if chosen.is_some() && copy_allowed { DROPEFFECT_COPY } else { DROPEFFECT_NONE };
        }
        self.update_description(None);
        if let (Some(h), Some(o)) = (&self.helper, obj) {
            unsafe { let _ = h.Drop(o, &POINT { x: pt.x, y: pt.y }, *effect); }
        }
        *self.data.borrow_mut() = None;
        match chosen {
            Some((id, _)) if unsafe { *effect } == DROPEFFECT_COPY => {
                let paths = obj.and_then(read_paths).unwrap_or_default();
                log::info!("drop action={id} files={paths:?}");
                emit(RingEvent::Chosen { index: w });
                (hooks().on_drop)(id, paths);
                post(WM_APP_HIDE, 160, 0); // let the chosen petal pulse
            }
            _ => {
                log::info!("drop cancelled (hub, outside, or disabled segment)");
                post(WM_APP_HIDE, 0, 0);
            }
        }
        Ok(())
    }
}

/// Windows calls these; a panic must never cross back into Windows, so each one is
/// guarded and a failure refuses the drop and closes the ring.
impl IDropTarget_Impl for DropTarget_Impl {
    fn DragEnter(&self, obj: Option<&IDataObject>, k: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        crate::crash::guarded("drag enter", || self.drag_enter(obj, k, pt, effect)).unwrap_or_else(|| refuse(effect))
    }

    fn DragOver(&self, k: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        crate::crash::guarded("drag over", || self.drag_over(k, pt, effect)).unwrap_or_else(|| refuse(effect))
    }

    fn DragLeave(&self) -> Result<()> {
        crate::crash::guarded("drag leave", || self.drag_leave()).unwrap_or_else(|| refuse(std::ptr::null_mut()))
    }

    fn Drop(&self, obj: Option<&IDataObject>, k: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        crate::crash::guarded("drop", || self.drop(obj, k, pt, effect)).unwrap_or_else(|| refuse(effect))
    }
}

fn refuse(effect: *mut DROPEFFECT) -> Result<()> {
    if !effect.is_null() {
        unsafe { *effect = DROPEFFECT_NONE };
    }
    post(WM_APP_HIDE, 0, 0);
    Ok(())
}

// ---------- showing and hiding ----------

/// Positions both windows on the cursor's monitor, sized for its DPI.
unsafe fn show_at(x: i32, y: i32) {
    unsafe {
        let mon = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut info);
        let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
        let scale = dpi_x as f64 / 96.0;
        let win = (WIN_LOGICAL * scale).round() as i32;
        let r = (RING_R_LOGICAL * scale).round() as i32;

        // Keep the ring on screen, but never so far from the cursor that it isn't over it.
        let work = info.rcWork;
        let reach = (r as f64 * 0.6) as i32;
        let cx = x.clamp(work.left + r, (work.right - r).max(work.left + r)).clamp(x - reach, x + reach);
        let cy = y.clamp(work.top + r, (work.bottom - r).max(work.top + r)).clamp(y - reach, y + reach);
        CENTER_X.store(cx, SeqCst);
        CENTER_Y.store(cy, SeqCst);
        RADIUS.store(r, SeqCst);
        HOVER.store(-1, SeqCst);
        FILE_DRAG.store(false, SeqCst);
        PAGE.store(0, SeqCst);
        *menu_lock() = None;

        // Capture before either window is visible.
        let backdrop = capture::backdrop(cx - r, cy - r, 2 * r);
        emit(RingEvent::Show { backdrop });

        let (left, top) = (cx - win / 2, cy - win / 2);
        let flags = SWP_NOACTIVATE | SWP_SHOWWINDOW;

        // The drop target first, on top of everything.
        let drop = overlay();
        let o = win / 2 - r;
        let _ = SetWindowRgn(drop, CreateEllipticRgn(o, o, o + 2 * r + 1, o + 2 * r + 1), true);
        let _ = SetWindowPos(drop, HWND_TOPMOST, left, top, win, win, flags);

        // Then the drawn ring directly beneath it. The ring belongs to the app's main
        // thread, so this is asynchronous: if the main thread is busy we don't wait
        // (a stalled trigger thread made Windows drop key events). Inserting it under
        // the drop target keeps the order right whenever it lands.
        let ring = ring_window();
        let flags = flags | SWP_ASYNCWINDOWPOS;
        let _ = SetWindowPos(ring, drop, left, top, win, win, flags);
        // Crossing onto a monitor with another DPI may resize it; put the size back.
        let _ = SetWindowPos(ring, drop, left, top, win, win, flags);

        // If Windows doesn't confirm a real file drag soon, this wasn't one.
        SetTimer(drop, TIMER_VERIFY, VERIFY_MS, None);
        log::debug!("ring shown at ({cx}, {cy}) r={r}");
    }
}

fn flip_page() {
    let menu = menu_lock();
    let Some(m) = menu.as_ref() else { return };
    let next = 1 - PAGE.load(SeqCst);
    if m.pages[next].is_empty() {
        return;
    }
    PAGE.store(next, SeqCst);
    drop(menu);
    HOVER.store(-2, SeqCst); // next DragOver re-reports hover for the new page
    emit(RingEvent::Page { page: next });
}

fn hide() {
    unsafe {
        let _ = KillTimer(overlay(), TIMER_VERIFY);
        let _ = ShowWindow(overlay(), SW_HIDE);
        let _ = ShowWindowAsync(ring_window(), SW_HIDE);
    }
    if SHOWN.swap(false, SeqCst) || FILE_DRAG.load(SeqCst) {
        emit(RingEvent::Hide);
    }
    FILE_DRAG.store(false, SeqCst);
    HOVER.store(-1, SeqCst);
    *menu_lock() = None;
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    crate::crash::guarded("ring window", || unsafe { wndproc_body(hwnd, msg, wparam, lparam) }).unwrap_or(LRESULT(0))
}

unsafe fn wndproc_body(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_APP_SHOW => {
                show_at(wparam.0 as i32, lparam.0 as i32);
                LRESULT(0)
            }
            WM_APP_FLIP => {
                flip_page();
                LRESULT(0)
            }
            WM_APP_HIDE => {
                if wparam.0 == 0 {
                    hide()
                } else {
                    SetTimer(hwnd, TIMER_HIDE, wparam.0 as u32, None);
                }
                LRESULT(0)
            }
            WM_TIMER => {
                let _ = KillTimer(hwnd, wparam.0);
                if wparam.0 == TIMER_VERIFY && !FILE_DRAG.load(SeqCst) {
                    log::info!("no file drag arrived, hiding ring");
                    hide();
                } else if wparam.0 == TIMER_HIDE {
                    hide();
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

unsafe fn run() -> Result<()> {
    unsafe {
        // Hook coordinates are physical pixels; window placement must match on scaled displays.
        let _ = windows::Win32::UI::HiDpi::SetThreadDpiAwarenessContext(
            windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
        OleInitialize(None)?;
        let hinst = GetModuleHandleW(None)?;
        let class = w!("FileBeaverDropTarget");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
            ..Default::default()
        };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class, w!("File Beaver"), WS_POPUP,
            0, 0, 1, 1, None, None, hinst, None,
        )?;
        OVERLAY.store(hwnd.0 as isize, SeqCst);
        // Alpha 1/255: invisible, but still hit-testable so Windows routes the drag to it.
        SetLayeredWindowAttributes(hwnd, COLORREF(0), 1, LWA_ALPHA)?;

        let helper: Option<IDropTargetHelper> = CoCreateInstance(&CLSID_DragDropHelper, None, CLSCTX_INPROC_SERVER).ok();
        let target: IDropTarget = DropTarget { helper, data: RefCell::new(None), described: RefCell::new(None) }.into();
        RegisterDragDrop(hwnd, &target)?;

        // The hooks get a thread of their own that does nothing but receive input and post
        // messages here. Windows skips (and can eventually remove) a low-level hook whose
        // thread is slow to respond, so it must never wait on window work.
        let hooks_ready = std::sync::mpsc::channel::<bool>();
        let module = hinst.0 as isize; // raw handles can't cross threads; numbers can
        std::thread::Builder::new()
            .name("shiftdrop-hooks".into())
            .spawn(move || {
                // (Still inside run()'s unsafe block.)
                let installed = {
                    let _ = windows::Win32::UI::HiDpi::SetThreadDpiAwarenessContext(
                        windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
                    );
                    let hinst = windows::Win32::Foundation::HMODULE(module as *mut _);
                    let m = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), hinst, 0);
                    let k = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), hinst, 0);
                    m.is_ok() && k.is_ok()
                };
                let _ = hooks_ready.0.send(installed);
                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, 0, 0).into() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            })
            .map_err(|e| Error::new(E_FAIL, e.to_string()))?;
        if hooks_ready.1.recv().unwrap_or(false) {
            log::info!("trigger ready");
        } else {
            log::error!("couldn't install the keyboard/mouse hooks");
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).into() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        Ok(())
    }
}
