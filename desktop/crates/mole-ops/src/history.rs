// History feature: read the shared logs the CLI also writes, so the app's
// history view and `mo history` show one merged truth.

use mole_core::history::{parse_deletions, parse_sessions, DeletionRecord, Session};
use std::path::PathBuf;

/// Resolve the operations.log path (MOLE_OPERATIONS_LOG override honored).
fn operations_log_path(home: &str) -> PathBuf {
    std::env::var("MOLE_OPERATIONS_LOG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(home).join(format!(
                "Library/Logs/{}/operations.log",
                mole_core::brand::LOG_DIR
            ))
        })
}

/// Resolve the deletions.log path (MOLE_DELETE_LOG override honored).
fn deletions_log_path(home: &str) -> PathBuf {
    std::env::var("MOLE_DELETE_LOG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(home).join(format!(
                "Library/Logs/{}/deletions.log",
                mole_core::brand::LOG_DIR
            ))
        })
}

/// Session rollup, newest first, capped at `limit` (the CLI clamps 1–200).
pub fn sessions(home: &str, limit: usize) -> Vec<Session> {
    let content = std::fs::read_to_string(operations_log_path(home)).unwrap_or_default();
    let mut sessions = parse_sessions(&content);
    sessions.truncate(limit.clamp(1, 200));
    sessions
}

/// Deletion records, newest first, capped at `limit`.
pub fn deletions(home: &str, limit: usize) -> Vec<DeletionRecord> {
    let content = std::fs::read_to_string(deletions_log_path(home)).unwrap_or_default();
    let mut records = parse_deletions(&content);
    records.truncate(limit.clamp(1, 200));
    records
}
