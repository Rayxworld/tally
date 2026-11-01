//! Safe desktop productivity actions used by the Tally GUI.

use crate::ledger::{Ledger, Operation, Transaction};
use crate::util::{
    iso_timestamp, move_file, numbered_name, sha256_file, split_name, transaction_id,
};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone)]
pub struct DuplicateGroup {
    pub size: u64,
    pub files: Vec<PathBuf>,
}

#[derive(Debug, Default, Clone)]
pub struct FolderStats {
    pub files: usize,
    pub bytes: u64,
    pub largest: Vec<(PathBuf, u64)>,
}

/// Hash only same-size candidates, then return exact SHA-256 duplicate groups.
pub fn find_duplicates(folder: &Path) -> Result<Vec<DuplicateGroup>, String> {
    let mut by_size: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    for entry in
        fs::read_dir(folder).map_err(|e| format!("cannot scan {}: {e}", folder.display()))?
    {
        let entry = entry.map_err(|e| e.to_string())?;
        let meta = entry.metadata().map_err(|e| e.to_string())?;
        if meta.is_file() && !meta.file_type().is_symlink() {
            by_size.entry(meta.len()).or_default().push(entry.path());
        }
    }
    let mut groups = Vec::new();
    for (size, files) in by_size.into_iter().filter(|(_, files)| files.len() > 1) {
        let mut hashes: HashMap<String, Vec<PathBuf>> = HashMap::new();
        for file in files {
            hashes
                .entry(
                    sha256_file(&file)
                        .map_err(|e| format!("cannot hash {}: {e}", file.display()))?,
                )
                .or_default()
                .push(file);
        }
        for files in hashes.into_values().filter(|files| files.len() > 1) {
            groups.push(DuplicateGroup { size, files });
        }
    }
    groups.sort_by_key(|g| std::cmp::Reverse(g.size * (g.files.len() as u64 - 1)));
    Ok(groups)
}

pub fn folder_stats(folder: &Path) -> Result<FolderStats, String> {
    let mut stats = FolderStats::default();
    for entry in
        fs::read_dir(folder).map_err(|e| format!("cannot scan {}: {e}", folder.display()))?
    {
        let entry = entry.map_err(|e| e.to_string())?;
        let meta = entry.metadata().map_err(|e| e.to_string())?;
        if meta.is_file() {
            stats.files += 1;
            stats.bytes += meta.len();
            stats.largest.push((entry.path(), meta.len()));
        }
    }
    stats
        .largest
        .sort_by_key(|(_, size)| std::cmp::Reverse(*size));
    stats.largest.truncate(3);
    Ok(stats)
}

/// Move every duplicate except the first file in each group to `_Duplicates`, recorded for undo.
pub fn move_duplicates(
    groups: &[DuplicateGroup],
    folder: &Path,
    ledger: &Ledger,
) -> Result<usize, String> {
    let destination = folder.join("_Duplicates");
    let mut operations = Vec::new();
    let mut created_dirs = Vec::new();
    if groups.iter().any(|g| g.files.len() > 1) && !destination.exists() {
        fs::create_dir_all(&destination).map_err(|e| e.to_string())?;
        created_dirs.push(destination.clone());
    }
    for group in groups {
        for source in group.files.iter().skip(1) {
            let name = source
                .file_name()
                .ok_or("invalid duplicate name")?
                .to_string_lossy();
            let mut target = destination.join(name.as_ref());
            let mut n = 1;
            while target.exists() {
                target = destination.join(numbered_name(&name, n));
                n += 1;
            }
            move_file(source, &target)
                .map_err(|e| format!("cannot move {}: {e}", source.display()))?;
            operations.push(Operation {
                source_path: source.clone(),
                destination_path: target,
                action: "moved_duplicate".into(),
            });
        }
    }
    if !operations.is_empty() {
        let now = SystemTime::now();
        ledger.append(&Transaction {
            transaction_id: transaction_id(now),
            timestamp: iso_timestamp(now),
            target: folder.to_path_buf(),
            operations,
            created_dirs,
        })?;
    }
    Ok(groups.iter().map(|g| g.files.len().saturating_sub(1)).sum())
}

/// Rename supplied files with a prefix, preserving extensions and never overwriting.
pub fn batch_prefix(files: &[PathBuf], prefix: &str, ledger: &Ledger) -> Result<usize, String> {
    if prefix.trim().is_empty() {
        return Err("Enter a prefix before renaming.".into());
    }
    let mut operations = Vec::new();
    for source in files {
        let parent = source.parent().ok_or("file has no parent")?;
        let name = source
            .file_name()
            .ok_or("invalid file name")?
            .to_string_lossy();
        let (stem, ext) = split_name(&name);
        let mut target = parent.join(format!("{prefix}{stem}{ext}"));
        let mut n = 1;
        while target.exists() && target != *source {
            target = parent.join(numbered_name(&format!("{prefix}{stem}{ext}"), n));
            n += 1;
        }
        if target != *source {
            move_file(source, &target)
                .map_err(|e| format!("cannot rename {}: {e}", source.display()))?;
            operations.push(Operation {
                source_path: source.clone(),
                destination_path: target,
                action: "renamed".into(),
            });
        }
    }
    let renamed = operations.len();
    if !operations.is_empty() {
        let now = SystemTime::now();
        let target = operations[0]
            .source_path
            .parent()
            .unwrap_or(Path::new("."))
            .to_path_buf();
        ledger.append(&Transaction {
            transaction_id: transaction_id(now),
            timestamp: iso_timestamp(now),
            target,
            operations,
            created_dirs: vec![],
        })?;
    }
    Ok(renamed)
}
