//! The sweep engine: decide where each file goes, then (unless dry-run) move it.

use crate::config::{Config, KnownDirs};
use crate::ledger::{Ledger, Operation, Transaction};
use crate::util::{
    compound_extension_of, extension_of, iso_timestamp, move_file, numbered_name, sha256_file,
    transaction_id,
};
use serde::{Deserialize, Serialize};
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

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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
    let src_len = fs::metadata(src).map(|m| m.len()).ok();
    let mut src_hash: Option<String> = None;

    for n in 0..MAX_COLLISIONS {
        let cand_name = if n == 0 {
            name.to_string()
        } else {
            numbered_name(name, n)
        };
        let cand = dest_dir.join(&cand_name);

        let (cand_exists, cand_len, cand_source) = if cand.exists() {
            let m = fs::metadata(&cand).ok();
            if m.as_ref().map(|m| m.is_dir()).unwrap_or(false) {
                continue; // a folder with this name: never touch it, try the next name
            }
            (true, m.map(|m| m.len()), None)
        } else if let Some(planned_src) = reserved.get(&key(&cand)) {
            let len = fs::metadata(planned_src).map(|m| m.len()).ok();
            (true, len, Some(planned_src.clone()))
        } else {
            (false, None, None)
        };

        if !cand_exists {
            reserved.insert(key(&cand), src.to_path_buf());
            return Ok(Resolution::Move(cand));
        }

        // Both files exist or are planned: can they be identical duplicates?
        // If file lengths differ, they CANNOT be identical copies. Skip hashing!
        let sizes_match = match (src_len, cand_len) {
            (Some(s), Some(c)) => s == c,
            _ => true,
        };

        if sizes_match {
            let existing_hash = if let Some(planned_src) = &cand_source {
                sha256_file(planned_src)?
            } else {
                sha256_file(&cand)?
            };

            let mine = match &src_hash {
                Some(h) => h.clone(),
                None => {
                    let h = sha256_file(src)?;
                    src_hash = Some(h.clone());
                    h
                }
            };

            if mine == existing_hash {
                return Ok(Resolution::Duplicate(unique_in_folder(
                    &dest_dir.join(dup_folder),
                    name,
                    src,
                    reserved,
                )));
            }
        }
        // Different size or different hash: keep looking for a free numbered name.
    }
    Err(io::Error::new(
        io::ErrorKind::Other,
        "too many name collisions",
    ))
}

fn unique_in_folder(
    folder: &Path,
    name: &str,
    src: &Path,
    reserved: &mut HashMap<String, PathBuf>,
) -> PathBuf {
    for n in 0..MAX_COLLISIONS {
        let cand_name = if n == 0 {
            name.to_string()
        } else {
            numbered_name(name, n)
        };
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

/// Return true when a folder contains markers of an actual software project.
/// A positive match protects the folder's top-level files from automatic sorting.
fn is_project_root(target: &Path) -> bool {
    let git_dir = target.join(".git");
    if git_dir.is_dir()
        || (git_dir.is_file()
            && fs::read_to_string(&git_dir)
                .map(|s| s.starts_with("gitdir:"))
                .unwrap_or(false))
    {
        return true;
    }

    const MANIFEST_MARKERS: &[&str] = &[
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

    const COMPANION_MARKERS: &[&str] = &[
        "src",
        "lib",
        "node_modules",
        "target",
        "Cargo.lock",
        "package-lock.json",
        "pnpm-lock.yaml",
        "yarn.lock",
        "poetry.lock",
        "Pipfile.lock",
        "composer.lock",
        "gradlew",
        "gradlew.bat",
        ".gitignore",
        "main.rs",
        "lib.rs",
        "index.js",
        "index.ts",
        "app.py",
        "main.py",
    ];

    // Avoid locking out an inbox folder (e.g. Downloads) merely because a user downloaded a single
    // manifest file like package.json without any project context. Require either a companion marker
    // or multiple manifests.
    let has_manifest = MANIFEST_MARKERS.iter().any(|m| target.join(m).exists());
    if has_manifest {
        let has_companion = COMPANION_MARKERS.iter().any(|c| target.join(c).exists());
        let manifest_count = MANIFEST_MARKERS
            .iter()
            .filter(|m| target.join(m).exists())
            .count();
        if has_companion || manifest_count >= 2 {
            return true;
        }
    }

    false
}

fn normalize_path(p: &Path) -> PathBuf {
    if let Ok(c) = fs::canonicalize(p) {
        let s = c.to_string_lossy();
        if let Some(stripped) = s.strip_prefix(r"\\?\") {
            return PathBuf::from(stripped);
        }
        return c;
    }
    p.to_path_buf()
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let na = normalize_path(a);
    let nb = normalize_path(b);
    na.to_string_lossy()
        .eq_ignore_ascii_case(&nb.to_string_lossy())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PlannedActionKind {
    Move,
    Duplicate,
    Skip,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedAction {
    pub source: PathBuf,
    pub destination: Option<PathBuf>,
    pub kind: PlannedActionKind,
    pub reason: String,
    pub rule_name: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SweepPlan {
    pub actions: Vec<PlannedAction>,
    pub summary: Summary,
    pub warning: Option<String>,
}

/// Build a read-only preview of the files Tally would organize.
/// This function never moves files or writes to the transaction ledger.
pub fn plan_sweep(
    cfg: &Config,
    known: &KnownDirs,
    target: &Path,
    now: SystemTime,
) -> Result<SweepPlan, String> {
    let mut plan = SweepPlan::default();

    if is_project_root(target) {
        plan.warning = Some(format!(
            "Recognized software project at {}. Automatic sorting is disabled for this folder.",
            target.display()
        ));
        return Ok(plan);
    }

    // Pre-validate all rule destinations up front
    for rule in &cfg.rules {
        known
            .expand(&rule.destination)
            .map_err(|e| format!("rule '{}': {e}", rule.name))?;
    }

    let mut by_ext: HashMap<String, &crate::config::Rule> = HashMap::new();
    for rule in &cfg.rules {
        for ext in &rule.extensions {
            by_ext.insert(ext.clone(), rule);
        }
    }

    let skip_names = cfg.skip_name_set();
    let skip_extensions: HashSet<String> = cfg
        .skip_extensions
        .iter()
        .map(|e| e.to_lowercase())
        .collect();
    let min_age = Duration::from_secs(cfg.min_age_seconds);

    let mut entries: Vec<fs::DirEntry> = fs::read_dir(target)
        .map_err(|e| format!("cannot read {}: {e}", target.display()))?
        .filter_map(Result::ok)
        .collect();
    entries.sort_by_key(|entry| entry.file_name());

    let mut reserved: HashMap<String, PathBuf> = HashMap::new();

    for entry in entries {
        let source = entry.path();

        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                plan.summary.failed += 1;
                plan.actions.push(PlannedAction {
                    source,
                    destination: None,
                    kind: PlannedActionKind::Failed,
                    reason: format!("cannot inspect file: {error}"),
                    rule_name: None,
                });
                continue;
            }
        };

        if file_type.is_dir() {
            continue;
        }

        let name = match entry.file_name().to_str() {
            Some(name) => name.to_owned(),
            None => {
                plan.summary.skipped += 1;
                plan.actions.push(PlannedAction {
                    source,
                    destination: None,
                    kind: PlannedActionKind::Skip,
                    reason: "name is not valid UTF-8".to_owned(),
                    rule_name: None,
                });
                continue;
            }
        };

        let skip_reason = if file_type.is_symlink() {
            Some("symbolic link")
        } else if name.starts_with('.') {
            Some("hidden file")
        } else if name.starts_with("~$") || skip_names.contains(&name.to_lowercase()) {
            Some("protected name")
        } else {
            None
        };

        if let Some(reason) = skip_reason {
            plan.summary.skipped += 1;
            plan.actions.push(PlannedAction {
                source,
                destination: None,
                kind: PlannedActionKind::Skip,
                reason: reason.to_owned(),
                rule_name: None,
            });
            continue;
        }

        let compound_ext = compound_extension_of(&name);
        let single_ext = extension_of(&name);

        let ext = match (
            compound_ext.as_ref().and_then(|c| by_ext.get(c)),
            single_ext.as_ref().and_then(|s| by_ext.get(s)),
        ) {
            (Some(_), _) => compound_ext.unwrap(),
            (None, Some(_)) => single_ext.unwrap(),
            (None, None) => match (compound_ext, single_ext) {
                (Some(c), _) => c,
                (None, Some(s)) => s,
                (None, None) => {
                    plan.summary.skipped += 1;
                    plan.actions.push(PlannedAction {
                        source,
                        destination: None,
                        kind: PlannedActionKind::Skip,
                        reason: "no extension".to_owned(),
                        rule_name: None,
                    });
                    continue;
                }
            },
        };

        if skip_extensions.contains(&ext) {
            plan.summary.skipped += 1;
            plan.actions.push(PlannedAction {
                source,
                destination: None,
                kind: PlannedActionKind::Skip,
                reason: "protected or partial-download extension".to_owned(),
                rule_name: None,
            });
            continue;
        }

        let matched_rule = match by_ext.get(&ext) {
            Some(rule) => *rule,
            None => {
                plan.summary.skipped += 1;
                plan.actions.push(PlannedAction {
                    source,
                    destination: None,
                    kind: PlannedActionKind::Skip,
                    reason: "no rule for this file type".to_owned(),
                    rule_name: None,
                });
                continue;
            }
        };

        let file_modified = fs::metadata(&source).and_then(|m| m.modified()).ok();
        let age_ok = file_modified
            .and_then(|modified| now.duration_since(modified).ok())
            .map(|age| age >= min_age)
            .unwrap_or(false);

        if !age_ok {
            plan.summary.skipped += 1;
            plan.actions.push(PlannedAction {
                source,
                destination: None,
                kind: PlannedActionKind::Skip,
                reason: "modified very recently (may still be downloading)".to_owned(),
                rule_name: None,
            });
            continue;
        }

        let dest_dir = match known.expand_with_time(&matched_rule.destination, file_modified) {
            Ok(d) => d,
            Err(e) => {
                plan.summary.failed += 1;
                plan.actions.push(PlannedAction {
                    source,
                    destination: None,
                    kind: PlannedActionKind::Failed,
                    reason: format!("cannot resolve destination: {e}"),
                    rule_name: Some(matched_rule.name.clone()),
                });
                continue;
            }
        };

        if same_dir(target, &dest_dir) {
            plan.summary.skipped += 1;
            plan.actions.push(PlannedAction {
                source,
                destination: None,
                kind: PlannedActionKind::Skip,
                reason: "already in its destination folder".to_owned(),
                rule_name: None,
            });
            continue;
        }

        match resolve_destination(
            &source,
            &dest_dir,
            &name,
            &cfg.duplicates_folder,
            &mut reserved,
        ) {
            Ok(Resolution::Move(destination)) => {
                plan.summary.moved += 1;
                plan.actions.push(PlannedAction {
                    source,
                    destination: Some(destination),
                    kind: PlannedActionKind::Move,
                    reason: format!("Matches the {} rule", matched_rule.name),
                    rule_name: Some(matched_rule.name.clone()),
                });
            }
            Ok(Resolution::Duplicate(destination)) => {
                plan.summary.duplicates += 1;
                plan.actions.push(PlannedAction {
                    source,
                    destination: Some(destination),
                    kind: PlannedActionKind::Duplicate,
                    reason: "Identical copy; preserve it in the duplicates folder".to_owned(),
                    rule_name: Some(matched_rule.name.clone()),
                });
            }
            Err(error) => {
                plan.summary.failed += 1;
                plan.actions.push(PlannedAction {
                    source,
                    destination: None,
                    kind: PlannedActionKind::Failed,
                    reason: error.to_string(),
                    rule_name: None,
                });
            }
        }
    }

    Ok(plan)
}

/// Sweep the top level of `target` (it does not enter sub-folders).
pub fn sweep(
    cfg: &Config,
    known: &KnownDirs,
    target: &Path,
    opts: &SweepOptions,
    ledger: &Ledger,
) -> Result<Summary, String> {
    let plan = plan_sweep(cfg, known, target, opts.now)?;

    if let Some(warning) = &plan.warning {
        println!("[!] {warning}");
        println!("No files were changed. Run Tally on a normal inbox folder instead.");
        return Ok(Summary::default());
    }

    println!(
        "{}Tally sweep over: {}",
        if opts.dry_run { "[dry-run] " } else { "" },
        target.display()
    );

    let mut operations: Vec<Operation> = Vec::new();
    let mut created_dirs: Vec<PathBuf> = Vec::new();
    let mut summary = Summary {
        skipped: plan.summary.skipped,
        ..Default::default()
    };

    for action in plan.actions {
        let name = action
            .source
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();

        match action.kind {
            PlannedActionKind::Skip => {
                if opts.verbose {
                    println!("[-] Skipped {name}: {}", action.reason);
                }
            }
            PlannedActionKind::Failed => {
                summary.failed += 1;
                println!("[!] Failed {name}: {}", action.reason);
            }
            PlannedActionKind::Move | PlannedActionKind::Duplicate => {
                let is_dup = action.kind == PlannedActionKind::Duplicate;
                let final_path = match action.destination {
                    Some(p) => p,
                    None => continue,
                };
                let label = if is_dup {
                    "identical copy set aside"
                } else {
                    action.rule_name.as_deref().unwrap_or("matched rule")
                };

                if opts.dry_run {
                    println!(
                        "[dry-run] would move {name} -> {} ({label})",
                        final_path.display()
                    );
                    if is_dup {
                        summary.duplicates += 1;
                    } else {
                        summary.moved += 1;
                    }
                    continue;
                }

                let result = final_path
                    .parent()
                    .map(|p| ensure_dir(p, &mut created_dirs))
                    .unwrap_or(Ok(()))
                    .and_then(|_| move_file(&action.source, &final_path));

                match result {
                    Ok(()) => {
                        println!("[+] {name} -> {} ({label})", final_path.display());
                        operations.push(Operation {
                            source_path: action.source.clone(),
                            destination_path: final_path,
                            action: if is_dup { "moved_duplicate" } else { "moved" }.to_string(),
                        });
                        if is_dup {
                            summary.duplicates += 1;
                        } else {
                            summary.moved += 1;
                        }
                    }
                    Err(e) => {
                        println!("[!] Failed {name}: {e}");
                        summary.failed += 1;
                    }
                }
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
        ledger
            .append(&tx)
            .map_err(|e| format!("files were moved but the undo record could not be saved: {e}"))?;
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
