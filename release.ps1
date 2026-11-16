# Makes the files of a release of Bubble Notes, in dist\.
#
#   .\release.ps1 "What changed in this version."
#
# 1. Set the new version in Cargo.toml (and nsh.json) first, and commit.
# 2. Run this: it builds the program at that version (in target\publish, so
#    a copy of the app that is running from target\release is not in the
#    way), then writes dist\BubbleNotes-<version>-windows-x86_64.exe,
#    dist\latest.json and dist\latest.json.sig (signed with the key in
#    ~\.bubblenotes\release.secret; `cargo run --example release -- keygen`
#    makes it the first time).
# 3. Publish the three files as a GitHub release (the command is printed).
#    Apps out there then find the version, check it and install it.
param([string]$Notes = "")
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot
$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
Write-Host "Building Bubble Notes $version..."
cargo build --release --target-dir target\publish
if ($LASTEXITCODE -ne 0) { exit 1 }
if (Test-Path dist) { Remove-Item dist -Recurse -Force }
cargo run --release --target-dir target\publish --example release -- files target\publish\release\nsh-app-bubblenotes.exe dist $Notes
if ($LASTEXITCODE -ne 0) { exit 1 }
Write-Host ""
Write-Host "To publish it:"
Write-Host "  gh release create v$version (Get-ChildItem dist).FullName --repo MichealKennedy/bubblenotes-rust --title `"Bubble Notes $version`" --notes `"$Notes`""
