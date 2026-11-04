//! End-to-end tests using real temporary folders.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};
use tally::config::{Config, KnownDirs};
use tally::ledger::Ledger;
use tally::sweep::{
    plan_personal_sweep, sweep, sweep_approved_group, ApprovedAction, PlannedActionKind, Summary,
    SweepOptions,
};
use tally::undo::undo;

#[test]
fn config_file_requires_a_path_argument() {
    for args in [vec!["--config-file"], vec!["--config-file", "--help"]] {
        let args = args.into_iter().map(str::to_owned).collect::<Vec<_>>();
        assert!(super::parse_args(&args).is_err());
    }
}

#[test]
fn config_file_accepts_a_path_argument() {
    let args = ["--config-file", "custom.json"]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let cli = super::parse_args(&args).unwrap();
    assert_eq!(cli.config_file.as_deref(), Some(Path::new("custom.json")));
}

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
        for name in [
            "documents",
            "pictures",
            "videos",
            "music",
            "downloads",
            "desktop",
        ] {
            map.insert(name.to_string(), root.join(name));
        }
        map.insert("home".to_string(), root.clone());
        let mut cfg = Config::default();
        cfg.min_age_seconds = 0;
        cfg.validate().unwrap();
        let ledger = Ledger::new(root.join("home").join("transactions.jsonl"));
        Env {
            root,
            inbox,
            known: KnownDirs::from_map(map),
            cfg,
            ledger,
        }
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
    assert_eq!(
        fs::read_to_string(e.path("documents/report.pdf")).unwrap(),
        "OLD"
    );
    assert_eq!(
        fs::read_to_string(e.path("documents/report (1).pdf")).unwrap(),
        "NEW"
    );
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
    assert_eq!(
        fs::read_to_string(e.path("documents/_Duplicates/report.pdf")).unwrap(),
        "SAME"
    );
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
    assert!(
        !exists(&e.path("documents")),
        "folders created by Tally should be removed"
    );
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
    assert_eq!(
        fs::read_to_string(e.path("documents/a.pdf")).unwrap(),
        "ORIGINAL"
    );
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
    e.write(
        "Cargo.toml",
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"",
    );
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

#[test]
fn lone_manifest_in_downloads_does_not_block_sorting() {
    let e = Env::new();

    // User downloaded a sample package.json into downloads along with pictures and documents
    e.write("package.json", "{\"name\": \"sample\"}");
    e.write("invoice.pdf", "invoice content");
    e.write("photo.png", "image data");

    let s = e.run(false);

    // PDF, PNG, and JSON should all be sorted according to their respective rules!
    assert_eq!(s.moved, 3);
    assert_eq!(s.failed, 0);
    assert!(exists(&e.path("documents/invoice.pdf")));
    assert!(exists(&e.path("pictures/photo.png")));
    assert!(exists(&e.path("documents/Data/package.json")));
}

#[test]
fn date_tokens_in_rules_sort_by_year_and_month() {
    let mut e = Env::new();
    e.cfg.rules = vec![tally::config::Rule {
        name: "DatedPictures".to_string(),
        destination: "{pictures}/{YYYY}/{MM}".to_string(),
        extensions: vec![".png".to_string()],
    }];

    e.write("snap.png", "image");
    let s = e.run(false);
    assert_eq!(s.moved, 1);
    assert_eq!(s.failed, 0);

    let (y, m, _) = tally::util::date_parts(std::time::SystemTime::now());
    let expected = e.path(&format!("pictures/{y:04}/{m:02}/snap.png"));
    assert!(
        exists(&expected),
        "Expected to find file at {}",
        expected.display()
    );
}

#[test]
fn personal_scan_finds_nested_files_but_skips_projects_and_protected_folders() {
    let e = Env::new();
    let downloads = e.path("downloads");
    fs::create_dir_all(downloads.join("phone exports")).unwrap();
    fs::create_dir_all(downloads.join("sample-project/src")).unwrap();
    fs::create_dir_all(downloads.join("Windows")).unwrap();
    fs::create_dir_all(e.path("documents")).unwrap();
    fs::write(downloads.join("phone exports/scan.pdf"), "document").unwrap();
    fs::write(downloads.join("sample-project/Cargo.toml"), "[package]").unwrap();
    fs::write(
        downloads.join("sample-project/src/report.pdf"),
        "project file",
    )
    .unwrap();
    fs::write(downloads.join("Windows/system.pdf"), "protected").unwrap();

    let plan = plan_personal_sweep(
        &e.cfg,
        &e.known,
        SystemTime::now() + Duration::from_secs(60),
    )
    .unwrap();

    assert!(plan.actions.iter().any(|action| {
        action.source == downloads.join("phone exports/scan.pdf")
            && action.kind == PlannedActionKind::Move
    }));
    assert!(plan.actions.iter().any(|action| {
        action.source == downloads.join("sample-project")
            && action.kind == PlannedActionKind::Skip
            && action.reason.contains("project")
    }));
    assert!(plan.actions.iter().any(|action| {
        action.source == downloads.join("Windows")
            && action.kind == PlannedActionKind::Skip
            && action.reason.contains("protected")
    }));
    assert!(!plan
        .actions
        .iter()
        .any(|action| action.source == downloads.join("sample-project/src/report.pdf")));
}

#[test]
fn personal_scan_preserves_destination_subfolders_and_never_reenters_duplicates() {
    let e = Env::new();
    let downloads = e.path("downloads");
    let documents = e.path("documents");
    fs::create_dir_all(downloads.join("camera")).unwrap();
    fs::create_dir_all(documents.join("Reports")).unwrap();
    fs::create_dir_all(documents.join("_Duplicates")).unwrap();
    fs::write(downloads.join("camera/new.pdf"), "new").unwrap();
    fs::write(documents.join("Reports/already-filed.pdf"), "filed").unwrap();
    fs::write(documents.join("_Duplicates/copy.pdf"), "copy").unwrap();

    let plan = plan_personal_sweep(
        &e.cfg,
        &e.known,
        SystemTime::now() + Duration::from_secs(60),
    )
    .unwrap();

    assert!(plan.actions.iter().any(|action| {
        action.source == documents.join("Reports/already-filed.pdf")
            && action.kind == PlannedActionKind::Skip
            && action.reason.contains("destination folder")
    }));
    assert!(plan.actions.iter().any(|action| {
        action.source == documents.join("_Duplicates")
            && action.kind == PlannedActionKind::Skip
            && action.reason.contains("duplicates folder")
    }));
    assert!(!plan
        .actions
        .iter()
        .any(|action| action.source == documents.join("_Duplicates/copy.pdf")));
}

#[test]
fn personal_scan_rejects_a_known_folder_redirected_to_a_protected_location() {
    let e = Env::new();
    let protected = e.path("Windows");
    fs::create_dir_all(&protected).unwrap();
    fs::write(protected.join("system.pdf"), "protected").unwrap();

    let mut map = HashMap::new();
    for name in ["desktop", "pictures", "videos", "music"] {
        map.insert(name.to_string(), e.path(name));
    }
    map.insert("downloads".to_string(), protected.clone());
    map.insert("documents".to_string(), e.path("documents"));
    map.insert("home".to_string(), e.root.clone());
    let known = KnownDirs::from_map(map);

    let plan = plan_personal_sweep(&e.cfg, &known, SystemTime::now()).unwrap();

    assert!(plan.actions.iter().any(|action| {
        action.source == protected
            && action.kind == PlannedActionKind::Skip
            && action.reason.contains("protected system location")
    }));
    assert!(!plan
        .actions
        .iter()
        .any(|action| action.source == protected.join("system.pdf")));
}

#[test]
fn approving_one_personal_destination_moves_only_that_group_and_is_undoable() {
    let e = Env::new();
    let downloads = e.path("downloads");
    fs::create_dir_all(downloads.join("camera")).unwrap();
    fs::create_dir_all(e.path("documents")).unwrap();
    fs::write(downloads.join("camera/scan.pdf"), "document").unwrap();
    fs::write(downloads.join("camera/photo.png"), "image").unwrap();

    let now = SystemTime::now() + Duration::from_secs(60);
    let plan = plan_personal_sweep(&e.cfg, &e.known, now).unwrap();
    let approved: Vec<ApprovedAction> = plan
        .actions
        .iter()
        .filter(|action| {
            action.kind == PlannedActionKind::Move
                && action.destination.as_deref() == Some(e.path("documents/scan.pdf").as_path())
        })
        .map(|action| ApprovedAction {
            source: action.source.clone(),
            destination: action.destination.clone().unwrap(),
        })
        .collect();
    assert_eq!(approved.len(), 1);

    let summary = sweep_approved_group(
        &e.cfg,
        &e.known,
        &e.path("documents"),
        approved,
        now,
        &e.ledger,
    )
    .unwrap();

    assert_eq!(summary.moved, 1);
    assert!(e.path("documents/scan.pdf").exists());
    assert!(downloads.join("camera/photo.png").exists());
    assert_eq!(e.ledger.load().unwrap().len(), 1);
    assert_eq!(undo(&e.ledger, 1, false).unwrap().restored, 1);
    assert!(downloads.join("camera/scan.pdf").exists());
}

#[test]
fn personal_group_approval_rejects_a_stale_destination_preview() {
    let e = Env::new();
    let downloads = e.path("downloads");
    fs::create_dir_all(&downloads).unwrap();
    fs::create_dir_all(e.path("documents")).unwrap();
    fs::write(downloads.join("invoice.pdf"), "invoice").unwrap();

    let now = SystemTime::now() + Duration::from_secs(60);
    let plan = plan_personal_sweep(&e.cfg, &e.known, now).unwrap();
    let approved = plan
        .actions
        .iter()
        .find(|action| action.source == downloads.join("invoice.pdf"))
        .map(|action| {
            vec![ApprovedAction {
                source: action.source.clone(),
                destination: action.destination.clone().unwrap(),
            }]
        })
        .unwrap();
    fs::write(e.path("documents/invoice.pdf"), "new file").unwrap();

    let result = sweep_approved_group(
        &e.cfg,
        &e.known,
        &e.path("documents"),
        approved,
        now,
        &e.ledger,
    );

    assert!(result.unwrap_err().contains("preview changed"));
    assert_eq!(
        fs::read_to_string(downloads.join("invoice.pdf")).unwrap(),
        "invoice"
    );
}
