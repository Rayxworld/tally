//! End-to-end tests using real temporary folders.

use crate::config::{Config, KnownDirs};
use crate::ledger::Ledger;
use crate::sweep::{sweep, Summary, SweepOptions};
use crate::undo::undo;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct Env {
    root: PathBuf,
    inbox: PathBuf,
    known: KnownDirs,
    cfg: Config,
    ledger: Ledger,
}

impl Env {
    fn new() -> Env {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("tally-test-{}-{id}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let inbox = root.join("inbox");
        fs::create_dir_all(&inbox).unwrap();
        let mut map = HashMap::new();
        for name in ["documents", "pictures", "videos", "music", "downloads", "desktop"] {
            map.insert(name.to_string(), root.join(name));
        }
        map.insert("home".to_string(), root.clone());
        let mut cfg = Config::default();
        cfg.min_age_seconds = 0;
        cfg.validate().unwrap();
        let ledger = Ledger::new(root.join("home").join("transactions.jsonl"));
        Env { root, inbox, known: KnownDirs::from_map(map), cfg, ledger }
    }

    fn write(&self, rel: &str, content: &str) -> PathBuf {
        let p = self.inbox.join(rel);
        fs::write(&p, content).unwrap();
        p
    }

    fn run(&self, dry_run: bool) -> Summary {
        let opts = SweepOptions {
            dry_run,
            verbose: true,
            now: SystemTime::now() + Duration::from_secs(60),
        };
        sweep(&self.cfg, &self.known, &self.inbox, &opts, &self.ledger).unwrap()
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn exists(p: &Path) -> bool {
    p.exists()
}

#[test]
fn sorts_by_extension_and_leaves_the_rest() {
    let e = Env::new();
    e.write("a.pdf", "pdf");
    e.write("b.PNG", "png");
    e.write("c.mp4", "vid");
    e.write("notes.unknownext", "x");
    e.write(".hidden.pdf", "h");
    e.write("desktop.ini", "i");
    e.write("movie.mp4.crdownload", "partial");
    fs::create_dir_all(e.inbox.join("node_modules")).unwrap();
    fs::write(e.inbox.join("node_modules").join("x.pdf"), "dep").unwrap();

    let s = e.run(false);
    assert_eq!(s.moved, 3);
    assert_eq!(s.failed, 0);
    assert!(exists(&e.path("documents/a.pdf")));
    assert!(exists(&e.path("pictures/b.PNG")));
    assert!(exists(&e.path("videos/c.mp4")));
    // untouched
    assert!(exists(&e.inbox.join("notes.unknownext")));
    assert!(exists(&e.inbox.join(".hidden.pdf")));
    assert!(exists(&e.inbox.join("desktop.ini")));
    assert!(exists(&e.inbox.join("movie.mp4.crdownload")));
    assert!(exists(&e.inbox.join("node_modules").join("x.pdf")));
}

#[test]
fn dry_run_changes_nothing() {
    let e = Env::new();
    e.write("a.pdf", "pdf");
    let s = e.run(true);
    assert_eq!(s.moved, 1);
    assert!(exists(&e.inbox.join("a.pdf")));
    assert!(!exists(&e.path("documents")));
    assert!(e.ledger.load().unwrap().is_empty());
}

#[test]
fn recent_files_are_skipped() {
    let mut e = Env::new();
    e.cfg.min_age_seconds = 3600;
    e.write("fresh.pdf", "x");
    let s = e.run(false);
    assert_eq!(s.moved, 0);
    assert_eq!(s.skipped, 1);
    assert!(exists(&e.inbox.join("fresh.pdf")));
}

#[test]
fn different_content_gets_a_numbered_name() {
    let e = Env::new();
    fs::create_dir_all(e.path("documents")).unwrap();
    fs::write(e.path("documents/report.pdf"), "OLD").unwrap();
    e.write("report.pdf", "NEW");
    e.run(false);
    assert_eq!(fs::read_to_string(e.path("documents/report.pdf")).unwrap(), "OLD");
    assert_eq!(fs::read_to_string(e.path("documents/report (1).pdf")).unwrap(), "NEW");
}

#[test]
fn identical_content_is_set_aside_not_deleted() {
    let e = Env::new();
    fs::create_dir_all(e.path("documents")).unwrap();
    fs::write(e.path("documents/report.pdf"), "SAME").unwrap();
    e.write("report.pdf", "SAME");
    let s = e.run(false);
    assert_eq!(s.duplicates, 1);
    assert!(!exists(&e.inbox.join("report.pdf")));
    assert_eq!(fs::read_to_string(e.path("documents/_Duplicates/report.pdf")).unwrap(), "SAME");
}

#[test]
fn dry_run_plans_consistent_names_for_same_run_collisions() {
    // Two files can't share a name in one folder, so simulate via two rules mapping to one dir:
    // a.txt in the inbox and a.txt already present in the destination, plus a second identical one.
    let e = Env::new();
    fs::create_dir_all(e.path("documents")).unwrap();
    fs::write(e.path("documents/a.txt"), "ONE").unwrap();
    e.write("a.txt", "TWO");
    let s = e.run(true);
    assert_eq!(s.moved, 1);
    assert!(exists(&e.inbox.join("a.txt")));
    assert!(!exists(&e.path("documents/a (1).txt")));
}

#[test]
fn undo_restores_files_and_removes_created_folders() {
    let e = Env::new();
    e.write("a.pdf", "A");
    e.write("tool.exe", "E");
    e.run(false);
    assert!(exists(&e.path("documents/Installers/tool.exe")));

    let s = undo(&e.ledger, 1, false).unwrap();
    assert_eq!(s.restored, 2);
    assert_eq!(fs::read_to_string(e.inbox.join("a.pdf")).unwrap(), "A");
    assert!(exists(&e.inbox.join("tool.exe")));
    assert!(!exists(&e.path("documents")), "folders created by Tally should be removed");
    assert!(e.ledger.load().unwrap().is_empty());
}

#[test]
fn undo_never_overwrites_and_keeps_transaction_for_retry() {
    let e = Env::new();
    e.write("a.pdf", "ORIGINAL");
    e.run(false);
    // User puts a different file at the original location.
    fs::write(e.inbox.join("a.pdf"), "NEWER").unwrap();
    let s = undo(&e.ledger, 1, false).unwrap();
    assert_eq!(s.failed, 1);
    assert_eq!(fs::read_to_string(e.inbox.join("a.pdf")).unwrap(), "NEWER");
    assert_eq!(fs::read_to_string(e.path("documents/a.pdf")).unwrap(), "ORIGINAL");
    assert_eq!(e.ledger.load().unwrap().len(), 1);

    // After the user frees the spot, undo succeeds.
    fs::remove_file(e.inbox.join("a.pdf")).unwrap();
    let s = undo(&e.ledger, 1, false).unwrap();
    assert_eq!(s.restored, 1);
    assert!(e.ledger.load().unwrap().is_empty());
}

#[test]
fn undo_supports_multiple_levels_in_order() {
    let e = Env::new();
    e.write("one.pdf", "1");
    e.run(false);
    e.write("two.pdf", "2");
    e.run(false);
    assert_eq!(e.ledger.load().unwrap().len(), 2);
    undo(&e.ledger, 1, false).unwrap();
    assert!(exists(&e.inbox.join("two.pdf")));
    assert!(exists(&e.path("documents/one.pdf")));
    undo(&e.ledger, 1, false).unwrap();
    assert!(exists(&e.inbox.join("one.pdf")));
}

#[test]
fn undo_dry_run_changes_nothing() {
    let e = Env::new();
    e.write("a.pdf", "A");
    e.run(false);
    undo(&e.ledger, 1, true).unwrap();
    assert!(exists(&e.path("documents/a.pdf")));
    assert_eq!(e.ledger.load().unwrap().len(), 1);
}

#[test]
fn does_not_sweep_a_folder_into_itself() {
    let mut e = Env::new();
    e.cfg.rules[0].destination = e.inbox.to_string_lossy().to_string();
    e.write("a.pdf", "A");
    let s = e.run(false);
    assert_eq!(s.moved, 0);
    assert!(exists(&e.inbox.join("a.pdf")));
}


#[test]
fn recognized_project_root_is_left_untouched() {
    let e = Env::new();

    // Simulate a Rust project in the inbox.
    e.write("Cargo.toml", "[package]\nname = \"demo\"\nversion = \"0.1.0\"");
    e.write("main.rs", "fn main() {}");
    e.write("report.pdf", "important document");

    let s = e.run(false);

    // Nothing inside a recognized project should be moved.
    assert_eq!(s.moved, 0);
    assert_eq!(s.duplicates, 0);
    assert_eq!(s.failed, 0);

    assert!(e.inbox.join("Cargo.toml").exists());
    assert!(e.inbox.join("main.rs").exists());
    assert!(e.inbox.join("report.pdf").exists());

    assert!(e.ledger.load().unwrap().is_empty());
}

