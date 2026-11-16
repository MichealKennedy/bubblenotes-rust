//! Media in notes: images pasted or dropped into a note (saved into the
//! note's `attachments/` folder), and YouTube links shown as cards.
//!
//! Videos can't play inside a note (web views don't draw in note windows),
//! so a YouTube link becomes a `youtube` block: a card with the video's
//! thumbnail, a play button and its title, which opens the video in the
//! browser. A link pasted alone becomes a card. In HTML a card is a link
//! inside a `div` (`<div data-youtube-card><a class="youtube-card"
//! data-youtube="ID" href="…">Title</a></div>`), so a browser still
//! follows it; in Markdown it's a plain link.

use std::cell::RefCell;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nsh_base::rich_editor::{
    commands::BoxCommand,
    extensions::Extension,
    model::{AttrParse, AttrRender, AttrSpec, DomOutputSpec, Fragment, Node, NodeSpec, ParseRule, Schema, Slice, TextRender},
    state::{EditorState, NodeSelection, PluginKey, PluginSpec, Selection, Transaction},
    view::{EditorSession, NodeView, NodeViewCx, NodeViews},
};
use nsh_base::theme;
use nsh_core::element::ImageSrc;
use nsh_core::prelude::*;

use crate::win;
use serde_json::{Value, json};

use crate::formats::ATTACHMENTS;
use crate::kit::Editor;

// ----- Pasted and dropped images ----------------------------------------------

/// The extension a pasted or dropped file is saved with when it's an image
/// the editor shows, from its MIME type or else its name.
pub fn image_extension(name: &str, mime: &str) -> Option<&'static str> {
    let by_mime = match mime.to_ascii_lowercase().as_str() {
        "image/png" => Some("png"),
        "image/jpeg" | "image/jpg" => Some("jpg"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        "image/bmp" | "image/x-bmp" => Some("bmp"),
        "image/svg+xml" => Some("svg"),
        _ => None,
    };
    by_mime.or_else(|| {
        let ext = Path::new(name).extension()?.to_str()?.to_ascii_lowercase();
        ["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"].into_iter().find(|e| *e == ext)
    })
}

/// Now, as a pasted image's name has it (`20261114-093005`).
pub fn stamp_now() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

/// Writes a pasted image's bytes into `note_dir/attachments/` as
/// `pasted-<stamp>-<n>.<ext>`, the first `n` from 1 whose name is free (a
/// file is never overwritten). Returns the new file's absolute path.
pub fn save_pasted(note_dir: &Path, stamp: &str, ext: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    let folder = note_dir.join(ATTACHMENTS);
    std::fs::create_dir_all(&folder).map_err(|e| format!("Couldn't create {}: {e}", folder.display()))?;
    for n in 1.. {
        let path = folder.join(format!("pasted-{stamp}-{n}.{ext}"));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(bytes).map_err(|e| format!("Couldn't write {}: {e}", path.display()))?;
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Couldn't write {}: {e}", path.display())),
        }
    }
    unreachable!()
}

/// The local path a pasted or dropped `file:` URL names (`file:///C:/My%20Pictures/a.png`
/// is `C:\My Pictures\a.png`); a plain absolute path stays as it is.
pub fn path_from_uri(uri: &str) -> Option<PathBuf> {
    let uri = uri.trim();
    let Some(rest) = uri.strip_prefix("file://") else {
        return Path::new(uri).is_absolute().then(|| PathBuf::from(uri));
    };
    let decoded = percent_decode(rest);
    // `file:///C:/x` has an empty host; `file://server/share/x` is a share.
    let path = match decoded.strip_prefix('/') {
        Some(local) if local.as_bytes().get(1) == Some(&b':') => local.replace('/', std::path::MAIN_SEPARATOR_STR),
        Some(_) if cfg!(windows) => decoded.replace('/', "\\"),
        Some(_) => decoded,
        None if cfg!(windows) => format!("\\\\{}", decoded.replace('/', "\\")),
        None => format!("//{decoded}"),
    };
    Some(PathBuf::from(path))
}

/// `%XX` escapes decoded (UTF-8); a broken escape stays as it is.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(hi), Some(lo)) = (bytes.get(i + 1).and_then(|b| hex(*b)), bytes.get(i + 2).and_then(|b| hex(*b)))
        {
            out.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Inserts images (absolute paths) at the selection, replacing it.
pub fn insert_images(editor: &Editor, images: &[PathBuf]) -> bool {
    if images.is_empty() {
        return false;
    }
    let content: Vec<Value> = images
        .iter()
        .map(|path| {
            let alt = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            json!({ "type": "image", "attrs": { "src": path.to_string_lossy(), "alt": alt } })
        })
        .collect();
    let selection = editor.state().selection().clone();
    editor.run("insertContentAt", json!({ "position": { "from": selection.from(), "to": selection.to() }, "content": content }))
}

// ----- YouTube links ------------------------------------------------------------

/// The video a YouTube link is for, and where it starts (seconds), for
/// `youtube.com/watch?v=`, `youtu.be/`, `youtube.com/shorts/`,
/// `/embed/`, `/live/` and `music.youtube.com/watch?v=` links, with or
/// without the scheme, `www.` or `m.`. `None` for anything else, also for
/// text around the link.
pub fn youtube_id(url: &str) -> Option<(String, Option<u32>)> {
    let url = url.trim();
    if url.is_empty() || url.chars().any(char::is_whitespace) {
        return None;
    }
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")).unwrap_or(url);
    let host_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let host = rest[..host_end].to_ascii_lowercase();
    let host = ["www.", "m.", "music."].iter().fold(host.as_str(), |h, p| h.strip_prefix(p).unwrap_or(h));
    let rest = &rest[host_end..];
    let (path, query) = match rest.split_once('?') {
        Some((path, query)) => (path, query),
        None => (rest, ""),
    };
    let (path, fragment) = path.split_once('#').unwrap_or((path, ""));
    let (query, query_fragment) = query.split_once('#').unwrap_or((query, ""));
    let param = |name: &str| {
        query.split('&').chain(fragment.split('&')).chain(query_fragment.split('&')).find_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            (k == name).then_some(v)
        })
    };
    let id = match host {
        "youtu.be" => path.trim_start_matches('/').split('/').next().unwrap_or(""),
        "youtube.com" => {
            let path = path.trim_end_matches('/');
            if path == "/watch" {
                param("v").unwrap_or("")
            } else {
                ["/shorts/", "/embed/", "/live/"].iter().find_map(|p| path.strip_prefix(p))?
            }
        }
        _ => return None,
    };
    let valid = id.len() == 11 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if !valid {
        return None;
    }
    let start = param("t").or_else(|| param("start")).and_then(seconds);
    Some((id.to_string(), start))
}

/// A start time as links write it: `90`, `90s`, `1m30s`, `1h2m3s`.
fn seconds(text: &str) -> Option<u32> {
    if let Ok(n) = text.parse() {
        return Some(n);
    }
    let (mut total, mut number) = (0_u32, String::new());
    for c in text.chars() {
        if c.is_ascii_digit() {
            number.push(c);
            continue;
        }
        let unit = match c {
            'h' => 3600,
            'm' => 60,
            's' => 1,
            _ => return None,
        };
        total = total.checked_add(number.parse::<u32>().ok()?.checked_mul(unit)?)?;
        number.clear();
    }
    number.is_empty().then_some(total)
}

/// The link a card opens: the video's page, from `start` seconds.
pub fn watch_url(id: &str, start: Option<u32>) -> String {
    match start {
        Some(s) if s > 0 => format!("https://www.youtube.com/watch?v={id}&t={s}s"),
        _ => format!("https://www.youtube.com/watch?v={id}"),
    }
}

fn thumbnail_url(id: &str) -> String {
    format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg")
}

/// The page a card plays its video in: YouTube's player, filling it. The
/// page is given a web address of its own (`PLAYER_BASE`): YouTube's player
/// refuses to play in a page without one.
fn player_page(id: &str, start: Option<u32>) -> String {
    let start = start.filter(|s| *s > 0).map(|s| format!("&start={s}")).unwrap_or_default();
    // (For checking by hand without sound.)
    let mute = if std::env::var_os("BUBBLENOTES_MUTE").is_some() { "&mute=1" } else { "" };
    format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><meta name="referrer" content="strict-origin-when-cross-origin"><style>html,body{{margin:0;height:100%;background:#000;overflow:hidden}}iframe{{position:absolute;inset:0;width:100%;height:100%;border:0}}</style></head><body><iframe src="https://www.youtube-nocookie.com/embed/{id}?autoplay=1&playsinline=1&rel=0{start}{mute}" allow="autoplay; encrypted-media; picture-in-picture; fullscreen" allowfullscreen referrerpolicy="strict-origin-when-cross-origin"></iframe></body></html>"#
    )
}

/// The address the player's page is given.
const PLAYER_BASE: &str = "https://bubblenotes.app/player/";

/// YouTube's description of a video (its title), as JSON.
fn oembed_url(url: &str) -> String {
    let encoded: String = url
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    format!("https://www.youtube.com/oembed?url={encoded}&format=json")
}

/// A card node for `url` (`None`: an empty card asking for a link).
pub fn card_node(schema: &Schema, url: Option<&str>) -> Option<Node> {
    let (id, link) = match url.and_then(youtube_id) {
        Some((id, start)) => {
            let link = watch_url(&id, start);
            (id, link)
        }
        None => (String::new(), String::new()),
    };
    schema.node_from_json(&json!({ "type": "youtube", "attrs": { "videoId": id, "url": link, "title": "" } })).ok()
}

/// The `youtube` node type.
pub fn extension() -> Extension {
    Extension::node("youtube", |_| {
        let mut spec = NodeSpec::default();
        spec.group = Some("block".into());
        spec.atom = true;
        spec.draggable = true;
        spec.attrs = vec![("videoId".into(), text_attr()), ("url".into(), text_attr()), ("title".into(), text_attr())];
        spec.parse_dom = vec![ParseRule::tag("div[data-youtube-card]").with_get_attrs(|input| {
            let link = input.element()?.query_selector("a[data-youtube]")?;
            let url = link.attr("href").unwrap_or_default().to_string();
            // A card without a title yet shows its link as its text.
            let text = link.text_content().trim().to_string();
            let title = if text == url { String::new() } else { text };
            let attrs = [("videoId", link.attr("data-youtube").unwrap_or_default().to_string()), ("url", url), ("title", title)];
            Some(attrs.into_iter().map(|(k, v)| (k.to_string(), Value::String(v))).collect())
        })];
        spec.to_dom = Some(DomOutputSpec::render(|node| {
            let get = |name: &str| node.attr(name).and_then(Value::as_str).unwrap_or_default().to_string();
            let (id, url, title) = (get("videoId"), get("url"), get("title"));
            // What a browser shows: the title, or the link until it's known.
            let label = if title.is_empty() { url.clone() } else { title };
            let link = DomOutputSpec::tag("a")
                .attr("class", "youtube-card")
                .attr("data-youtube", id)
                .attr("href", url)
                .child(DomOutputSpec::Text(label));
            DomOutputSpec::tag("div").attr("data-youtube-card", "").child(link)
        }));
        // Plain text (and what a copy puts on the clipboard) is the link.
        spec.render_text = Some(TextRender::Template("{url}".into()));
        spec
    })
}

/// A text attribute written by the type's own rules, not as an HTML
/// attribute of its name.
fn text_attr() -> AttrSpec {
    let mut attr = AttrSpec::default();
    attr.default = Some(Value::String(String::new()));
    attr.parse_html = Some(AttrParse::Skip);
    attr.render_html = Some(AttrRender::Skip);
    attr
}

/// The plugin turning a pasted YouTube link (alone) into a card.
pub fn paste_plugin() -> Arc<dyn PluginSpec> {
    Arc::new(YoutubePaste)
}

struct YoutubePaste;

impl PluginSpec for YoutubePaste {
    fn key(&self) -> PluginKey {
        PluginKey::new("youtubePaste")
    }

    fn handle_paste(&self, state: &EditorState, slice: &Slice, dispatch: &mut dyn FnMut(Transaction)) -> bool {
        let selection = state.selection();
        let from = selection.resolved_from();
        // A link pasted over selected text links it; in code it stays text.
        if (selection.json_id() == "text" && !selection.empty()) || from.parent().ty().spec().code {
            return false;
        }
        let text = slice.content.text_between(0, slice.content.size(), Some("\n"), None);
        if youtube_id(&text).is_none() {
            return false;
        }
        let Some(card) = card_node(state.schema(), Some(&text)) else { return false };
        let mut tr = state.tr();
        let block = from.parent();
        if selection.empty() && block.is_textblock() && block.content().size() == 0 && from.depth > 0 {
            // An empty line becomes the card.
            let start = from.before(from.depth);
            if tr.replace_with(start, from.after(from.depth), Fragment::from(card.clone())).is_err() {
                return false;
            }
            caret_after(&mut tr, start + card.node_size());
        } else {
            tr.replace_selection_with(card, false);
            // Nowhere for a block here: the paste goes on as text.
            if !tr.doc_changed() {
                return false;
            }
        }
        tr.scroll_into_view();
        dispatch(tr);
        true
    }
}

/// The caret at the first place to type after the card ending at `pos`: an
/// empty line is added when the card is the last thing in its container
/// (else the next key typed would replace the selected card).
fn caret_after(tr: &mut Transaction, pos: usize) {
    let last = tr.doc().resolve(pos).is_ok_and(|at| at.node_after().is_none());
    if last && let Ok(line) = tr.schema().node_from_json(&json!({ "type": "paragraph" })) {
        let _ = tr.insert(pos, Fragment::from(line));
    }
    if let Ok(at) = tr.doc().resolve(pos.min(tr.doc().content().size())) {
        let selection = Selection::near(&at, 1);
        tr.set_selection(selection);
    }
}

/// The "YouTube video" action: the link selected, or on the caret's line,
/// becomes a card; without one, an empty card that a pasted link fills
/// (it's selected, so pasting replaces it).
pub fn youtube_command() -> BoxCommand {
    Arc::new(|state, dispatch| {
        let selection = state.selection();
        let from = selection.resolved_from().clone();
        let block = from.parent();
        if !block.is_textblock() || from.depth == 0 {
            return false;
        }
        let Some(dispatch) = dispatch else { return true };
        let (start, end) = (from.before(from.depth), from.after(from.depth));
        let line = block.text_content();
        let picked = if selection.empty() {
            line.clone()
        } else {
            state.doc().text_between(selection.from(), selection.to(), Some(" "), None)
        };
        let found = youtube_id(&picked).is_some();
        let Some(card) = card_node(state.schema(), found.then_some(picked.as_str())) else { return false };
        let size = card.node_size();
        let mut tr = state.tr();
        if found && picked.trim() != line.trim() {
            // A link inside a longer line: only it becomes the card.
            tr.replace_selection_with(card, false);
            if !tr.doc_changed() {
                return false;
            }
            dispatch(tr);
            return true;
        }
        let at = if found || line.trim().is_empty() {
            if tr.replace_with(start, end, Fragment::from(card)).is_err() {
                return false;
            }
            start
        } else {
            if tr.insert(end, Fragment::from(card)).is_err() {
                return false;
            }
            end
        };
        if found {
            caret_after(&mut tr, at + size);
        } else {
            tr.set_selection(NodeSelection::create(tr.doc(), at));
        }
        tr.scroll_into_view();
        dispatch(tr);
        true
    })
}

/// The node views of this module's types, added to `views`; images too,
/// so they fit the note's width.
pub fn with_node_views(views: NodeViews) -> NodeViews {
    views.with("youtube", Arc::new(YoutubeView)).with("image", Arc::new(ImageView))
}

// ----- Images that fit ------------------------------------------------------------

/// How wide the text of an editor element is, provided (as a context) to
/// its node views, so an image is never wider than the note.
#[derive(Clone, Copy)]
pub struct TextWidth(pub Signal<f32>);

/// The narrowest an image can be dragged to, in points.
const MIN_IMAGE_WIDTH: f32 = 32.0;

/// An image at the size it asks for (its natural size when it asks for
/// none), scaled down to the text's width.
struct ImageView;

impl NodeView for ImageView {
    fn create(&self, cx: &NodeViewCx, node: &Node) -> AnyElement {
        let attrs = node.attrs();
        let session = cx.session.clone();
        let get_pos = std::rc::Rc::new(cx.get_pos());
        let window = nsh_core::element::desktop::current_window();
        let owner = Owner::current();
        let hovered = signal(false);
        // The width being dragged to, while the corner handle is held.
        let dragged = signal(None::<f32>);
        let src = attrs.get("src").and_then(Value::as_str).unwrap_or_default().to_string();
        let number = |key: &str| {
            attrs.get(key).and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))).map(|n| n as f32)
        };
        let (width, height) = (number("width"), number("height"));
        let natural = signal(None::<(f32, f32)>);
        if (width.is_none() || height.is_none()) && !src.is_empty() {
            let src = src.clone();
            let _ = spawn(async move {
                if let Ok(size) = image_size(src).await {
                    natural.set(Some((size.width.0 as f32, size.height.0 as f32)));
                }
            });
        }
        let room = use_context::<TextWidth>();
        let size = move || {
            let natural = natural.get().filter(|(w, h)| *w > 0.0 && *h > 0.0);
            let aspect = natural.map(|(w, h)| w / h).or(match (width, height) {
                (Some(w), Some(h)) if h > 0.0 => Some(w / h),
                _ => None,
            });
            let max = room.map(|r| r.0.get()).filter(|m| *m > 0.0).unwrap_or(f32::MAX);
            if let Some(w) = dragged.get() {
                let w = w.clamp(MIN_IMAGE_WIDTH, max);
                return (w, aspect.map_or(0.0, |a| w / a));
            }
            let (w, h) = match (width, height) {
                (Some(w), Some(h)) => (w, h),
                (Some(w), None) => (w, aspect.map_or(0.0, |a| w / a)),
                (None, Some(h)) => (aspect.map_or(0.0, |a| h * a), h),
                (None, None) => natural.unwrap_or_default(),
            };
            if w > max { (max, h * max / w) } else { (w, h) }
        };
        let mut el = image(src).rounded(px(8.0)).w(move || px(size().0)).h(move || px(size().1));
        if let Some(alt) = attrs.get("alt").and_then(Value::as_str) {
            el = el.label(alt.to_string());
        }

        // The corner handle: drag it to resize the image (its shape kept).
        let resize = move || {
            let Some(start) = win::cursor() else { return };
            let scale = untracked(|| window.state()).map(|s| s.scale_factor).unwrap_or(1.0).max(0.1);
            let from = untracked(size).0;
            let (session, get_pos) = (session.clone(), get_pos.clone());
            owner.run(|| {
                let _ = spawn(async move {
                    dragged.set(Some(from));
                    while win::primary_button_down() {
                        timer(std::time::Duration::from_millis(8)).await;
                        let Some(now) = win::cursor() else { break };
                        dragged.set(Some((from + (now.0 - start.0) / scale).max(MIN_IMAGE_WIDTH)));
                    }
                    // Where the pointer ended up, also when the drag was
                    // over before the first look at it.
                    if let Some(now) = win::cursor() {
                        dragged.set(Some((from + (now.0 - start.0) / scale).max(MIN_IMAGE_WIDTH)));
                    }
                    let to = untracked(size).0.round();
                    if (to - from).abs() >= 1.0
                        && let Some(pos) = get_pos()
                    {
                        let state = session.state();
                        let mut tr = state.tr();
                        // Only the width is kept: the height follows the
                        // image's shape.
                        let sized = tr.set_node_attribute(pos, "width", Value::from(to)).is_ok()
                            && tr.set_node_attribute(pos, "height", Value::Null).is_ok();
                        if sized {
                            let _ = session.dispatch(tr);
                        }
                    }
                    dragged.set(None);
                });
            });
        };
        let handle = view()
            .absolute()
            .right(px(4.0))
            .bottom(px(4.0))
            .w(px(14.0))
            .h(px(14.0))
            .rounded(px(4.0))
            .border_2()
            .border_color(nsh_core::Color(0x1f2937cc))
            .bg(nsh_core::Color(0xffffffee))
            .cursor(Cursor::NwseResize)
            .label("Resize image")
            // Unseen rather than gone until the pointer is over the image, so
            // a press that arrives with the pointer still lands on it.
            .style_when(move || !hovered.get() && dragged.get().is_none(), |s| s.opacity(0.0))
            .on_press_down(resize);
        view()
            .relative()
            .w(move || px(size().0))
            .h(move || px(size().1))
            .on_mouse_enter(move |_: &MouseEvent| hovered.set(true))
            .on_mouse_leave(move |_: &MouseEvent| hovered.set(false))
            .child(el)
            .child(handle)
            .into_element()
    }
}

// ----- The YouTube card ---------------------------------------------------------------

thread_local! {
    /// The cards whose title is being fetched, by session and video (two
    /// windows can show one session's card).
    static FETCHING: RefCell<Vec<(EditorSession, String)>> = const { RefCell::new(Vec::new()) };
}

/// Starts fetching the title of `id` in `session` unless that's under way.
fn start_fetch(session: &EditorSession, id: &str) -> bool {
    FETCHING.with(|f| {
        let mut f = f.borrow_mut();
        let busy = f.iter().any(|(s, i)| s == session && i == id);
        if !busy {
            f.push((session.clone(), id.to_string()));
        }
        !busy
    })
}

fn end_fetch(session: &EditorSession, id: &str) {
    FETCHING.with(|f| f.borrow_mut().retain(|(s, i)| !(s == session && i == id)));
}

const PLAY_ICON: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 68 48"><path d="M66.5 7.7a8.5 8.5 0 0 0-6-6C55.2.3 34 .3 34 .3s-21.2 0-26.5 1.4a8.5 8.5 0 0 0-6 6C.1 13 .1 24 .1 24s0 11 1.4 16.3a8.5 8.5 0 0 0 6 6C12.8 47.7 34 47.7 34 47.7s21.2 0 26.5-1.4a8.5 8.5 0 0 0 6-6C67.9 35 67.9 24 67.9 24s0-11-1.4-16.3z" fill="#ff0033" fill-opacity="0.92"/><path d="M45 24 27 14v20z" fill="#fff"/></svg>"##;

const SMALL_ICON: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 68 48"><path d="M66.5 7.7a8.5 8.5 0 0 0-6-6C55.2.3 34 .3 34 .3s-21.2 0-26.5 1.4a8.5 8.5 0 0 0-6 6C.1 13 .1 24 .1 24s0 11 1.4 16.3a8.5 8.5 0 0 0 6 6C12.8 47.7 34 47.7 34 47.7s21.2 0 26.5-1.4a8.5 8.5 0 0 0 6-6C67.9 35 67.9 24 67.9 24s0-11-1.4-16.3z" fill="#ff0033"/><path d="M45 24 27 14v20z" fill="#fff"/></svg>"##;

/// A card: the thumbnail with a play button, the title under it. A press
/// on the thumbnail plays the video there (YouTube's player in a web view,
/// made only then); "Stop" brings the thumbnail back and ↗ opens the video
/// in the browser. Pressing the title only selects the card.
struct YoutubeView;

impl NodeView for YoutubeView {
    fn create(&self, cx: &NodeViewCx, first: &Node) -> AnyElement {
        let node = cx.node();
        let selected = cx.selected();
        let attr = move |name: &'static str| {
            move || node.with(|n| n.attrs().get(name).and_then(Value::as_str).unwrap_or_default().to_string())
        };
        let (id, url, title) = (attr("videoId"), attr("url"), attr("title"));
        let ring = move || if selected.get() { theme().primary } else { theme().border };

        let first_id = first.attrs().get("videoId").and_then(Value::as_str).unwrap_or_default().to_string();
        if first_id.is_empty() {
            return view()
                .w_full()
                .px_4()
                .py(px(18.0))
                .gap_1()
                .items_center()
                .rounded(px(12.0))
                .border_2()
                .border_dashed()
                .border_color(ring)
                .bg(move || theme().muted)
                .child(svg(ImageSrc::svg_markup(SMALL_ICON)).w(px(28.0)).h(px(20.0)))
                .child(text("Paste a YouTube link").text_size(px(13.0)).font_weight(FontWeight::MEDIUM).text_color(move || theme().foreground))
                .child(
                    text(move || if selected.get() { "Press Ctrl+V to turn this card into the video" } else { "Select this card, then paste a link" })
                        .text_size(px(11.0))
                        .text_center()
                        .text_color(move || theme().muted_foreground),
                )
                .into_element();
        }

        let first_title = first.attrs().get("title").and_then(Value::as_str).unwrap_or_default();
        if first_title.is_empty() && start_fetch(&cx.session, &first_id) {
            fetch_title(cx, first_id.clone(), first.attrs().get("url").and_then(Value::as_str).unwrap_or_default().to_string());
            // The fetch ends with this element (it's made again on some
            // changes): the next one may fetch again.
            let (session, id) = (cx.session.clone(), first_id.clone());
            cleanup(move || end_fetch(&session, &id));
        }

        let open = url.clone();
        // The video plays in the card once its picture is pressed.
        let playing = signal(false);
        let shown_title = move || {
            let t = title();
            if t.is_empty() { "YouTube video".to_string() } else { t }
        };
        view()
            .w_full()
            .rounded(px(12.0))
            .overflow_hidden()
            .border_1()
            .border_color(ring)
            .style_when(move || selected.get(), |s| s.border_2())
            .bg(move || theme().card)
            .shadow_sm()
            .child(
                view().relative().w_full().aspect_ratio(16.0 / 9.0).bg(nsh_core::Color(0x000000ff)).child(dynamic(move || {
                    let (id, url) = (id(), url());
                    if playing.get() {
                        let start = youtube_id(&url).and_then(|(_, start)| start);
                        web_view_html(player_page(&id, start), Some(PLAYER_BASE)).absolute().inset_0().scrollable(false).into_element()
                    } else {
                        view()
                            .absolute()
                            .inset_0()
                            .cursor(Cursor::Pointer)
                            .label("Play the video")
                            .on_press(move || playing.set(true))
                            .child(image(thumbnail_url(&id)).absolute().inset_0().object_fit(ResizeMode::Cover))
                            .child(view().absolute().inset_0().items_center().justify_center().child(svg(ImageSrc::svg_markup(PLAY_ICON)).w(px(56.0)).h(px(40.0))))
                            .into_element()
                    }
                })),
            )
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py(px(8.0))
                    .child(svg(ImageSrc::svg_markup(SMALL_ICON)).w(px(16.0)).h(px(12.0)))
                    .child(
                        text(shown_title)
                            .flex_1()
                            .truncate()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(move || theme().card_foreground),
                    )
                    .child(
                        text("Stop")
                            .style_when(move || !playing.get(), |s| s.hidden())
                            .px_2()
                            .rounded(px(6.0))
                            .text_size(px(12.0))
                            .text_color(move || theme().muted_foreground)
                            .hover(|s| s.bg(nsh_core::Color(0x00000018)))
                            .cursor(Cursor::Pointer)
                            .label("Stop the video")
                            .on_press(move || playing.set(false)),
                    )
                    .child(
                        text("↗")
                            .px(px(6.0))
                            .rounded(px(6.0))
                            .text_size(px(13.0))
                            .text_color(move || theme().muted_foreground)
                            .hover(|s| s.bg(nsh_core::Color(0x00000018)))
                            .cursor(Cursor::Pointer)
                            .label("Open on YouTube in the browser")
                            .on_press(move || {
                                let _ = open_url(open());
                            }),
                    ),
            )
            .into_element()
    }

    /// The same element follows the card's title as it arrives.
    fn update(&self, _: &Node, _: &[nsh_base::rich_editor::state::Decoration]) -> bool {
        true
    }
}

/// Fetches the video's title and stores it in the card (not as a step of
/// the undo history). Offline, the card keeps saying "YouTube video".
fn fetch_title(cx: &NodeViewCx, id: String, url: String) {
    let session = cx.session.clone();
    let get_pos = cx.get_pos();
    let _ = spawn(async move {
        let title = match fetch(oembed_url(&url)).await {
            Ok(response) if response.ok() => serde_json::from_slice::<Value>(&response.body)
                .ok()
                .and_then(|v| v.get("title").and_then(Value::as_str).map(str::to_string)),
            _ => None,
        };
        end_fetch(&session, &id);
        let Some(title) = title.filter(|t| !t.is_empty()) else { return };
        let Some(pos) = get_pos() else { return };
        let state = session.state();
        // The card may have moved or changed while the title came.
        let same = state.doc().node_at(pos).is_some_and(|n| {
            n.ty().name() == "youtube" && n.attrs().get("videoId").and_then(Value::as_str) == Some(id.as_str())
        });
        if !same {
            return;
        }
        let mut tr = state.tr();
        if tr.set_node_attribute(pos, "title", Value::String(title)).is_ok() {
            tr.set_add_to_history(false);
            let _ = session.dispatch(tr);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::{Format, same_content};
    use crate::kit::{Kit, Relay};

    #[test]
    fn youtube_links_of_every_shape() {
        let id = "dQw4w9WgXcQ";
        let plain = Some((id.to_string(), None));
        for url in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "http://youtube.com/watch?v=dQw4w9WgXcQ",
            "youtube.com/watch?v=dQw4w9WgXcQ",
            "https://m.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://www.youtube.com/watch?feature=share&v=dQw4w9WgXcQ&list=PL123",
            "https://youtu.be/dQw4w9WgXcQ",
            "https://youtu.be/dQw4w9WgXcQ?si=abc",
            "www.youtu.be/dQw4w9WgXcQ",
            "https://www.youtube.com/shorts/dQw4w9WgXcQ",
            "https://youtube.com/shorts/dQw4w9WgXcQ?feature=share",
            "https://music.youtube.com/watch?v=dQw4w9WgXcQ&feature=share",
            "https://www.youtube.com/embed/dQw4w9WgXcQ",
            "  https://www.youtube.com/watch?v=dQw4w9WgXcQ\n",
        ] {
            assert_eq!(youtube_id(url), plain, "{url}");
        }
        assert_eq!(youtube_id("https://youtu.be/dQw4w9WgXcQ?t=42"), Some((id.into(), Some(42))));
        assert_eq!(youtube_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=1m30s"), Some((id.into(), Some(90))));
        assert_eq!(youtube_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=90s"), Some((id.into(), Some(90))));
        assert_eq!(youtube_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ#t=1h0m5s"), Some((id.into(), Some(3605))));
        assert_eq!(youtube_id("https://www.youtube.com/embed/dQw4w9WgXcQ?start=7"), Some((id.into(), Some(7))));
        for not in [
            "",
            "https://vimeo.com/123456",
            "https://example.com/watch?v=dQw4w9WgXcQ",
            "https://notyoutube.com/watch?v=dQw4w9WgXcQ",
            "https://www.youtube.com/",
            "https://www.youtube.com/channel/UC123",
            "https://www.youtube.com/watch?v=short",
            "https://www.youtube.com/watch?list=PL123",
            "look: https://youtu.be/dQw4w9WgXcQ",
            "https://youtu.be/dQw4w9WgXcQ https://youtu.be/dQw4w9WgXcQ",
        ] {
            assert_eq!(youtube_id(not), None, "{not}");
        }
    }

    #[test]
    fn the_opened_link_keeps_the_start_time() {
        assert_eq!(watch_url("dQw4w9WgXcQ", Some(90)), "https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=90s");
        assert_eq!(watch_url("dQw4w9WgXcQ", None), "https://www.youtube.com/watch?v=dQw4w9WgXcQ");
        assert!(oembed_url("https://www.youtube.com/watch?v=a&t=1s").ends_with("url=https%3A%2F%2Fwww.youtube.com%2Fwatch%3Fv%3Da%26t%3D1s&format=json"));
    }

    #[test]
    fn cards_round_trip_through_html() {
        let schema = Kit::new(Relay::default()).schema.clone();
        let html = r#"<p>Before</p><div data-youtube-card=""><a class="youtube-card" data-youtube="dQw4w9WgXcQ" href="https://www.youtube.com/watch?v=dQw4w9WgXcQ&amp;t=5s">Never &amp; gonna</a></div><div data-youtube-card=""><a class="youtube-card" data-youtube="aaaaaaaaaaa" href="https://www.youtube.com/watch?v=aaaaaaaaaaa">https://www.youtube.com/watch?v=aaaaaaaaaaa</a></div><div data-youtube-card=""><a class="youtube-card" data-youtube="" href=""></a></div><p>After</p>"#;
        let doc = Format::Html.parse(&schema, html).unwrap();
        let names: Vec<_> = (0..doc.child_count()).map(|i| doc.child(i).ty().name().to_string()).collect();
        assert_eq!(names, ["paragraph", "youtube", "youtube", "youtube", "paragraph"]);
        let card = doc.child(1);
        assert_eq!(card.attrs().get("videoId"), Some(&json!("dQw4w9WgXcQ")));
        assert_eq!(card.attrs().get("url"), Some(&json!("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=5s")));
        assert_eq!(card.attrs().get("title"), Some(&json!("Never & gonna")));
        assert_eq!(doc.child(2).attrs().get("title"), Some(&json!("")));
        assert_eq!(doc.child(3).attrs().get("videoId"), Some(&json!("")));

        let out = crate::formats::html_fragment(&doc).unwrap();
        assert!(out.contains(r#"<a class="youtube-card" data-youtube="dQw4w9WgXcQ""#), "{out}");
        let back = Format::Html.parse(&schema, &out).unwrap();
        assert!(same_content(&back, &doc), "{out}");
        assert_eq!(crate::formats::html_fragment(&back).unwrap(), out);
    }

    #[test]
    fn cards_are_links_in_markdown_and_text() {
        let schema = Kit::new(Relay::default()).schema.clone();
        let html = r#"<div data-youtube-card=""><a class="youtube-card" data-youtube="dQw4w9WgXcQ" href="https://www.youtube.com/watch?v=dQw4w9WgXcQ">A song</a></div><div data-youtube-card=""><a class="youtube-card" data-youtube="aaaaaaaaaaa" href="https://youtu.be/aaaaaaaaaaa">https://youtu.be/aaaaaaaaaaa</a></div>"#;
        let doc = Format::Html.parse(&schema, html).unwrap();
        let md = Format::Markdown.serialize(&doc).unwrap();
        assert_eq!(md, "[A song](https://www.youtube.com/watch?v=dQw4w9WgXcQ)\n\n[https://youtu.be/aaaaaaaaaaa](https://youtu.be/aaaaaaaaaaa)\n");
        let text = Format::Text.serialize(&doc).unwrap();
        assert!(text.contains("https://www.youtube.com/watch?v=dQw4w9WgXcQ"), "{text}");
    }

    #[test]
    fn a_pasted_link_alone_becomes_a_card() {
        let kit = Kit::new(Relay::default());
        let editor = kit.editor(None);
        let state = editor.state();
        let slice = Slice::new(Fragment::from(kit.schema.text("https://youtu.be/dQw4w9WgXcQ?t=3", vec![])), 0, 0);
        let mut applied = None;
        assert!(YoutubePaste.handle_paste(&state, &slice, &mut |tr| applied = Some(tr)));
        let doc = applied.unwrap().doc().clone();
        let card = doc.child(0);
        assert_eq!(card.ty().name(), "youtube");
        assert_eq!(card.attrs().get("url"), Some(&json!("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=3s")));

        let other = Slice::new(Fragment::from(kit.schema.text("see https://youtu.be/dQw4w9WgXcQ", vec![])), 0, 0);
        assert!(!YoutubePaste.handle_paste(&state, &other, &mut |_| {}));
    }

    /// What `handle_paste` makes of `link` pasted into `editor` (dispatched
    /// through its session), or `None` when it passes.
    fn paste(editor: &Editor, link: &str) -> Option<Node> {
        let state = editor.state();
        let slice = Slice::new(Fragment::from(editor.kit.schema.text(link, vec![])), 0, 0);
        let session = editor.session.clone();
        YoutubePaste.handle_paste(&state, &slice, &mut |tr| {
            let _ = session.dispatch(tr);
        })
        .then(|| editor.doc())
    }

    fn names(doc: &Node) -> Vec<String> {
        (0..doc.child_count()).map(|i| doc.child(i).ty().name().to_string()).collect()
    }

    #[test]
    fn a_link_pasted_on_the_last_empty_line_leaves_the_caret_after_the_card() {
        let kit = Kit::new(Relay::default());
        let editor = kit.editor(Some(Format::Html.parse(&kit.schema, "<p>Hello</p><p></p>").unwrap()));
        let end = editor.doc().content().size() - 1;
        let mut tr = editor.state().tr();
        tr.set_selection(Selection::near(&tr.doc().resolve(end).unwrap(), 1));
        editor.session.dispatch(tr).unwrap();
        let doc = paste(&editor, "https://youtu.be/dQw4w9WgXcQ").unwrap();
        assert_eq!(names(&doc), ["paragraph", "youtube", "paragraph"]);
        let selection = editor.state().selection().clone();
        assert_eq!(selection.json_id(), "text");
        assert!(selection.from() > 7 + doc.child(1).node_size());
    }

    #[test]
    fn a_link_pasted_in_a_line_of_text_splits_it_around_the_card() {
        let kit = Kit::new(Relay::default());
        let editor = kit.editor(Some(Format::Html.parse(&kit.schema, "<p>Hello</p>").unwrap()));
        // The caret is at the start: the card goes before the text.
        let doc = paste(&editor, "https://youtu.be/dQw4w9WgXcQ").unwrap();
        assert!(names(&doc).contains(&"youtube".to_string()), "{:?}", names(&doc));
        assert!(doc.text_content().contains("Hello"));
    }

    #[test]
    fn a_link_pasted_on_the_empty_card_fills_it() {
        let kit = Kit::new(Relay::default());
        let editor = kit.editor(Some(Format::Html.parse(&kit.schema, "<p>Notes</p>").unwrap()));
        assert!(editor.run_command(&youtube_command()));
        let doc = paste(&editor, "https://www.youtube.com/watch?v=dQw4w9WgXcQ").unwrap();
        let cards: Vec<Node> = (0..doc.child_count()).map(|i| doc.child(i).clone()).filter(|n| n.ty().name() == "youtube").collect();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].attrs().get("videoId"), Some(&json!("dQw4w9WgXcQ")));
    }

    #[test]
    fn the_action_takes_a_selected_link_inside_a_line() {
        let kit = Kit::new(Relay::default());
        let editor = kit.editor(Some(Format::Html.parse(&kit.schema, "<p>see https://youtu.be/dQw4w9WgXcQ now</p>").unwrap()));
        // Select the link alone: after "see " (the paragraph's text starts at 1).
        let (from, to) = (1 + 4, 1 + 4 + "https://youtu.be/dQw4w9WgXcQ".len());
        let mut tr = editor.state().tr();
        tr.set_selection(nsh_base::rich_editor::state::TextSelection::create(tr.doc(), from, Some(to)));
        editor.session.dispatch(tr).unwrap();
        assert!(editor.run_command(&youtube_command()));
        let doc = editor.doc();
        assert!(names(&doc).contains(&"youtube".to_string()), "{:?}", names(&doc));
        assert!(doc.text_content().contains("see") && doc.text_content().contains("now"));
        assert!(!doc.text_content().contains("youtu.be"));
    }

    #[test]
    fn the_action_turns_the_line_into_a_card_or_adds_an_empty_one() {
        let kit = Kit::new(Relay::default());
        let doc = Format::Html.parse(&kit.schema, "<p>https://www.youtube.com/shorts/dQw4w9WgXcQ</p>").unwrap();
        let editor = kit.editor(Some(doc));
        assert!(editor.run_command(&youtube_command()));
        assert_eq!(editor.doc().child(0).ty().name(), "youtube");
        assert_eq!(editor.doc().child(0).attrs().get("videoId"), Some(&json!("dQw4w9WgXcQ")));

        let editor = kit.editor(Some(Format::Html.parse(&kit.schema, "<p>Hello</p>").unwrap()));
        assert!(editor.run_command(&youtube_command()));
        let doc = editor.doc();
        assert_eq!(doc.child(0).ty().name(), "paragraph");
        assert_eq!(doc.child(1).ty().name(), "youtube");
        assert_eq!(doc.child(1).attrs().get("videoId"), Some(&json!("")));
        assert_eq!(editor.state().selection().json_id(), "node");
    }

    #[test]
    fn pasted_images_get_new_names_in_attachments() {
        let tmp = std::env::temp_dir().join(format!("bubblenotes-media-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let a = save_pasted(&tmp, "20261114-093005", "png", b"one").unwrap();
        let b = save_pasted(&tmp, "20261114-093005", "png", b"two").unwrap();
        assert_eq!(a, tmp.join("attachments").join("pasted-20261114-093005-1.png"));
        assert_eq!(b, tmp.join("attachments").join("pasted-20261114-093005-2.png"));
        assert_eq!(std::fs::read(&a).unwrap(), b"one");
        assert_eq!(stamp_now().len(), "20261114-093005".len());
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn pasted_images_are_saved_as_already_in_place() {
        let kit = Kit::new(Relay::default());
        let tmp = std::env::temp_dir().join(format!("bubblenotes-paste-test-{}", std::process::id()));
        let note = tmp.join("Work").join("note.html");
        std::fs::create_dir_all(note.parent().unwrap()).unwrap();
        let saved = save_pasted(note.parent().unwrap(), &stamp_now(), "png", b"png").unwrap();
        let editor = kit.editor(None);
        assert!(insert_images(&editor, std::slice::from_ref(&saved)));
        let (written, moved) = crate::formats::images_for_saving(&kit.schema, &editor.doc(), &note).unwrap();
        assert!(moved.is_empty());
        let html = crate::formats::html_fragment(&written).unwrap();
        assert!(html.contains("src=\"attachments/pasted-"), "{html}");
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn file_urls_become_paths() {
        if cfg!(windows) {
            assert_eq!(path_from_uri("file:///C:/My%20Pictures/caf%C3%A9.png"), Some(PathBuf::from(r"C:\My Pictures\café.png")));
            assert_eq!(path_from_uri("file://server/share/a.png"), Some(PathBuf::from(r"\\server\share\a.png")));
            assert_eq!(path_from_uri(r"C:\a.png"), Some(PathBuf::from(r"C:\a.png")));
        }
        assert_eq!(path_from_uri("relative.png"), None);
        assert_eq!(image_extension("shot.PNG", ""), Some("png"));
        assert_eq!(image_extension("image.png", "image/jpeg"), Some("jpg"));
        assert_eq!(image_extension("notes.txt", "text/plain"), None);
    }
}
