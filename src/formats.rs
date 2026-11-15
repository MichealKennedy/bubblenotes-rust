//! Reading and writing documents: Markdown (`.md`), HTML (`.html`) or plain
//! text (`.txt`), chosen by the file's extension; what Markdown would lose;
//! and image paths, absolute while a document is open, relative to the
//! note in its file.

use std::path::{Path, PathBuf};

use nsh_base::rich_editor::{
    formats::{DocFormat, HtmlFormat, MarkdownFormat, MarkdownNames, PlainTextFormat},
    model::{Node, Schema},
};
use serde_json::Value;

/// How a file is read and written, from its extension.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Format {
    Markdown,
    Html,
    Text,
}

impl Format {
    pub fn of(path: &Path) -> Self {
        match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
            Some("html" | "htm") => Format::Html,
            Some("txt" | "text") => Format::Text,
            _ => Format::Markdown,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Format::Markdown => "Markdown",
            Format::Html => "HTML",
            Format::Text => "Plain text",
        }
    }

    pub fn parse(self, schema: &Schema, text: &str) -> Result<Node, String> {
        match self {
            Format::Markdown => markdown().parse(schema, text).map_err(|e| e.to_string()),
            Format::Html => HtmlFormat::default().parse(schema, html_body(text)).map(|doc| spaces(&doc, false)).map_err(|e| e.to_string()),
            Format::Text => PlainTextFormat::default().parse(schema, text).map_err(|e| e.to_string()),
        }
    }

    pub fn serialize(self, doc: &Node) -> Result<String, String> {
        match self {
            // The editor keeps an empty paragraph at the end to type into:
            // no trailing blank lines for it, one final newline.
            Format::Markdown => markdown()
                .serialize(&cards_as_links(doc))
                .map(|s| format!("{}\n", s.trim_end().replace(LINK_OPEN, "[[").replace(LINK_CLOSE, "]]")))
                .map_err(|e| e.to_string()),
            Format::Html => HtmlFormat::default().serialize(&spaces(doc, true)).map(html_page).map_err(|e| e.to_string()),
            Format::Text => PlainTextFormat::default().serialize(doc).map_err(|e| e.to_string()),
        }
    }
}

/// The document as HTML without a page around it: what a note's file holds
/// inside its `<body>`, and what goes on the clipboard.
pub fn html_fragment(doc: &Node) -> Result<String, String> {
    HtmlFormat::default().serialize(&spaces(doc, true)).map(|html| with_image_widths(&html, doc)).map_err(|e| e.to_string())
}

/// A no-break space: what HTML keeps where it would drop a space.
const HARD_SPACE: char = '\u{a0}';

/// `doc` with its spaces as HTML needs them to survive (`hard`), or as the
/// editor has them (not `hard`).
///
/// HTML reads a run of spaces as one, and none at the start or end of a
/// block. So, as browsers do for editable text, a space HTML would drop is
/// written as a no-break space (the first of a block, the last, and every
/// second one of a run), and reading turns no-break spaces back into
/// spaces. A code block keeps its spaces in HTML as it is, and is left alone.
fn spaces(doc: &Node, hard: bool) -> Node {
    fn walk(value: &mut Value, hard: bool) {
        if value.get("type").and_then(Value::as_str) == Some("codeBlock") {
            return;
        }
        let Some(children) = value.get_mut("content").and_then(Value::as_array_mut) else { return };
        // Whether a space here would be dropped: at the block's start, after
        // a line break, after a space.
        let mut dropped = true;
        // The text a space at whose end would be dropped: the last before a
        // line break or the block's end.
        let mut last_text = None;
        let end_hard = |children: &mut Vec<Value>, at: Option<usize>| {
            if let Some(i) = at
                && let Some(kept) = children[i].get("text").and_then(Value::as_str).and_then(|t| t.strip_suffix(' '))
            {
                children[i]["text"] = Value::String(format!("{kept}{HARD_SPACE}"));
            }
        };
        for i in 0..children.len() {
            let kind = children[i].get("type").and_then(Value::as_str).unwrap_or_default().to_string();
            if kind != "text" {
                walk(&mut children[i], hard);
                dropped = kind == "hardBreak";
                if dropped && hard {
                    end_hard(children, last_text);
                }
                last_text = None;
                continue;
            }
            let Some(text) = children[i].get("text").and_then(Value::as_str) else { continue };
            let out: String = if hard {
                text.chars()
                    .map(|c| {
                        let c = if c == ' ' && dropped { HARD_SPACE } else { c };
                        dropped = c == ' ';
                        c
                    })
                    .collect()
            } else {
                text.replace(HARD_SPACE, " ")
            };
            children[i]["text"] = Value::String(out);
            last_text = Some(i);
        }
        if hard {
            end_hard(children, last_text);
        }
    }
    let from = if hard { ' ' } else { HARD_SPACE };
    let mut found = false;
    doc.descendants(&mut |node, _, _, _| {
        found |= node.text().is_some_and(|t| t.contains(from));
        !found
    });
    if !found {
        return doc.clone();
    }
    let mut json = doc.to_json();
    walk(&mut json, hard);
    doc.ty().schema().node_from_json(&json).unwrap_or_else(|_| doc.clone())
}

/// `html` with each image's width written on its `<img>`. NSH's editor
/// reads an image's `width` from HTML but doesn't write it, so a resized
/// image would be back at its full size the next time its note opens.
fn with_image_widths(html: &str, doc: &Node) -> String {
    // The images' widths, in the order their tags come.
    let mut widths = Vec::new();
    doc.descendants(&mut |node, _, _, _| {
        if node.ty().name() == "image" {
            let width = node.attrs().get("width").and_then(|w| w.as_f64().or_else(|| w.as_str().and_then(|s| s.parse().ok())));
            widths.push(width.filter(|w| *w > 0.0));
        }
        true
    });
    if widths.iter().all(Option::is_none) {
        return html.to_string();
    }
    let mut out = String::with_capacity(html.len() + widths.len() * 16);
    let mut rest = html;
    let mut widths = widths.into_iter();
    while let Some(at) = rest.find("<img") {
        let tag_end = at + "<img".len();
        out.push_str(&rest[..tag_end]);
        if let Some(Some(width)) = widths.next() {
            out.push_str(&format!(" width=\"{}\"", width.round() as i64));
        }
        rest = &rest[tag_end..];
    }
    out.push_str(rest);
    out
}

fn markdown() -> MarkdownFormat {
    MarkdownFormat::new(MarkdownNames::default())
}

/// Stand-ins for a note link's `[[` and `]]` while Markdown is written
/// (it would escape real brackets).
const LINK_OPEN: char = '\u{e000}';
const LINK_CLOSE: char = '\u{e001}';

/// `doc` with each YouTube card as a paragraph holding its link (titled
/// when the title is known; an empty card is left out): what Markdown
/// writes for cards, which it has no syntax for.
fn cards_as_links(doc: &Node) -> Node {
    fn walk(value: &mut Value) {
        let Some(children) = value.get_mut("content").and_then(Value::as_array_mut) else { return };
        children.retain(|c| c.get("type").and_then(Value::as_str) != Some("youtube") || c.pointer("/attrs/url").and_then(Value::as_str).is_some_and(|u| !u.is_empty()));
        for child in children {
            if child.get("type").and_then(Value::as_str) == Some("youtube") {
                let get = |name: &str| child.pointer(&format!("/attrs/{name}")).and_then(Value::as_str).unwrap_or_default().to_string();
                let (url, title) = (get("url"), get("title"));
                let label = if title.is_empty() { url.clone() } else { title };
                *child = serde_json::json!({
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": label, "marks": [{ "type": "link", "attrs": { "href": url } }] }],
                });
            } else if child.get("type").and_then(Value::as_str) == Some("noteLink") {
                // A link to a note is written as it's typed.
                let title = child.pointer("/attrs/title").and_then(Value::as_str).unwrap_or_default().to_string();
                // (Marked with private-use characters, turned into the
                // brackets once Markdown has escaped the rest.)
                *child = serde_json::json!({ "type": "text", "text": format!("{LINK_OPEN}{title}{LINK_CLOSE}") });
            } else {
                walk(child);
            }
        }
    }
    let mut found = false;
    doc.descendants(&mut |node, _, _, _| {
        found |= matches!(node.ty().name(), "youtube" | "noteLink");
        !found
    });
    if !found {
        return doc.clone();
    }
    let mut json = doc.to_json();
    walk(&mut json);
    doc.ty().schema().node_from_json(&json).unwrap_or_else(|_| doc.clone())
}

/// What writing `doc` as Markdown would lose, in words; `None` when it reads
/// back the same.
pub fn markdown_losses(schema: &Schema, doc: &Node) -> Option<String> {
    let md = markdown();
    let round_trip = md.serialize(doc).ok().and_then(|text| md.parse(schema, &text).ok());
    if round_trip.as_ref().is_some_and(|back| same_content(back, doc)) {
        return None;
    }
    let mut names: Vec<String> = Vec::new();
    for name in md.losses(doc) {
        let words = match name.as_str() {
            "textStyle" => "text color",
            "toggle" | "toggleTitle" | "toggleBody" => "toggles",
            "image" => "images",
            "youtube" => "YouTube cards (kept as links)",
            other => other,
        };
        if !names.iter().any(|n| n == words) {
            names.push(words.to_string());
        }
    }
    let mut aligned = false;
    doc.descendants(&mut |node, _, _, _| {
        aligned |= node.attrs().get("textAlign").is_some_and(|a| !a.is_null());
        true
    });
    if aligned {
        names.push("alignment".into());
    }
    Some(if names.is_empty() { "some formatting".to_string() } else { names.join(", ") })
}

/// Whether two documents hold the same content, not counting empty
/// paragraphs at the end (the editor adds one to type into).
pub fn same_content(a: &Node, b: &Node) -> bool {
    let kept = |doc: &Node| {
        let mut n = doc.child_count();
        while n > 0 && doc.child(n - 1).ty().name() == "paragraph" && doc.child(n - 1).child_count() == 0 {
            n -= 1;
        }
        n
    };
    let (n, m) = (kept(a), kept(b));
    n == m && (0..n).all(|i| a.child(i) == b.child(i))
}

/// A standalone HTML page around a document's HTML, readable in a browser.
fn html_page(body: String) -> String {
    format!(
        "<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<style>body {{ max-width: 46rem; margin: 2rem auto; padding: 0 1rem; font-family: system-ui, sans-serif; line-height: 1.6; }} table {{ border-collapse: collapse; }} td, th {{ border: 1px solid #ccc; padding: .3rem .6rem; }} img {{ max-width: 100%; }} summary {{ cursor: pointer; font-weight: 500; }} [data-toggle-body] {{ padding-left: 1.4rem; }}</style>\n</head>\n<body>\n{body}\n</body>\n</html>\n"
    )
}

/// The document part of a file read as HTML: what's inside `<body>`, or all.
fn html_body(text: &str) -> &str {
    let lower = text.to_ascii_lowercase();
    let start = lower.find("<body").and_then(|i| lower[i..].find('>').map(|j| i + j + 1));
    let end = lower.rfind("</body>");
    match (start, end) {
        (Some(s), Some(e)) if s <= e => &text[s..e],
        _ => text,
    }
}

// ----- Image paths ------------------------------------------------------------

/// The folder a note's images are copied into, next to the note.
pub const ATTACHMENTS: &str = "attachments";

/// Every image's `src` in `doc`, rewritten by `f` (`None` keeps it). The
/// document goes through its JSON form and back.
pub fn map_image_sources(schema: &Schema, doc: &Node, f: &mut dyn FnMut(&str) -> Option<String>) -> Node {
    fn walk(value: &mut Value, f: &mut dyn FnMut(&str) -> Option<String>) {
        if value.get("type").and_then(Value::as_str) == Some("image")
            && let Some(src) = value.pointer_mut("/attrs/src")
            && let Some(next) = src.as_str().and_then(&mut *f)
        {
            *src = Value::String(next);
        }
        if let Some(children) = value.get_mut("content").and_then(Value::as_array_mut) {
            for child in children {
                walk(child, f);
            }
        }
    }
    let mut json = doc.to_json();
    walk(&mut json, f);
    schema.node_from_json(&json).unwrap_or_else(|_| doc.clone())
}

/// Opening a note: its relative image paths become absolute ones (the
/// editor shows images from absolute paths).
pub fn images_to_absolute(schema: &Schema, doc: &Node, note: &Path) -> Node {
    let Some(dir) = note.parent() else { return doc.clone() };
    map_image_sources(schema, doc, &mut |src| {
        let decoded = percent_decode(src);
        let path = Path::new(&decoded);
        (is_local(src) && path.is_relative()).then(|| dir.join(path).to_string_lossy().into_owned())
    })
}

/// Saving a note: each local image outside the note's folder is copied into
/// `attachments/` next to it (a name is never reused for another file).
/// Returns the document to write (relative paths) and the new absolute path
/// of each image that moved, for the open document to follow.
pub fn images_for_saving(
    schema: &Schema,
    doc: &Node,
    note: &Path,
) -> Result<(Node, Vec<(String, String)>), String> {
    let dir = note.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut moved = Vec::new();
    let mut error = None;
    let written = map_image_sources(schema, doc, &mut |src| {
        if !is_local(src) {
            return None;
        }
        let path = PathBuf::from(src.strip_prefix("file://").unwrap_or(src));
        if path.is_relative() {
            return None; // Already relative to the note.
        }
        let inside = path.strip_prefix(&dir).ok().map(Path::to_path_buf);
        let relative = match inside {
            Some(relative) => relative,
            None => match copy_into_attachments(&path, &dir) {
                Ok(copied) => {
                    moved.push((src.to_string(), dir.join(&copied).to_string_lossy().into_owned()));
                    copied
                }
                Err(e) => {
                    error.get_or_insert(e);
                    return None;
                }
            },
        };
        // Forward slashes and no spaces in links, on every platform.
        Some(
            relative
                .components()
                .map(|c| c.as_os_str().to_string_lossy().replace(' ', "%20"))
                .collect::<Vec<_>>()
                .join("/"),
        )
    });
    match error {
        Some(e) => Err(e),
        None => Ok((written, moved)),
    }
}

/// Copies `image` into `dir/attachments/`, under its own name or a numbered
/// one when that's taken by another file. Returns the path relative to `dir`.
fn copy_into_attachments(image: &Path, dir: &Path) -> Result<PathBuf, String> {
    let folder = dir.join(ATTACHMENTS);
    std::fs::create_dir_all(&folder).map_err(|e| format!("Couldn't create {}: {e}", folder.display()))?;
    let stem = image.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "image".into());
    let ext = image.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let bytes = std::fs::read(image).map_err(|e| format!("Couldn't read {}: {e}", image.display()))?;
    for n in 0.. {
        let name = if n == 0 { format!("{stem}{ext}") } else { format!("{stem}-{n}{ext}") };
        let target = folder.join(&name);
        match std::fs::read(&target) {
            Ok(existing) if existing == bytes => return Ok(Path::new(ATTACHMENTS).join(name)),
            Ok(_) => continue,
            Err(_) => {
                std::fs::write(&target, &bytes).map_err(|e| format!("Couldn't copy to {}: {e}", target.display()))?;
                return Ok(Path::new(ATTACHMENTS).join(name));
            }
        }
    }
    unreachable!()
}

/// Whether `src` is a file on this computer (not a web address or data).
fn is_local(src: &str) -> bool {
    !(src.contains("://") && !src.starts_with("file://")) && !src.starts_with("data:")
}

fn percent_decode(s: &str) -> String {
    s.replace("%20", " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::{Kit, Relay};

    fn schema() -> Schema {
        Kit::new(Relay::default()).schema.clone()
    }

    #[test]
    fn a_resized_image_keeps_its_width_in_the_note_file() {
        let schema = schema();
        let html = r#"<p><img src="a.png" alt="a"> and <img src="b.png" width="120"></p>"#;
        let doc = Format::Html.parse(&schema, html).unwrap();
        let saved = html_fragment(&doc).unwrap();
        assert!(saved.contains(r#"<img width="120" src="b.png""#), "{saved}");
        assert!(!saved.contains(r#"<img width="120" src="a.png""#) && saved.matches("width=").count() == 1, "{saved}");
        let again = Format::Html.parse(&schema, &saved).unwrap();
        assert_eq!(html_fragment(&again).unwrap(), saved);
    }

    #[test]
    fn markdown_round_trips_its_own_constructs() {
        let schema = schema();
        let src = "# Title\n\nSome **bold**, *it*, ~~strike~~, `code` and [a link](https://x.y).\n\n- [ ] task\n- [x] done\n\n> quote\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n\n```rust\nfn main() {}\n```\n";
        let doc = Format::Markdown.parse(&schema, src).unwrap();
        assert_eq!(Format::Markdown.serialize(&doc).unwrap(), src);
        assert_eq!(markdown_losses(&schema, &doc), None);
    }

    #[test]
    fn markdown_losses_name_rich_formatting() {
        let schema = schema();
        let html = r#"<p>Plain <u>under</u> <mark>hi</mark> <span style="color: #dc2626">red</span></p><p style="text-align: center">mid</p>"#;
        let doc = Format::Html.parse(&schema, html).unwrap();
        let lost = markdown_losses(&schema, &doc).unwrap();
        for word in ["underline", "highlight", "text color", "alignment"] {
            assert!(lost.contains(word), "{lost:?} names {word}");
        }
    }

    #[test]
    fn toggles_round_trip_through_html_and_are_lost_in_markdown() {
        let schema = schema();
        let html = r#"<details data-toggle=""><summary>Packing</summary><div data-toggle-body="" data-open="false"><p>Foulies</p></div></details>"#;
        let doc = Format::Html.parse(&schema, html).unwrap();
        let toggle = doc.child(0);
        assert_eq!(toggle.ty().name(), "toggle");
        assert_eq!(toggle.attrs().get("open"), Some(&Value::Bool(false)));
        assert_eq!(toggle.child(1).attrs().get("open"), Some(&Value::Bool(false)));
        let html_out = Format::Html.serialize(&doc).unwrap();
        // A closed toggle has no `open` at all (browsers take any as open).
        assert!(html_out.contains("<details data-toggle=\"\"><summary>"), "{html_out}");
        assert!(!html_out.contains(" open"), "{html_out}");
        assert!(html_out.contains("data-open=\"false\""), "{html_out}");
        let back = Format::Html.parse(&schema, &html_out).unwrap();
        assert!(same_content(&back, &doc));
        assert!(markdown_losses(&schema, &doc).unwrap().contains("toggles"));
    }

    #[test]
    fn images_are_copied_into_attachments_and_written_relative() {
        let schema = schema();
        let tmp = std::env::temp_dir().join(format!("richnotes-test-{}", std::process::id()));
        let (notes, elsewhere) = (tmp.join("notes"), tmp.join("pictures"));
        std::fs::create_dir_all(&notes).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        let picture = elsewhere.join("my boat.png");
        std::fs::write(&picture, b"png bytes").unwrap();
        let note = notes.join("trip.md");

        let md = format!("![Seawyf]({})\n", picture.display().to_string().replace(' ', "%20"));
        let doc = Format::Markdown.parse(&schema, &md).unwrap();
        let doc = map_image_sources(&schema, &doc, &mut |s| Some(percent_decode(s)));
        let (written, moved) = images_for_saving(&schema, &doc, &note).unwrap();
        let text = Format::Markdown.serialize(&written).unwrap();
        assert_eq!(text, "![Seawyf](attachments/my%20boat.png)\n");
        assert_eq!(std::fs::read(notes.join("attachments/my boat.png")).unwrap(), b"png bytes");
        assert_eq!(moved.len(), 1);

        // Reopened: absolute again, pointing at the copy.
        let reopened = images_to_absolute(&schema, &Format::Markdown.parse(&schema, &text).unwrap(), &note);
        let src = reopened.to_json().pointer("/content/0/content/0/attrs/src").cloned().unwrap();
        assert_eq!(src, Value::String(notes.join("attachments/my boat.png").to_string_lossy().into_owned()));

        // Saving again copies nothing new.
        let (_, moved_again) = images_for_saving(&schema, &reopened, &note).unwrap();
        assert!(moved_again.is_empty());
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn same_content_ignores_trailing_empty_paragraphs() {
        let schema = schema();
        let a = Format::Markdown.parse(&schema, "Hello\n").unwrap();
        let b = Format::Html.parse(&schema, "<p>Hello</p><p></p>").unwrap();
        assert!(same_content(&a, &b));
    }

    #[test]
    fn spaces_survive_being_saved_as_html() {
        let schema = schema();
        let doc = schema
            .node_from_json(&serde_json::json!({
                "type": "doc",
                "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": " two  three   end " }] },
                    { "type": "paragraph", "content": [
                        { "type": "text", "text": "bold ", "marks": [{ "type": "bold" }] },
                        { "type": "text", "text": " plain " },
                        { "type": "text", "text": "x  y", "marks": [{ "type": "code" }] },
                    ] },
                    { "type": "codeBlock", "content": [{ "type": "text", "text": "a  b" }] },
                ],
            }))
            .unwrap();
        let html = html_fragment(&doc).unwrap();
        // No two spaces in a row, none at a paragraph's start or end.
        assert!(!html.contains("  <") && !html.contains("> ") && !html.contains(" </p>"), "{html}");
        assert!(html.contains("a  b"), "a code block keeps its spaces: {html}");
        let back = Format::Html.parse(&schema, &html).unwrap();
        assert_eq!(back.child(0).text_content(), " two  three   end ");
        assert_eq!(back.child(1).text_content(), "bold  plain x  y");
        assert_eq!(back.child(2).text_content(), "a  b");
    }
}
