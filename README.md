# Tally

Tally is a safe, undoable file inbox for Windows. It keeps Downloads and Desktop clear without deleting or overwriting your files.

## Why Tally

- Preview every organization action before confirming it.
- Find exact SHA-256 duplicates and recover space safely.
- See large files and storage categories at a glance.
- Teach Tally an unfamiliar extension once; it remembers where it belongs.
- Undo organization, clone moves, and batch renames from History.
- Watch selected folders with a quiet-period debounce.

## Install

Download `Tally-Setup-1.0.0.exe` from the GitHub Release and run it. This installer includes the desktop app and CLI, and adds the CLI to the system PATH. Open a new terminal after installation to use `tally`. The Desktop UI includes the file browser, system tray control, command palette, tags, duplicate review, storage visualizer, archive extraction, and batch rename.

## Update Tally

Tally does not currently have an in-app automatic updater. To update an existing
installation, download the newest `Tally_<version>_x64-setup.exe` from
[GitHub Releases](https://github.com/Rayxworld/tally/releases), close Tally,
and run the installer. Install it to the existing location if prompted. Back
up important files and configuration before upgrading.

## Publish a new version

When a feature or fix is ready to share:

1. Update the version consistently in `Cargo.toml`,
   `desktop-ui/package.json`, `desktop-ui/src-tauri/tauri.conf.json`, and
   `installer.iss`. Update the matching `Cargo.lock` files as needed.
2. Run the tests and build the Windows installers:

   ```powershell
   cargo test --all-targets
   cargo build --release --bin tally
   cd desktop-ui
   npm install
   npm run build:windows
   cd ..
   & "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe" installer.iss
   ```

3. Commit and push the changes to `main`.
4. Create and push a matching version tag (for example, `v1.1.0`) on that
   commit. On GitHub, create a Release from that tag.
5. Attach `installer-output\Tally-Setup-<version>.exe` to the GitHub Release.
   This is the combined installer with the CLI on PATH. The Tauri NSIS/MSI
   installers in `desktop-ui\src-tauri\target\release\bundle\` install the
   desktop app only.
6. Users update by downloading and running the new setup installer; announce
   the release link to existing users.

Do not create a release from an old commit: the version tag must point to the
commit containing the release changes. Publishing a GitHub Release alone does
not make the app automatically update itself.

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
| `Esc` | Close menus and dialogs |
| `Ctrl + click` | Add/remove an item from selection |
| `Shift + click` | Select a range |

Right-click an unrecognized file and choose **Teach Tally this type** to create a saved routing rule.

## Build a Windows release

```powershell
cd desktop-ui
npm install
npm run build:windows
```

The Tauri installers below contain the desktop app only. To build the
recommended installer with the CLI and PATH setup, follow
[Publish a new version](#publish-a-new-version).

```text
desktop-ui\src-tauri\target\release\bundle\nsis\Tally_1.0.0_x64-setup.exe
desktop-ui\src-tauri\target\release\bundle\msi\Tally_1.0.0_x64_en-US.msi
```

## Command line

```powershell
cargo build --release --bin tally
.\target\release\tally.exe --sweep $HOME\Downloads --dry-run
.\target\release\tally.exe --stats $HOME\Downloads
.\target\release\tally.exe --watch $HOME\Downloads
.\target\release\tally.exe --undo
```

## Safety

- Only top-level files in the chosen folder are organized.
- Tally skips hidden files, shortcuts, partial downloads, and very recent files.
- Name collisions receive a numbered name; Tally never overwrites an existing file.
- Identical files are set aside in `_Duplicates`, never deleted.
- Undo never overwrites a file that is already present.
