# Tally by Rayxworld

Tally by Rayxworld is a safe, undoable file inbox for Windows. The app is called Tally; the command-line executable and command remain `tally`.

## Why Tally

- Preview every organization action before confirming it.
- Find exact SHA-256 duplicates and recover space safely.
- See large files and storage categories at a glance.
- Teach Tally an unfamiliar extension once; it remembers where it belongs.
- Undo organization, clone moves, and batch renames from History.
- Watch selected folders with a quiet-period debounce.

## Install

Download the **Tally by Rayxworld** x64 NSIS installer from the
[GitHub Releases](https://github.com/Rayxworld/tally/releases) page. This
updater-compatible installer includes the desktop app and CLI, and adds the
CLI to your user PATH. Open a new terminal after installation to use `tally`.

## Update Tally

Tally checks GitHub Releases for signed updates. In Settings, choose **Check for
updates**; if a newer version is available, Tally verifies the signature,
downloads and installs the update, then restarts. The CLI is included in the
same signed installer and is updated with the desktop app.

The original v1.0.0 Inno Setup installer is not compatible with in-app updates.
Users of that installer must uninstall it first, then install the new
Tauri/NSIS installer once from GitHub Releases. This also removes the old
system-wide CLI PATH entry so the new, auto-updated CLI is selected.
Subsequent updates can be installed from within Tally.

## Organize personal folders

The desktop **Scan & Organize** action scans Desktop, Downloads, Documents,
Pictures, Videos, and Music, including their subfolders. It does not scan
entire drives, follow symbolic links, or enter detected software projects and
protected system/application folders. Tally previews each proposed move with
its source and destination, then requires separate approval for every
destination group. Files with unknown types stay untouched until you add a
routing rule. Skipped items and scan errors show their reasons in the preview.
Each approved destination group can be undone from History.

The CLI is intentionally narrower: `tally --sweep <DIR>` only organizes
supported files directly inside the folder you name.

## Publish a new version

When a feature or fix is ready to share:

1. Update the version in `Cargo.toml`, `desktop-ui/package.json`,
   `desktop-ui/src-tauri/Cargo.toml`, and
   `desktop-ui/src-tauri/tauri.conf.json`. Update `installer.iss` only when
   building the legacy installer.
2. Keep the updater signing key on the release computer. It is stored at
   `%USERPROFILE%\.tauri\tally-updater.key`; do not commit it or upload it as a
   GitHub Actions secret. Keep a secure backup. Losing it prevents future
   updates from being signed. The public key in `tauri.conf.json` must remain
   paired with this private key.
3. Run tests and build the updater-compatible Windows installer locally:

   ```powershell
   cargo test --all-targets
   Push-Location desktop-ui
   npm ci
   npm run lint
   npm run build
   Pop-Location
   cargo build --release --bin tally
   Push-Location desktop-ui
   $env:TAURI_SIGNING_PRIVATE_KEY = (Get-Content "$HOME\.tauri\tally-updater.key" -Raw).Trim()
   npm run build:windows -- --bundles nsis
   Remove-Item Env:\TAURI_SIGNING_PRIVATE_KEY
   Pop-Location
   ```

4. Generate the updater manifest from the signed installer and its signature:

   ```powershell
   node scripts\create-updater-manifest.mjs v1.1.0 desktop-ui\src-tauri\target\release\bundle\nsis latest.json
   ```

5. Commit and push the versioned source, then create and publish the matching
   GitHub Release with `RELEASE_NOTES.md`, the `*_x64-setup.exe` installer,
   its `.sig` signature, and `latest.json`. For example:

   ```powershell
   git tag v1.1.0
   git push origin main
   git push origin v1.1.0
   gh release create v1.1.0 `
     "desktop-ui\src-tauri\target\release\bundle\nsis\Tally by Rayxworld_1.1.0_x64-setup.exe" `
     "desktop-ui\src-tauri\target\release\bundle\nsis\Tally by Rayxworld_1.1.0_x64-setup.exe.sig" `
     latest.json `
     --title "Tally by Rayxworld v1.1.0" `
     --notes-file RELEASE_NOTES.md
   ```

   GitHub Actions validates tagged releases; it does not receive the signing
   key or build/publish the signed installer. The `latest.json` release asset
   and matching signed installer/signature are required for in-app updates.

Do not publish an unsigned installer or change the updater public key. The
version tag must point to the commit containing the release changes.

## Website

The static landing page is in [`website/`](./website/). It links visitors to
the latest GitHub Release. To preview it locally:

```powershell
cd website
python -m http.server 8000
```

Open `http://localhost:8000`. The GitHub Actions workflow deploys changes in
`website/` to GitHub Pages when they are pushed to `main`. Enable Pages for the
repository in **Settings → Pages**, with **GitHub Actions** as the build and
deployment source. GitHub displays the published site URL in the Pages settings
and in the completed deployment workflow.

## Desktop shortcuts

| Shortcut | Action |
| --- | --- |
| `Ctrl + K` | Open Command Palette |
| `Ctrl + /` | Open Help & Shortcuts |
| `Ctrl + T` | Open a new tab |
| `Alt + Left` / `Alt + Right` | Navigate back / forward |
| `Alt + Up` | Open the parent folder |
| `F5` | Refresh the current folder |
| `Esc` | Close menus and dialogs |
| `Ctrl + click` | Add/remove an item from selection |
| `Shift + click` | Select a range |

Right-click an unrecognized file and choose **Teach Tally this type** to create a saved routing rule.

## Build a Windows release locally

```powershell
cargo build --release --bin tally
Push-Location desktop-ui
$env:TAURI_SIGNING_PRIVATE_KEY = (Get-Content "$HOME\.tauri\tally-updater.key" -Raw).Trim()
npm run build:windows -- --bundles nsis
Remove-Item Env:\TAURI_SIGNING_PRIVATE_KEY
Pop-Location
```

The signed NSIS installer contains both the desktop app and CLI. The legacy
Inno Setup installer is not compatible with in-app updates.

## Command line

```powershell
cargo build --release --bin tally
.\target\release\tally.exe --sweep $HOME\Downloads --dry-run
.\target\release\tally.exe --stats $HOME\Downloads
.\target\release\tally.exe --watch $HOME\Downloads
.\target\release\tally.exe --undo
```

The installed command-line program is `tally.exe`; the desktop app has its own
executable name. They are distinct because Windows treats file names as
case-insensitive.

## Safety

- The desktop scan is limited to standard personal folders; the CLI only
  processes top-level files in the chosen folder.
- Tally skips hidden files, shortcuts, partial downloads, and very recent files.
- Name collisions receive a numbered name; Tally never overwrites an existing file.
- Identical files are set aside in `_Duplicates`, never deleted.
- Undo never overwrites a file that is already present.
