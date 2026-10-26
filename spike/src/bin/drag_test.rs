//! Automated check for the spike: performs an Explorer-identical file drag
//! (shell IDataObject for a real file) with Shift held, drops on a wedge,
//! then does a Shift-drag that is NOT a file drag. Read shiftdrop-spike.log after.

use std::thread::sleep;
use std::time::Duration;

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::*;
use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

#[implement(IDropSource)]
struct Source;

impl IDropSource_Impl for Source_Impl {
    fn QueryContinueDrag(&self, esc: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
        if std::env::var("DRAG_DEBUG").is_ok() {
            println!("  QCD t={:?} keys={:#x}", START.get().map(|s| s.elapsed()), keys.0);
        }
        if esc.as_bool() { DRAGDROP_S_CANCEL } else if keys.0 & MK_LBUTTON.0 == 0 { DRAGDROP_S_DROP } else { S_OK }
    }
    fn GiveFeedback(&self, _e: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

fn mouse(flags: MOUSE_EVENT_FLAGS, x: i32, y: i32) {
    unsafe {
        let sw = GetSystemMetrics(SM_CXSCREEN);
        let sh = GetSystemMetrics(SM_CYSCREEN);
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

fn shift(down: bool) {
    unsafe {
        let input = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VK_SHIFT,
                    dwFlags: if down { KEYBD_EVENT_FLAGS(0) } else { KEYEVENTF_KEYUP },
                    ..Default::default()
                },
            },
        };
        SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
}

fn glide(from: (i32, i32), to: (i32, i32)) {
    for i in 1..=20 {
        mouse(MOUSE_EVENT_FLAGS(0), from.0 + (to.0 - from.0) * i / 20, from.1 + (to.1 - from.1) * i / 20);
        sleep(Duration::from_millis(25));
    }
}

fn main() -> Result<()> {
    let file = std::env::args().nth(1).expect("usage: drag_test <file>");
    unsafe {
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
        OleInitialize(None)?;
        // A blank window under the test area so our clicks land on us, not the user's apps.
        let hinst = GetModuleHandleW(None)?;
        unsafe extern "system" fn pad_proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
            DefWindowProcW(h, m, w, l)
        }
        let wc = WNDCLASSW { lpfnWndProc: Some(pad_proc), hInstance: hinst.into(), lpszClassName: w!("DragTestPad"), ..Default::default() };
        RegisterClassW(&wc);
        let pad = CreateWindowExW(WS_EX_TOPMOST, w!("DragTestPad"), w!("drag test"), WS_POPUP | WS_VISIBLE, 100, 100, 700, 600, None, None, hinst, None)?;
        let _ = SetForegroundWindow(pad);
        sleep(Duration::from_millis(300));

        let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(file.as_str()), None::<&IBindCtx>)?;
        let data: IDataObject = item.BindToHandler(None, &BHID_DataObject)?;
        let source: IDropSource = Source.into();

        // --- Test 1: real file drag + Shift, drop on the "Crop" wedge (left side) ---
        let start = (400, 400);
        mouse(MOUSE_EVENT_FLAGS(0), start.0, start.1);
        sleep(Duration::from_millis(100));
        mouse(MOUSEEVENTF_LEFTDOWN, start.0, start.1);
        // Pump so this thread's key state sees the button down before DoDragDrop checks it.
        for _ in 0..10 {
            let mut m = MSG::default();
            while PeekMessageW(&mut m, None, 0, 0, PM_REMOVE).into() {
                let _ = TranslateMessage(&m);
                DispatchMessageW(&m);
            }
            sleep(Duration::from_millis(10));
        }
        std::thread::spawn(move || {
            sleep(Duration::from_millis(300));
            shift(true);
            sleep(Duration::from_millis(100));
            glide(start, (420, 400)); // ring appears centred near here
            sleep(Duration::from_millis(300));
            glide((420, 400), (310, 400)); // left = Crop
            sleep(Duration::from_millis(300));
            mouse(MOUSEEVENTF_LEFTUP, 310, 400);
            shift(false);
        });
        let _ = START.set(std::time::Instant::now());
        let mut effect = DROPEFFECT::default();
        let hr = DoDragDrop(&data, &source, DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK, &mut effect);
        println!("TEST1 DoDragDrop hr={hr:?} effect={} (1=COPY, 2=MOVE)", effect.0);
        println!("TEST1 source still exists: {}", std::path::Path::new(&file).exists());

        sleep(Duration::from_millis(800));

        // --- Test 2: Shift-drag that is NOT a file drag (no DoDragDrop) ---
        mouse(MOUSEEVENTF_LEFTDOWN, start.0, start.1);
        shift(true);
        glide(start, (480, 420));
        sleep(Duration::from_millis(1000));
        mouse(MOUSEEVENTF_LEFTUP, 480, 420);
        shift(false);
        println!("TEST2 done (expect 'no OLE file drag arrived' in spike log)");

        sleep(Duration::from_millis(500));
        let _ = DestroyWindow(pad);
    }
    Ok(())
}
