//! Transaction ledger: an append-only JSON Lines file, one transaction per line.

use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    pub source_path: PathBuf,
    pub destination_path: PathBuf,
    /// "moved" or "moved_duplicate"
    pub action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    pub transaction_id: String,
    pub timestamp: String,
    /// The folder that was swept.
    pub target: PathBuf,
    pub operations: Vec<Operation>,
    /// Folders Tally created during this transaction (removed on undo if empty).
    pub created_dirs: Vec<PathBuf>,
}

pub struct Ledger {
    path: PathBuf,
}

impl Ledger {
    pub fn new(path: PathBuf) -> Ledger {
        Ledger { path }
    }

    pub fn load(&self) -> Result<Vec<Transaction>, String> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let text = fs::read_to_string(&self.path)
            .map_err(|e| format!("cannot read ledger {}: {e}", self.path.display()))?;
        let mut out = Vec::new();
        for (i, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let tx: Transaction = serde_json::from_str(line).map_err(|e| {
                format!("ledger {} is corrupt at line {}: {e}", self.path.display(), i + 1)
            })?;
            out.push(tx);
        }
        Ok(out)
    }

    pub fn append(&self, tx: &Transaction) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        let line = serde_json::to_string(tx).map_err(|e| format!("cannot encode transaction: {e}"))?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| format!("cannot open ledger {}: {e}", self.path.display()))?;
        writeln!(file, "{line}").map_err(|e| format!("cannot write ledger: {e}"))?;
        file.sync_all().map_err(|e| format!("cannot flush ledger: {e}"))
    }

    /// Replace the whole ledger (used after undo). Writes a temp file, then renames it.
    pub fn save_all(&self, txs: &[Transaction]) -> Result<(), String> {
        let tmp = self.path.with_extension("jsonl.tmp");
        let mut body = String::new();
        for tx in txs {
            body.push_str(&serde_json::to_string(tx).map_err(|e| e.to_string())?);
            body.push('\n');
        }
        fs::write(&tmp, body).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
        fs::rename(&tmp, &self.path).map_err(|e| format!("cannot update ledger: {e}"))
    }
}
