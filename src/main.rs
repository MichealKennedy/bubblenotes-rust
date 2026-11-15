//! `cargo run`: starts Bubble Notes on this machine's native backend
//! (Win32 on Windows, GTK 4 on Linux, AppKit on macOS). It starts in the
//! tray, with no window of its own showing.

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(target_os = "linux")]
use nsh_platform_linux as platform;
#[cfg(target_os = "macos")]
use nsh_platform_macos as platform;
#[cfg(target_os = "windows")]
use nsh_platform_windows as platform;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn main() -> impl std::process::Termination {
    env_logger::init();
    // The main window is only a host for the tray and the note windows.
    let mut main_window = nsh_core::desktop::MainWindowOptions::default();
    main_window.default_visible = Some(false);
    platform::run_app(
        platform::AppSource::Rust(platform::Root::of::<nsh_app_bubblenotes::BubbleNotes>().main_window(main_window)),
        platform::AppOptions {
            title: "Bubble Notes".to_owned(),
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            app_id: "dev.bubblenotes.app".to_owned(),
            #[cfg(target_os = "windows")]
            skip_taskbar: true,
            #[cfg(target_os = "windows")]
            // A notes folder of its own (development) runs beside the real app.
            single_instance: std::env::var_os("BUBBLENOTES_DIR").is_none() || std::env::var_os("BUBBLENOTES_SINGLE").is_some(),
            ..Default::default()
        },
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("Run this binary on macOS, Windows or Linux; use `nsh run --target ...` elsewhere.");
