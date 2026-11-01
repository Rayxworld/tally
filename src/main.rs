#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;
use std::time::SystemTime;

use tally::config::{Config, KnownDirs};
use tally::ledger::Ledger;
use tally::sweep::{sweep, SweepOptions};
use tally::undo;

const HELP: &str = "\
Tally - safe, undoable folder organizer

USAGE:
    tally --sweep <DIR> [--dry-run] [--verbose]   Sort files in DIR (top level only)
    tally --undo [N]    [--dry-run]               Reverse the last N sweeps (default 1)
    tally --history                               List recorded sweeps
    tally --stats <DIR>                           Show space use and largest files
    tally --watch <DIR>                           Keep a folder organized after a 5s quiet period
    tally --init-config                           Write a default config file
    tally --show-config                           Print the configuration in effect
    tally --help | --version

OPTIONS:
    --dry-run              Show what would happen; change nothing
    --verbose              Also list files that were skipped, and why
    --config-file <PATH>   Use this config file instead of the default location

Set TALLY_HOME to change where the config file and undo history are stored.
Tally never overwrites or deletes your files.";

enum Mode {
    Sweep(PathBuf),
    Stats(PathBuf),
    Watch(PathBuf),
    Undo(usize),
    History,
    InitConfig,
    ShowConfig,
    Help,
    Version,
}

struct Cli {
    mode: Mode,
    dry_run: bool,
    verbose: bool,
    config_file: Option<PathBuf>,
}

fn parse_args(args: &[String]) -> Result<Cli, String> {
    let mut mode: Option<Mode> = None;
    let mut dry_run = false;
    let mut verbose = false;
    let mut config_file = None;

    let set_mode = |m: Mode, mode: &mut Option<Mode>| -> Result<(), String> {
        if mode.is_some() {
            return Err("choose only one action at a time".into());
        }
        *mode = Some(m);
        Ok(())
    };

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--sweep" => {
                i += 1;
                let dir = args
                    .get(i)
                    .filter(|a| !a.starts_with("--"))
                    .ok_or("--sweep needs a folder, e.g. tally --sweep ~/Downloads")?;
                set_mode(Mode::Sweep(PathBuf::from(dir)), &mut mode)?;
            }
            "--undo" => {
                let mut n = 1usize;
                if let Some(next) = args.get(i + 1) {
                    if !next.starts_with("--") {
                        n = next
                            .parse::<usize>()
                            .ok()
                            .filter(|n| *n > 0)
                            .ok_or_else(|| {
                                format!("--undo expects a positive number, got '{next}'")
                            })?;
                        i += 1;
                    }
                }
                set_mode(Mode::Undo(n), &mut mode)?;
            }
            "--history" => set_mode(Mode::History, &mut mode)?,
            "--stats" | "--watch" => {
                let flag = args[i].as_str();
                i += 1;
                let dir = args
                    .get(i)
                    .filter(|a| !a.starts_with("--"))
                    .ok_or_else(|| format!("{flag} needs a folder"))?;
                set_mode(
                    if flag == "--stats" {
                        Mode::Stats(PathBuf::from(dir))
                    } else {
                        Mode::Watch(PathBuf::from(dir))
                    },
                    &mut mode,
                )?;
            }
            "--init-config" => set_mode(Mode::InitConfig, &mut mode)?,
            "--show-config" => set_mode(Mode::ShowConfig, &mut mode)?,
            "--help" | "-h" => set_mode(Mode::Help, &mut mode)?,
            "--version" | "-V" => set_mode(Mode::Version, &mut mode)?,
            "--dry-run" => dry_run = true,
            "--verbose" | "-v" => verbose = true,
            "--config-file" => {
                i += 1;
                let p = args.get(i).ok_or("--config-file needs a path")?;
                config_file = Some(PathBuf::from(p));
            }
            other => return Err(format!("unknown argument '{other}'. Try: tally --help")),
        }
        i += 1;
    }
    Ok(Cli {
        mode: mode.unwrap_or(Mode::Help),
        dry_run,
        verbose,
        config_file,
    })
}

fn tally_home() -> PathBuf {
    if let Some(p) = std::env::var_os("TALLY_HOME") {
        return PathBuf::from(p);
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("tally")
}

fn run(cli: Cli) -> Result<bool, String> {
    let home = tally_home();
    let config_path = cli
        .config_file
        .clone()
        .unwrap_or_else(|| home.join("tally.config.json"));
    let ledger = Ledger::new(home.join("transactions.jsonl"));

    match cli.mode {
        Mode::Help => {
            println!("{HELP}");
            Ok(true)
        }
        Mode::Version => {
            println!("tally {}", env!("CARGO_PKG_VERSION"));
            Ok(true)
        }
        Mode::InitConfig => {
            if config_path.exists() {
                return Err(format!(
                    "{} already exists; not overwriting it",
                    config_path.display()
                ));
            }
            if let Some(parent) = config_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
            }
            let json =
                serde_json::to_string_pretty(&Config::default()).map_err(|e| e.to_string())?;
            std::fs::write(&config_path, json + "\n")
                .map_err(|e| format!("cannot write {}: {e}", config_path.display()))?;
            println!("Wrote default config to {}", config_path.display());
            Ok(true)
        }
        Mode::ShowConfig => {
            let cfg = Config::load(&config_path)?;
            let note = if config_path.exists() {
                "file"
            } else {
                "built-in defaults; no file found at"
            };
            eprintln!("# {note} {}", config_path.display());
            println!(
                "{}",
                serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?
            );
            Ok(true)
        }
        Mode::History => {
            let txs = ledger.load()?;
            if txs.is_empty() {
                println!("No sweeps recorded yet.");
            }
            for tx in txs.iter().rev() {
                println!(
                    "{}  {}  {} files  from {}",
                    tx.transaction_id,
                    tx.timestamp,
                    tx.operations.len(),
                    tx.target.display()
                );
            }
            Ok(true)
        }
        Mode::Stats(dir) => {
            let target = std::fs::canonicalize(&dir)
                .map_err(|e| format!("cannot open {}: {e}", dir.display()))?;
            let stats = tally::productivity::folder_stats(&target)?;
            println!(
                "{}: {} files, {} bytes",
                target.display(),
                stats.files,
                stats.bytes
            );
            for (path, bytes) in stats.largest {
                println!("  {} bytes  {}", bytes, path.display());
            }
            Ok(true)
        }
        Mode::Watch(dir) => {
            let target = std::fs::canonicalize(&dir)
                .map_err(|e| format!("cannot open {}: {e}", dir.display()))?;
            if !target.is_dir() {
                return Err(format!("{} is not a folder", target.display()));
            }
            let cfg = Config::load(&config_path)?;
            let known = KnownDirs::detect();
            let mut last_signature = folder_signature(&target)?;
            let mut changed_at: Option<std::time::Instant> = None;
            println!("Watching {}. Press Ctrl+C to stop.", target.display());
            loop {
                std::thread::sleep(Duration::from_secs(1));
                let signature = folder_signature(&target)?;
                if signature != last_signature {
                    last_signature = signature;
                    changed_at = Some(std::time::Instant::now());
                }
                if changed_at.is_some_and(|time| time.elapsed() >= Duration::from_secs(5)) {
                    let opts = SweepOptions {
                        dry_run: false,
                        verbose: true,
                        now: SystemTime::now(),
                    };
                    let summary = sweep(&cfg, &known, &target, &opts, &ledger)?;
                    println!(
                        "Watch sweep: {} moved, {} duplicates, {} skipped.",
                        summary.moved, summary.duplicates, summary.skipped
                    );
                    last_signature = folder_signature(&target)?;
                    changed_at = None;
                }
            }
        }
        Mode::Undo(n) => {
            let s = undo::undo(&ledger, n, cli.dry_run)?;
            Ok(s.failed == 0)
        }
        Mode::Sweep(dir) => {
            let target = std::fs::canonicalize(&dir)
                .map_err(|e| format!("cannot open {}: {e}", dir.display()))?;
            if !target.is_dir() {
                return Err(format!("{} is not a folder", target.display()));
            }
            let cfg = Config::load(&config_path)?;
            let known = KnownDirs::detect();
            let opts = SweepOptions {
                dry_run: cli.dry_run,
                verbose: cli.verbose,
                now: SystemTime::now(),
            };
            let s = sweep(&cfg, &known, &target, &opts, &ledger)?;
            Ok(s.failed == 0)
        }
    }
}

fn folder_signature(folder: &std::path::Path) -> Result<(u64, u128), String> {
    let mut count = 0u64;
    let mut total = 0u128;
    for item in
        std::fs::read_dir(folder).map_err(|e| format!("cannot watch {}: {e}", folder.display()))?
    {
        let meta = item
            .map_err(|e| e.to_string())?
            .metadata()
            .map_err(|e| e.to_string())?;
        if meta.is_file() {
            count += 1;
            total = total.wrapping_add(meta.len() as u128);
            if let Ok(modified) = meta.modified() {
                total = total.wrapping_add(
                    modified
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos(),
                );
            }
        }
    }
    Ok((count, total))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cli = match parse_args(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    match run(cli) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(1)
        }
    }
}
