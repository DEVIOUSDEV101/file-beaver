//! Crash safety. Reports stay on this PC; nothing is ever sent anywhere.
//! - Rust panics: a text report with the message and backtrace.
//! - Native crashes (e.g. inside a Windows codec): a minidump plus a short text report.
//! - A "running" marker tells the next launch whether the last session ended cleanly,
//!   so it can say so and point at the report.

use std::path::PathBuf;
use std::sync::OnceLock;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GENERIC_WRITE};
use windows::Win32::Storage::FileSystem::{CreateFileW, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_NONE};
use windows::Win32::System::Diagnostics::Debug::{
    MiniDumpNormal, MiniDumpWriteDump, SetUnhandledExceptionFilter, EXCEPTION_POINTERS, MINIDUMP_EXCEPTION_INFORMATION,
};
use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId};

/// Where reports go: %LOCALAPPDATA%\app.filebeaver.desktop\crashes
pub fn dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("app.filebeaver.desktop").join("crashes")
}

fn marker() -> PathBuf {
    dir().join("running")
}

fn stamp() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("{secs}-{}", std::process::id())
}

/// Minidump path, prepared up front: inside a native crash we avoid allocating.
static DUMP_PATH: OnceLock<Vec<u16>> = OnceLock::new();

/// Installs the panic hook and native crash handler and records that a session is running.
/// Returns true if the previous session didn't exit cleanly.
pub fn install() -> bool {
    let _ = std::fs::create_dir_all(dir());
    let crashed_before = marker().exists();
    let _ = std::fs::write(marker(), std::process::id().to_string());

    let dump: Vec<u16> = dir().join(format!("native-{}.dmp", stamp())).as_os_str().encode_wide_null();
    let _ = DUMP_PATH.set(dump);

    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
        let report = format!(
            "File Beaver {} crash report (Rust panic)\nthread: {thread}\nos: {}\n\n{info}\n\nbacktrace:\n{}\n",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::backtrace::Backtrace::force_capture()
        );
        let _ = std::fs::write(dir().join(format!("panic-{}.txt", stamp())), report);
        log::error!("panic on thread {thread}: {info}");
        default(info);
    }));

    unsafe {
        SetUnhandledExceptionFilter(Some(native_crash));
    }
    crashed_before
}

unsafe extern "system" fn native_crash(info: *const EXCEPTION_POINTERS) -> i32 {
    if let Some(path) = DUMP_PATH.get() {
        unsafe {
            if let Ok(file) = CreateFileW(PCWSTR(path.as_ptr()), GENERIC_WRITE.0, FILE_SHARE_NONE, None, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, None) {
                let exc = MINIDUMP_EXCEPTION_INFORMATION { ThreadId: GetCurrentThreadId(), ExceptionPointers: info as *mut _, ClientPointers: false.into() };
                let _ = MiniDumpWriteDump(GetCurrentProcess(), GetCurrentProcessId(), file, MiniDumpNormal, Some(&exc), None, None);
                let _ = CloseHandle(file);
            }
        }
    }
    0 // EXCEPTION_CONTINUE_SEARCH: let Windows end the process as usual
}

/// Call when the app exits normally.
pub fn clean_exit() {
    let _ = std::fs::remove_file(marker());
}

/// Newest report file, if any.
pub fn latest_report() -> Option<PathBuf> {
    std::fs::read_dir(dir())
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "txt" || x == "dmp"))
        .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
}

/// Runs `f`; a panic becomes None (and a log line) instead of unwinding into Windows,
/// where it would abort the whole app.
pub fn guarded<T>(what: &str, f: impl FnOnce() -> T) -> Option<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(v) => Some(v),
        Err(_) => {
            log::error!("recovered from a crash in {what}");
            None
        }
    }
}

trait WideNull {
    fn encode_wide_null(&self) -> Vec<u16>;
}

impl WideNull for std::ffi::OsStr {
    fn encode_wide_null(&self) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        self.encode_wide().chain(std::iter::once(0)).collect()
    }
}
