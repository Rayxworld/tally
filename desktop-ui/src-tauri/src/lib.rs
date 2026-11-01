use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};
use tally::config::{Config, KnownDirs, Rule};
use tally::ledger::Transaction;
use tally::sweep::{plan_sweep, sweep, Summary, SweepOptions, SweepPlan};
use tally::undo::{undo, UndoSummary};
use tauri::Manager;

#[derive(Serialize)]
struct FileEntry {
    name: String,
    path: String,
    is_dir: bool,
    extension: Option<String>,
    size: u64,
    modified: Option<u64>,
}

#[derive(Serialize)]
struct DirectoryResult {
    path: String,
    parent: Option<String>,
    entries: Vec<FileEntry>,
}

#[derive(Serialize)]
struct SpecialLocation {
    name: String,
    path: String,
    is_drive: bool,
}

#[derive(Serialize)]
struct FilePreviewDto {
    kind: String, // "image", "text", "binary"
    content: String,
    line_count: Option<usize>,
}

fn to_base64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = if chunk.len() > 1 { chunk[1] } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] } else { 0 };

        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 3) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(((b1 & 15) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(b2 & 63) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn clean_path_string(path: &Path) -> String {
    let s = path.to_string_lossy().to_string();
    if let Some(stripped) = s.strip_prefix(r"\\?\") {
        stripped.to_string()
    } else {
        s
    }
}

fn entry_from_path(path: &Path) -> Result<FileEntry, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|e| format!("Cannot read {}: {e}", path.display()))?;

    let is_dir = metadata.is_dir();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());

    let modified = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs());

    Ok(FileEntry {
        name,
        path: clean_path_string(path),
        is_dir,
        extension: if is_dir {
            None
        } else {
            path.extension().map(|e| e.to_string_lossy().into_owned())
        },
        size: if is_dir { 0 } else { metadata.len() },
        modified,
    })
}

#[tauri::command]
fn list_directory(path: String) -> Result<DirectoryResult, String> {
    let requested = PathBuf::from(&path);
    let canonical = requested
        .canonicalize()
        .map_err(|e| format!("Cannot open '{}': {e}", requested.display()))?;

    if !canonical.is_dir() {
        return Err(format!("'{}' is not a directory.", canonical.display()));
    }

    let mut entries = Vec::new();

    for item in fs::read_dir(&canonical)
        .map_err(|e| format!("Cannot list '{}': {e}", canonical.display()))?
    {
        let item = match item {
            Ok(item) => item,
            Err(_) => continue,
        };

        let item_path = item.path();

        if let Ok(entry) = entry_from_path(&item_path) {
            entries.push(entry);
        }
    }

    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    let parent = canonical
        .parent()
        .map(|p| clean_path_string(p));

    Ok(DirectoryResult {
        path: clean_path_string(&canonical),
        parent,
        entries,
    })
}

#[tauri::command]
fn get_special_locations() -> Vec<SpecialLocation> {
    let mut locations = Vec::new();

    if let Some(home) = dirs::home_dir() {
        locations.push(SpecialLocation {
            name: "Home".into(),
            path: clean_path_string(&home),
            is_drive: false,
        });

        // Use Windows' registered known folders first. This keeps Quick Access correct
        // when OneDrive or a corporate profile redirects Pictures/Documents.
        for (name, known, fallback) in [
            ("Desktop", dirs::desktop_dir(), "Desktop"),
            ("Downloads", dirs::download_dir(), "Downloads"),
            ("Documents", dirs::document_dir(), "Documents"),
            ("Pictures", dirs::picture_dir(), "Pictures"),
            ("Music", dirs::audio_dir(), "Music"),
            ("Videos", dirs::video_dir(), "Videos"),
        ] {
            let path = known.unwrap_or_else(|| home.join(fallback));
            // Keep the familiar location visible even if Windows has not created it yet.
            if !locations.iter().any(|location: &SpecialLocation| location.name == name) {
                locations.push(SpecialLocation {
                    name: name.into(),
                    path: clean_path_string(&path),
                    is_drive: false,
                });
            }
        }
    }

    if let Ok(user_profile) = env::var("USERPROFILE") {
        let path = PathBuf::from(&user_profile);
        if path.is_dir() && !locations.iter().any(|l| l.path.eq_ignore_ascii_case(&user_profile)) {
            locations.push(SpecialLocation {
                name: "User folder".into(),
                path: clean_path_string(&path),
                is_drive: false,
            });
        }
    }

    // Detect Windows drives
    #[cfg(target_os = "windows")]
    {
        for letter in b'A'..=b'Z' {
            let root_str = format!("{}:\\", letter as char);
            let p = Path::new(&root_str);
            if p.exists() {
                locations.push(SpecialLocation {
                    name: format!("Local Disk ({}:)", letter as char),
                    path: root_str,
                    is_drive: true,
                });
            }
        }
    }

    locations
}

#[tauri::command]
fn open_path(path: String) -> Result<(), String> {
    let p = PathBuf::from(&path);
    if !p.exists() {
        return Err(format!("File or folder does not exist: {path}"));
    }

    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/C", "start", "", &path])
            .spawn()
            .map_err(|e| format!("Could not open file: {e}"))?;
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("Could not open file: {e}"))?;
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("Could not open file: {e}"))?;
    }

    Ok(())
}

#[tauri::command]
fn reveal_in_file_manager(path: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        Command::new("explorer")
            .arg(format!("/select,\"{}\"", path))
            .spawn()
            .map_err(|e| format!("Could not open file manager: {e}"))?;
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .args(["-R", &path])
            .spawn()
            .map_err(|e| format!("Could not open file manager: {e}"))?;
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        let parent = Path::new(&path).parent().unwrap_or_else(|| Path::new(&path));
        Command::new("xdg-open")
            .arg(parent)
            .spawn()
            .map_err(|e| format!("Could not open file manager: {e}"))?;
    }

    Ok(())
}

#[tauri::command]
fn create_directory(parent_path: String, name: String) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("Directory name cannot be empty.".into());
    }
    let target = PathBuf::from(&parent_path).join(trimmed);
    if target.exists() {
        return Err("A file or directory with that name already exists.".into());
    }
    fs::create_dir_all(&target).map_err(|e| format!("Failed to create directory: {e}"))?;
    Ok(clean_path_string(&target))
}

#[tauri::command]
fn delete_path(path: String) -> Result<(), String> {
    let p = PathBuf::from(&path);
    if !p.exists() {
        return Err("Target path does not exist.".into());
    }
    if p.is_dir() {
        fs::remove_dir_all(&p).map_err(|e| format!("Failed to delete folder: {e}"))
    } else {
        fs::remove_file(&p).map_err(|e| format!("Failed to delete file: {e}"))
    }
}

#[tauri::command]
fn rename_path(old_path: String, new_name: String) -> Result<String, String> {
    let source = PathBuf::from(&old_path);
    let parent = source
        .parent()
        .ok_or_else(|| "Cannot rename root directory.".to_string())?;
    let target = parent.join(new_name.trim());
    if target.exists() {
        return Err("A file with that name already exists.".into());
    }
    fs::rename(&source, &target).map_err(|e| format!("Rename failed: {e}"))?;
    Ok(clean_path_string(&target))
}

#[tauri::command]
fn get_file_preview(path: String) -> Result<FilePreviewDto, String> {
    let p = PathBuf::from(&path);
    if !p.exists() || p.is_dir() {
        return Err("Not a file".into());
    }

    let ext = p
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    // Image preview
    if ["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "ico"].contains(&ext.as_str()) {
        let metadata = fs::metadata(&p).map_err(|e| e.to_string())?;
        if metadata.len() <= 10 * 1024 * 1024 {
            let bytes = fs::read(&p).map_err(|e| e.to_string())?;
            let mime = match ext.as_str() {
                "png" => "image/png",
                "jpg" | "jpeg" => "image/jpeg",
                "gif" => "image/gif",
                "webp" => "image/webp",
                "bmp" => "image/bmp",
                "svg" => "image/svg+xml",
                "ico" => "image/x-icon",
                _ => "application/octet-stream",
            };
            return Ok(FilePreviewDto {
                kind: "image".into(),
                content: format!("data:{mime};base64,{}", to_base64(&bytes)),
                line_count: None,
            });
        }
    }

    // Text & code preview
    use std::io::Read;
    let mut file = match fs::File::open(&p) {
        Ok(f) => f,
        Err(_) => {
            return Ok(FilePreviewDto {
                kind: "binary".into(),
                content: String::new(),
                line_count: None,
            });
        }
    };
    let mut buf = vec![0u8; 32 * 1024];
    let bytes_read = file.read(&mut buf).unwrap_or(0);
    buf.truncate(bytes_read);

    if let Ok(text) = String::from_utf8(buf) {
        let lines: Vec<&str> = text.lines().take(35).collect();
        let total_lines = text.lines().count();
        return Ok(FilePreviewDto {
            kind: "text".into(),
            content: lines.join("\n"),
            line_count: Some(total_lines),
        });
    }

    Ok(FilePreviewDto {
        kind: "binary".into(),
        content: String::new(),
        line_count: None,
    })
}

// Watched Folders Management
#[tauri::command]
fn get_watched_folders() -> Vec<String> {
    let path = tally::tally_home().join("watched_folders.json");
    if let Ok(data) = fs::read_to_string(&path) {
        if let Ok(folders) = serde_json::from_str::<Vec<String>>(&data) {
            return folders;
        }
    }
    Vec::new()
}

/// Lightweight Windows watch service: polls only configured top-level folders,
/// waits for five quiet seconds, then uses the same safe sweep engine as the UI.
fn start_watch_service() {
    std::thread::spawn(|| {
        let mut signatures: std::collections::HashMap<PathBuf, (u64, u128)> = std::collections::HashMap::new();
        let mut changed: std::collections::HashMap<PathBuf, std::time::Instant> = std::collections::HashMap::new();
        loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            let watched: Vec<PathBuf> = get_watched_folders().into_iter().map(PathBuf::from).filter(|p| p.is_dir()).collect();
            signatures.retain(|path, _| watched.contains(path));
            changed.retain(|path, _| watched.contains(path));
            for folder in watched {
                let signature = match watch_signature(&folder) { Ok(value) => value, Err(_) => continue };
                match signatures.insert(folder.clone(), signature) {
                    Some(previous) if previous != signature => { changed.insert(folder.clone(), std::time::Instant::now()); }
                    None => continue, // establish baseline; never sweep merely because Tally started
                    _ => {}
                }
                if changed.get(&folder).is_some_and(|time| time.elapsed() >= std::time::Duration::from_secs(5)) {
                    if let Ok(config) = Config::load(&tally::config_path()) {
                        let options = SweepOptions { dry_run: false, verbose: false, now: SystemTime::now() };
                        let _ = sweep(&config, &KnownDirs::detect(), &folder, &options, &tally::open_ledger());
                    }
                    signatures.insert(folder.clone(), watch_signature(&folder).unwrap_or(signature));
                    changed.remove(&folder);
                }
            }
        }
    });
}

fn watch_signature(folder: &Path) -> Result<(u64, u128), String> {
    let mut count = 0u64; let mut total = 0u128;
    for entry in fs::read_dir(folder).map_err(|e| e.to_string())? {
        let metadata = entry.map_err(|e| e.to_string())?.metadata().map_err(|e| e.to_string())?;
        if metadata.is_file() { count += 1; total = total.wrapping_add(metadata.len() as u128); total = total.wrapping_add(metadata.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0)); }
    }
    Ok((count, total))
}

#[tauri::command]
fn toggle_watch_folder(path: String, enable: bool) -> Result<Vec<String>, String> {
    let mut folders = get_watched_folders();
    let norm = clean_path_string(Path::new(&path));
    if enable {
        if !folders.iter().any(|f| f.eq_ignore_ascii_case(&norm)) {
            folders.push(norm);
        }
    } else {
        folders.retain(|f| !f.eq_ignore_ascii_case(&norm));
    }
    let home = tally::tally_home();
    let _ = fs::create_dir_all(&home);
    let json = serde_json::to_string_pretty(&folders).map_err(|e| e.to_string())?;
    fs::write(home.join("watched_folders.json"), json).map_err(|e| e.to_string())?;
    Ok(folders)
}

#[tauri::command]
fn toggle_windows_context_menu(enable: bool) -> Result<bool, String> {
    #[cfg(target_os = "windows")]
    {
        let exe = env::current_exe().map_err(|e| e.to_string())?;
        let exe_str = exe.to_string_lossy().to_string();

        if enable {
            Command::new("reg")
                .args([
                    "add",
                    r"HKCU\Software\Classes\Directory\shell\Tally",
                    "/ve",
                    "/d",
                    "Organize with Tally",
                    "/f",
                ])
                .output()
                .map_err(|e| e.to_string())?;

            let _ = Command::new("reg")
                .args([
                    "add",
                    r"HKCU\Software\Classes\Directory\shell\Tally",
                    "/v",
                    "Icon",
                    "/d",
                    &exe_str,
                    "/f",
                ])
                .output();

            let cmd = format!("\"{}\" \"%1\"", exe_str);
            Command::new("reg")
                .args([
                    "add",
                    r"HKCU\Software\Classes\Directory\shell\Tally\command",
                    "/ve",
                    "/d",
                    &cmd,
                    "/f",
                ])
                .output()
                .map_err(|e| e.to_string())?;

            Ok(true)
        } else {
            let _ = Command::new("reg")
                .args([
                    "delete",
                    r"HKCU\Software\Classes\Directory\shell\Tally",
                    "/f",
                ])
                .output();
            Ok(false)
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(false)
    }
}

// -------------------------------------------------------------
// Tally Core Engine Integration Commands
// -------------------------------------------------------------

#[tauri::command]
fn get_tally_plan(path: String) -> Result<SweepPlan, String> {
    let target = PathBuf::from(path.trim());
    if !target.is_dir() {
        return Err(format!("'{}' is not a directory.", target.display()));
    }
    let config = Config::load(&tally::config_path())?;
    let known = KnownDirs::detect();
    plan_sweep(&config, &known, &target, SystemTime::now())
}

#[tauri::command]
fn execute_tally_sweep(path: String) -> Result<Summary, String> {
    let target = PathBuf::from(path.trim());
    if !target.is_dir() {
        return Err(format!("'{}' is not a directory.", target.display()));
    }
    let config = Config::load(&tally::config_path())?;
    let known = KnownDirs::detect();
    let ledger = tally::open_ledger();
    let opts = SweepOptions {
        dry_run: false,
        verbose: true,
        now: SystemTime::now(),
    };
    sweep(&config, &known, &target, &opts, &ledger)
}

#[tauri::command]
fn undo_tally_sweep(count: usize) -> Result<UndoSummary, String> {
    let ledger = tally::open_ledger();
    undo(&ledger, count, false)
}

#[tauri::command]
fn get_tally_history() -> Result<Vec<Transaction>, String> {
    let ledger = tally::open_ledger();
    ledger.load()
}

#[tauri::command]
fn get_tally_config() -> Result<Config, String> {
    Config::load(&tally::config_path())
}

#[tauri::command]
fn save_tally_config(mut config: Config) -> Result<(), String> {
    config.validate()?;
    let path = tally::config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    fs::write(&path, json + "\n").map_err(|e| e.to_string())
}

/// Teach Tally where an otherwise unknown extension belongs. The choice becomes
/// a normal, editable config rule and takes effect on the very next sweep.
#[tauri::command]
fn assign_extension(extension: String, destination: String, label: String) -> Result<Config, String> {
    let extension = extension.trim().to_ascii_lowercase();
    if !extension.starts_with('.') || extension.len() < 2 || !extension[1..].chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("Choose a file with a normal extension, such as .blend or .epub.".into());
    }
    if destination.trim().is_empty() { return Err("Choose where Tally should put this file type.".into()); }
    let mut config = Config::load(&tally::config_path())?;
    for rule in &mut config.rules { rule.extensions.retain(|value| value != &extension); }
    config.rules.retain(|rule| !rule.extensions.is_empty());
    if let Some(rule) = config.rules.iter_mut().find(|rule| rule.destination.eq_ignore_ascii_case(destination.trim())) {
        rule.extensions.push(extension);
        rule.extensions.sort(); rule.extensions.dedup();
    } else {
        config.rules.push(Rule { name: if label.trim().is_empty() { "My file types".into() } else { label.trim().into() }, destination: destination.trim().into(), extensions: vec![extension] });
    }
    save_tally_config(config.clone())?;
    Ok(config)
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct DuplicateFile {
    path: String,
    name: String,
    size: u64,
    modified: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct DuplicateGroup {
    hash: String,
    size: u64,
    wasted_bytes: u64,
    files: Vec<DuplicateFile>,
}

#[tauri::command]
fn scan_duplicates(path: String) -> Result<Vec<DuplicateGroup>, String> {
    let target = PathBuf::from(&path);
    if !target.is_dir() {
        return Err(format!("{} is not a directory", target.display()));
    }

    let mut size_map: std::collections::HashMap<u64, Vec<PathBuf>> = std::collections::HashMap::new();
    let entries = fs::read_dir(&target).map_err(|e| format!("cannot read directory: {e}"))?;

    for entry in entries.flatten() {
        let p = entry.path();
        if let Ok(meta) = fs::symlink_metadata(&p) {
            if meta.is_file() {
                let sz = meta.len();
                if sz > 0 {
                    size_map.entry(sz).or_default().push(p);
                }
            }
        }
    }

    let mut groups: Vec<DuplicateGroup> = Vec::new();

    for (size, paths) in size_map {
        if paths.len() < 2 {
            continue;
        }

        let mut prefix_map: std::collections::HashMap<Vec<u8>, Vec<PathBuf>> = std::collections::HashMap::new();
        for p in paths {
            let mut buf = vec![0u8; 4096];
            let n = match fs::File::open(&p).and_then(|mut f| std::io::Read::read(&mut f, &mut buf)) {
                Ok(n) => n,
                Err(_) => continue,
            };
            buf.truncate(n);
            prefix_map.entry(buf).or_default().push(p);
        }

        for (_, sub_paths) in prefix_map {
            if sub_paths.len() < 2 {
                continue;
            }

            let mut hash_map: std::collections::HashMap<String, Vec<PathBuf>> = std::collections::HashMap::new();
            for p in sub_paths {
                if let Ok(h) = tally::util::sha256_file(&p) {
                    hash_map.entry(h).or_default().push(p);
                }
            }

            for (hash, matched_paths) in hash_map {
                if matched_paths.len() >= 2 {
                    let mut file_entries = Vec::new();
                    for mp in &matched_paths {
                        let name = mp
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        let modified = fs::metadata(mp)
                            .ok()
                            .and_then(|m| m.modified().ok())
                            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                            .map(|d| d.as_secs());
                        file_entries.push(DuplicateFile {
                            path: clean_path_string(mp),
                            name,
                            size,
                            modified,
                        });
                    }
                    let wasted = size * (matched_paths.len() as u64 - 1);
                    groups.push(DuplicateGroup {
                        hash,
                        size,
                        wasted_bytes: wasted,
                        files: file_entries,
                    });
                }
            }
        }
    }

    groups.sort_by(|a, b| b.wasted_bytes.cmp(&a.wasted_bytes));
    Ok(groups)
}

#[tauri::command]
fn isolate_duplicates(folder_path: String, duplicate_paths: Vec<String>) -> Result<usize, String> {
    if duplicate_paths.is_empty() {
        return Ok(0);
    }
    let parent_dir = PathBuf::from(&folder_path);
    let target_dir = parent_dir.join("_Duplicates");
    let mut created_dirs = Vec::new();
    if !target_dir.exists() {
        fs::create_dir_all(&target_dir).map_err(|e| format!("cannot create duplicate directory: {e}"))?;
        created_dirs.push(target_dir.clone());
    }

    let mut ops = Vec::new();
    let mut count = 0;
    for src_str in duplicate_paths {
        let src = PathBuf::from(&src_str);
        if !src.exists() || !src.is_file() {
            continue;
        }
        let file_name = match src.file_name() {
            Some(n) => n.to_string_lossy().to_string(),
            None => continue,
        };

        let mut dest = target_dir.join(&file_name);
        let mut n = 2;
        while dest.exists() {
            let numbered = tally::util::numbered_name(&file_name, n);
            dest = target_dir.join(numbered);
            n += 1;
        }

        if let Err(e) = tally::util::move_file(&src, &dest) {
            return Err(format!("failed to isolate {}: {e}", src.display()));
        }

        ops.push(tally::ledger::Operation {
            source_path: src,
            destination_path: dest,
            action: "moved_duplicate".to_string(),
        });
        count += 1;
    }

    if !ops.is_empty() {
        let tx = tally::ledger::Transaction {
            transaction_id: tally::util::transaction_id(SystemTime::now()),
            timestamp: tally::util::iso_timestamp(SystemTime::now()),
            target: parent_dir,
            operations: ops,
            created_dirs,
        };
        let ledger = tally::open_ledger();
        let _ = ledger.append(&tx);
    }

    Ok(count)
}

#[derive(Deserialize)]
struct BatchRenameItem {
    from: String,
    to_name: String,
}

#[tauri::command]
fn batch_rename(items: Vec<BatchRenameItem>) -> Result<usize, String> {
    if items.is_empty() {
        return Ok(0);
    }

    let mut ops = Vec::new();
    let mut count = 0;
    let mut common_parent = PathBuf::new();

    for item in items {
        let src = PathBuf::from(&item.from);
        if !src.exists() {
            continue;
        }
        let parent = match src.parent() {
            Some(p) => p.to_path_buf(),
            None => continue,
        };
        if common_parent.as_os_str().is_empty() {
            common_parent = parent.clone();
        }

        let trimmed_new = item.to_name.trim();
        if trimmed_new.is_empty() {
            continue;
        }
        let dest = parent.join(trimmed_new);

        if src == dest {
            continue;
        }
        if dest.exists() {
            return Err(format!("Destination already exists: {}", dest.display()));
        }

        if let Err(e) = tally::util::move_file(&src, &dest) {
            return Err(format!("Failed to rename {}: {e}", src.display()));
        }

        ops.push(tally::ledger::Operation {
            source_path: src,
            destination_path: dest,
            action: "renamed".to_string(),
        });
        count += 1;
    }

    if !ops.is_empty() {
        let tx = tally::ledger::Transaction {
            transaction_id: tally::util::transaction_id(SystemTime::now()),
            timestamp: tally::util::iso_timestamp(SystemTime::now()),
            target: common_parent,
            operations: ops,
            created_dirs: Vec::new(),
        };
        let ledger = tally::open_ledger();
        let _ = ledger.append(&tx);
    }

    Ok(count)
}

fn tags_file_path() -> PathBuf {
    tally::tally_home().join("tags.json")
}

#[tauri::command]
fn get_file_tags() -> Result<std::collections::HashMap<String, String>, String> {
    let p = tags_file_path();
    if !p.exists() {
        return Ok(std::collections::HashMap::new());
    }
    let data = fs::read_to_string(&p).map_err(|e| e.to_string())?;
    let map: std::collections::HashMap<String, String> = serde_json::from_str(&data).unwrap_or_default();
    Ok(map)
}

#[tauri::command]
fn set_file_tag(path: String, tag: Option<String>) -> Result<(), String> {
    let p = tags_file_path();
    if let Some(parent) = p.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut map = get_file_tags().unwrap_or_default();
    let cleaned = clean_path_string(Path::new(&path));
    match tag {
        Some(t) if !t.trim().is_empty() => {
            map.insert(cleaned, t.trim().to_string());
        }
        _ => {
            map.remove(&cleaned);
        }
    }
    let json = serde_json::to_string_pretty(&map).map_err(|e| e.to_string())?;
    fs::write(&p, json).map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Serialize)]
struct StorageCategory {
    label: String,
    count: usize,
    total_bytes: u64,
    color: String,
}

#[derive(Serialize)]
struct StorageBreakdown {
    total_bytes: u64,
    file_count: usize,
    dir_count: usize,
    top_files: Vec<FileEntry>,
    categories: Vec<StorageCategory>,
}

#[tauri::command]
fn get_folder_storage(path: String) -> Result<StorageBreakdown, String> {
    let target = PathBuf::from(&path);
    if !target.is_dir() {
        return Err(format!("{} is not a directory", target.display()));
    }

    let mut total_bytes = 0u64;
    let mut file_count = 0usize;
    let mut dir_count = 0usize;
    let mut files = Vec::new();

    let mut cat_map: std::collections::HashMap<&'static str, (usize, u64, &'static str)> = std::collections::HashMap::new();
    cat_map.insert("Images", (0, 0, "#0ea5e9"));
    cat_map.insert("Videos", (0, 0, "#8b5cf6"));
    cat_map.insert("Archives", (0, 0, "#f59e0b"));
    cat_map.insert("Documents", (0, 0, "#10b981"));
    cat_map.insert("Code", (0, 0, "#ec4899"));
    cat_map.insert("Other", (0, 0, "#64748b"));

    let read_res = fs::read_dir(&target).map_err(|e| e.to_string())?;
    for entry in read_res.flatten() {
        let p = entry.path();
        if let Ok(meta) = fs::symlink_metadata(&p) {
            if meta.is_dir() {
                dir_count += 1;
            } else if meta.is_file() {
                file_count += 1;
                let sz = meta.len();
                total_bytes += sz;

                if let Ok(fe) = entry_from_path(&p) {
                    files.push(fe);
                }

                let ext = p
                    .extension()
                    .map(|e| e.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                let cat = match ext.as_str() {
                    "jpg" | "jpeg" | "png" | "gif" | "webp" | "svg" | "bmp" | "ico" => "Images",
                    "mp4" | "mkv" | "avi" | "mov" | "webm" | "flv" | "wmv" => "Videos",
                    "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar" | "zst" => "Archives",
                    "pdf" | "docx" | "doc" | "xlsx" | "pptx" | "txt" | "md" | "rtf" => "Documents",
                    "rs" | "js" | "ts" | "jsx" | "tsx" | "py" | "go" | "cpp" | "c" | "h" | "html" | "css" | "json" => "Code",
                    _ => "Other",
                };
                if let Some(val) = cat_map.get_mut(cat) {
                    val.0 += 1;
                    val.1 += sz;
                }
            }
        }
    }

    files.sort_by(|a, b| b.size.cmp(&a.size));
    files.truncate(6);

    let categories = cat_map
        .into_iter()
        .filter(|(_, (cnt, bytes, _))| *cnt > 0 || *bytes > 0)
        .map(|(label, (count, total_bytes, color))| StorageCategory {
            label: label.to_string(),
            count,
            total_bytes,
            color: color.to_string(),
        })
        .collect();

    Ok(StorageBreakdown {
        total_bytes,
        file_count,
        dir_count,
        top_files: files,
        categories,
    })
}

/// Extract a supported archive beside itself without touching the original.
#[tauri::command]
fn extract_archive(path: String) -> Result<String, String> {
    let source = PathBuf::from(&path);
    if !source.is_file() { return Err("Choose an archive file first.".into()); }
    let extension = source.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if !["zip", "tar", "gz", "tgz", "bz2", "xz"].contains(&extension.as_str()) {
        return Err("Tally can extract ZIP and TAR-family archives on Windows. Install 7-Zip for RAR or 7Z files.".into());
    }
    let parent = source.parent().ok_or("Archive has no parent folder")?;
    let stem = source.file_stem().and_then(|s| s.to_str()).unwrap_or("Extracted files");
    let mut destination = parent.join(format!("{stem} extracted"));
    let mut number = 2;
    while destination.exists() { destination = parent.join(format!("{stem} extracted ({number})")); number += 1; }
    fs::create_dir_all(&destination).map_err(|e| format!("Cannot create extraction folder: {e}"))?;
    #[cfg(target_os = "windows")]
    let result = Command::new("tar.exe").arg("-xf").arg(&source).arg("-C").arg(&destination).output();
    #[cfg(not(target_os = "windows"))]
    let result = Command::new("tar").arg("-xf").arg(&source).arg("-C").arg(&destination).output();
    match result {
        Ok(output) if output.status.success() => Ok(clean_path_string(&destination)),
        Ok(output) => { let _ = fs::remove_dir(&destination); Err(format!("Extraction failed: {}", String::from_utf8_lossy(&output.stderr).trim())) }
        Err(error) => { let _ = fs::remove_dir(&destination); Err(format!("Could not start archive extractor: {error}")) }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            start_watch_service();
            #[cfg(target_os = "windows")]
            {
                let show = tauri::menu::MenuItem::with_id(app, "show", "Show Tally", true, None::<&str>)?;
                let quit = tauri::menu::MenuItem::with_id(app, "quit", "Quit Tally", true, None::<&str>)?;
                let menu = tauri::menu::Menu::with_items(app, &[&show, &quit])?;
                let icon = app.default_window_icon().ok_or("Missing Tally application icon")?.clone();
                tauri::tray::TrayIconBuilder::with_id("tally-tray")
                    .icon(icon)
                    .tooltip("Tally — file organization under your control")
                    .menu(&menu)
                    .on_menu_event(|app, event| match event.id.as_ref() {
                        "show" => if let Some(window) = app.get_webview_window("main") { let _ = window.show(); let _ = window.set_focus(); },
                        "quit" => app.exit(0),
                        _ => {}
                    })
                    .build(app)?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_directory,
            get_special_locations,
            open_path,
            reveal_in_file_manager,
            create_directory,
            delete_path,
            rename_path,
            get_file_preview,
            get_watched_folders,
            toggle_watch_folder,
            toggle_windows_context_menu,
            get_tally_plan,
            execute_tally_sweep,
            undo_tally_sweep,
            get_tally_history,
            get_tally_config,
            save_tally_config,
            assign_extension,
            scan_duplicates,
            isolate_duplicates,
            batch_rename,
            get_file_tags,
            set_file_tag,
            get_folder_storage,
            extract_archive,
        ])
        .run(tauri::generate_context!())
        .expect("error while building tauri application");
}
