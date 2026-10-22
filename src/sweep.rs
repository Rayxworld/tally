//! The sweep engine: decide where each file goes, then (unless dry-run) move it.

use crate::config::{Config, KnownDirs};
use crate::ledger::{Ledger, Operation, Transaction};
use crate::util::{extension_of, iso_timestamp, move_file, numbered_name, sha256_file, transaction_id};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub struct SweepOptions {
    pub dry_run: bool,
    pub verbose: bool,
    pub now: SystemTime,
}

#[derive(Debug, Default, PartialEq)]
pub struct Summary {
    pub moved: usize,
    pub duplicates: usize,
    pub skipped: usize,
    pub failed: usize,
}

#[derive(Debug, PartialEq)]
pub enum Resolution {
    /// Move to this path.
    Move(PathBuf),
    /// Identical content already exists at the destination; set this copy aside here.
    Duplicate(PathBuf),
}

const MAX_COLLISIONS: u32 = 10_000;

fn key(p: &Path) -> String {
    p.to_string_lossy().to_lowercase()
}

/// Work out the final path for `src` inside `dest_dir` without overwriting anything.
///
/// `reserved` tracks destinations already claimed earlier in the same run, which matters
/// for dry runs (nothing has actually moved yet) and keeps plans consistent.
pub fn resolve_destination(
    src: &Path,
    dest_dir: &Path,
    name: &str,
    dup_folder: &str,
    reserved: &mut HashMap<String, PathBuf>,
) -> io::Result<Resolution> {
    let mut src_hash: Option<String> = None;

    for n in 0..MAX_COLLISIONS {
        let cand_name = if n == 0 { name.to_string() } else { numbered_name(name, n) };
        let cand = dest_dir.join(&cand_name);

        let existing_hash: Option<String> = if cand.exists() {
            if cand.is_dir() {
                continue; // a folder with this name: never touch it, try the next name
            }
            Some(sha256_file(&cand)?)
        } else if let Some(planned_src) = reserved.get(&key(&cand)) {
            Some(sha256_file(planned_src)?)
        } else {
            None
        };

        match existing_hash {
            None => {
                reserved.insert(key(&cand), src.to_path_buf());
                return Ok(Resolution::Move(cand));
            }
            Some(existing) => {
                let mine = match &src_hash {
                    Some(h) => h.clone(),
                    None => {
                        let h = sha256_file(src)?;
                        src_hash = Some(h.clone());
                        h
                    }
                };
                if mine == existing {
                    return Ok(Resolution::Duplicate(unique_in_folder(
                        &dest_dir.join(dup_folder),
                        name,
                        src,
                        reserved,
                    )));
                }
                // Different content: keep looking for a free numbered name.
            }
        }
    }
    Err(io::Error::new(io::ErrorKind::Other, "too many name collisions"))
}

fn unique_in_folder(
    folder: &Path,
    name: &str,
    src: &Path,
    reserved: &mut HashMap<String, PathBuf>,
) -> PathBuf {
    for n in 0..MAX_COLLISIONS {
        let cand_name = if n == 0 { name.to_string() } else { numbered_name(name, n) };
        let cand = folder.join(cand_name);
        if !cand.exists() && !reserved.contains_key(&key(&cand)) {
            reserved.insert(key(&cand), src.to_path_buf());
            return cand;
        }
    }
    folder.join(numbered_name(name, MAX_COLLISIONS))
}

/// Create `dir` (and parents), recording which folders did not exist before.
fn ensure_dir(dir: &Path, created: &mut Vec<PathBuf>) -> io::Result<()> {
    let mut missing = Vec::new();
    let mut cur = Some(dir);
    while let Some(p) = cur {
        if p.exists() {
            break;
        }
        missing.push(p.to_path_buf());
        cur = p.parent();
    }
    fs::create_dir_all(dir)?;
    missing.reverse();
    created.extend(missing);
    Ok(())
}

/// Return true when a folder contains markers of a known software project.
/// A positive match protects the folder's top-level files from automatic sorting.
fn is_project_root(target: &Path) -> bool {
    const PROJECT_MARKERS: &[&str] = &[
        ".git",
        "Cargo.toml",
        "package.json",
        "pyproject.toml",
        "pubspec.yaml",
        "pom.xml",
        "build.gradle",
        "build.gradle.kts",
        "go.mod",
        "Gemfile",
        "composer.json",
        "project.godot",
    ];

    PROJECT_MARKERS.iter().any(|marker| target.join(marker).exists())
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let ca = fs::canonicalize(a).unwrap_or_else(|_| a.to_path_buf());
    let cb = fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
    ca == cb
}

/// Sweep the top level of `target` (it does not enter sub-folders).
pub fn sweep(
    cfg: &Config,
    known: &KnownDirs,
    target: &Path,
    opts: &SweepOptions,
    ledger: &Ledger,
) -> Result<Summary, String> {
    // Resolve every rule's destination up front so config mistakes fail before any move.
    let mut by_ext: HashMap<String, (String, PathBuf)> = HashMap::new();
    for r in &cfg.rules {
        let dest = known
            .expand(&r.destination)
            .map_err(|e| format!("rule '{}': {e}", r.name))?;
        for ext in &r.extensions {
            by_ext.insert(ext.clone(), (r.name.clone(), dest.clone()));
        }
    }
    let skip_names: HashSet<String> = cfg.skip_name_set();
    let min_age = Duration::from_secs(cfg.min_age_seconds);

        if is_project_root(target) {
        println!(
            "[!] Recognized software project at {}. Automatic sorting is disabled for this folder.",
            target.display()
        );
        println!("No files were changed. Run Tally on a normal inbox folder instead.");
        return Ok(Summary {
            moved: 0,
            duplicates: 0,
            skipped: 0,
            failed: 0,
        });
    }
    let mut entries: Vec<fs::DirEntry> = fs::read_dir(target)
        .map_err(|e| format!("cannot read {}: {e}", target.display()))?
        .filter_map(Result::ok)
        .collect();
    entries.sort_by_key(|e| e.file_name());

    let mut summary = Summary::default();
    let mut reserved: HashMap<String, PathBuf> = HashMap::new();
    let mut operations: Vec<Operation> = Vec::new();
    let mut created_dirs: Vec<PathBuf> = Vec::new();

    let skip = |summary: &mut Summary, name: &str, why: &str, verbose: bool| {
        summary.skipped += 1;
        if verbose {
            println!("[-] Skipped {name}: {why}");
        }
    };

    println!(
        "{}Tally sweep over: {}",
        if opts.dry_run { "[dry-run] " } else { "" },
        target.display()
    );

    for entry in entries {
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        // Folders are never moved, which protects .git, node_modules, projects, etc.
        if file_type.is_dir() {
            continue;
        }
        let name = match entry.file_name().to_str() {
            Some(n) => n.to_string(),
            None => {
                skip(&mut summary, &entry.file_name().to_string_lossy(), "name is not valid UTF-8", opts.verbose);
                continue;
            }
        };
        if file_type.is_symlink() {
            skip(&mut summary, &name, "symbolic link", opts.verbose);
            continue;
        }
        if name.starts_with('.') {
            skip(&mut summary, &name, "hidden file", opts.verbose);
            continue;
        }
        if name.starts_with("~$") || skip_names.contains(&name.to_lowercase()) {
            skip(&mut summary, &name, "protected name", opts.verbose);
            continue;
        }
        let ext = match extension_of(&name) {
            Some(e) => e,
            None => {
                skip(&mut summary, &name, "no extension", opts.verbose);
                continue;
            }
        };
        if cfg.skip_extensions.contains(&ext) {
            skip(&mut summary, &name, "protected or partial-download extension", opts.verbose);
            continue;
        }
        let (rule_name, dest_dir) = match by_ext.get(&ext) {
            Some(v) => v,
            None => {
                skip(&mut summary, &name, "no rule for this file type", opts.verbose);
                continue;
            }
        };
        let age_ok = fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|m| opts.now.duration_since(m).ok())
            .map(|age| age >= min_age)
            .unwrap_or(false);
        if !age_ok {
            skip(&mut summary, &name, "modified very recently (may still be downloading)", opts.verbose);
            continue;
        }
        if same_dir(target, dest_dir) {
            skip(&mut summary, &name, "already in its destination folder", opts.verbose);
            continue;
        }

        let resolution = match resolve_destination(&path, dest_dir, &name, &cfg.duplicates_folder, &mut reserved) {
            Ok(r) => r,
            Err(e) => {
                println!("[!] Failed {name}: {e}");
                summary.failed += 1;
                continue;
            }
        };
        let (final_path, is_dup) = match &resolution {
            Resolution::Move(p) => (p.clone(), false),
            Resolution::Duplicate(p) => (p.clone(), true),
        };
        let label = if is_dup { "identical copy set aside" } else { rule_name.as_str() };

        if opts.dry_run {
            println!("[dry-run] would move {name} -> {} ({label})", final_path.display());
            if is_dup { summary.duplicates += 1 } else { summary.moved += 1 }
            continue;
        }

        let result = final_path
            .parent()
            .map(|p| ensure_dir(p, &mut created_dirs))
            .unwrap_or(Ok(()))
            .and_then(|_| move_file(&path, &final_path));
        match result {
            Ok(()) => {
                println!("[+] {name} -> {} ({label})", final_path.display());
                operations.push(Operation {
                    source_path: path.clone(),
                    destination_path: final_path,
                    action: if is_dup { "moved_duplicate" } else { "moved" }.to_string(),
                });
                if is_dup { summary.duplicates += 1 } else { summary.moved += 1 }
            }
            Err(e) => {
                println!("[!] Failed {name}: {e}");
                summary.failed += 1;
            }
        }
    }

    if opts.dry_run {
        println!(
            "\nDry run complete. Nothing was changed. {} would be moved, {} identical copies would be set aside, {} skipped.",
            summary.moved, summary.duplicates, summary.skipped
        );
        return Ok(summary);
    }

    if !operations.is_empty() {
        let tx = Transaction {
            transaction_id: transaction_id(opts.now),
            timestamp: iso_timestamp(opts.now),
            target: target.to_path_buf(),
            operations,
            created_dirs,
        };
        ledger.append(&tx).map_err(|e| {
            format!("files were moved but the undo record could not be saved: {e}")
        })?;
    }

    println!(
        "\nSweep complete. {} moved, {} identical copies set aside, {} skipped, {} failed.",
        summary.moved, summary.duplicates, summary.skipped, summary.failed
    );
    if summary.moved + summary.duplicates > 0 {
        println!("Run 'tally --undo' to reverse this sweep.");
    }
    Ok(summary)
}
