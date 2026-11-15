//! Files: new, open, save (asking first when Markdown would drop
//! formatting), and inserting images. Images are referenced where they are
//! while a note is open; saving copies them into `attachments/` next to the
//! note and writes relative paths.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use nsh_base::alert_dialog;
use nsh_base::rich_editor::model::Node;
use nsh_core::file_dialog::{FileFilter, OpenDialog, SaveDialog};
use nsh_core::prelude::*;
use serde_json::{Value, json};

use crate::formats::{self, Format};
use crate::kit::{Editor, Kit};
use crate::ui::Ctx;

/// The window's file state and what changes it (all `Copy` handles).
#[derive(Clone, Copy)]
pub struct Files {
    pub ctx: Ctx,
    pub kit: Stored<Rc<Kit>>,
    /// The document as last opened or saved, to tell unsaved changes.
    pub saved: Signal<Node>,
    pub path: Signal<Option<PathBuf>>,
    pub owner: Owner,
}

impl Files {
    /// Whether the document differs from the file (read reactively).
    pub fn dirty(self) -> bool {
        self.ctx.revision.get();
        !self.ctx.editor.with(|e| self.saved.with(|s| formats::same_content(&e.doc(), s)))
    }

    fn has_changes(self) -> bool {
        untracked(|| self.dirty())
    }

    fn load(self, editor: Editor, file: Option<PathBuf>) {
        self.saved.set(editor.doc());
        self.ctx.editor.set(editor);
        self.path.set(file);
        self.ctx.changed();
    }

    pub fn new_file(self) {
        self.confirm_discard(move || {
            self.load(self.kit.get().editor(None), None);
            self.ctx.status.set("New note".into());
        });
    }

    pub fn open(self) {
        self.confirm_discard(move || {
            self.owner.run(|| {
                let picked = pick_paths(
                    OpenDialog::files()
                        .title("Open")
                        .filter(FileFilter::new("Notes", ["md", "markdown", "html", "htm", "txt"])),
                );
                let _ = spawn(async move {
                    let file = match picked.await {
                        Ok(Some(paths)) => match paths.into_iter().next() {
                            Some(file) => file,
                            None => return,
                        },
                        Ok(None) => return,
                        Err(e) => return self.ctx.status.set(format!("Couldn't show the open dialog: {e}")),
                    };
                    match self.read(&file) {
                        Ok(editor) => {
                            self.ctx.status.set(format!("Opened {}", file.display()));
                            self.load(editor, Some(file));
                        }
                        Err(e) => self.ctx.status.set(format!("Couldn't open {}: {e}", file.display())),
                    }
                });
            });
        });
    }

    fn read(self, file: &Path) -> Result<Editor, String> {
        let kit = self.kit.get();
        let text = std::fs::read_to_string(file).map_err(|e| e.to_string())?;
        let doc = Format::of(file).parse(&kit.schema, &text)?;
        Ok(kit.editor(Some(formats::images_to_absolute(&kit.schema, &doc, file))))
    }

    pub fn save(self) {
        match self.path.get_untracked() {
            Some(file) => self.save_checked(file),
            None => self.save_as("md"),
        }
    }

    /// Asks where to save, suggesting a name ending in `extension`.
    pub fn save_as(self, extension: &'static str) {
        self.owner.run(|| {
            let stem = self
                .path
                .get_untracked()
                .as_deref()
                .and_then(Path::file_stem)
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Untitled".into());
            let picked = pick_save_path(SaveDialog::new().title("Save As").file_name(format!("{stem}.{extension}")));
            let _ = spawn(async move {
                match picked.await {
                    Ok(Some(file)) => self.save_checked(file),
                    Ok(None) => {}
                    Err(e) => self.ctx.status.set(format!("Couldn't show the save dialog: {e}")),
                }
            });
        });
    }

    /// Saves to `file`; when that's Markdown and Markdown can't hold all of
    /// the note's formatting, asks first and offers HTML.
    fn save_checked(self, file: PathBuf) {
        if Format::of(&file) != Format::Markdown {
            return self.write(file);
        }
        let lost = self.ctx.editor.with_untracked(|e| formats::markdown_losses(&e.kit.schema, &e.doc()));
        let Some(lost) = lost else { return self.write(file) };
        self.owner.run(|| {
            let chosen = alert_dialog()
                .title("Markdown can't keep all of this formatting")
                .description(format!(
                    "Saving as Markdown drops: {lost}. HTML keeps everything and opens in any browser."
                ))
                .action("html", "Save as HTML…")
                .destructive("markdown", "Save as Markdown anyway")
                .cancel("cancel", "Cancel")
                .show();
            let _ = spawn(async move {
                match chosen.await.ok().and_then(|r| r.action).as_deref() {
                    Some("html") => self.save_as("html"),
                    Some("markdown") => self.write(file),
                    _ => {}
                }
            });
        });
    }

    /// Writes the note to `file` in the format its extension names, its
    /// images copied next to it.
    fn write(self, file: PathBuf) {
        let editor = self.ctx.editor.get_untracked();
        let result = formats::images_for_saving(&editor.kit.schema, &editor.doc(), &file).and_then(|(doc, moved)| {
            let text = Format::of(&file).serialize(&doc)?;
            std::fs::write(&file, text).map_err(|e| e.to_string())?;
            Ok(moved)
        });
        match result {
            Ok(moved) => {
                // The open note follows the images that moved, so the next
                // save doesn't copy them again.
                follow_moved_images(&editor, &moved);
                self.saved.set(editor.doc());
                self.ctx.status.set(format!("Saved {}", file.display()));
                self.path.set(Some(file));
                self.ctx.changed();
            }
            Err(e) => self.ctx.status.set(format!("Couldn't save {}: {e}", file.display())),
        }
    }

    /// Asks for an image and inserts it after the selection.
    pub fn insert_image(self) {
        self.owner.run(|| {
            let picked = pick_paths(
                OpenDialog::files()
                    .title("Insert image")
                    .filter(FileFilter::new("Images", ["png", "jpg", "jpeg", "gif", "webp", "svg"])),
            );
            let _ = spawn(async move {
                let Ok(Some(paths)) = picked.await else { return };
                let Some(image) = paths.into_iter().next() else { return };
                let alt = image.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                self.ctx.editor.with_untracked(|e| {
                    let at = e.state().selection().to();
                    e.run(
                        "insertContentAt",
                        json!({
                            "position": at,
                            "content": { "type": "image", "attrs": { "src": image.to_string_lossy(), "alt": alt } },
                        }),
                    );
                });
                self.ctx.status.set("Image inserted: it's copied next to the note when you save".into());
                self.ctx.changed();
            });
        });
    }

    /// Runs `then` now when there's nothing to lose, else once the user
    /// agrees to discard the unsaved changes.
    fn confirm_discard(self, then: impl Fn() + 'static) {
        if !self.has_changes() {
            return then();
        }
        self.owner.run(|| {
            let chosen = alert_dialog()
                .title("Discard unsaved changes?")
                .description("Your changes to this note haven't been saved.")
                .cancel("cancel", "Cancel")
                .destructive("discard", "Discard")
                .show();
            let _ = spawn(async move {
                if chosen.await.is_ok_and(|r| r.action.as_deref() == Some("discard")) {
                    then();
                }
            });
        });
    }
}

/// Points the open document's images at their copies (one step, outside
/// the undo history's way).
fn follow_moved_images(editor: &Editor, moved: &[(String, String)]) {
    if moved.is_empty() {
        return;
    }
    let state = editor.state();
    let mut tr = state.tr();
    state.doc().descendants(&mut |node, pos, _, _| {
        if node.ty().name() == "image" {
            let src = node.attrs().get("src").and_then(Value::as_str).unwrap_or_default();
            if let Some((_, new)) = moved.iter().find(|(old, _)| old == src) {
                let _ = tr.set_node_attribute(pos, "src", Value::String(new.clone()));
            }
        }
        true
    });
    let _ = editor.session.dispatch(tr);
}
