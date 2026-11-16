//! Printing a note, and saving it as a PDF.
//!
//! NSH has no printing, so both go through a browser, which lays out HTML
//! for paper well: the note is written as a page made for printing
//! (`page`), in the temporary folder. "Print" opens it in the default
//! browser, where it asks for the print dialog itself; "Save as PDF" has
//! Edge (which comes with Windows) or Chrome write the PDF without showing
//! a window.

use std::path::{Path, PathBuf};

use nsh_core::file_dialog::{FileFilter, SaveDialog};
use nsh_core::prelude::*;

use crate::app::{App, Note};
use crate::store;

/// The styles of a printed note: dark text on white whatever the note's
/// colours, the note's own type.
const STYLE: &str = r#"
@page { margin: 18mm; }
body { font-family: "Segoe UI", system-ui, sans-serif; color: #111; background: #fff; line-height: 1.5; max-width: 46em; margin: 0 auto; }
h1.note-title { font-size: 1.5em; border-bottom: 1px solid #ccc; padding-bottom: .3em; margin: 0 0 1em; }
img { max-width: 100%; height: auto; }
pre { background: #f4f4f5; padding: .7em 1em; border-radius: 6px; white-space: pre-wrap; }
code { font-family: Consolas, monospace; background: #f4f4f5; padding: 0 .25em; border-radius: 3px; }
pre code { background: none; padding: 0; }
blockquote { border-left: 3px solid #ccc; margin-left: 0; padding-left: 1em; color: #444; }
table { border-collapse: collapse; } td, th { border: 1px solid #bbb; padding: .3em .6em; }
ul[data-type="taskList"] { list-style: none; padding-left: .2em; }
ul[data-type="taskList"] li { display: flex; gap: .5em; align-items: baseline; }
ul[data-type="taskList"] li > div { flex: 1; }
ul[data-type="taskList"] p { margin: 0 0 .2em; }
summary { font-weight: 600; }
div[data-toggle-body] { padding-left: 1.2em; }
a { color: #1d4ed8; }
* { -webkit-print-color-adjust: exact; print-color-adjust: exact; }
"#;

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// `path` as a `file:` URL.
pub fn file_url(path: &Path) -> String {
    let mut out = String::from("file:///");
    for c in path.to_string_lossy().replace('\\', "/").trim_start_matches('/').chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '/' | ':' | '-' | '_' | '.' | '~' => out.push(c),
            _ => {
                let mut bytes = [0; 4];
                for b in c.encode_utf8(&mut bytes).bytes() {
                    out.push_str(&format!("%{b:02X}"));
                }
            }
        }
    }
    out
}

/// A page made for printing: `body` (a note's HTML, its images relative to
/// `folder`) under its title. Folded toggles are printed unfolded. With
/// `ask`, the page asks for the print dialog once it's loaded.
pub fn page(title: &str, body: &str, folder: &Path, size: f32, font: &str, ask: bool) -> String {
    let mut base = file_url(folder);
    if !base.ends_with('/') {
        base.push('/');
    }
    let body = body.replace("<details data-toggle=\"\">", "<details data-toggle=\"\" open=\"\">");
    let family = if font.is_empty() { String::new() } else { format!("font-family: \"{}\", \"Segoe UI\", sans-serif; ", escape(font)) };
    let script = if ask { "<script>window.addEventListener('load', () => setTimeout(() => window.print(), 300));</script>\n" } else { "" };
    format!(
        "<!doctype html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<base href=\"{base}\">\n<title>{title}</title>\n<style>{STYLE}body {{ {family}font-size: {size}px; }}</style>\n{script}</head>\n<body>\n<h1 class=\"note-title\">{title}</h1>\n{body}\n</body>\n</html>\n",
        title = escape(title),
    )
}

/// Writes the note's page for printing in the temporary folder.
fn write_page(app: App, note: &Note, ask: bool) -> Result<PathBuf, String> {
    app.save_text(note);
    let meta = note.meta.get_untracked();
    let vault = app.vault.get();
    let body = vault.read_body(&meta).map_err(|e| e.to_string())?;
    let file = vault.path_of(&meta);
    let folder = file.parent().unwrap_or(vault.root());
    let dir = std::env::temp_dir().join("bubblenotes-print");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}.html", store::file_stem_for(&meta.title)));
    let html = page(&meta.title, &body, folder, meta.look.body_size, &meta.look.body_font, ask);
    std::fs::write(&path, html).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Opens the note in the browser, with its print dialog.
pub fn print(app: App, note: &Note) {
    let opened = write_page(app, note, true).and_then(|path| open_file(&path));
    if let Err(e) = opened {
        app.problem.set(Some(format!("Couldn't print: {e}")));
    }
}

/// Opens `path` with the program Windows has for it.
#[cfg(windows)]
fn open_file(path: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    // No console window (CREATE_NO_WINDOW).
    std::process::Command::new("cmd").args(["/C", "start", ""]).arg(path).creation_flags(0x0800_0000).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(not(windows))]
fn open_file(path: &Path) -> Result<(), String> {
    std::process::Command::new("xdg-open").arg(path).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// A browser that writes PDFs without a window: Edge, else Chrome.
fn pdf_browser() -> Option<PathBuf> {
    let roots = ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"];
    let programs = ["Microsoft\\Edge\\Application\\msedge.exe", "Google\\Chrome\\Application\\chrome.exe"];
    programs
        .iter()
        .flat_map(|program| roots.iter().map(move |root| (root, program)))
        .filter_map(|(root, program)| Some(PathBuf::from(std::env::var_os(root)?).join(program)))
        .find(|path| path.exists())
}

/// Asks where, then saves the note there as a PDF and opens it.
pub fn save_pdf(app: App, note: Note) {
    let Some(browser) = pdf_browser() else {
        return app.problem.set(Some("Saving as PDF needs Edge or Chrome. Use Print… and choose “Save as PDF” there.".into()));
    };
    let stem = store::file_stem_for(&note.meta.with_untracked(|m| m.title.clone()));
    let dialog = SaveDialog::new().title("Save note as PDF").file_name(format!("{stem}.pdf")).filter(FileFilter::new("PDF", ["pdf"]));
    app.owner.run(|| {
        let _ = spawn(async move {
            let path = match pick_save_path(dialog).await {
                Ok(Some(path)) => path,
                Ok(None) => return,
                Err(e) => return app.problem.set(Some(format!("Couldn't show the save dialog: {e}"))),
            };
            write_pdf_at(app, note, browser, path).await;
        });
    });
}

/// Saves the note as the PDF `path` and opens it.
async fn write_pdf_at(app: App, note: Note, browser: PathBuf, path: PathBuf) {
    {
        {
            let page = match write_page(app, &note, false) {
                Ok(page) => page,
                Err(e) => return app.problem.set(Some(format!("Couldn't save the PDF: {e}"))),
            };
            // The browser takes a second or two: off the app's thread.
            let (done, result) = std::sync::mpsc::channel();
            let target = path.clone();
            std::thread::spawn(move || {
                let _ = done.send(write_pdf(&browser, &page, &target));
            });
            let outcome = loop {
                match result.try_recv() {
                    Ok(outcome) => break outcome,
                    Err(std::sync::mpsc::TryRecvError::Empty) => timer(std::time::Duration::from_millis(150)).await,
                    Err(_) => break Err("the browser stopped".to_string()),
                }
            };
            match outcome.and_then(|()| open_file(&path)) {
                Ok(()) => {}
                Err(e) => app.problem.set(Some(format!("Couldn't save the PDF: {e}"))),
            }
        }
    }
}

/// For development: the note as the PDF `path`, without asking where.
pub fn save_pdf_to(app: App, note: Note, path: PathBuf) {
    let Some(browser) = pdf_browser() else { return };
    app.owner.run(|| {
        let _ = spawn(write_pdf_at(app, note, browser, path));
    });
}

/// Has `browser` print `page` (an HTML file) to the PDF `target`.
fn write_pdf(browser: &Path, page: &Path, target: &Path) -> Result<(), String> {
    let _ = std::fs::remove_file(target);
    // Its own profile folder: a running Edge would otherwise take the
    // request over and write nothing.
    let profile = std::env::temp_dir().join("bubblenotes-print").join("profile");
    let mut command = std::process::Command::new(browser);
    command
        .arg("--headless")
        .arg("--disable-gpu")
        .arg("--no-pdf-header-footer")
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg(format!("--print-to-pdf={}", target.display()))
        .arg(file_url(page));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let output = command.output().map_err(|e| e.to_string())?;
    if target.exists() { Ok(()) } else { Err(format!("the browser wrote no file ({})", output.status)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_urls() {
        assert_eq!(file_url(Path::new("C:\\Users\\me\\Bubble Notes\\a&b.html")), "file:///C:/Users/me/Bubble%20Notes/a%26b.html");
    }

    #[test]
    fn a_printed_page_unfolds_toggles_and_finds_images() {
        let body = "<details data-toggle=\"\"><summary>T</summary></details><img src=\"attachments/a.png\">";
        let html = page("A <b>", body, Path::new("C:\\Notes\\Work"), 14.0, "", true);
        assert!(html.contains("<base href=\"file:///C:/Notes/Work/\">"), "{html}");
        assert!(html.contains("<details data-toggle=\"\" open=\"\">"));
        assert!(html.contains("<title>A &lt;b&gt;</title>"));
        assert!(html.contains("window.print()"));
        assert!(!page("A", body, Path::new("C:\\Notes"), 14.0, "", false).contains("window.print()"));
    }
}
