//! Microsoft Word, driven through COM automation, for what File Beaver can't do itself:
//! Word documents to PDF, and PDF to an editable DOCX (Word's own PDF reflow).
//!
//! Word is fragile to automate, so every conversion:
//! - starts its own invisible Word. Word always starts a new process for automation, so the
//!   user's open Word and documents are never touched;
//! - works on a private copy of the file, so the original is never locked or changed and
//!   Word never asks "this file is in use";
//! - runs on its own STA thread while the caller watches. If Word hangs, shows a dialog or
//!   the user cancels, the Word process this conversion started (and only that one) is killed;
//! - closes without saving and quits Word, and kills it if it lingers. No WINWORD.EXE is
//!   left behind.

use std::mem::ManuallyDrop;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::SeqCst};
use std::sync::{mpsc, Arc, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use windows::core::{Interface, GUID, HSTRING, IUnknown, PCWSTR, VARIANT};
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Diagnostics::ToolHelp::*;
use windows::Win32::System::Registry::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::formats::Format;

const LOCALE_USER_DEFAULT: u32 = 0x0400;
const DISPID_PROPERTYPUT: i32 = -3;
const VT_DISPATCH: u16 = 9;

// Word's own constants.
const WD_ALERTS_NONE: i32 = 0;
const MSO_AUTOMATION_SECURITY_FORCE_DISABLE: i32 = 3;
const WD_DO_NOT_SAVE_CHANGES: i32 = 0;
const WD_EXPORT_FORMAT_PDF: i32 = 17;
const WD_FORMAT_DOCUMENT_DEFAULT: i32 = 16;

/// A visible window from our invisible Word this long means it's waiting for an answer.
const DIALOG_GRACE: Duration = Duration::from_secs(4);

/// Whether Word is installed, from the registry only: never starts Word.
pub fn installed() -> bool {
    static INSTALLED: OnceLock<bool> = OnceLock::new();
    *INSTALLED.get_or_init(|| server_exe().is_some_and(|p| p.is_file()))
}

/// Installed, and not already found to be unactivated (which can only be learnt by trying).
pub fn usable() -> bool {
    installed() && !UNACTIVATED.load(SeqCst)
}

/// WINWORD.EXE as registered for automation (Word.Application's local server).
fn server_exe() -> Option<PathBuf> {
    let clsid = reg_string(r"Word.Application\CLSID")?;
    exe_from_command(&reg_string(&format!(r"CLSID\{}\LocalServer32", clsid.trim()))?)
}

/// `"C:\...\WINWORD.EXE" /Automation` or `C:\...\WINWORD.EXE /Automation` -> the exe path.
fn exe_from_command(cmd: &str) -> Option<PathBuf> {
    let cmd = cmd.trim();
    if let Some(rest) = cmd.strip_prefix('"') {
        return rest.split('"').next().filter(|s| !s.is_empty()).map(PathBuf::from);
    }
    let end = cmd.to_ascii_lowercase().find(".exe")? + 4;
    Some(PathBuf::from(&cmd[..end]))
}

/// The default value of a key under HKEY_CLASSES_ROOT.
fn reg_string(key: &str) -> Option<String> {
    let key = HSTRING::from(key);
    let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ;
    let mut len = 0u32;
    unsafe {
        if RegGetValueW(HKEY_CLASSES_ROOT, &key, PCWSTR::null(), flags, None, None, Some(&mut len)) != ERROR_SUCCESS {
            return None;
        }
        let mut buf = vec![0u16; (len as usize).div_ceil(2) + 1];
        let mut len = (buf.len() * 2) as u32;
        if RegGetValueW(HKEY_CLASSES_ROOT, &key, PCWSTR::null(), flags, None, Some(buf.as_mut_ptr().cast()), Some(&mut len)) != ERROR_SUCCESS {
            return None;
        }
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..end]))
    }
}

/// Converts `src` (a Word document to PDF, or a PDF to DOCX) and returns the result's bytes.
/// `stage` is told what's happening ("Opening in Word", ...).
pub fn convert(src: &Path, target: Format, cancel: &AtomicBool, stage: &mut dyn FnMut(&str)) -> Result<Vec<u8>> {
    let from = Format::from_path(src).context("unknown file type")?;
    match (from, target) {
        (f, Format::Pdf) if f.is_word() => {}
        (Format::Pdf, Format::Docx) => {}
        _ => bail!("Word can't convert {} to {}", from.label(), target.label()),
    }
    if !installed() {
        bail!("this needs Microsoft Word");
    }
    if UNACTIVATED.load(SeqCst) {
        bail!("{UNACTIVATED_MESSAGE}");
    }
    let work = WorkDir::new()?;
    // Plain ASCII names in a private folder: no "file in use", no long or odd paths for Word.
    let input = work.0.join(format!("in.{}", from.ext()));
    std::fs::copy(src, &input).with_context(|| format!("can't read {}", src.display()))?;
    let output = work.0.join(format!("out.{}", target.ext()));

    let size_mb = std::fs::metadata(&input).map(|m| m.len() / (1 << 20)).unwrap_or(0);
    let limit = Duration::from_secs((90 + 20 * size_mb).min(600));
    run_watched(&input, &output, target, limit, cancel, stage)?;
    let bytes = std::fs::read(&output).context("Word didn't save a result")?;
    if bytes.is_empty() {
        bail!("Word saved an empty file");
    }
    Ok(bytes)
}

/// A private temp folder, removed when dropped.
struct WorkDir(PathBuf);

impl WorkDir {
    fn new() -> Result<WorkDir> {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("FileBeaver-word-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).context("can't create a temporary folder")?;
        Ok(WorkDir(dir))
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        // Word can hold the files for a moment after it exits.
        for _ in 0..20 {
            if std::fs::remove_dir_all(&self.0).is_ok() || !self.0.exists() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

enum Msg {
    Stage(&'static str),
    Done(Result<()>),
}

/// Runs the Word session on its own thread and watches it.
fn run_watched(input: &Path, output: &Path, target: Format, limit: Duration, cancel: &AtomicBool, stage: &mut dyn FnMut(&str)) -> Result<()> {
    let before = word_pids();
    let pid = Arc::new(AtomicU32::new(0));
    let (tx, rx) = mpsc::channel::<Msg>();
    let (input, output, worker_pid, worker_before) = (input.to_path_buf(), output.to_path_buf(), pid.clone(), before.clone());
    std::thread::Builder::new()
        .name("shiftdrop-word".into())
        .spawn(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                unsafe {
                    let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                }
                let r = session(&input, &output, target, &worker_before, &worker_pid, &tx);
                unsafe { CoUninitialize() };
                r
            }))
            .unwrap_or_else(|_| Err(anyhow!("internal error while talking to Word")));
            let _ = tx.send(Msg::Done(r));
        })
        .context("can't start Word")?;

    let started = Instant::now();
    let mut ours: Option<OwnedProcess> = None;
    let mut dialog_since: Option<Instant> = None;
    let failure = loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(Msg::Stage(s)) => stage(s),
            Ok(Msg::Done(r)) => {
                // Word was told to quit; make sure it really goes.
                let p = ours.or_else(|| OwnedProcess::open(pid.load(SeqCst)));
                if let Some(p) = p {
                    p.wait_or_kill(Duration::from_secs(10));
                } else {
                    kill_new_automation_words(&before);
                }
                return r;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break anyhow!("lost contact with Word"),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if ours.is_none() {
            ours = OwnedProcess::open(pid.load(SeqCst));
        }
        if cancel.load(SeqCst) {
            break anyhow!("cancelled");
        }
        if started.elapsed() > limit {
            break anyhow!("Word took too long and was stopped");
        }
        if let Some(p) = &ours {
            if has_visible_window(p.pid) {
                let since = *dialog_since.get_or_insert_with(Instant::now);
                if since.elapsed() > DIALOG_GRACE {
                    break anyhow!("Word stopped to ask something");
                }
            } else {
                dialog_since = None;
            }
        }
    };
    log::warn!("stopping Word: {failure}");
    match ours.or_else(|| OwnedProcess::open(pid.load(SeqCst))) {
        Some(p) => p.kill(),
        // Word hadn't been identified yet (stuck starting up).
        None => kill_new_automation_words(&before),
    }
    // Let the worker notice the dead server and finish; its result no longer matters.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Ok(Msg::Done(_)) | Err(mpsc::RecvTimeoutError::Disconnected) = rx.recv_timeout(Duration::from_millis(200)) {
            break;
        }
    }
    Err(failure)
}

/// Set once Word turns out not to be activated, so later files fail at once.
static UNACTIVATED: AtomicBool = AtomicBool::new(false);
const UNACTIVATED_MESSAGE: &str = "Word isn't activated on this PC";

/// Word's window title when it runs unlicensed, e.g. "Report - Word (Unlicensed Product)".
/// English only; in other languages the watchdog catches the stuck save instead.
fn is_unlicensed_title(title: &str) -> bool {
    let t = title.to_lowercase();
    ["unlicensed product", "unlicenced product", "product activation failed"].iter().any(|m| t.contains(m))
}

/// Off only in tests of the open, cancel and corrupt-file paths, which unactivated Word can run.
static CHECK_LICENCE: AtomicBool = AtomicBool::new(true);

fn shows_unlicensed(pid: u32) -> bool {
    CHECK_LICENCE.load(SeqCst) && pid != 0 && window_titles(pid, "OpusApp").iter().any(|t| is_unlicensed_title(t))
}

/// The Word conversion itself, on an STA thread.
fn session(input: &Path, output: &Path, target: Format, before: &[u32], pid: &AtomicU32, tx: &mpsc::Sender<Msg>) -> Result<()> {
    let stage = |s| {
        let _ = tx.send(Msg::Stage(s));
    };
    stage("Starting Word");
    let app = unsafe {
        let clsid = CLSIDFromProgID(windows::core::w!("Word.Application")).context("Word isn't registered")?;
        Disp(CoCreateInstance::<_, IDispatch>(&clsid, None, CLSCTX_LOCAL_SERVER).map_err(|e| anyhow!("Word didn't start ({})", e.message().trim()))?)
    };
    // Our process is the Word that COM just started. If two appeared at once, tell them
    // apart by a unique caption (which hides the licence state in the title, so it's the fallback).
    match new_automation_words(before).as_slice() {
        [one] => pid.store(*one, SeqCst),
        _ => {
            let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
            let caption = format!("File Beaver {} {nanos}", std::process::id());
            if app.put("Caption", VARIANT::from(caption.as_str())).is_ok() {
                if let Some(p) = pid_of_window("OpusApp", &caption) {
                    pid.store(p, SeqCst);
                }
            }
        }
    }
    let result = (|| -> Result<()> {
        // Every setting is best effort; older Words may lack some.
        let _ = app.put("Visible", VARIANT::from(false));
        let _ = app.put("DisplayAlerts", VARIANT::from(WD_ALERTS_NONE));
        // Never run macros in the documents we open.
        let _ = app.put("AutomationSecurity", VARIANT::from(MSO_AUTOMATION_SECURITY_FORCE_DISABLE));
        let _ = app.put("ScreenUpdating", VARIANT::from(false));
        if let Ok(options) = app.get("Options") {
            let _ = options.put("ConfirmConversions", VARIANT::from(false));
            let _ = options.put("UpdateLinksAtOpen", VARIANT::from(false));
            let _ = options.put("DoNotPromptForConvert", VARIANT::from(true));
            let _ = options.put("SaveNormalPrompt", VARIANT::from(false));
        }
        // Unactivated Word can open files but never saves them: it stops on a hidden
        // "subscription" dialog, or never finishes opening a PDF. Its title gives it away.
        if shows_unlicensed(pid.load(SeqCst)) {
            UNACTIVATED.store(true, SeqCst);
            bail!("{UNACTIVATED_MESSAGE}");
        }
        stage("Opening in Word");
        let docs = app.get("Documents")?;
        let doc = docs
            .call(
                "Open",
                &[
                    ("FileName", VARIANT::from(input.to_string_lossy().as_ref())),
                    ("ConfirmConversions", VARIANT::from(false)),
                    ("ReadOnly", VARIANT::from(true)),
                    ("AddToRecentFiles", VARIANT::from(false)),
                    // A wrong password makes Word fail on protected files instead of asking.
                    ("PasswordDocument", VARIANT::from("\u{1}FileBeaver")),
                    ("Visible", VARIANT::from(false)),
                    ("NoEncodingDialog", VARIANT::from(true)),
                ],
            )
            .and_then(|v| dispatch(&v))
            .map_err(open_error)?;
        // Checked again now the document's window exists, in case the first look was early.
        if shows_unlicensed(pid.load(SeqCst)) {
            UNACTIVATED.store(true, SeqCst);
            let _ = doc.call("Close", &[("SaveChanges", VARIANT::from(WD_DO_NOT_SAVE_CHANGES))]);
            return Err(anyhow!("{UNACTIVATED_MESSAGE}"));
        }
        let saved = (|| -> Result<()> {
            let out = VARIANT::from(output.to_string_lossy().as_ref());
            if target == Format::Pdf {
                stage("Saving PDF");
                doc.call("ExportAsFixedFormat", &[("OutputFileName", out), ("ExportFormat", VARIANT::from(WD_EXPORT_FORMAT_PDF)), ("OpenAfterExport", VARIANT::from(false))])?;
            } else {
                stage("Saving DOCX");
                doc.call("SaveAs2", &[("FileName", out), ("FileFormat", VARIANT::from(WD_FORMAT_DOCUMENT_DEFAULT)), ("AddToRecentFiles", VARIANT::from(false))])?;
            }
            Ok(())
        })()
        .map_err(|e| {
            log::warn!("Word couldn't save: {e:#}");
            anyhow!("Word couldn't save it")
        });
        let _ = doc.call("Close", &[("SaveChanges", VARIANT::from(WD_DO_NOT_SAVE_CHANGES))]);
        saved
    })();
    let _ = app.call("Quit", &[("SaveChanges", VARIANT::from(WD_DO_NOT_SAVE_CHANGES))]);
    result
}

/// Word's message for a file it can't open, shortened to what the user can act on.
fn open_error(e: anyhow::Error) -> anyhow::Error {
    let text = format!("{e:#}").to_lowercase();
    if text.contains("password") {
        anyhow!("it's password protected")
    } else {
        log::warn!("Word couldn't open the file: {e:#}");
        anyhow!("damaged, Word can't open it")
    }
}

/// Late-bound calls on a Word object.
struct Disp(IDispatch);

impl Disp {
    fn invoke(&self, name: &str, flags: DISPATCH_FLAGS, args: &[(&str, VARIANT)]) -> Result<VARIANT> {
        let names: Vec<HSTRING> = std::iter::once(name).chain(args.iter().map(|(n, _)| *n)).map(HSTRING::from).collect();
        let ptrs: Vec<PCWSTR> = names.iter().map(|h| PCWSTR(h.as_ptr())).collect();
        let mut ids = vec![0i32; names.len()];
        unsafe { self.0.GetIDsOfNames(&GUID::zeroed(), ptrs.as_ptr(), ptrs.len() as u32, LOCALE_USER_DEFAULT, ids.as_mut_ptr()) }
            .map_err(|_| anyhow!("Word has no {name}"))?;
        let mut values: Vec<VARIANT> = args.iter().map(|(_, v)| v.clone()).collect();
        let mut named: Vec<i32> = if flags == DISPATCH_PROPERTYPUT { vec![DISPID_PROPERTYPUT] } else { ids[1..].to_vec() };
        let params = DISPPARAMS {
            rgvarg: if values.is_empty() { std::ptr::null_mut() } else { values.as_mut_ptr() },
            rgdispidNamedArgs: if named.is_empty() { std::ptr::null_mut() } else { named.as_mut_ptr() },
            cArgs: values.len() as u32,
            cNamedArgs: named.len() as u32,
        };
        let mut result = VARIANT::new();
        let mut excep = EXCEPINFO::default();
        let r = unsafe { self.0.Invoke(ids[0], &GUID::zeroed(), LOCALE_USER_DEFAULT, flags, &params, Some(&mut result), Some(&mut excep), None) };
        let description = unsafe {
            let d = ManuallyDrop::take(&mut excep.bstrDescription).to_string();
            drop(ManuallyDrop::take(&mut excep.bstrSource));
            drop(ManuallyDrop::take(&mut excep.bstrHelpFile));
            d
        };
        match r {
            Ok(()) => Ok(result),
            Err(_) if !description.trim().is_empty() => Err(anyhow!("{}", description.trim())),
            Err(e) => Err(anyhow!("{name} failed ({})", e.message())),
        }
    }

    fn get(&self, name: &str) -> Result<Disp> {
        dispatch(&self.invoke(name, DISPATCH_PROPERTYGET, &[])?)
    }

    fn put(&self, name: &str, value: VARIANT) -> Result<()> {
        self.invoke(name, DISPATCH_PROPERTYPUT, &[("", value)]).map(drop)
    }

    fn call(&self, name: &str, args: &[(&str, VARIANT)]) -> Result<VARIANT> {
        self.invoke(name, DISPATCH_METHOD, args)
    }
}

/// The object in a VARIANT returned by Word.
fn dispatch(v: &VARIANT) -> Result<Disp> {
    unsafe {
        let raw = v.as_raw();
        let p = raw.Anonymous.Anonymous.Anonymous.punkVal;
        if raw.Anonymous.Anonymous.vt != VT_DISPATCH || p.is_null() {
            bail!("Word returned no object");
        }
        let unknown = IUnknown::from_raw_borrowed(&p).context("Word returned no object")?;
        Ok(Disp(unknown.cast::<IDispatch>()?))
    }
}

/// A process we started and may kill, held by handle so a reused pid can't hit another one.
struct OwnedProcess {
    pid: u32,
    handle: HANDLE,
}

impl OwnedProcess {
    fn open(pid: u32) -> Option<OwnedProcess> {
        if pid == 0 {
            return None;
        }
        let handle = unsafe { OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, false, pid) }.ok()?;
        Some(OwnedProcess { pid, handle })
    }

    fn kill(&self) {
        log::warn!("ending Word (pid {})", self.pid);
        unsafe {
            let _ = TerminateProcess(self.handle, 1);
            WaitForSingleObject(self.handle, 5000);
        }
    }

    fn wait_or_kill(&self, wait: Duration) {
        if unsafe { WaitForSingleObject(self.handle, wait.as_millis() as u32) } != WAIT_OBJECT_0 {
            self.kill();
        }
    }
}

impl Drop for OwnedProcess {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

struct ProcEntry {
    pid: u32,
    parent: u32,
    exe: String,
}

fn processes() -> Vec<ProcEntry> {
    let mut out = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        while ok {
            let end = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            out.push(ProcEntry { pid: e.th32ProcessID, parent: e.th32ParentProcessID, exe: String::from_utf16_lossy(&e.szExeFile[..end]) });
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    out
}

pub fn word_pids() -> Vec<u32> {
    processes().into_iter().filter(|p| p.exe.eq_ignore_ascii_case("WINWORD.EXE")).map(|p| p.pid).collect()
}

/// Last resort when Word hung before we learned its pid: kill Words that appeared since
/// `before` and were started by COM (parent svchost.exe). A Word the user opened has
/// Explorer or another app as its parent, so it is left alone.
fn kill_new_automation_words(before: &[u32]) {
    for pid in new_automation_words(before) {
        if let Some(o) = OwnedProcess::open(pid) {
            o.kill();
        }
    }
}

/// Words started by COM (parent svchost.exe) that weren't running at `before`.
fn new_automation_words(before: &[u32]) -> Vec<u32> {
    let all = processes();
    all.iter()
        .filter(|p| p.exe.eq_ignore_ascii_case("WINWORD.EXE") && !before.contains(&p.pid))
        .filter(|p| all.iter().any(|q| q.pid == p.parent && q.exe.eq_ignore_ascii_case("svchost.exe")))
        .map(|p| p.pid)
        .collect()
}

/// Titles of the process's top-level windows of one class.
fn window_titles(pid: u32, class: &str) -> Vec<String> {
    unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let state = &mut *(lparam.0 as *mut (u32, Vec<u16>, Vec<String>));
        let mut owner = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut owner));
        if owner == state.0 {
            let mut buf = [0u16; 256];
            let n = GetClassNameW(hwnd, &mut buf) as usize;
            if buf[..n] == state.1[..] {
                let mut title = [0u16; 512];
                let n = GetWindowTextW(hwnd, &mut title) as usize;
                state.2.push(String::from_utf16_lossy(&title[..n]));
            }
        }
        TRUE
    }
    let mut state = (pid, class.encode_utf16().collect::<Vec<u16>>(), Vec::new());
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut state as *mut _ as isize));
    }
    state.2
}

fn pid_of_window(class: &str, title: &str) -> Option<u32> {
    let hwnd = unsafe { FindWindowW(&HSTRING::from(class), &HSTRING::from(title)) }.ok()?;
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    (pid != 0).then_some(pid)
}

/// Whether the process shows any top-level window. Our Word is invisible, so a visible
/// window is a dialog waiting for someone to answer it.
fn has_visible_window(pid: u32) -> bool {
    unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let state = &mut *(lparam.0 as *mut (u32, bool));
        let mut owner = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut owner));
        if owner == state.0 && IsWindowVisible(hwnd).as_bool() {
            state.1 = true;
            return FALSE;
        }
        TRUE
    }
    let mut state = (pid, false);
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut state as *mut _ as isize));
    }
    state.1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_exe_from_a_server_command() {
        let exe = r"C:\Program Files\Microsoft Office\Root\Office16\WINWORD.EXE";
        assert_eq!(exe_from_command(&format!("\"{exe}\" /Automation")), Some(PathBuf::from(exe)));
        assert_eq!(exe_from_command(&format!("{exe} /Automation")), Some(PathBuf::from(exe)));
        assert_eq!(exe_from_command("  "), None);
    }

    #[test]
    fn spots_unactivated_word_by_its_title() {
        assert!(is_unlicensed_title("Report [Read-only] - Word (Unlicensed Product)"));
        assert!(is_unlicensed_title("Report - Word (Product Activation Failed)"));
        assert!(!is_unlicensed_title("Report [Read-only] - Word"));
    }

    #[test]
    fn refuses_what_word_cant_convert() {
        let never = AtomicBool::new(false);
        assert!(convert(Path::new("a.png"), Format::Pdf, &never, &mut |_| {}).is_err());
        assert!(convert(Path::new("a.docx"), Format::Docx, &never, &mut |_| {}).is_err());
    }

    // The tests below drive the real Word and skip when it isn't installed. They share
    // Word, so they take turns.
    static WORD_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn test_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("filebeaver-word-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A small real DOCX with two paragraphs and a table.
    fn write_docx(path: &Path) {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        let o = zip::write::SimpleFileOptions::default();
        zip.start_file("[Content_Types].xml", o).unwrap();
        zip.write_all(br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#).unwrap();
        zip.start_file("_rels/.rels", o).unwrap();
        zip.write_all(br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#).unwrap();
        zip.start_file("word/document.xml", o).unwrap();
        zip.write_all(br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Beaver round trip</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>Cell one</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Cell two</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p><w:r><w:t>The quick brown fox jumps over the lazy dog.</w:t></w:r></w:p></w:body></w:document>"#).unwrap();
        zip.finish().unwrap();
    }

    fn document_text(docx: &[u8]) -> String {
        use std::io::Read;
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(docx)).unwrap();
        let mut s = String::new();
        zip.by_name("word/document.xml").unwrap().read_to_string(&mut s).unwrap();
        s
    }

    /// `licence`: false for tests that never save, so they also run on unactivated Word.
    fn skip_unless_word(licence: bool) -> bool {
        // Each test starts Word afresh, even if an earlier one found it unactivated.
        UNACTIVATED.store(false, SeqCst);
        CHECK_LICENCE.store(licence, SeqCst);
        if !installed() {
            eprintln!("skipped: Microsoft Word isn't installed");
        }
        installed()
    }

    #[test]
    fn round_trip_through_word() {
        let _turn = WORD_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        if !skip_unless_word(true) {
            return;
        }
        let dir = test_dir("roundtrip");
        let src = dir.join("Round trip.docx");
        write_docx(&src);
        let original = std::fs::read(&src).unwrap();
        let never = AtomicBool::new(false);
        let before = word_pids();
        let mut stages = Vec::new();
        let pdf = match convert(&src, Format::Pdf, &never, &mut |s| stages.push(s.to_string())) {
            Err(e) if e.to_string() == UNACTIVATED_MESSAGE => {
                eprintln!("skipped: Word on this PC isn't activated");
                assert_eq!(word_pids(), before, "a Word was left running");
                return;
            }
            r => r.unwrap(),
        };
        assert!(pdf.starts_with(b"%PDF"));
        assert!(stages.iter().any(|s| s == "Saving PDF"), "{stages:?}");
        assert_eq!(std::fs::read(&src).unwrap(), original, "the original changed");

        let pdf_path = dir.join("Round trip.pdf");
        std::fs::write(&pdf_path, &pdf).unwrap();
        let docx = convert(&pdf_path, Format::Docx, &never, &mut |_| {}).unwrap();
        let text = document_text(&docx);
        assert!(text.contains("Beaver round trip"), "the text isn't editable text in the DOCX");
        assert!(text.contains("<w:tbl"), "the table didn't come back as a table");
        assert_eq!(word_pids(), before, "a Word was left running");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn corrupt_document_fails_cleanly_without_leaving_word() {
        let _turn = WORD_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        if !skip_unless_word(false) {
            return;
        }
        let dir = test_dir("corrupt");
        let src = dir.join("Broken.docx");
        // Starts like a zip, so it gets past the quick check and Word has to deal with it.
        let mut bytes = vec![b'P', b'K', 3, 4];
        bytes.extend((0..4000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8));
        std::fs::write(&src, bytes).unwrap();
        let before = word_pids();
        let started = Instant::now();
        let err = convert(&src, Format::Pdf, &AtomicBool::new(false), &mut |_| {}).unwrap_err().to_string();
        eprintln!("corrupt file: {err} ({:?})", started.elapsed());
        assert!(!err.is_empty());
        assert!(started.elapsed() < Duration::from_secs(60), "took {:?}", started.elapsed());
        assert_eq!(word_pids(), before, "a Word was left running");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A stuck Word is killed, and only the one we started: a Word the user opened survives.
    #[test]
    fn watchdog_kills_only_its_own_word() {
        let _turn = WORD_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        if !skip_unless_word(false) {
            return;
        }
        // Stand-in for the user's own Word: started directly, not through COM.
        let mut users = std::process::Command::new(server_exe().unwrap()).arg("/q").spawn().unwrap();
        std::thread::sleep(Duration::from_secs(3));
        let dir = test_dir("watchdog");
        let src = dir.join("Slow.docx");
        write_docx(&src);
        let before = word_pids();
        // Far too little time for Word to even start.
        let err = run_watched(&src, &dir.join("out.pdf"), Format::Pdf, Duration::from_millis(300), &AtomicBool::new(false), &mut |_| {}).unwrap_err();
        assert!(err.to_string().contains("too long"), "{err}");
        std::thread::sleep(Duration::from_secs(1));
        assert_eq!(word_pids(), before, "our Word wasn't stopped");
        assert!(users.try_wait().unwrap().is_none(), "the user's Word was closed");
        let _ = users.kill();
        let _ = users.wait();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cancel_stops_word() {
        let _turn = WORD_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        if !skip_unless_word(false) {
            return;
        }
        let dir = test_dir("cancel");
        let src = dir.join("Cancel me.docx");
        write_docx(&src);
        let before = word_pids();
        let cancel = AtomicBool::new(false);
        let err = convert(&src, Format::Pdf, &cancel, &mut |s| {
            if s == "Opening in Word" {
                cancel.store(true, SeqCst);
            }
        })
        .unwrap_err();
        assert_eq!(err.to_string(), "cancelled");
        assert_eq!(word_pids(), before, "a Word was left running");
        let _ = std::fs::remove_dir_all(dir);
    }
}
