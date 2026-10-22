//! Undo: walk transactions backward and put every file back where it came from.

use crate::ledger::{Ledger, Operation};
use crate::util::move_file;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Default, PartialEq)]
pub struct UndoSummary {
    pub restored: usize,
    pub missing: usize,
    pub failed: usize,
}

/// Undo the most recent `count` transactions. With `dry_run`, only report what would happen.
pub fn undo(ledger: &Ledger, count: usize, dry_run: bool) -> Result<UndoSummary, String> {
    let mut txs = ledger.load()?;
    let mut summary = UndoSummary::default();
    if txs.is_empty() {
        println!("Nothing to undo.");
        return Ok(summary);
    }

    for _ in 0..count {
        let Some(mut tx) = txs.pop() else { break };
        println!(
            "{}Undoing {} ({}, {} files)",
            if dry_run { "[dry-run] " } else { "" },
            tx.transaction_id,
            tx.timestamp,
            tx.operations.len()
        );

        let mut retained: Vec<Operation> = Vec::new();
        for op in tx.operations.iter().rev() {
            let from = &op.destination_path;
            let to = &op.source_path;

            if !from.exists() {
                println!("[?] {} is no longer there; skipping", from.display());
                summary.missing += 1;
                continue;
            }
            if to.exists() {
                println!(
                    "[!] Cannot restore {}: something already exists at {}",
                    from.display(),
                    to.display()
                );
                summary.failed += 1;
                retained.push(op.clone());
                continue;
            }
            if dry_run {
                println!("[dry-run] would restore {} -> {}", from.display(), to.display());
                summary.restored += 1;
                continue;
            }
            let moved = to
                .parent()
                .map(fs::create_dir_all)
                .unwrap_or(Ok(()))
                .and_then(|_| move_file(from, to));
            match moved {
                Ok(()) => {
                    println!("[+] Restored {}", to.display());
                    summary.restored += 1;
                }
                Err(e) => {
                    println!("[!] Failed to restore {}: {e}", to.display());
                    summary.failed += 1;
                    retained.push(op.clone());
                }
            }
        }

        if dry_run {
            continue;
        }

        // Remove folders Tally created, deepest first. remove_dir only succeeds on empty folders.
        let mut dirs: Vec<PathBuf> = tx.created_dirs.clone();
        dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
        for d in dirs {
            let _ = fs::remove_dir(&d);
        }

        if !retained.is_empty() {
            retained.reverse(); // back to original order
            tx.operations = retained;
            txs.push(tx);
            println!("Some files could not be restored; this transaction was kept so you can retry.");
            break;
        }
    }

    if !dry_run {
        ledger.save_all(&txs)?;
    }
    println!(
        "\n{}Undo complete. {} restored, {} already gone, {} failed.",
        if dry_run { "Dry run. " } else { "" },
        summary.restored,
        summary.missing,
        summary.failed
    );
    Ok(summary)
}
