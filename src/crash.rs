//! The crash log. When the app fails (a panic on any thread, or on Windows
//! a fault in native code), what happened is added to
//! `.bubblenotes/crash.log` in the notes folder, the user is told where,
//! and the app ends: a half-dead app in the tray is worse than none, and
//! the notes are saved as they're typed.
//!
//! Each entry has the time, the app's version, what failed and where, and
//! the calls that led there.

use std::io::Write;
use std::path::PathBuf;

use crate::store::{META_DIR, Vault};

/// The log's file name, in the notes folder's `.bubblenotes`.
pub const FILE: &str = "crash.log";
/// The log is started again past this size: it keeps the latest failures.
const MOST: u64 = 512 * 1024;

/// Where the log is written.
pub fn path() -> PathBuf {
    Vault::chosen_root().join(META_DIR).join(FILE)
}

/// Starts logging failures. Call once, before the app runs.
pub fn install() {
    let before = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let what = info.payload().downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_else(|| "(no message)".into());
        let at = info.location().map_or_else(|| "an unknown place".to_string(), |l| format!("{}:{}:{}", l.file(), l.line(), l.column()));
        let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
        let calls = std::backtrace::Backtrace::force_capture().to_string();
        let written = write(&entry(&now(), &format!("panic in thread '{thread}' at {at}"), &what, &calls));
        // (What a console shows, as before.)
        before(info);
        end(written);
    }));
    native::install();
}

/// A log entry.
fn entry(when: &str, kind: &str, what: &str, calls: &str) -> String {
    let mut out = format!("==== {when} · Bubble Notes {} ====\n{kind}\n{what}\n", env!("CARGO_PKG_VERSION"));
    if !calls.trim().is_empty() {
        out.push_str("calls:\n");
        out.push_str(calls.trim_end());
        out.push('\n');
    }
    out.push('\n');
    out
}

fn now() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Adds `entry` to the log (in the temporary folder when the notes folder
/// can't be written). Where it went.
fn write(entry: &str) -> Option<PathBuf> {
    let add = |path: PathBuf| {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok()?;
        }
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > MOST) {
            let _ = std::fs::remove_file(&path);
        }
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&path).ok()?;
        file.write_all(entry.as_bytes()).ok()?;
        Some(path)
    };
    add(path()).or_else(|| add(std::env::temp_dir().join("bubblenotes-crash.log")))
}

/// Tells the user and ends the app. (A development run just ends: nobody
/// is there to press OK.)
fn end(log: Option<PathBuf>) -> ! {
    if std::env::var_os("BUBBLENOTES_DIR").is_none() {
        let text = match &log {
            Some(path) => format!("Bubble Notes ran into a problem and has to close.\n\nYour notes are saved as you type. What went wrong was written to:\n{}", path.display()),
            None => "Bubble Notes ran into a problem and has to close.\n\nYour notes are saved as you type.".to_string(),
        };
        native::tell(&text);
    }
    std::process::exit(101)
}

#[cfg(target_os = "windows")]
mod native {
    use nsh_platform_windows::windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MessageBoxW};
    use nsh_platform_windows::windows::core::PCWSTR;

    /// The start of Windows' record of a fault (`EXCEPTION_RECORD`).
    #[repr(C)]
    struct Record {
        code: u32,
        flags: u32,
        next: *const Record,
        address: *const core::ffi::c_void,
    }

    /// `EXCEPTION_POINTERS`.
    #[repr(C)]
    struct Pointers {
        record: *const Record,
        context: *const core::ffi::c_void,
    }

    type Filter = unsafe extern "system" fn(*const Pointers) -> i32;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetUnhandledExceptionFilter(filter: Option<Filter>) -> Option<Filter>;
    }

    /// Faults in native code (an access violation, say) are logged too.
    pub fn install() {
        // SAFETY: the filter is a plain function that lives as long as the
        // program.
        unsafe { SetUnhandledExceptionFilter(Some(fault)) };
    }

    unsafe extern "system" fn fault(info: *const Pointers) -> i32 {
        // SAFETY: Windows hands a valid record for the fault being handled.
        let (code, address) = unsafe { info.as_ref().and_then(|i| i.record.as_ref()).map_or((0, 0), |r| (r.code, r.address as usize)) };
        let calls = std::backtrace::Backtrace::force_capture().to_string();
        let written = super::write(&super::entry(&super::now(), "fault in native code", &format!("code {code:#010x} at address {address:#x}"), &calls));
        super::end(written)
    }

    pub fn tell(text: &str) {
        let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
        let (text, title) = (wide(text), wide("Bubble Notes"));
        // SAFETY: both strings end in a zero and outlive the call.
        unsafe { MessageBoxW(None, PCWSTR(text.as_ptr()), PCWSTR(title.as_ptr()), MB_OK | MB_ICONERROR | MB_SETFOREGROUND) };
    }
}

#[cfg(not(target_os = "windows"))]
mod native {
    pub fn install() {}
    pub fn tell(_: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entry_says_when_what_and_where() {
        let text = entry("2026-11-15 21:00:00", "panic in thread 'main' at src/app.rs:1:1", "index out of bounds", "0: a\n1: b\n");
        assert!(text.starts_with(&format!("==== 2026-11-15 21:00:00 · Bubble Notes {} ====\n", env!("CARGO_PKG_VERSION"))));
        assert!(text.contains("panic in thread 'main' at src/app.rs:1:1\nindex out of bounds\ncalls:\n0: a\n1: b\n"));
        assert!(text.ends_with("\n\n"));
        // Without calls, no heading for them.
        assert!(!entry("t", "k", "w", "  ").contains("calls:"));
    }
}
