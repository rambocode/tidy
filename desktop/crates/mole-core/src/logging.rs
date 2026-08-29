// Log writers, byte-compatible with the CLI (lib/core/log.sh and
// _mole_delete_log in lib/core/file_ops.sh) so `mo history` aggregates GUI
// sessions without any change. Lines are appended with O_APPEND single writes,
// which keeps concurrent CLI + app appends atomic under PIPE_BUF.

use crate::plan::SizeKb;
use chrono::Local;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Rotation threshold for operations.log (the CLI's OPLOG_MAX_SIZE_DEFAULT).
const OPLOG_MAX_SIZE: u64 = 5 * 1024 * 1024;

/// Retention for operations.log: the GUI keeps only the newest sessions —
/// history is log-only storage without a screen, so old sessions are pruned
/// by count after every real run.
const OPLOG_KEEP_SESSIONS: usize = 15;

/// Port of `bytes_to_human`: decimal units with the shell's exact rounding,
/// used in session-end markers the history parser reads back.
pub fn bytes_to_human(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        let scaled = (bytes * 100 + 500_000_000) / 1_000_000_000;
        format!("{}.{:02}GB", scaled / 100, scaled % 100)
    } else if bytes >= 1_000_000 {
        let scaled = (bytes * 10 + 500_000) / 1_000_000;
        format!("{}.{:01}MB", scaled / 10, scaled % 10)
    } else if bytes >= 1_000 {
        format!("{}KB", (bytes + 500) / 1_000)
    } else {
        format!("{bytes}B")
    }
}

/// Append one line to a log file, creating parents; failures are reported to
/// the caller so the one-shot warning contract can fire.
fn append_line(file: &Path, line: &str) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut f = OpenOptions::new().create(true).append(true).open(file)?;
    // One write call per line: atomic for concurrent appenders under PIPE_BUF.
    f.write_all(format!("{line}\n").as_bytes())
}

/// The forensic deletions log: TSV `iso_ts \t mode \t size_kb \t status \t path`.
pub struct DeletionsLog {
    path: PathBuf,
    warned: AtomicBool,
}

impl DeletionsLog {
    /// Resolve the log path: MOLE_DELETE_LOG override, else ~/Library/Logs/mole/deletions.log.
    pub fn from_env(home: &str) -> Self {
        let path = std::env::var("MOLE_DELETE_LOG")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                PathBuf::from(home).join(format!(
                    "Library/Logs/{}/deletions.log",
                    crate::brand::LOG_DIR
                ))
            });
        Self::at(path)
    }

    /// Log writing to an explicit path (tests, sandboxed runs).
    pub fn at(path: PathBuf) -> Self {
        Self {
            path,
            warned: AtomicBool::new(false),
        }
    }

    /// Port of `_mole_delete_log`: one TSV record per deletion attempt.
    /// `size_kb` may be the literal "unknown" — never coerced to 0.
    pub fn record(&self, mode: &str, size_kb: &SizeKb, status: &str, target: &str) {
        let ts = Local::now().format("%Y-%m-%dT%H:%M:%S%z").to_string();
        let line = format!("{ts}\t{mode}\t{}\t{status}\t{target}", size_kb.log_field());
        if append_line(&self.path, &line).is_err() && !self.warned.swap(true, Ordering::SeqCst) {
            // The deletions log is the only audit trail for Trash-routed
            // removals; surface the breakage once instead of silently no-oping.
            eprintln!(
                "Warning: deletions audit log unavailable ({}). Forensic trail incomplete this session.",
                self.path.display()
            );
        }
    }

    /// Raw record with a literal size field (used by tests and special statuses).
    pub fn record_raw_size(&self, mode: &str, size_field: &str, status: &str, target: &str) {
        let ts = Local::now().format("%Y-%m-%dT%H:%M:%S%z").to_string();
        let line = format!("{ts}\t{mode}\t{size_field}\t{status}\t{target}");
        let _ = append_line(&self.path, &line);
    }
}

/// The operations log `mo history` builds its session rollup from.
pub struct OperationsLog {
    path: PathBuf,
    enabled: bool,
}

impl OperationsLog {
    /// Resolve path + MO_NO_OPLOG from the environment (CLI-compatible).
    pub fn from_env(home: &str) -> Self {
        let path = std::env::var("MOLE_OPERATIONS_LOG")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                PathBuf::from(home).join(format!(
                    "Library/Logs/{}/operations.log",
                    crate::brand::LOG_DIR
                ))
            });
        let enabled = std::env::var("MO_NO_OPLOG")
            .map(|v| v != "1")
            .unwrap_or(true);
        Self { path, enabled }
    }

    /// Log at an explicit path, always enabled (tests, sandboxed runs).
    pub fn at(path: PathBuf) -> Self {
        Self {
            path,
            enabled: true,
        }
    }

    /// Rotate once when over 5MB, moving the old file aside (CLI parity).
    pub fn rotate_if_needed(&self) {
        if !self.enabled {
            return;
        }
        if let Ok(meta) = fs::metadata(&self.path) {
            if meta.len() > OPLOG_MAX_SIZE {
                let mut old = self.path.as_os_str().to_os_string();
                old.push(".old");
                let _ = fs::rename(&self.path, PathBuf::from(old));
            }
        }
    }

    /// Port of `log_operation`: `[ts] [cmd] ACTION path (detail)`.
    pub fn operation(&self, command: &str, action: &str, path: &str, detail: &str) {
        if !self.enabled || path.is_empty() {
            return;
        }
        let ts = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let mut line = format!("[{ts}] [{command}] {action} {path}");
        if !detail.is_empty() {
            line.push_str(&format!(" ({detail})"));
        }
        let _ = append_line(&self.path, &line);
    }

    /// Port of `log_operation_session_start`: blank line + start marker.
    pub fn session_start(&self, command: &str) {
        if !self.enabled {
            return;
        }
        let ts = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let _ = append_line(&self.path, "");
        let _ = append_line(
            &self.path,
            &format!("# ========== {command} session started at {ts} =========="),
        );
    }

    /// Trim operations.log to the newest OPLOG_KEEP_SESSIONS sessions.
    /// Line format is untouched (byte-compatible with the CLI parser); only
    /// whole leading session blocks are dropped. The rewrite goes through a
    /// same-directory temp file + rename, so readers never see a torn file;
    /// a CLI append racing the rename can lose that one line — accepted for
    /// a retention pass that runs once per completed session.
    pub fn prune_sessions(&self) {
        if !self.enabled {
            return;
        }
        let Ok(content) = fs::read_to_string(&self.path) else {
            return;
        };
        let lines: Vec<&str> = content.lines().collect();
        let starts: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.starts_with("# ========== ") && l.contains(" session started at "))
            .map(|(i, _)| i)
            .collect();
        if starts.len() <= OPLOG_KEEP_SESSIONS {
            return;
        }
        let mut cut = starts[starts.len() - OPLOG_KEEP_SESSIONS];
        // Keep the blank separator session_start wrote before the marker so
        // the kept blocks look exactly like appended ones.
        if cut > 0 && lines[cut - 1].is_empty() {
            cut -= 1;
        }
        let mut kept = lines[cut..].join("\n");
        kept.push('\n');
        let mut tmp = self.path.as_os_str().to_os_string();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        if fs::write(&tmp, kept).is_ok() {
            let _ = fs::rename(&tmp, &self.path);
        }
    }

    /// Port of `log_operation_session_end`: end marker with items + human size.
    pub fn session_end(&self, command: &str, items: u64, size_kb: u64) {
        if !self.enabled {
            return;
        }
        let ts = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let size_human = if size_kb > 0 {
            bytes_to_human(size_kb * 1024)
        } else {
            "0B".to_string()
        };
        let _ = append_line(
            &self.path,
            &format!("# ========== {command} session ended at {ts}, {items} items, {size_human} =========="),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_size_matches_shell_rounding() {
        assert_eq!(bytes_to_human(0), "0B");
        assert_eq!(bytes_to_human(999), "999B");
        assert_eq!(bytes_to_human(1000), "1KB");
        assert_eq!(bytes_to_human(1500), "2KB");
        assert_eq!(bytes_to_human(1_500_000), "1.5MB");
        assert_eq!(bytes_to_human(1_234_000_000), "1.23GB");
    }

    #[test]
    fn deletions_log_is_exact_tsv() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deletions.log");
        let log = DeletionsLog::at(path.clone());
        log.record("trash", &SizeKb::Known(42), "ok", "/tmp/x");
        log.record("permanent", &SizeKb::Unknown, "rejected", "/tmp/y z");
        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        let f1: Vec<&str> = lines[0].split('\t').collect();
        assert_eq!(&f1[1..], &["trash", "42", "ok", "/tmp/x"]);
        // Timestamp shape: 2026-08-17T12:34:56+0800
        assert_eq!(f1[0].len(), 24);
        let f2: Vec<&str> = lines[1].split('\t').collect();
        assert_eq!(&f2[1..], &["permanent", "unknown", "rejected", "/tmp/y z"]);
    }

    #[test]
    fn prune_keeps_only_the_newest_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("operations.log");
        let log = OperationsLog::at(path.clone());
        for i in 0..20 {
            log.session_start("clean");
            log.operation("clean", "REMOVED", &format!("/tmp/cache-{i}"), "1KB");
            log.session_end("clean", 1, 1);
        }
        log.prune_sessions();
        let content = std::fs::read_to_string(&path).unwrap();
        let starts = content
            .lines()
            .filter(|l| l.starts_with("# ========== ") && l.contains(" session started at "))
            .count();
        assert_eq!(starts, 15);
        // The oldest surviving session is #5; earlier ones are gone, the
        // newest one stays, and the leading blank separator is preserved.
        assert!(!content.contains("/tmp/cache-4"));
        assert!(content.contains("/tmp/cache-5"));
        assert!(content.contains("/tmp/cache-19"));
        assert!(content.starts_with('\n'));
    }

    #[test]
    fn prune_is_a_noop_under_the_retention_count() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("operations.log");
        let log = OperationsLog::at(path.clone());
        log.session_start("clean");
        log.session_end("clean", 0, 0);
        let before = std::fs::read_to_string(&path).unwrap();
        log.prune_sessions();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn oplog_line_and_session_markers_match_cli() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("operations.log");
        let log = OperationsLog::at(path.clone());
        log.session_start("clean");
        log.operation("clean", "REMOVED", "/tmp/cache", "15.2MB");
        log.operation("clean", "SKIPPED", "/tmp/keep", "whitelist");
        log.session_end("clean", 1, 15565);
        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines[0], "");
        assert!(lines[1].starts_with("# ========== clean session started at "));
        assert!(lines[1].ends_with(" =========="));
        assert!(lines[2].contains("] [clean] REMOVED /tmp/cache (15.2MB)"));
        assert!(lines[3].contains("] [clean] SKIPPED /tmp/keep (whitelist)"));
        assert!(lines[4].starts_with("# ========== clean session ended at "));
        assert!(lines[4].ends_with(", 1 items, 15.9MB =========="));
    }
}
