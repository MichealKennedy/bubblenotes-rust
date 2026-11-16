//! What the app sets up with the system from its settings: the global
//! shortcuts and starting with Windows.
//!
//! A development run (`BUBBLENOTES_DIR` set) leaves both alone: several
//! instances would fight over the shortcuts (`BUBBLENOTES_HOTKEYS=1` lets
//! one have them), and it must not register itself to start with Windows.

use nsh_core::desktop::Shortcut;
use nsh_core::element::desktop::{register_global_shortcut, unregister_global_shortcut};
use nsh_core::prelude::*;

use crate::app::{App, NoteCommand};
use crate::formats::Format;
use crate::model::Settings;

/// What a global shortcut does.
#[derive(Clone, Copy)]
enum Hotkey {
    NewNote,
    ClipboardNote,
    ToggleNotes,
    Search,
}

const HOTKEYS: [Hotkey; 4] = [Hotkey::NewNote, Hotkey::ClipboardNote, Hotkey::ToggleNotes, Hotkey::Search];

impl Hotkey {
    /// Its keystroke in the settings ("" for none).
    fn keystroke(self, settings: &Settings) -> &str {
        match self {
            Hotkey::NewNote => &settings.hotkey_new_note,
            Hotkey::ClipboardNote => &settings.hotkey_clipboard_note,
            Hotkey::ToggleNotes => &settings.hotkey_toggle_notes,
            Hotkey::Search => &settings.hotkey_search,
        }
    }

    fn run(self, app: App) {
        match self {
            Hotkey::NewNote => {
                let note = app.new_note("");
                app.show_on_desktop(&note);
                app.command.set(Some((note.id.clone(), NoteCommand::Focus)));
            }
            Hotkey::ClipboardNote => app.owner.run(|| {
                let _ = spawn(async move {
                    let text = clipboard_string().await;
                    clipboard_note(app, &text);
                });
            }),
            Hotkey::ToggleNotes => app.update_settings(|s| s.notes_shown = !s.notes_shown),
            Hotkey::Search => app.search_open.update(|open| *open = !*open),
        }
    }
}

/// Registers the global shortcuts and keeps them, and the start-with-
/// Windows entry, in line with the settings as they change. Call once.
pub fn install(app: App) {
    let dev = std::env::var_os("BUBBLENOTES_DIR").is_some();
    if dev && std::env::var("BUBBLENOTES_HOTKEYS").as_deref() != Ok("1") {
        eprintln!("system: development run (BUBBLENOTES_DIR): no global shortcuts (BUBBLENOTES_HOTKEYS=1 for them)");
    } else {
        install_hotkeys(app);
    }
    if dev {
        eprintln!("system: development run (BUBBLENOTES_DIR): start with Windows left alone");
    } else {
        install_startup(app);
    }
}

fn install_hotkeys(app: App) {
    let registered: Stored<[String; 4]> = stored(Default::default());
    effect(move || {
        let wanted: [String; 4] = app.settings.with(|s| HOTKEYS.map(|h| h.keystroke(s).trim().to_string()));
        let before = registered.get();
        if wanted == before {
            return;
        }
        untracked(|| {
            // Everything that changed goes first, so a keystroke moving from
            // one action to another isn't taken away right after.
            for (old, new) in before.iter().zip(&wanted) {
                if old != new && !old.is_empty() {
                    unregister_global_shortcut(old);
                }
            }
            for (index, hotkey) in HOTKEYS.into_iter().enumerate() {
                let key = &wanted[index];
                if key == &before[index] || key.is_empty() {
                    continue;
                }
                if wanted[..index].contains(key) {
                    app.problem.set(Some(format!("The shortcut “{key}” is set for two things: only the first gets it.")));
                    continue;
                }
                register(app, hotkey, key);
            }
        });
        registered.replace(wanted);
    });
}

/// Registers `key` for `hotkey`, or says what's wrong with it.
fn register(app: App, hotkey: Hotkey, key: &str) {
    // `register_global_shortcut` panics on a keystroke it can't read.
    if let Err(e) = Shortcut::parse(key, cfg!(target_os = "macos")) {
        app.problem.set(Some(format!("The shortcut “{key}” can't be used: {e}")));
        return;
    }
    let key = key.to_string();
    // Under the app's owner, so it lasts until it's unregistered.
    app.owner.run(|| {
        let answer = register_global_shortcut(&key, move || hotkey.run(app));
        let _ = spawn(async move {
            // `false` also comes when it's unregistered: only complain while
            // it's still the one wanted.
            if !answer.await && app.settings.with_untracked(|s| hotkey.keystroke(s).trim() == key) {
                app.problem.set(Some(format!("The shortcut “{key}” couldn't be registered: another app may be using it.")));
            }
        });
    });
}

/// A new note on the desktop holding `text` (read as Markdown), titled by
/// its first line.
fn clipboard_note(app: App, text: &str) {
    let note = app.new_note("");
    match Format::Markdown.parse(&app.kit.get().schema, text) {
        Ok(doc) => app.set_doc(&note, doc),
        Err(e) => app.problem.set(Some(format!("Couldn't read the clipboard's text: {e}"))),
    }
    let title = clipboard_title(text);
    if !title.is_empty() {
        app.update(&note, |m| m.title = title);
    }
    app.show_on_desktop(&note);
    app.command.set(Some((note.id.clone(), NoteCommand::Focus)));
}

/// The first line with text, without Markdown's heading and list marks, at
/// most 40 characters.
fn clipboard_title(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default();
    let line = line.trim_start_matches(['#', '>', '-', '*', ' ']).trim();
    line.chars().take(40).collect::<String>().trim_end().to_string()
}

// ----- Start with Windows --------------------------------------------------------

fn install_startup(app: App) {
    let last: Stored<Option<bool>> = stored(None);
    effect(move || {
        let want = app.settings.with(|s| s.start_with_windows);
        if last.get() == Some(want) {
            return;
        }
        // (At the start, or the user just changed the setting.)
        let starting = last.replace(Some(want)).is_none();
        if let Err(e) = startup::sync(want, starting) {
            untracked(|| app.problem.set(Some(format!("Couldn't change starting with Windows: {e}"))));
        }
    });
}

#[cfg(target_os = "windows")]
mod startup {
    use nsh_platform_windows::windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
    use nsh_platform_windows::windows::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
    };
    use nsh_platform_windows::windows::core::{PCWSTR, w};

    const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
    const VALUE: PCWSTR = w!("Bubble Notes");

    /// Writes or removes the app's entry in the user's Run key, when it
    /// isn't already as wanted.
    /// `starting`: the app has just started, and an entry for another copy
    /// of the app (this one is a portable copy, say) is left alone: only
    /// the user turning the setting off removes that.
    pub fn sync(want: bool, starting: bool) -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let command = format!("\"{}\"", exe.display());
        let now = read()?;
        match (want, now) {
            (true, Some(now)) if now == command => Ok(()),
            (true, _) => write(&command),
            (false, Some(now)) if starting && now != command => Ok(()),
            (false, Some(_)) => remove(),
            (false, None) => Ok(()),
        }
    }

    fn check(code: WIN32_ERROR) -> Result<(), String> {
        if code == ERROR_SUCCESS { Ok(()) } else { Err(windows_message(code)) }
    }

    fn windows_message(code: WIN32_ERROR) -> String {
        std::io::Error::from_raw_os_error(code.0 as i32).to_string()
    }

    /// The entry's command, `None` when there's none.
    fn read() -> Result<Option<String>, String> {
        let mut buffer = vec![0u16; 1024];
        let mut bytes = (buffer.len() * 2) as u32;
        // SAFETY: `buffer` holds `bytes` bytes for the call to fill.
        let code = unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE, RRF_RT_REG_SZ, None, Some(buffer.as_mut_ptr().cast()), Some(&mut bytes)) };
        if code == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        check(code)?;
        let chars = (bytes as usize / 2).min(buffer.len());
        let text = String::from_utf16_lossy(&buffer[..chars]);
        Ok(Some(text.trim_end_matches('\0').to_string()))
    }

    fn write(command: &str) -> Result<(), String> {
        let wide: Vec<u16> = command.encode_utf16().chain([0]).collect();
        // SAFETY: `wide` is a NUL-terminated UTF-16 string of the length given, in bytes.
        let code = unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE, REG_SZ.0, Some(wide.as_ptr().cast()), (wide.len() * 2) as u32) };
        check(code)
    }

    fn remove() -> Result<(), String> {
        // SAFETY: constant, NUL-terminated key and value names.
        let code = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE) };
        if code == ERROR_FILE_NOT_FOUND { Ok(()) } else { check(code) }
    }
}

#[cfg(not(target_os = "windows"))]
mod startup {
    /// Only Windows has a Run key.
    pub fn sync(_want: bool) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_titles() {
        assert_eq!(clipboard_title("\n\n  # Shopping list  \n- milk"), "Shopping list");
        assert_eq!(clipboard_title(""), "");
        let long = "é".repeat(50);
        assert_eq!(clipboard_title(&long).chars().count(), 40);
    }

    #[test]
    fn keystrokes_are_checked_before_registering() {
        assert!(Shortcut::parse("ctrl+alt+n", false).is_ok());
        assert!(Shortcut::parse("ctrl+alt+", false).is_err());
        assert!(Shortcut::parse("ctrl+banana", false).is_err());
    }
}
