// Read-side of the shared logs (port of lib/core/history.sh parsing): session
// rollups from operations.log and the deletions.log TSV feed. Parsers are
// tolerant — malformed lines are skipped, like the shell reader — so the CLI
// and the app can read each other's files across versions.

use serde::Serialize;

/// Per-action counts inside one session.
#[derive(Debug, Default, Clone, Serialize)]
pub struct ActionCounts {
    pub removed: u64,
    pub trashed: u64,
    pub skipped: u64,
    pub failed: u64,
    pub rebuilt: u64,
    pub other: u64,
}

/// One `# ========== cmd session ... ==========` bounded session.
#[derive(Debug, Clone, Serialize)]
pub struct Session {
    pub command: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    /// Items/size as recorded in the end marker (None for unterminated sessions).
    pub items: Option<u64>,
    pub size_human: Option<String>,
    pub operation_count: u64,
    pub actions: ActionCounts,
}

/// One deletions.log TSV record.
#[derive(Debug, Clone, Serialize)]
pub struct DeletionRecord {
    pub timestamp: String,
    pub mode: String,
    /// None when the log recorded the honest literal "unknown".
    pub size_kb: Option<u64>,
    pub status: String,
    pub path: String,
}

/// Parse a start marker; returns (command, timestamp).
fn parse_session_start(line: &str) -> Option<(String, String)> {
    let inner = line
        .strip_prefix("# ========== ")?
        .strip_suffix(" ==========")?;
    let (command, ts) = inner.split_once(" session started at ")?;
    Some((command.to_string(), ts.to_string()))
}

/// Parse an end marker; returns (command, timestamp, items, size_human).
fn parse_session_end(line: &str) -> Option<(String, String, u64, String)> {
    let inner = line
        .strip_prefix("# ========== ")?
        .strip_suffix(" ==========")?;
    let (command, rest) = inner.split_once(" session ended at ")?;
    let mut parts = rest.rsplitn(3, ", ");
    let size_human = parts.next()?.to_string();
    let items = parts.next()?.strip_suffix(" items")?.parse().ok()?;
    let ts = parts.next()?.to_string();
    Some((command.to_string(), ts, items, size_human))
}

/// Parse operations.log content into sessions, newest first.
pub fn parse_sessions(content: &str) -> Vec<Session> {
    let mut sessions: Vec<Session> = Vec::new();
    let mut current: Option<Session> = None;

    for line in content.lines() {
        if let Some((command, ts)) = parse_session_start(line) {
            // An unterminated previous session still counts (CLI parity).
            if let Some(s) = current.take() {
                sessions.push(s);
            }
            current = Some(Session {
                command,
                started_at: ts,
                ended_at: None,
                items: None,
                size_human: None,
                operation_count: 0,
                actions: ActionCounts::default(),
            });
            continue;
        }
        if let Some((_, ts, items, size_human)) = parse_session_end(line) {
            if let Some(mut s) = current.take() {
                s.ended_at = Some(ts);
                s.items = Some(items);
                s.size_human = Some(size_human);
                sessions.push(s);
            }
            continue;
        }
        // Operation line: `[ts] [cmd] ACTION path (detail)`.
        if let Some(s) = current.as_mut() {
            if let Some(rest) = line.strip_prefix('[') {
                if let Some((_, after_ts)) = rest.split_once("] [") {
                    if let Some((_, after_cmd)) = after_ts.split_once("] ") {
                        let action = after_cmd.split_whitespace().next().unwrap_or("");
                        s.operation_count += 1;
                        match action {
                            "REMOVED" => s.actions.removed += 1,
                            "TRASHED" => s.actions.trashed += 1,
                            "SKIPPED" => s.actions.skipped += 1,
                            "FAILED" => s.actions.failed += 1,
                            "REBUILT" => s.actions.rebuilt += 1,
                            _ => s.actions.other += 1,
                        }
                    }
                }
            }
        }
    }
    if let Some(s) = current.take() {
        sessions.push(s);
    }
    sessions.reverse();
    sessions
}

/// Parse deletions.log content, newest first; malformed lines are skipped.
pub fn parse_deletions(content: &str) -> Vec<DeletionRecord> {
    let mut records: Vec<DeletionRecord> = content
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.splitn(5, '\t').collect();
            if fields.len() != 5 {
                return None;
            }
            Some(DeletionRecord {
                timestamp: fields[0].to_string(),
                mode: fields[1].to_string(),
                size_kb: fields[2].parse().ok(),
                status: fields[3].to_string(),
                path: fields[4].to_string(),
            })
        })
        .collect();
    records.reverse();
    records
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::OperationsLog;

    #[test]
    fn round_trips_with_the_writer() {
        // Writer → parser round trip guards the byte-level contract from both sides.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("operations.log");
        let log = OperationsLog::at(path.clone());
        log.session_start("clean");
        log.operation("clean", "TRASHED", "/tmp/a", "10KB");
        log.operation("clean", "SKIPPED", "/tmp/b", "whitelist");
        log.session_end("clean", 1, 10);
        let content = std::fs::read_to_string(&path).unwrap();
        let sessions = parse_sessions(&content);
        assert_eq!(sessions.len(), 1);
        let s = &sessions[0];
        assert_eq!(s.command, "clean");
        assert_eq!(s.items, Some(1));
        assert_eq!(s.operation_count, 2);
        assert_eq!(s.actions.trashed, 1);
        assert_eq!(s.actions.skipped, 1);
        assert!(s.ended_at.is_some());
    }

    #[test]
    fn deletions_unknown_size_stays_none() {
        let content = "2026-08-17T10:00:00+0800\ttrash\tunknown\tok\t/tmp/x\n2026-08-17T10:00:01+0800\tpermanent\t42\trejected\t/tmp/y\nmalformed line\n";
        let records = parse_deletions(content);
        assert_eq!(records.len(), 2);
        // Newest first.
        assert_eq!(records[0].size_kb, Some(42));
        assert_eq!(records[1].size_kb, None);
        assert_eq!(records[1].status, "ok");
    }
}
