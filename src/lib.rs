//! Shared Tally file-organization engine.

pub mod config;
pub mod ledger;
pub mod productivity;
pub mod sweep;
pub mod undo;
pub mod util;

/// Resolve Tally's configuration and transaction directory.
pub fn tally_home() -> std::path::PathBuf {
    if let Some(path) = std::env::var_os("TALLY_HOME") {
        return std::path::PathBuf::from(path);
    }

    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("tally")
}

/// Resolve the config file path.
pub fn config_path() -> std::path::PathBuf {
    tally_home().join("tally.config.json")
}

/// Create a ledger using Tally's standard transaction history location.
pub fn open_ledger() -> ledger::Ledger {
    ledger::Ledger::new(tally_home().join("transactions.jsonl"))
}
