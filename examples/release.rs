//! The release tool: what a release of Bubble Notes on GitHub is made of.
//!
//! `cargo run --example release -- keygen`
//!     makes the key releases are signed with (once), in
//!     `~/.bubblenotes/release.secret`, and prints its public half, which
//!     `src/update.rs` holds (`PUBLIC_KEY`). Keep the secret file safe and
//!     backed up, and out of the repository: without it no update can be
//!     published that the apps out there accept.
//!
//! `cargo run --example release -- files <program file> <folder> [notes]`
//!     writes into the folder what a GitHub release needs: the program
//!     under its release name, `latest.json` (the version, where the
//!     program is, its SHA-256 and size) and `latest.json.sig` (its
//!     signature). The version is this package's (`Cargo.toml`), the one
//!     compiled into the program: build the program in the same step
//!     (`release.ps1` does), or the app would take its own release for a
//!     newer one.

use std::path::{Path, PathBuf};

use base64::{Engine as _, engine::general_purpose::STANDARD as B64};

const ID: &str = "dev.bubblenotes.app";
const VERSION: &str = env!("CARGO_PKG_VERSION");

fn secret_path() -> PathBuf {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_default();
    home.join(".bubblenotes").join("release.secret")
}

fn public_of(secret: &str) -> Result<String, String> {
    let seed: [u8; 32] = B64.decode(secret.trim()).ok().and_then(|b| b.try_into().ok()).ok_or("the secret key isn't 32 bytes of base64")?;
    Ok(B64.encode(ed25519_dalek::SigningKey::from_bytes(&seed).verifying_key().to_bytes()))
}

fn keygen() -> Result<(), String> {
    let path = secret_path();
    if !path.exists() {
        let mut seed = [0u8; 32];
        getrandom::getrandom(&mut seed).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(path.parent().unwrap_or(Path::new("."))).map_err(|e| e.to_string())?;
        std::fs::write(&path, B64.encode(seed)).map_err(|e| e.to_string())?;
        println!("made {}", path.display());
    } else {
        println!("kept {}", path.display());
    }
    let secret = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    println!("public key: {}", public_of(&secret)?);
    Ok(())
}

fn files(program: &Path, folder: &Path, notes: Option<&str>) -> Result<(), String> {
    let secret = std::fs::read_to_string(secret_path()).map_err(|e| format!("{}: {e} (run `keygen` first)", secret_path().display()))?;
    let bytes = std::fs::read(program).map_err(|e| format!("{}: {e}", program.display()))?;
    let name = format!("BubbleNotes-{VERSION}-{}.exe", nsh_updater::platform());
    std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    std::fs::write(folder.join(&name), &bytes).map_err(|e| e.to_string())?;
    let mut manifest = serde_json::json!({
        "id": ID,
        "version": VERSION,
        "platforms": { nsh_updater::platform(): { "url": name, "sha256": nsh_updater::sha256(&bytes), "size": bytes.len() } },
    });
    if let Some(notes) = notes.filter(|n| !n.is_empty()) {
        manifest["notes"] = notes.into();
    }
    let manifest = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
    let signature = nsh_updater::sign(&secret, &manifest).map_err(|e| e.to_string())?;
    // What was just signed is what the apps will check.
    nsh_updater::verify(&public_of(&secret)?, &manifest, signature.as_bytes()).map_err(|e| e.to_string())?;
    std::fs::write(folder.join(nsh_updater::MANIFEST), &manifest).map_err(|e| e.to_string())?;
    std::fs::write(folder.join(nsh_updater::SIGNATURE), signature).map_err(|e| e.to_string())?;
    println!("version {VERSION}: {name}, {} and {} in {}", nsh_updater::MANIFEST, nsh_updater::SIGNATURE, folder.display());
    Ok(())
}

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let done = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["keygen"] => keygen(),
        ["files", program, folder] => files(Path::new(program), Path::new(folder), None),
        ["files", program, folder, notes] => files(Path::new(program), Path::new(folder), Some(notes)),
        _ => Err("usage: release keygen | release files <program file> <folder> [notes]".into()),
    };
    match done {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("release: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
