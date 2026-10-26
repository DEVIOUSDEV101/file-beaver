//! End-to-end check of the floating beaver against the running app:
//! 1. drag a real file onto the beaver (no Shift) -> the ring opens around it -> drop on
//!    the top petal; 2. drag the beaver somewhere else; 3. right-click -> Hide beaver.
//! usage: beaver_test <file> <out_dir>

use std::path::PathBuf;
use std::thread::sleep;
use std::time::{Duration, Instant};

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::*;
use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
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

fn tap(vk: VIRTUAL_KEY) {
    unsafe {
        for up in [false, true] {
            let input = INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }, ..Default::default() } },
            };
            SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
            sleep(Duration::from_millis(50));
        }
    }
}

fn glide(from: (i32, i32), to: (i32, i32)) {
    for i in 1..=25 {
        mouse(MOUSE_EVENT_FLAGS(0), from.0 + (to.0 - from.0) * i / 25, from.1 + (to.1 - from.1) * i / 25);
        sleep(Duration::from_millis(20));
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

fn rect_of(h: HWND) -> RECT {
    let mut r = RECT::default();
    unsafe {
        let _ = GetWindowRect(h, &mut r);
    }
    r
}

fn center(r: RECT) -> (i32, i32) {
    ((r.left + r.right) / 2, (r.top + r.bottom) / 2)
}

fn beaver() -> HWND {
    unsafe { FindWindowW(w!("FileBeaverWidget"), None).expect("beaver window not found (is the app running?)") }
}

unsafe extern "system" fn pad_proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(h, m, w, l) }
}

fn main() -> Result<()> {
    let file = std::env::args().nth(1).expect("usage: beaver_test <file> <out_dir>");
    let out = PathBuf::from(std::env::args().nth(2).expect("out dir"));
    // "click": release the file ON the beaver, then click a petal (the ring stays open).
    let click_mode = std::env::args().nth(3).as_deref() == Some("click");
    std::fs::create_dir_all(&out).unwrap();
    unsafe {
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        OleInitialize(None)?;
    }
    let b = beaver();
    let br = rect_of(b);
    println!("beaver visible={} at ({}, {}) size {}", unsafe { IsWindowVisible(b) }.as_bool(), br.left, br.top, br.right - br.left);

    // ---- 1. drag a file onto the beaver, no Shift ----
    unsafe {
        let hinst = GetModuleHandleW(None)?;
        let wc = WNDCLASSW { lpfnWndProc: Some(pad_proc), hInstance: hinst.into(), lpszClassName: w!("BeaverTestPad"), hbrBackground: HBRUSH(GetStockObject(WHITE_BRUSH).0), ..Default::default() };
        RegisterClassW(&wc);
        // A pad to start the drag on, well away from the beaver.
        let (px, py) = (br.left - 900, br.top - 700);
        let pad = CreateWindowExW(WS_EX_TOPMOST, w!("BeaverTestPad"), w!("beaver test"), WS_POPUP | WS_VISIBLE, px, py, 300, 300, None, None, hinst, None)?;
        let _ = SetForegroundWindow(pad);
        pump(300);
        let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(file.as_str()), None::<&IBindCtx>)?;
        let data: IDataObject = item.BindToHandler(None, &BHID_DataObject)?;
        let source: IDropSource = Source.into();
        let start = (px + 150, py + 150);
        let target = center(br);
        mouse(MOUSE_EVENT_FLAGS(0), start.0, start.1);
        pump(100);
        mouse(MOUSEEVENTF_LEFTDOWN, start.0, start.1);
        pump(100);
        let out2 = out.clone();
        std::thread::spawn(move || {
            sleep(Duration::from_millis(200));
            glide(start, target);
            // Wait for the ring ("File Beaver menu") to appear around the beaver.
            let t0 = Instant::now();
            let ring = loop {
                if let Ok(h) = FindWindowW(None, w!("File Beaver menu")) {
                    if IsWindowVisible(h).as_bool() {
                        break Some(h);
                    }
                }
                if t0.elapsed() > Duration::from_secs(3) {
                    break None;
                }
                sleep(Duration::from_millis(30));
            };
            let Some(ring) = ring else {
                println!("FAIL: ring didn't open when the file reached the beaver");
                mouse(MOUSEEVENTF_LEFTUP, target.0, target.1);
                return;
            };
            sleep(Duration::from_millis(300));
            if click_mode {
                // Snapshot the ring while still holding the file over the beaver, then let go.
                let rr = rect_of(ring);
                let (cx, cy) = center(rr);
                let half = (rr.right - rr.left) / 2 + 20;
                let _ = std::process::Command::new("ffmpeg")
                    .args(["-loglevel", "error", "-y", "-f", "gdigrab", "-offset_x", &(cx - half).to_string(), "-offset_y", &(cy - half).to_string(), "-video_size", &format!("{}x{}", half * 2, half * 2), "-i", "desktop", "-frames:v", "1"])
                    .arg(out2.join("beaver-hold.png"))
                    .status();
                mouse(MOUSEEVENTF_LEFTUP, target.0, target.1);
                return;
            }
            let rr = rect_of(ring);
            let (cx, cy) = center(rr);
            let r = (rr.right - rr.left) as f64 * 180.0 / 384.0;
            println!("ring opened {} ms after reaching the beaver, centred {} px from it", t0.elapsed().as_millis(), (((cx - target.0).pow(2) + (cy - target.1).pow(2)) as f64).sqrt() as i32);
            let png = (cx, cy - (0.665 * r) as i32);
            glide(target, png);
            sleep(Duration::from_millis(400));
            let half = (rr.right - rr.left) / 2 + 20;
            let _ = std::process::Command::new("ffmpeg")
                .args(["-loglevel", "error", "-y", "-f", "gdigrab", "-offset_x", &(cx - half).to_string(), "-offset_y", &(cy - half).to_string(), "-video_size", &format!("{}x{}", half * 2, half * 2), "-i", "desktop", "-frames:v", "1"])
                .arg(out2.join("beaver-ring.png"))
                .status();
            mouse(MOUSEEVENTF_LEFTUP, png.0, png.1);
        });
        let mut effect = DROPEFFECT::default();
        let _ = DoDragDrop(&data, &source, DROPEFFECT_COPY | DROPEFFECT_MOVE, &mut effect);
        println!("drop effect={} (1=COPY)", effect.0);
        pump(300);
        let _ = DestroyWindow(pad);
        if click_mode {
            sleep(Duration::from_millis(700));
            let ring = FindWindowW(None, w!("File Beaver menu")).ok().filter(|h| IsWindowVisible(*h).as_bool());
            println!("ring still open after dropping on the beaver: {}", ring.is_some());
            let Some(ring) = ring else { return Ok(()) };
            let rr = rect_of(ring);
            let (cx, cy) = center(rr);
            let r = (rr.right - rr.left) as f64 * 180.0 / 384.0;
            let png = (cx, cy - (0.665 * r) as i32);
            glide(target, png);
            sleep(Duration::from_millis(400));
            mouse(MOUSEEVENTF_LEFTDOWN, png.0, png.1);
            sleep(Duration::from_millis(60));
            mouse(MOUSEEVENTF_LEFTUP, png.0, png.1);
            sleep(Duration::from_millis(800));
            println!("ring closed after clicking PNG: {}", !IsWindowVisible(ring).as_bool());
            return Ok(());
        }
    }
    sleep(Duration::from_millis(800));

    // ---- 2. drag the beaver 200 px left ----
    let from = center(rect_of(b));
    mouse(MOUSE_EVENT_FLAGS(0), from.0, from.1);
    sleep(Duration::from_millis(100));
    mouse(MOUSEEVENTF_LEFTDOWN, from.0, from.1);
    sleep(Duration::from_millis(100));
    glide(from, (from.0 - 200, from.1 - 40));
    sleep(Duration::from_millis(100));
    mouse(MOUSEEVENTF_LEFTUP, from.0 - 200, from.1 - 40);
    sleep(Duration::from_millis(600));
    let moved = rect_of(b);
    println!("beaver moved: ({}, {}) -> ({}, {})", br.left, br.top, moved.left, moved.top);

    // ---- 3. right-click -> Hide beaver (second menu item) ----
    let c = center(moved);
    mouse(MOUSE_EVENT_FLAGS(0), c.0, c.1);
    sleep(Duration::from_millis(100));
    mouse(MOUSEEVENTF_RIGHTDOWN, c.0, c.1);
    sleep(Duration::from_millis(60));
    mouse(MOUSEEVENTF_RIGHTUP, c.0, c.1);
    sleep(Duration::from_millis(500));
    tap(VK_DOWN);
    tap(VK_DOWN);
    tap(VK_RETURN);
    sleep(Duration::from_millis(600));
    println!("beaver visible after Hide: {}", unsafe { IsWindowVisible(b) }.as_bool());
    Ok(())
}
