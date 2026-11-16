//! The notes folder (a "vault") on disk: one HTML file per note, a folder
//! per category, and an index of everything else about the notes
//! (`.bubblenotes/index.json`).
//!
//! ```text
//! Bubble Notes/
//!   Shopping.html              a note with no category
//!   Work/Ideas/Launch.html     a note in the category "Work/Ideas"
//!   attachments/               images of the notes in this folder
//!   .bubblenotes/index.json    looks, positions, alarms, settings
//!   .bubblenotes/trash/        deleted notes, for a while
//!   .bubblenotes/history/<id>/ earlier versions of a note's text
//! ```
//!
//! The folder is the user's: HTML files they add are picked up as notes,
//! and files they delete drop out of the index (`Vault::load`). Each note's
//! file carries its id, so the index can find it again after a move.
//!
//! Nothing here knows about the editor or the UI: bodies are HTML strings.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::model::{Index, NoteMeta};

const META_DIR: &str = ".bubblenotes";
const INDEX_FILE: &str = "index.json";
const TRASH_DIR: &str = "trash";
const HISTORY_DIR: &str = "history";
/// A note's text is kept as an earlier version at most this often
/// (milliseconds): a version is the text as it was before a spell of
/// editing, not every save.
pub const VERSION_EVERY: f64 = 10.0 * 60_000.0;
/// How many earlier versions a note keeps.
const VERSIONS_KEPT: usize = 40;
/// Folders that aren't categories.
const RESERVED_DIRS: &[&str] = &["attachments"];

pub struct Vault {
    root: PathBuf,
}

/// Now, in milliseconds since the Unix epoch.
pub fn now_ms() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as f64).unwrap_or(0.0)
}

/// A new note id: unique on this machine, sortable by when it was made.
pub fn new_id() -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed) & 0xFFFF;
    format!("{:011x}{:04x}", now_ms() as u64, n)
}

impl Vault {
    /// The notes folder at `root`, made when it isn't there.
    pub fn open(root: impl Into<PathBuf>) -> io::Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(root.join(META_DIR).join(TRASH_DIR))?;
        Ok(Self { root })
    }

    /// Where the notes folder is by default: `Documents/Bubble Notes`.
    pub fn default_root() -> PathBuf {
        let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_default();
        home.join("Documents").join("Bubble Notes")
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn index_path(&self) -> PathBuf {
        self.root.join(META_DIR).join(INDEX_FILE)
    }

    /// A note's file (in the trash when it's deleted).
    pub fn path_of(&self, meta: &NoteMeta) -> PathBuf {
        if meta.trashed.is_some() {
            self.root.join(META_DIR).join(TRASH_DIR).join(format!("{}.html", meta.id))
        } else {
            self.root.join(meta.file.replace('/', std::path::MAIN_SEPARATOR_STR))
        }
    }

    /// The index, brought in line with the files: notes whose file is gone
    /// are dropped, files moved by hand are found again by their id, and
    /// HTML files the index doesn't know become notes.
    pub fn load(&self) -> Index {
        let mut index: Index = std::fs::read_to_string(self.index_path())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        // An id that can't be one (the index was edited by hand) is replaced.
        for note in index.notes.iter_mut().filter(|n| !valid_id(&n.id)) {
            note.id = new_id();
        }
        let mut on_disk = Vec::new();
        self.scan(&self.root, "", &mut on_disk);

        let mut claimed: HashSet<String> = HashSet::new();
        // Notes whose file is where the index says.
        for note in index.notes.iter().filter(|n| n.trashed.is_none()) {
            if on_disk.iter().any(|(file, _)| *file == note.file) {
                claimed.insert(note.file.clone());
            }
        }
        // Notes whose file moved: found by the id inside it.
        for note in index.notes.iter_mut().filter(|n| n.trashed.is_none()) {
            if claimed.contains(&note.file) {
                continue;
            }
            if let Some((file, _)) = on_disk.iter().find(|(file, id)| !claimed.contains(file) && id.as_deref() == Some(&note.id)) {
                note.file = file.clone();
                note.category = category_of(file);
                claimed.insert(file.clone());
            }
        }
        index.notes.retain(|n| if n.trashed.is_some() { self.path_of(n).exists() } else { claimed.contains(&n.file) });

        // Files nobody claimed are new notes.
        let known: HashSet<String> = index.notes.iter().map(|n| n.id.clone()).collect();
        for (file, id) in on_disk {
            if claimed.contains(&file) {
                continue;
            }
            let now = now_ms();
            let title = Path::new(&file).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            index.notes.push(NoteMeta {
                // A copied file has its original's id: it gets its own.
                id: id.filter(|id| !known.contains(id)).unwrap_or_else(new_id),
                title,
                category: category_of(&file),
                file,
                created: now,
                modified: now,
                on_desktop: false,
                look: index.settings.default_look.clone(),
                ..NoteMeta::default()
            });
        }
        index
    }

    /// The note files under `dir` as (relative path, the id inside).
    fn scan(&self, dir: &Path, prefix: &str, found: &mut Vec<(String, Option<String>)>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            let relative = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            if path.is_dir() {
                if !name.starts_with('.') && !RESERVED_DIRS.contains(&name.as_str()) {
                    self.scan(&path, &relative, found);
                }
            } else if name.to_lowercase().ends_with(".html") {
                let id = std::fs::read_to_string(&path).ok().and_then(|text| id_in(&text));
                found.push((relative, id));
            }
        }
    }

    pub fn save_index(&self, index: &Index) -> io::Result<()> {
        let text = serde_json::to_string_pretty(index).map_err(io::Error::other)?;
        write_atomic(&self.index_path(), &text)
    }

    /// A note's text, as the HTML of its body.
    pub fn read_body(&self, meta: &NoteMeta) -> io::Result<String> {
        Ok(body_of(&std::fs::read_to_string(self.path_of(meta))?).to_string())
    }

    /// Writes a note's file (`body` is the HTML of its content). The note
    /// must have a place (`place`).
    pub fn write_body(&self, meta: &NoteMeta, body: &str) -> io::Result<()> {
        let path = self.path_of(meta);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // What's about to be replaced stays as an earlier version.
        if let Ok(old) = self.read_body(meta)
            && old != body
        {
            let _ = self.keep_version(&meta.id, meta.modified, &old, VERSION_EVERY);
        }
        write_atomic(&path, &document(meta, body))
    }

    /// Gives the note the file its title and category call for, moving its
    /// file when it has one elsewhere. Call it for a new note and after its
    /// title or category changes. Names taken by other files get " (2)", ...
    pub fn place(&self, meta: &mut NoteMeta) -> io::Result<()> {
        let category = clean_category(&meta.category);
        meta.category = category.clone();
        let stem = file_stem_for(&meta.title);
        let wanted = |n: u32| {
            let name = if n <= 1 { format!("{stem}.html") } else { format!("{stem} ({n}).html") };
            if category.is_empty() { name } else { format!("{category}/{name}") }
        };
        // Its name is fine as it is.
        if wanted(1) == meta.file {
            return Ok(());
        }
        let old = (!meta.file.is_empty()).then(|| self.root.join(meta.file.replace('/', std::path::MAIN_SEPARATOR_STR)));
        let mut n = 1;
        let file = loop {
            let candidate = wanted(n);
            let path = self.root.join(candidate.replace('/', std::path::MAIN_SEPARATOR_STR));
            // Free, or it's this note's own file under another letter case.
            if !path.exists() || old.as_ref().is_some_and(|old| same_file(old, &path)) {
                break candidate;
            }
            n += 1;
        };
        let new = self.root.join(file.replace('/', std::path::MAIN_SEPARATOR_STR));
        if let Some(dir) = new.parent() {
            std::fs::create_dir_all(dir)?;
        }
        if meta.trashed.is_none()
            && let Some(old) = old.filter(|old| old.exists() && *old != new)
        {
            std::fs::rename(old, &new)?;
        }
        meta.file = file;
        Ok(())
    }

    /// Moves the note to the trash.
    pub fn trash(&self, meta: &mut NoteMeta) -> io::Result<()> {
        if meta.trashed.is_some() {
            return Ok(());
        }
        let from = self.path_of(meta);
        meta.trashed = Some(now_ms());
        let to = self.path_of(meta);
        if from.exists() { std::fs::rename(from, to) } else { Ok(()) }
    }

    /// Brings the note back from the trash, to its category.
    pub fn restore(&self, meta: &mut NoteMeta) -> io::Result<()> {
        if meta.trashed.is_none() {
            return Ok(());
        }
        let from = self.path_of(meta);
        meta.trashed = None;
        meta.file = String::new();
        self.place(meta)?;
        let to = self.path_of(meta);
        if from.exists() { std::fs::rename(from, to) } else { Ok(()) }
    }

    // ----- Earlier versions ----------------------------------------------------

    fn history_dir(&self, id: &str) -> PathBuf {
        self.root.join(META_DIR).join(HISTORY_DIR).join(id)
    }

    fn version_path(&self, id: &str, at: f64) -> PathBuf {
        self.history_dir(id).join(format!("{}.html", at as u64))
    }

    /// When the note's earlier versions are from (each the time its text
    /// was last changed, in milliseconds since the Unix epoch), newest
    /// first.
    pub fn versions(&self, id: &str) -> Vec<f64> {
        if !valid_id(id) {
            return Vec::new();
        }
        let mut found: Vec<u64> = std::fs::read_dir(self.history_dir(id))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| entry.file_name().to_string_lossy().strip_suffix(".html")?.parse().ok())
            .collect();
        found.sort_unstable_by(|a, b| b.cmp(a));
        found.into_iter().map(|at| at as f64).collect()
    }

    /// The text of the version from `at`, as the HTML of its body.
    pub fn read_version(&self, id: &str, at: f64) -> io::Result<String> {
        std::fs::read_to_string(self.version_path(id, at))
    }

    /// Keeps `body` as the note's version from `at`, unless one was kept
    /// less than `every` milliseconds ago (0: always) or the text is
    /// empty. Only the newest ones stay. Says whether it was kept.
    pub fn keep_version(&self, id: &str, at: f64, body: &str, every: f64) -> io::Result<bool> {
        if !valid_id(id) || body.trim().is_empty() || body.trim() == "<p></p>" {
            return Ok(false);
        }
        let versions = self.versions(id);
        if every > 0.0
            && let Some(&newest) = versions.first()
        {
            // How long ago it was kept is the file's own age: `newest` is
            // when that text was written, which may be long before.
            let kept = std::fs::metadata(self.version_path(id, newest)).and_then(|m| m.modified()).ok();
            let age = kept.and_then(|t| t.elapsed().ok()).map(|d| d.as_millis() as f64);
            if age.is_none_or(|age| age < every) {
                return Ok(false);
            }
        }
        if versions.first().is_some_and(|&newest| self.read_version(id, newest).is_ok_and(|text| text == body)) {
            return Ok(false);
        }
        std::fs::create_dir_all(self.history_dir(id))?;
        write_atomic(&self.version_path(id, at), body)?;
        for old in self.versions(id).into_iter().skip(VERSIONS_KEPT) {
            let _ = std::fs::remove_file(self.version_path(id, old));
        }
        Ok(true)
    }

    /// Deletes the note's file for good (wherever it is), and its earlier
    /// versions.
    pub fn delete_forever(&self, meta: &NoteMeta) -> io::Result<()> {
        if valid_id(&meta.id) {
            let _ = std::fs::remove_dir_all(self.history_dir(&meta.id));
        }
        match std::fs::remove_file(self.path_of(meta)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// Every category: the folders, the notes' and the empty ones the
    /// settings keep, each with its parents, sorted.
    pub fn categories(&self, index: &Index) -> Vec<String> {
        let mut all: HashSet<String> = HashSet::new();
        let mut add = |category: &str| {
            let mut path = String::new();
            for part in category.split('/').filter(|p| !p.is_empty()) {
                if !path.is_empty() {
                    path.push('/');
                }
                path.push_str(part);
                all.insert(path.clone());
            }
        };
        for note in index.notes.iter().filter(|n| n.trashed.is_none()) {
            add(&note.category);
        }
        for category in &index.settings.categories {
            add(category);
        }
        let mut folders = Vec::new();
        self.scan_folders(&self.root, "", &mut folders);
        for folder in &folders {
            add(folder);
        }
        let mut all: Vec<String> = all.into_iter().collect();
        all.sort_by_key(|c| c.to_lowercase());
        all
    }

    fn scan_folders(&self, dir: &Path, prefix: &str, found: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.path().is_dir() && !name.starts_with('.') && !RESERVED_DIRS.contains(&name.as_str()) {
                let relative = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
                self.scan_folders(&entry.path(), &relative, found);
                found.push(relative);
            }
        }
    }

    /// Removes a category's folder when nothing is left in it.
    pub fn remove_empty_category(&self, category: &str) {
        let category = clean_category(category);
        if !category.is_empty() {
            // Its images went along with its notes; an empty folder of them
            // would keep the category's folder (and the category) around.
            let folder = self.root.join(category.replace('/', std::path::MAIN_SEPARATOR_STR));
            let _ = std::fs::remove_dir(folder.join(RESERVED_DIRS[0]));
            let _ = std::fs::remove_dir(self.root.join(category.replace('/', std::path::MAIN_SEPARATOR_STR)));
        }
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy())
}

/// Writes through a temporary file, so a crash never leaves half a file.
fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    std::fs::write(&temp, text)?;
    std::fs::rename(&temp, path)
}

/// The category a file's folder stands for.
fn category_of(file: &str) -> String {
    file.rsplit_once('/').map(|(dir, _)| dir.to_string()).unwrap_or_default()
}

/// A category path with each part made a valid folder name.
pub fn clean_category(category: &str) -> String {
    category
        .split(['/', '\\'])
        .map(file_stem_for_part)
        .filter(|p| !p.is_empty())
        .filter(|p| !RESERVED_DIRS.contains(&p.to_lowercase().as_str()))
        .collect::<Vec<_>>()
        .join("/")
}

fn file_stem_for_part(part: &str) -> String {
    let cleaned: String = part.chars().map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '-' } else { c }).collect();
    let cleaned = cleaned.trim().trim_matches('.').trim();
    cleaned.chars().take(80).collect::<String>().trim().to_string()
}

/// A file name (without extension) for a note titled `title`.
pub fn file_stem_for(title: &str) -> String {
    let stem = file_stem_for_part(title);
    const DEVICES: &[&str] = &["con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "lpt1", "lpt2", "lpt3"];
    if stem.is_empty() {
        "Untitled".into()
    } else if DEVICES.contains(&stem.to_lowercase().as_str()) {
        format!("_{stem}")
    } else {
        stem
    }
}

// ----- The note file ------------------------------------------------------------

const ID_META: &str = "<meta name=\"bubblenotes-id\" content=\"";

/// A note's file: a whole HTML page any browser opens.
fn document(meta: &NoteMeta, body: &str) -> String {
    format!(
        "<!doctype html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n{ID_META}{}\">\n<title>{}</title>\n</head>\n<body>\n{body}\n</body>\n</html>\n",
        escape(&meta.id),
        escape(&meta.title),
    )
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Whether `id` can be a note's id: letters, digits, `-` and `_`. An id
/// names files and folders (the note's in the trash, its earlier
/// versions), and a file dropped into the notes folder can claim any:
/// one like `..` must never be taken for a path.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The id a note file carries.
fn id_in(text: &str) -> Option<String> {
    let start = text.find(ID_META)? + ID_META.len();
    let end = text[start..].find('"')?;
    let id = &text[start..start + end];
    valid_id(id).then(|| id.to_string())
}

/// What's inside `<body>` (the whole text of a file without one).
fn body_of(text: &str) -> &str {
    let Some(open) = text.find("<body") else { return text };
    let Some(open_end) = text[open..].find('>') else { return text };
    let start = open + open_end + 1;
    let end = text.rfind("</body>").filter(|end| *end >= start).unwrap_or(text.len());
    text[start..end].trim_matches(['\n', '\r'])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_vault(name: &str) -> Vault {
        let root = std::env::temp_dir().join(format!("bubblenotes-test-{name}-{}", new_id()));
        Vault::open(root).unwrap()
    }

    fn note(vault: &Vault, title: &str, category: &str, body: &str) -> NoteMeta {
        let mut meta = NoteMeta { id: new_id(), title: title.into(), category: category.into(), ..NoteMeta::default() };
        vault.place(&mut meta).unwrap();
        vault.write_body(&meta, body).unwrap();
        meta
    }

    #[test]
    fn ids_are_unique() {
        let ids: HashSet<String> = (0..500).map(|_| new_id()).collect();
        assert_eq!(ids.len(), 500);
    }

    #[test]
    fn a_note_round_trips() {
        let vault = temp_vault("roundtrip");
        let meta = note(&vault, "Shopping: list?", "Home/Food", "<p>Milk &amp; eggs</p>");
        assert_eq!(meta.file, "Home/Food/Shopping- list-.html");
        assert_eq!(vault.read_body(&meta).unwrap(), "<p>Milk &amp; eggs</p>");
        let index = Index { notes: vec![meta.clone()], ..Index::default() };
        vault.save_index(&index).unwrap();
        assert_eq!(vault.load().notes, vec![meta]);
        std::fs::remove_dir_all(vault.root()).unwrap();
    }

    #[test]
    fn replaced_text_stays_as_an_earlier_version() {
        let vault = temp_vault("history");
        let mut meta = note(&vault, "Plan", "", "<p>one</p>");
        assert!(vault.versions(&meta.id).is_empty());
        meta.modified = 1000.0;
        vault.write_body(&meta, "<p>two</p>").unwrap();
        assert_eq!(vault.versions(&meta.id), vec![1000.0]);
        assert_eq!(vault.read_version(&meta.id, 1000.0).unwrap(), "<p>one</p>");
        // Soon after, the next save keeps nothing: one version for a spell
        // of editing.
        meta.modified = 2000.0;
        vault.write_body(&meta, "<p>three</p>").unwrap();
        assert_eq!(vault.versions(&meta.id), vec![1000.0]);
        // Writing the same text keeps nothing either; asking does, once.
        assert!(vault.keep_version(&meta.id, 3000.0, "<p>three</p>", 0.0).unwrap());
        assert!(!vault.keep_version(&meta.id, 4000.0, "<p>three</p>", 0.0).unwrap());
        assert_eq!(vault.versions(&meta.id), vec![3000.0, 1000.0]);
        // Only the newest stay.
        for i in 0..VERSIONS_KEPT + 5 {
            vault.keep_version(&meta.id, 5000.0 + i as f64, &format!("<p>{i}</p>"), 0.0).unwrap();
        }
        assert_eq!(vault.versions(&meta.id).len(), VERSIONS_KEPT);
        // Versions aren't notes, and go with the note.
        let index = Index { notes: vec![meta.clone()], ..Index::default() };
        vault.save_index(&index).unwrap();
        assert_eq!(vault.load().notes.len(), 1);
        vault.delete_forever(&meta).unwrap();
        assert!(vault.versions(&meta.id).is_empty());
        std::fs::remove_dir_all(vault.root()).unwrap();
    }

    #[test]
    fn an_id_that_is_a_path_is_never_used_as_one() {
        let vault = temp_vault("ids");
        let keep = note(&vault, "Keep", "", "<p>keep</p>");
        // A file claiming the id "..", and one claiming a path.
        for (name, id) in [("Evil.html", ".."), ("Worse.html", "..\\..\\x")] {
            std::fs::write(vault.root().join(name), format!("<html><head>{ID_META}{id}\"></head><body><p>x</p></body></html>")).unwrap();
        }
        let index = Index { notes: vec![keep.clone()], ..Index::default() };
        vault.save_index(&index).unwrap();
        let loaded = vault.load();
        assert_eq!(loaded.notes.len(), 3);
        assert!(loaded.notes.iter().all(|n| valid_id(&n.id)), "{:?}", loaded.notes.iter().map(|n| &n.id).collect::<Vec<_>>());
        // Even asked directly, such an id touches nothing.
        let evil = NoteMeta { id: "..".into(), file: "Evil.html".into(), ..NoteMeta::default() };
        assert!(!vault.keep_version("..", 1.0, "<p>x</p>", 0.0).unwrap());
        vault.delete_forever(&evil).unwrap();
        assert!(vault.root().join(META_DIR).join(INDEX_FILE).exists());
        assert_eq!(vault.read_body(&keep).unwrap(), "<p>keep</p>");
        // An index edited by hand to hold one gets a new id.
        std::fs::write(vault.root().join(META_DIR).join(INDEX_FILE), r#"{ "notes": [ { "id": "../..", "title": "Keep", "file": "Keep.html" } ] }"#).unwrap();
        assert!(vault.load().notes.iter().all(|n| valid_id(&n.id)));
        std::fs::remove_dir_all(vault.root()).unwrap();
    }

    #[test]
    fn same_titles_get_numbered_files() {
        let vault = temp_vault("names");
        let a = note(&vault, "Idea", "", "<p>a</p>");
        let b = note(&vault, "Idea", "", "<p>b</p>");
        assert_eq!((a.file.as_str(), b.file.as_str()), ("Idea.html", "Idea (2).html"));
        assert_eq!(vault.read_body(&a).unwrap(), "<p>a</p>");
        std::fs::remove_dir_all(vault.root()).unwrap();
    }

    #[test]
    fn renaming_and_moving_keeps_the_text() {
        let vault = temp_vault("move");
        let mut meta = note(&vault, "Draft", "", "<p>text</p>");
        let old = vault.path_of(&meta);
        meta.title = "Final".into();
        meta.category = "Work".into();
        vault.place(&mut meta).unwrap();
        assert_eq!(meta.file, "Work/Final.html");
        assert!(!old.exists());
        assert_eq!(vault.read_body(&meta).unwrap(), "<p>text</p>");
        // Placing again changes nothing.
        vault.place(&mut meta).unwrap();
        assert_eq!(meta.file, "Work/Final.html");
        std::fs::remove_dir_all(vault.root()).unwrap();
    }

    #[test]
    fn load_follows_the_folder() {
        let vault = temp_vault("reconcile");
        let moved = note(&vault, "Moved", "", "<p>m</p>");
        let gone = note(&vault, "Gone", "", "<p>g</p>");
        vault.save_index(&Index { notes: vec![moved.clone(), gone.clone()], ..Index::default() }).unwrap();
        // By hand: one file moved into a folder, one deleted, one added.
        std::fs::create_dir_all(vault.root().join("Archive")).unwrap();
        std::fs::rename(vault.path_of(&moved), vault.root().join("Archive").join("Renamed.html")).unwrap();
        std::fs::remove_file(vault.path_of(&gone)).unwrap();
        std::fs::write(vault.root().join("Dropped in.html"), "<p>hello</p>").unwrap();

        let index = vault.load();
        let by_id = |id: &str| index.notes.iter().find(|n| n.id == id);
        assert_eq!(by_id(&moved.id).map(|n| (n.file.as_str(), n.category.as_str())), Some(("Archive/Renamed.html", "Archive")));
        assert!(by_id(&gone.id).is_none());
        let added = index.notes.iter().find(|n| n.title == "Dropped in").expect("the new file is a note");
        assert!(!added.on_desktop);
        assert_eq!(vault.read_body(added).unwrap(), "<p>hello</p>");
        assert_eq!(index.notes.len(), 2);
        std::fs::remove_dir_all(vault.root()).unwrap();
    }

    #[test]
    fn trash_and_restore() {
        let vault = temp_vault("trash");
        let mut meta = note(&vault, "Old", "Work", "<p>old</p>");
        let file = vault.path_of(&meta);
        vault.trash(&mut meta).unwrap();
        assert!(!file.exists() && vault.path_of(&meta).exists());
        vault.save_index(&Index { notes: vec![meta.clone()], ..Index::default() }).unwrap();
        assert_eq!(vault.load().notes.len(), 1, "a trashed note stays in the index");
        vault.restore(&mut meta).unwrap();
        assert_eq!(meta.file, "Work/Old.html");
        assert_eq!(vault.read_body(&meta).unwrap(), "<p>old</p>");
        vault.delete_forever(&meta).unwrap();
        assert!(!vault.path_of(&meta).exists());
        std::fs::remove_dir_all(vault.root()).unwrap();
    }

    #[test]
    fn categories_include_parents_and_empty_ones() {
        let vault = temp_vault("categories");
        let meta = note(&vault, "A", "Work/Ideas", "");
        let mut index = Index { notes: vec![meta], ..Index::default() };
        index.settings.categories.push("Later".into());
        std::fs::create_dir_all(vault.root().join("attachments")).unwrap();
        assert_eq!(vault.categories(&index), ["Later", "Work", "Work/Ideas"]);
        std::fs::remove_dir_all(vault.root()).unwrap();
    }

    #[test]
    fn names_are_made_safe() {
        assert_eq!(file_stem_for("  a/b: c?  "), "a-b- c-");
        assert_eq!(file_stem_for("..."), "Untitled");
        assert_eq!(file_stem_for("CON"), "_CON");
        assert_eq!(clean_category("Work\\ Ideas /attachments//x"), "Work/Ideas/x");
    }

    #[test]
    fn body_and_id_are_read_back() {
        let meta = NoteMeta { id: "abc".into(), title: "T <1>".into(), ..NoteMeta::default() };
        let page = document(&meta, "<p>one</p>\n<p>two</p>");
        assert_eq!(id_in(&page).as_deref(), Some("abc"));
        assert_eq!(body_of(&page), "<p>one</p>\n<p>two</p>");
        assert_eq!(body_of("<p>bare</p>"), "<p>bare</p>");
    }
}
