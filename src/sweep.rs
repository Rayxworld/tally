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

fn inside_dir(path: &Path, directory: &Path) -> bool {
    let path = key(&normalize_path(path));
    let directory = key(&normalize_path(directory));
    path == directory
        || path.strip_prefix(&directory).is_some_and(|remaining| {
            remaining.starts_with(std::path::MAIN_SEPARATOR)
                || remaining.starts_with(if cfg!(windows) { '/' } else { '\\' })
        })
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

#[derive(Debug, Deserialize)]
pub struct ApprovedAction {
    pub source: PathBuf,
    pub destination: PathBuf,
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
    if is_project_root(target) {
        return Ok(SweepPlan {
            warning: Some(format!(
                "Recognized software project at {}. Automatic sorting is disabled for this folder.",
                target.display()
            )),
            ..SweepPlan::default()
        });
    }

    let mut entries: Vec<fs::DirEntry> = fs::read_dir(target)
        .map_err(|e| format!("cannot read {}: {e}", target.display()))?
        .filter_map(Result::ok)
        .collect();
    entries.sort_by_key(|entry| entry.file_name());
    let sources = entries.into_iter().map(|entry| entry.path()).collect();
    plan_sweep_paths(cfg, known, sources, now)
}

fn plan_sweep_paths(
    cfg: &Config,
    known: &KnownDirs,
    mut sources: Vec<PathBuf>,
    now: SystemTime,
) -> Result<SweepPlan, String> {
    let mut plan = SweepPlan::default();

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

    sources.sort();

    let mut reserved: HashMap<String, PathBuf> = HashMap::new();

    for source in sources {
        let metadata = match fs::symlink_metadata(&source) {
            Ok(metadata) => metadata,
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

        let file_type = metadata.file_type();
        if file_type.is_dir() {
            continue;
        }

        let name = match source.file_name().and_then(|name| name.to_str()) {
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

        if source
            .parent()
            .is_some_and(|parent| inside_dir(parent, &dest_dir))
        {
            plan.summary.skipped += 1;
            plan.actions.push(PlannedAction {
                source,
                destination: None,
                kind: PlannedActionKind::Skip,
                reason: "already inside its destination folder".to_owned(),
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

fn skipped_path(path: PathBuf, reason: &str, plan: &mut SweepPlan) {
    plan.summary.skipped += 1;
    plan.actions.push(PlannedAction {
        source: path,
        destination: None,
        kind: PlannedActionKind::Skip,
        reason: reason.to_owned(),
        rule_name: None,
    });
}

fn protected_directory(name: &str) -> bool {
    [
        "$recycle.bin",
        "appdata",
        "application data",
        "node_modules",
        "program files",
        "program files (x86)",
        "programdata",
        "recovery",
        "system volume information",
        "windows",
        "windows.old",
    ]
    .iter()
    .any(|protected| name.eq_ignore_ascii_case(protected))
}

fn protected_location(path: &Path, home: Option<&Path>) -> bool {
    let path = normalize_path(path);
    let home = home.map(normalize_path);
    let path = home
        .as_deref()
        .filter(|home| inside_dir(&path, home))
        .and_then(|home| path.strip_prefix(home).ok())
        .unwrap_or(&path);
    path.components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .is_some_and(protected_directory)
    })
}

fn is_volume_root(path: &Path) -> bool {
    let component_count = path.components().count();
    #[cfg(target_os = "windows")]
    {
        component_count <= 2
    }
    #[cfg(not(target_os = "windows"))]
    {
        component_count <= 1
    }
}

/// Plan supported files found recursively in the user's standard personal folders.
/// Symbolic links, project roots, hidden directories, and system-like directories are not traversed.
pub fn plan_personal_sweep(
    cfg: &Config,
    known: &KnownDirs,
    now: SystemTime,
) -> Result<SweepPlan, String> {
    let mut roots = Vec::new();
    let mut skipped = SweepPlan::default();
    for root in known.personal_folders() {
        let metadata = match fs::symlink_metadata(&root) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                skipped.summary.failed += 1;
                skipped.actions.push(PlannedAction {
                    source: root,
                    destination: None,
                    kind: PlannedActionKind::Failed,
                    reason: format!("cannot inspect personal folder: {error}"),
                    rule_name: None,
                });
                continue;
            }
        };
        if metadata.file_type().is_symlink() {
            skipped_path(
                root,
                "personal folder is a symbolic link; not scanned",
                &mut skipped,
            );
            continue;
        }
        if !metadata.is_dir() {
            continue;
        }
        let root = root
            .canonicalize()
            .map(|path| normalize_path(&path))
            .map_err(|error| format!("cannot resolve personal folder: {error}"))?;
        if is_volume_root(&root)
            || protected_location(&root, known.home_dir())
            || known.home_dir().is_some_and(|home| inside_dir(home, &root))
        {
            skipped_path(
                root,
                "personal folder resolves to a drive, profile, or protected system location; not scanned",
                &mut skipped,
            );
            continue;
        }
        roots.push(root);
    }
    roots.sort();
    roots.dedup_by(|a, b| key(a) == key(b));

    if roots.is_empty() {
        if skipped.summary.failed == 0 && skipped.summary.skipped == 0 {
            return Err("None of the standard personal folders are available to scan.".into());
        }
        return Ok(skipped);
    }

    let mut unique_roots = Vec::new();
    for root in roots {
        if !unique_roots
            .iter()
            .any(|parent: &PathBuf| root.starts_with(parent))
        {
            unique_roots.push(root);
        }
    }

    let mut sources = Vec::new();
    let mut stack = unique_roots.clone();

    while let Some(directory) = stack.pop() {
        if is_project_root(&directory) {
            skipped_path(
                directory,
                "recognized project folder; left untouched",
                &mut skipped,
            );
            continue;
        }

        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) => {
                skipped.summary.failed += 1;
                skipped.actions.push(PlannedAction {
                    source: directory.clone(),
                    destination: None,
                    kind: PlannedActionKind::Failed,
                    reason: format!("cannot scan folder: {error}"),
                    rule_name: None,
                });
                continue;
            }
        };

        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    skipped.summary.failed += 1;
                    skipped.actions.push(PlannedAction {
                        source: directory.clone(),
                        destination: None,
                        kind: PlannedActionKind::Failed,
                        reason: format!("cannot inspect folder entry: {error}"),
                        rule_name: None,
                    });
                    continue;
                }
            };

            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(error) => {
                    skipped.summary.failed += 1;
                    skipped.actions.push(PlannedAction {
                        source: path,
                        destination: None,
                        kind: PlannedActionKind::Failed,
                        reason: format!("cannot inspect file: {error}"),
                        rule_name: None,
                    });
                    continue;
                }
            };

            let name = entry.file_name().to_string_lossy().into_owned();
            if file_type.is_symlink() {
                skipped_path(path, "symbolic link; not followed", &mut skipped);
            } else if file_type.is_dir() {
                if name.starts_with('.') {
                    skipped_path(path, "hidden folder; not scanned", &mut skipped);
                } else if protected_directory(&name) {
                    skipped_path(
                        path,
                        "protected system or application folder; not scanned",
                        &mut skipped,
                    );
                } else if name.eq_ignore_ascii_case(&cfg.duplicates_folder) {
                    skipped_path(path, "Tally duplicates folder; not scanned", &mut skipped);
                } else {
                    stack.push(path);
                }
            } else if file_type.is_file() {
                sources.push(path);
            } else {
                skipped_path(path, "not a regular file", &mut skipped);
            }
        }
    }

    let mut plan = plan_sweep_paths(cfg, known, sources, now)?;
    plan.summary.skipped += skipped.summary.skipped;
    plan.summary.failed += skipped.summary.failed;
    plan.actions.extend(skipped.actions);
    plan.actions.sort_by(|a, b| a.source.cmp(&b.source));
    Ok(plan)
}

/// Execute only the exact file moves the user approved for a destination group.
/// Replanning and exact path checks prevent stale previews from moving unexpected files.
pub fn sweep_approved_group(
    cfg: &Config,
    known: &KnownDirs,
    group_destination: &Path,
    approved: Vec<ApprovedAction>,
    now: SystemTime,
    ledger: &Ledger,
) -> Result<Summary, String> {
    if approved.is_empty() {
        return Err("No files were selected for approval.".into());
    }

    let plan = plan_personal_sweep(cfg, known, now)?;
    let approved_by_source: HashMap<String, PathBuf> = approved
        .into_iter()
        .map(|action| (key(&action.source), action.destination))
        .collect();
    let mut selected = Vec::with_capacity(approved_by_source.len());

    for action in plan.actions {
        let Some(expected_destination) = approved_by_source.get(&key(&action.source)) else {
            continue;
        };
        let destination = action
            .destination
            .as_ref()
            .ok_or_else(|| "The preview changed; scan again before approving.".to_owned())?;
        if action.kind == PlannedActionKind::Skip
            || action.kind == PlannedActionKind::Failed
            || destination != expected_destination
            || destination.parent() != Some(group_destination)
        {
            return Err("The preview changed; scan again before approving.".into());
        }
        selected.push(action);
    }

    if selected.len() != approved_by_source.len() {
        return Err(
            "Some approved files changed or disappeared; scan again before approving.".into(),
        );
    }

    execute_plan(
        SweepPlan {
            summary: Summary {
                moved: selected
                    .iter()
                    .filter(|action| action.kind == PlannedActionKind::Move)
                    .count(),
                duplicates: selected
                    .iter()
                    .filter(|action| action.kind == PlannedActionKind::Duplicate)
                    .count(),
                ..Summary::default()
            },
            actions: selected,
            warning: None,
        },
        group_destination,
        &SweepOptions {
            dry_run: false,
            verbose: true,
            now,
        },
        ledger,
    )
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
    execute_plan(plan, target, opts, ledger)
}

fn execute_plan(
    plan: SweepPlan,
    target: &Path,
    opts: &SweepOptions,
    ledger: &Ledger,
) -> Result<Summary, String> {
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
