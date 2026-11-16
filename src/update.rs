//! Updates. The app looks for a newer version among the releases of its
//! GitHub repository and, when told to (or by itself, with "Install updates
//! automatically"), puts it in place of its own program file; the new
//! version runs from the next start.
//!
//! The rules are `nsh-updater`'s: a release carries `latest.json` (the
//! version, the program's file, its SHA-256 and size) and `latest.json.sig`,
//! its signature by the key whose public half is `PUBLIC_KEY`. A manifest
//! that isn't signed by it, is for another app or isn't newer is refused,
//! and so is a program file that isn't what the manifest says. Releases
//! are made with `release.ps1` (`examples/release.rs`).
//!
//! Putting it in place is this module's own (the app is one file, perhaps
//! with a `Notes` folder beside it, which mustn't be touched): the new
//! file is written beside the program, the running program's file is
//! renamed aside (Windows lets a running program be renamed, not
//! overwritten) and the new one takes its name. What was renamed aside is
//! deleted at the next start.
//!
//! A development run (`BUBBLENOTES_DIR`) never asks GitHub; with
//! `BUBBLENOTES_UPDATE=<url of a latest.json>` it asks there (a local
//! server, for checking by hand).

use std::path::{Path, PathBuf};

use nsh_core::prelude::*;
use nsh_updater::{Available, Config, Download};

use crate::app::App;

/// The repository whose releases are the app's updates.
pub const REPO: &str = "MichealKennedy/bubblenotes-rust";
/// The public half of the key releases are signed with (`cargo run
/// --example release -- keygen` prints it).
pub const PUBLIC_KEY: &str = "cjlW0xDFfcXv+zxyhTiq8qg3FIZC0F/8PY8U8uiF/Wk=";
/// The app's id, as in `nsh.json`.
const ID: &str = "dev.bubblenotes.app";
/// The running version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// How long after the start the first look is, and how often after.
const FIRST_AFTER: std::time::Duration = std::time::Duration::from_secs(20);
const EVERY: std::time::Duration = std::time::Duration::from_secs(24 * 3600);

/// Where updating stands.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum State {
    /// Not looked yet.
    #[default]
    Idle,
    Checking,
    /// Looked: this is the newest version (or there's no release yet).
    UpToDate,
    /// A newer version waits to be installed.
    Available(Offer),
    Downloading(Offer),
    /// The newer version is in place: it runs from the next start.
    Ready(String),
    /// Looking or installing failed: why, for the user.
    Failed(String),
}

/// A newer version on offer.
#[derive(Clone, Debug, PartialEq)]
pub struct Offer {
    pub version: String,
    pub notes: Option<String>,
    url: String,
    sha256: String,
    size: Option<u64>,
}

impl State {
    /// In words, for the Options window.
    pub fn describe(&self) -> String {
        match self {
            State::Idle => String::new(),
            State::Checking => "Looking for updates…".into(),
            State::UpToDate => "You have the latest version.".into(),
            State::Available(offer) => format!("Version {} is available.", offer.version),
            State::Downloading(offer) => format!("Getting version {}…", offer.version),
            State::Ready(version) => format!("Version {version} is installed: it starts the next time the app does."),
            State::Failed(why) => format!("Couldn't update: {why}"),
        }
    }
}

/// Where this run looks for updates; `None`: nowhere (a development run).
fn config() -> Option<Config> {
    let test = std::env::var("BUBBLENOTES_UPDATE").ok().filter(|url| !url.is_empty());
    if test.is_none() && std::env::var_os("BUBBLENOTES_DIR").is_some() {
        return None;
    }
    config_for(test.as_deref())
}

/// The configuration: GitHub's releases, or the manifest at `url`.
fn config_for(url: Option<&str>) -> Option<Config> {
    let mut json = serde_json::json!({ "id": ID, "version": VERSION, "publicKey": PUBLIC_KEY });
    match url {
        Some(url) => json["url"] = url.into(),
        None => json["github"] = REPO.into(),
    }
    Config::parse(json.to_string().as_bytes()).inspect_err(|e| eprintln!("update: {e}")).ok()
}

/// What `manifest` (with its `signature`) offers this machine, if it's
/// newer than this version. An offer with no SHA-256 (a page to visit, not
/// a file) is refused: nothing unchecked is installed.
fn offer(config: &Config, manifest: &[u8], signature: &[u8]) -> Result<Option<Offer>, String> {
    let found: Option<Available> = config.check(manifest, Some(signature), &nsh_updater::platform()).map_err(|e| e.to_string())?;
    let Some(found) = found else { return Ok(None) };
    let sha256 = found.sha256.ok_or("the release names no checksum for its file")?;
    Ok(Some(Offer { version: found.version, notes: found.notes, url: found.url, sha256, size: found.size }))
}

/// Whether `bytes` are the file `offer` names (its size and SHA-256).
fn verified(offer: &Offer, bytes: &[u8]) -> Result<(), String> {
    let mut download = Download::new(&offer.sha256, offer.size);
    download.update(bytes).map_err(|e| e.to_string())?;
    download.finish().map_err(|e| e.to_string())
}

/// The names used beside the program's file while it's replaced.
fn beside(program: &Path, what: &str) -> PathBuf {
    let mut name = program.file_name().unwrap_or_default().to_os_string();
    name.push(what);
    program.with_file_name(name)
}

const NEW: &str = ".new";
const OLD: &str = ".old-";

/// Puts `bytes` in place of the program file at `program`: written beside
/// it, the program renamed aside, the new file given its name. If the last
/// step fails the program gets its name back. Nothing else in the folder
/// is touched.
fn replace_program(program: &Path, bytes: &[u8], stamp: u64) -> std::io::Result<()> {
    let (new, old) = (beside(program, NEW), beside(program, &format!("{OLD}{stamp}")));
    std::fs::write(&new, bytes)?;
    if let Err(e) = std::fs::rename(program, &old) {
        let _ = std::fs::remove_file(&new);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&new, program) {
        let _ = std::fs::rename(&old, program);
        let _ = std::fs::remove_file(&new);
        return Err(e);
    }
    Ok(())
}

/// Removes what an earlier update left beside `program`: the versions
/// renamed aside (in use until the app that ran them ended) and a new file
/// that never took its place.
fn remove_leftovers(program: &Path) {
    let Some(dir) = program.parent() else { return };
    let (new, old) = (beside(program, NEW), beside(program, OLD));
    let old = old.file_name().unwrap_or_default().to_string_lossy().to_string();
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path == new || entry.file_name().to_string_lossy().starts_with(&old) {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Starts looking for updates, as the settings say. Call once.
pub fn install(app: App) {
    if let Ok(program) = std::env::current_exe() {
        remove_leftovers(&program);
    }
    if config().is_none() {
        return;
    }
    // (A test run looks at once.)
    let first = if std::env::var_os("BUBBLENOTES_UPDATE").is_some() { std::time::Duration::from_secs(2) } else { FIRST_AFTER };
    let look = move || {
        if app.settings.with_untracked(|s| s.check_updates) {
            app.check_for_updates(false);
        }
    };
    let _ = set_timeout(first, look);
    let _ = set_interval(EVERY, look);
}

impl App {
    /// Looks for a newer version. `asked`: the user asked (a failure is
    /// then said; by itself the app keeps quiet about being offline or
    /// there being no release). With "Install updates automatically" a
    /// newer version found is installed at once.
    pub fn check_for_updates(self, asked: bool) {
        let Some(config) = config() else {
            if asked {
                self.update.set(State::Failed("this is a development run, which doesn't look for updates".into()));
            }
            return;
        };
        if matches!(self.update.get_untracked(), State::Checking | State::Downloading(_) | State::Ready(_)) {
            return;
        }
        self.update.set(State::Checking);
        let app = self;
        self.owner.run(|| {
            let _ = spawn(async move {
                let quiet = |why: String| {
                    eprintln!("update: {why}");
                    app.update.set(if asked { State::Failed(why) } else { State::Idle });
                };
                let manifest = match fetch(config.manifest_url()).await {
                    Ok(r) if r.ok() => r.body,
                    // No release yet (GitHub answers 404): nothing newer.
                    Ok(_) => return app.update.set(State::UpToDate),
                    Err(e) => return quiet(format!("the update server couldn't be reached ({e:?})")),
                };
                let Some(signature_url) = config.signature_url() else { return quiet("no key to check updates with".into()) };
                let signature = match fetch(signature_url).await {
                    Ok(r) if r.ok() => r.body,
                    _ => return quiet("the release isn't signed".into()),
                };
                match offer(&config, &manifest, &signature) {
                    Ok(None) => app.update.set(State::UpToDate),
                    Ok(Some(offer)) => {
                        eprintln!("update: version {} is available", offer.version);
                        app.update.set(State::Available(offer));
                        if app.settings.with_untracked(|s| s.auto_update) {
                            app.install_update();
                        }
                    }
                    Err(why) => quiet(why),
                }
            });
        });
    }

    /// Gets the version on offer, checks it and puts it in place of the
    /// program's file. It runs from the next start (`restart_to_update`).
    pub fn install_update(self) {
        let State::Available(offer) = self.update.get_untracked() else { return };
        self.update.set(State::Downloading(offer.clone()));
        let app = self;
        self.owner.run(|| {
            let _ = spawn(async move {
                let failed = |why: String| {
                    eprintln!("update: {why}");
                    app.update.set(State::Failed(why));
                };
                let bytes = match fetch(offer.url.clone()).await {
                    Ok(r) if r.ok() => r.body,
                    _ => return failed("the new version couldn't be downloaded".into()),
                };
                if let Err(why) = verified(&offer, &bytes) {
                    return failed(why);
                }
                let program = match std::env::current_exe() {
                    Ok(program) => program,
                    Err(e) => return failed(e.to_string()),
                };
                // (A restart starts the program by this name, not by the
                // one its file is about to be given.)
                nsh_core::restart::remember_program();
                match replace_program(&program, &bytes, crate::store::now_ms() as u64) {
                    Ok(()) => {
                        eprintln!("update: version {} is in place", offer.version);
                        app.update.set(State::Ready(offer.version));
                    }
                    Err(e) => failed(format!("the program's file couldn't be replaced ({e}). Is its folder read-only?")),
                }
            });
        });
    }

    /// Starts the app again, as the version just installed.
    pub fn restart_to_update(self) {
        self.save_now();
        nsh_core::element::desktop::restart();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{Engine as _, engine::general_purpose::STANDARD as B64};

    /// A key of the test's own, and a configuration that trusts it.
    fn test_config() -> (String, Config) {
        let secret = B64.encode([7u8; 32]);
        let public = B64.encode(ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]).verifying_key().to_bytes());
        let json = serde_json::json!({ "id": ID, "version": "1.0.0", "publicKey": public, "url": "https://example.invalid/latest.json" });
        (secret, Config::parse(json.to_string().as_bytes()).unwrap())
    }

    fn manifest(version: &str, sha256: Option<&str>) -> Vec<u8> {
        let mut asset = serde_json::json!({ "url": "BubbleNotes.exe", "size": 3 });
        if let Some(sha256) = sha256 {
            asset["sha256"] = sha256.into();
        }
        serde_json::json!({ "id": ID, "version": version, "notes": "Better.", "platforms": { nsh_updater::platform(): asset } }).to_string().into_bytes()
    }

    #[test]
    fn the_real_configuration_is_valid() {
        let github = config_for(None).expect("the GitHub configuration");
        assert_eq!(github.manifest_url(), format!("https://github.com/{REPO}/releases/latest/download/latest.json"));
        assert!(github.signature_url().is_some(), "unsigned releases must be refused");
        assert!(config_for(Some("http://127.0.0.1:1/latest.json")).is_some());
        // Not just anywhere.
        assert!(config_for(Some("http://example.com/latest.json")).is_none());
    }

    #[test]
    fn only_a_newer_signed_release_for_this_app_is_offered() {
        let (secret, config) = test_config();
        let sha = nsh_updater::sha256(b"new");
        let sign = |m: &[u8]| nsh_updater::sign(&secret, m).unwrap().into_bytes();
        // Newer and signed: offered, its file beside the manifest.
        let newer = manifest("1.0.1", Some(&sha));
        let found = offer(&config, &newer, &sign(&newer)).unwrap().unwrap();
        assert_eq!((found.version.as_str(), found.url.as_str(), found.notes.as_deref()), ("1.0.1", "https://example.invalid/BubbleNotes.exe", Some("Better.")));
        // The same version, or an older one: nothing.
        for version in ["1.0.0", "0.9.0"] {
            let m = manifest(version, Some(&sha));
            assert_eq!(offer(&config, &m, &sign(&m)).unwrap(), None, "{version}");
        }
        // Signed by another key, or changed after signing: refused.
        let other = nsh_updater::sign(&B64.encode([8u8; 32]), &newer).unwrap().into_bytes();
        assert!(offer(&config, &newer, &other).is_err());
        let changed = manifest("9.9.9", Some(&sha));
        assert!(offer(&config, &changed, &sign(&newer)).is_err());
        assert!(offer(&config, &newer, b"").is_err());
        // For another app, or with no checksum: refused.
        let foreign = String::from_utf8(newer.clone()).unwrap().replace(ID, "dev.other.app").into_bytes();
        assert!(offer(&config, &foreign, &sign(&foreign)).is_err());
        let unchecked = manifest("1.0.1", None);
        assert!(offer(&config, &unchecked, &sign(&unchecked)).is_err());
    }

    #[test]
    fn a_download_must_be_the_file_the_release_names() {
        let offer = Offer { version: "1.0.1".into(), notes: None, url: String::new(), sha256: nsh_updater::sha256(b"new"), size: Some(3) };
        assert!(verified(&offer, b"new").is_ok());
        assert!(verified(&offer, b"neW").is_err());
        assert!(verified(&offer, b"newer").is_err());
        assert!(verified(&offer, b"").is_err());
    }

    #[test]
    fn the_program_is_replaced_and_nothing_else_is_touched() {
        let dir = std::env::temp_dir().join(format!("bubblenotes-update-{}", crate::store::now_ms() as u64));
        std::fs::create_dir_all(dir.join("Notes")).unwrap();
        let program = dir.join("BubbleNotes.exe");
        std::fs::write(&program, b"old").unwrap();
        std::fs::write(dir.join("Notes").join("A.html"), b"a note").unwrap();
        std::fs::write(dir.join("BubbleNotes.exe.config"), b"other").unwrap();

        replace_program(&program, b"new", 5).unwrap();
        assert_eq!(std::fs::read(&program).unwrap(), b"new");
        assert_eq!(std::fs::read(dir.join("BubbleNotes.exe.old-5")).unwrap(), b"old");
        assert!(!dir.join("BubbleNotes.exe.new").exists());
        // Again before a restart: each old version aside under its own name.
        replace_program(&program, b"newer", 6).unwrap();
        assert_eq!(std::fs::read(dir.join("BubbleNotes.exe.old-6")).unwrap(), b"new");

        remove_leftovers(&program);
        assert_eq!(std::fs::read(&program).unwrap(), b"newer");
        assert!(!dir.join("BubbleNotes.exe.old-5").exists() && !dir.join("BubbleNotes.exe.old-6").exists());
        assert_eq!(std::fs::read(dir.join("Notes").join("A.html")).unwrap(), b"a note");
        assert_eq!(std::fs::read(dir.join("BubbleNotes.exe.config")).unwrap(), b"other");

        // A program that isn't there: nothing is left behind.
        assert!(replace_program(&dir.join("Missing.exe"), b"x", 7).is_err());
        assert!(!dir.join("Missing.exe.new").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn states_in_words() {
        assert_eq!(State::Idle.describe(), "");
        assert_eq!(State::Ready("1.0.1".into()).describe(), "Version 1.0.1 is installed: it starts the next time the app does.");
    }
}
