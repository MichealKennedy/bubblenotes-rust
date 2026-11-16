//! What the app remembers about each note besides its text, and the app's
//! settings. All of it is plain data saved as JSON (`store`); every field
//! has a default, so older files keep loading as fields are added.

use serde::{Deserialize, Serialize};

/// A colour as `0xRRGGBB`.
pub type Rgb = u32;

/// A note's place on the screen, in points (screen coordinates).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Default for Rect {
    fn default() -> Self {
        Self { x: 120.0, y: 120.0, w: 280.0, h: 260.0 }
    }
}

/// How a note looks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Look {
    /// Background colour; with `background_to`, the top of a gradient.
    pub background: Rgb,
    /// The gradient's bottom colour, when the background is a gradient.
    pub background_to: Option<Rgb>,
    /// 1.0 is solid; less lets the desktop show through the background
    /// (the text stays solid).
    pub opacity: f32,
    pub corner_radius: f32,
    pub show_title: bool,
    pub title_color: Rgb,
    pub title_size: f32,
    pub title_bold: bool,
    pub body_color: Rgb,
    pub body_size: f32,
    /// A font family name; empty for the app's default.
    pub body_font: String,
    pub always_on_top: bool,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            background: 0xFDE68A,
            background_to: None,
            opacity: 1.0,
            corner_radius: 14.0,
            show_title: true,
            title_color: 0x1F2937,
            title_size: 13.0,
            title_bold: true,
            body_color: 0x1F2937,
            body_size: 14.0,
            body_font: String::new(),
            always_on_top: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BubbleShape {
    /// A glossy 3D ball.
    #[default]
    Sphere,
    Circle,
    Square,
    Triangle,
    /// The label drawn large, with no shape behind it.
    Emoji,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BubbleOpenOn {
    #[default]
    Hover,
    Click,
}

/// Where the note opens relative to its bubble.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BubbleDirection {
    #[default]
    Bottom,
    Right,
    Top,
    Left,
    /// Centered over the bubble.
    OnTop,
}

/// A note shown as a small bubble that opens into the note.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Bubble {
    pub enabled: bool,
    /// Width and height, in points.
    pub size: f32,
    pub shape: BubbleShape,
    pub color: Rgb,
    /// The second colour of a gradient, when there is one.
    pub color_to: Option<Rgb>,
    /// Text or an emoji drawn inside.
    pub label: String,
    pub label_color: Rgb,
    pub border_color: Rgb,
    pub border_width: f32,
    pub open_on: BubbleOpenOn,
    pub direction: BubbleDirection,
    /// Roll the note open and shut instead of showing it at once.
    pub animate: bool,
    pub animate_ms: u32,
    /// Close when the pointer leaves; otherwise it stays open until the
    /// bubble is clicked or wheeled shut.
    pub close_on_leave: bool,
    /// The wheel over the bubble opens (up) and closes (down) it.
    pub wheel: bool,
    /// A small badge on the bubble: the time to the note's alarm, or how
    /// many of its checklist items are done.
    pub badge: bool,
    /// Where the bubble sits (its top left corner), in points.
    pub x: f32,
    pub y: f32,
}

impl Default for Bubble {
    fn default() -> Self {
        Self {
            enabled: false,
            size: 56.0,
            shape: BubbleShape::Sphere,
            color: 0xE0457B,
            color_to: None,
            label: "💬".into(),
            label_color: 0xFFFFFF,
            border_color: 0xFFFFFF,
            border_width: 0.0,
            open_on: BubbleOpenOn::Hover,
            direction: BubbleDirection::Bottom,
            animate: true,
            animate_ms: 180,
            close_on_leave: true,
            wheel: true,
            badge: true,
            x: 120.0,
            y: 120.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Repeat {
    #[default]
    None,
    Hourly,
    Daily,
    Weekdays,
    Weekly,
    Monthly,
    Yearly,
}

/// A screen edge a note is docked to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Edge {
    Left,
    Right,
    Top,
}

/// An alarm: at `at` the note shows itself, with a notification and a sound.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Alarm {
    /// Milliseconds since the Unix epoch.
    pub at: f64,
    #[serde(default)]
    pub repeat: Repeat,
}

/// Everything about a note except its text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NoteMeta {
    /// Never changes, also when the note is renamed or moved.
    pub id: String,
    pub title: String,
    /// The category's path (`"Work/Ideas"`), empty for none. It's the
    /// note's folder in the notes folder.
    pub category: String,
    /// Its file in the notes folder, as a relative path with `/` (the store
    /// sets and changes it: `Vault::place`).
    pub file: String,
    /// Milliseconds since the Unix epoch.
    pub created: f64,
    pub modified: f64,
    /// Floating on the desktop (else only in the Notes Manager).
    pub on_desktop: bool,
    pub rect: Rect,
    /// Only the title bar shows.
    pub rolled_up: bool,
    /// The text can't be edited and the note can't be moved.
    pub locked: bool,
    /// Clicks go through the note to what's under it (it can only be
    /// looked at), until Ctrl+Alt is held.
    pub ghost: bool,
    /// Shown only while this program is in front (its file name, in lower
    /// case: `chrome.exe`); `None`: always.
    pub pinned_app: Option<String>,
    /// Mostly see-through (`idle_opacity`) until the pointer is over it or
    /// it's being typed in.
    pub fade_idle: bool,
    /// How solid a faded note is (0 to 1).
    pub idle_opacity: f32,
    /// Tucked away at this edge of its screen, as a thin strip that slides
    /// out into the note under the pointer. `rect` is the note slid out.
    pub docked: Option<Edge>,
    pub look: Look,
    pub bubble: Bubble,
    /// Hidden until then (milliseconds since the Unix epoch).
    pub sleep_until: Option<f64>,
    pub alarm: Option<Alarm>,
    /// When it was deleted; it stays in the trash for a while.
    pub trashed: Option<f64>,
}

impl Default for NoteMeta {
    fn default() -> Self {
        Self {
            id: String::new(),
            title: "Untitled".into(),
            category: String::new(),
            file: String::new(),
            created: 0.0,
            modified: 0.0,
            on_desktop: true,
            rect: Rect::default(),
            rolled_up: false,
            locked: false,
            ghost: false,
            pinned_app: None,
            fade_idle: false,
            idle_opacity: 0.3,
            docked: None,
            look: Look::default(),
            bubble: Bubble::default(),
            sleep_until: None,
            alarm: None,
            trashed: None,
        }
    }
}

impl NoteMeta {
    /// Asleep at `now` (milliseconds since the Unix epoch).
    pub fn asleep(&self, now: f64) -> bool {
        self.sleep_until.is_some_and(|until| until > now)
    }
}

/// A named look, to apply to a note or to new notes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    pub look: Look,
}

/// The built-in themes: soft colours, readable dark text.
pub fn builtin_themes() -> Vec<Theme> {
    let soft = |name: &str, background: Rgb| Theme {
        name: name.into(),
        look: Look { background, ..Look::default() },
    };
    let mut themes = vec![
        soft("Butter", 0xFDE68A),
        soft("Sky", 0xBAE0FD),
        soft("Mint", 0xBBF0D0),
        soft("Rose", 0xFBC8D4),
        soft("Lavender", 0xDDD0FB),
        soft("Peach", 0xFDD3B0),
        soft("Paper", 0xFAFAF7),
    ];
    themes.push(Theme {
        name: "Graphite".into(),
        look: Look { background: 0x27272A, title_color: 0xF4F4F5, body_color: 0xE4E4E7, ..Look::default() },
    });
    themes.push(Theme {
        name: "Glossy Blue".into(),
        look: Look { background: 0x8EC5F0, background_to: Some(0x2E8FE0), ..Look::default() },
    });
    themes
}

/// The app's settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The look of new notes.
    pub default_look: Look,
    /// The user's own themes (after the built-in ones).
    pub themes: Vec<Theme>,
    /// Categories with no notes yet (those with notes are their folders).
    pub categories: Vec<String>,
    /// Global shortcut: a new note, from anywhere. Empty for none.
    pub hotkey_new_note: String,
    /// Global shortcut: a new note holding the clipboard's text.
    pub hotkey_clipboard_note: String,
    /// Global shortcut: show or hide every note.
    pub hotkey_toggle_notes: String,
    /// Global shortcut: the quick search box.
    pub hotkey_search: String,
    /// The look new notes get in a category (by the category's path); a
    /// category without one takes its parent's, then the default look.
    pub category_looks: std::collections::BTreeMap<String, Look>,
    pub start_with_windows: bool,
    /// Notes on the desktop are shown (else all hidden, until "Show All").
    pub notes_shown: bool,
    /// Days a deleted note stays in the trash.
    pub trash_days: u32,
    pub snap_to_edges: bool,
    /// How dark selected text's background is, in percent of the text's
    /// colour.
    pub selection_strength: u32,
    /// The app looks for a newer version of itself (at its start, then
    /// once a day).
    pub check_updates: bool,
    /// A newer version found is installed by itself (it runs from the next
    /// start); else the user is told and installs it.
    pub auto_update: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            default_look: Look::default(),
            themes: Vec::new(),
            categories: Vec::new(),
            hotkey_new_note: "ctrl+alt+n".into(),
            hotkey_clipboard_note: "ctrl+alt+v".into(),
            hotkey_toggle_notes: "ctrl+alt+h".into(),
            hotkey_search: "ctrl+alt+f".into(),
            category_looks: std::collections::BTreeMap::new(),
            start_with_windows: false,
            notes_shown: true,
            trash_days: 30,
            snap_to_edges: true,
            selection_strength: 35,
            check_updates: true,
            auto_update: true,
        }
    }
}

/// The category whose notes are templates: a new note can start as a copy
/// of one.
pub const TEMPLATES: &str = "Templates";

impl Settings {
    /// The look a new note gets in `category`: the category's own, else
    /// its nearest parent's, else the default.
    pub fn look_for(&self, category: &str) -> Look {
        let mut path = category;
        loop {
            if let Some(look) = self.category_looks.get(path) {
                return look.clone();
            }
            match path.rsplit_once('/') {
                Some((parent, _)) => path = parent,
                None => return self.default_look.clone(),
            }
        }
    }
}

/// What's saved in the notes folder's index file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Index {
    pub settings: Settings,
    pub notes: Vec<NoteMeta>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_old_index_with_missing_fields_still_loads() {
        let index: Index = serde_json::from_str(r#"{ "notes": [ { "id": "a1", "title": "Hi", "look": { "background": 255 } } ] }"#).unwrap();
        assert_eq!(index.notes[0].title, "Hi");
        assert_eq!(index.notes[0].look.background, 255);
        assert_eq!(index.notes[0].look.body_size, Look::default().body_size);
        assert_eq!(index.settings, Settings::default());
    }

    #[test]
    fn a_category_gives_new_notes_its_look_or_its_parents() {
        let mut settings = Settings::default();
        let blue = Look { background: 0x0000FF, ..Look::default() };
        settings.category_looks.insert("Work".into(), blue.clone());
        assert_eq!(settings.look_for("Work"), blue);
        assert_eq!(settings.look_for("Work/Ideas/Later"), blue);
        assert_eq!(settings.look_for("Home"), settings.default_look);
        assert_eq!(settings.look_for(""), settings.default_look);
    }

    #[test]
    fn sleep_ends() {
        let note = NoteMeta { sleep_until: Some(1000.0), ..NoteMeta::default() };
        assert!(note.asleep(999.0));
        assert!(!note.asleep(1000.0));
    }
}
