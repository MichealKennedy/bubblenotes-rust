//! The Options window: the app's settings.
//!
//! Every control changes the settings at once (`App::update_settings`);
//! what follows from them (the global shortcuts, starting with Windows) is
//! applied by `system`, which watches the settings.

use nsh_base::{number_input, theme};
use nsh_core::element::desktop::{OpenChange, window};
use nsh_core::prelude::*;

use crate::app::App;
use crate::config_panel::{committed_input, row, small_button, switch_row, theme_swatches};

/// The window, open while `App::options_open`.
pub fn options_window(app: App) -> impl IntoElement {
    window()
        .id("options")
        .title("Options")
        .default_size(size(px(520.0), px(560.0)))
        .open(move || app.options_open.get())
        .on_open_change(move |c: &OpenChange| {
            if !c.open {
                app.options_open.set(false);
            }
        })
        .content(move || {
            view().flex_1().bg(move || theme().background).child(
                scroll().flex_1().child(
                    view()
                        .p(px(20.0))
                        .gap(px(14.0))
                        .child(folder_card(app))
                        .child(general_card(app))
                        .child(shortcuts_card(app))
                        .child(new_notes_card(app))
                        .child(about()),
                ),
            )
        })
}

/// A titled group of settings.
fn card(title: &'static str, content: impl IntoElement + 'static) -> impl IntoElement {
    view()
        .gap(px(6.0))
        .child(text(title).text_size(px(12.0)).font_weight(FontWeight::SEMIBOLD).text_color(move || theme().muted_foreground))
        .child(
            view()
                .px(px(14.0))
                .py(px(8.0))
                .gap(px(2.0))
                .rounded(px(12.0))
                .border_1()
                .border_color(move || theme().border)
                .bg(move || theme().card)
                .child(content),
        )
}

fn hint(words: &'static str) -> impl IntoElement {
    text(words).text_size(px(12.0)).text_color(move || theme().muted_foreground)
}

fn folder_card(app: App) -> impl IntoElement {
    let root = app.vault.get().root().to_path_buf();
    let shown = root.display().to_string();
    card(
        "NOTES FOLDER",
        view()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .min_h(px(36.0))
            .child(view().flex_1().child(text(shown).truncate().text_size(px(13.0)).text_color(move || theme().foreground)))
            .child(small_button(|| "Open folder".into(), move || {
                let path = root.clone();
                app.owner.run(|| {
                    let _ = spawn(async move {
                        if let Err(e) = open_path(path).await {
                            app.problem.set(Some(format!("Couldn't open the notes folder: {e}")));
                        }
                    });
                });
            })),
    )
}

fn general_card(app: App) -> impl IntoElement {
    let settings = app.settings;
    card(
        "GENERAL",
        view()
            .child(switch_row("Start with Windows", move || settings.with(|s| s.start_with_windows), move |on| {
                app.update_settings(|s| s.start_with_windows = on)
            }))
            .child(switch_row("Snap notes to screen edges", move || settings.with(|s| s.snap_to_edges), move |on| {
                app.update_settings(|s| s.snap_to_edges = on)
            }))
            .child(row(
                "Keep deleted notes for (days)",
                number_input()
                    .min(1.0)
                    .max(365.0)
                    .step(1.0)
                    .accessibility_label("Days in the trash")
                    .value(move || Some(settings.with(|s| s.trash_days) as f64))
                    .on_change(move |v: &Option<f64>| {
                        if let Some(days) = v {
                            app.update_settings(|s| s.trash_days = days.clamp(1.0, 365.0) as u32);
                        }
                    })
                    .input_class("w-16 h-7 px-2 rounded-md border border-input bg-background text-sm text-foreground")
                    .decrement_button(text("−").text_size(px(11.0)))
                    .increment_button(text("+").text_size(px(11.0)))
                    .decrement_button_class("hidden")
                    .increment_button_class("hidden"),
            ))
            .child(row(
                "Selected text darkness (%)",
                number_input()
                    .min(10.0)
                    .max(80.0)
                    .step(5.0)
                    .accessibility_label("How dark selected text is")
                    .value(move || Some(settings.with(|s| s.selection_strength) as f64))
                    .on_change(move |v: &Option<f64>| {
                        if let Some(percent) = v {
                            app.update_settings(|s| s.selection_strength = percent.clamp(10.0, 80.0) as u32);
                        }
                    })
                    .input_class("w-16 h-7 px-2 rounded-md border border-input bg-background text-sm text-foreground")
                    .decrement_button(text("−").text_size(px(11.0)))
                    .increment_button(text("+").text_size(px(11.0)))
                    .decrement_button_class("hidden")
                    .increment_button_class("hidden"),
            )),
    )
}

fn shortcuts_card(app: App) -> impl IntoElement {
    let settings = app.settings;
    let shortcut = |label: &'static str, get: fn(&crate::model::Settings) -> String, set: fn(&mut crate::model::Settings, String)| {
        row(
            label,
            committed_input(move || settings.with(get), move |t| app.update_settings(|s| set(s, t.trim().to_lowercase())), "none").w(px(160.0)),
        )
    };
    card(
        "GLOBAL SHORTCUTS",
        view()
            .child(shortcut("New note", |s| s.hotkey_new_note.clone(), |s, t| s.hotkey_new_note = t))
            .child(shortcut("New note from clipboard", |s| s.hotkey_clipboard_note.clone(), |s, t| s.hotkey_clipboard_note = t))
            .child(shortcut("Show or hide all notes", |s| s.hotkey_toggle_notes.clone(), |s, t| s.hotkey_toggle_notes = t))
            .child(view().py(px(6.0)).child(hint("Keys joined by +, such as ctrl+alt+n. Leave a field empty to turn its shortcut off."))),
    )
}

fn new_notes_card(app: App) -> impl IntoElement {
    let settings = app.settings;
    card(
        "NEW NOTES",
        view().gap(px(10.0)).py(px(6.0)).child(hint("The look new notes start with.")).child(theme_swatches(
            app,
            move || settings.with(|s| s.default_look.clone()),
            move |look| app.update_settings(|s| s.default_look = look),
        )),
    )
}

fn about() -> impl IntoElement {
    view().items_center().py(px(6.0)).child(hint(concat!("Bubble Notes ", env!("CARGO_PKG_VERSION"), " · floating notes for the desktop, made with NSH")))
}
