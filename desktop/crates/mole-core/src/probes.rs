// Live-state probes behind a trait so tests never touch the real process
// table or fork lsof. The tri-state contract is Mole's core guard invariant:
// "could not tell" (Unknown) must DENY deletion, never read as "not running".

use std::path::Path;
use std::process::Command;

/// Tri-state probe answer: the shell's 0 / 1 / 2 exit-code contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriState {
    /// A matching process is running / the database is in use.
    Active,
    /// Conclusively idle.
    Idle,
    /// Could not tell — treated as Active by every guard (fail closed).
    Unknown,
}

/// Probes the validator consults for live-state questions.
pub trait LiveProbes {
    /// Port of `_mole_user_cache_owner_process_state`: is a process owning
    /// this reverse-DNS cache id running?
    fn owner_process_state(&self, owner: &str) -> TriState;
    /// Port of `_mole_sqlite_database_in_use`: does any process hold the
    /// SQLite family (main / -wal / -shm) open?
    fn sqlite_in_use(&self, main_path: &Path) -> TriState;
    /// Compound-guard probe (mole_clean_process_guard contract): is any of the
    /// named executables running? Unknown must DENY at every caller.
    fn any_process_running(&self, names: &[&str]) -> TriState;
}

/// Production probes: one `ps` snapshot per instance plus bounded `lsof` calls.
pub struct SystemProbes {
    /// Filtered process table (one line per foreign process), None when unavailable.
    table: Option<String>,
}

impl SystemProbes {
    /// Snapshot the process table once; mirrors `_mole_process_table` memoization.
    pub fn new() -> Self {
        Self {
            table: capture_process_table(),
        }
    }
}

impl Default for SystemProbes {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveProbes for SystemProbes {
    fn owner_process_state(&self, owner: &str) -> TriState {
        if owner.is_empty() {
            return TriState::Unknown;
        }
        let table = match &self.table {
            Some(t) => t,
            // An unreadable process table is not proof the owner is idle.
            None => return TriState::Unknown,
        };
        owner_state_in_table(owner, table)
    }

    fn sqlite_in_use(&self, main_path: &Path) -> TriState {
        // WAL-mode -shm only exists while at least one connection is open.
        let shm = with_suffix(main_path, "-shm");
        if shm.is_file() {
            return TriState::Active;
        }
        let mut any_probe_failed = false;
        for candidate in [main_path.to_path_buf(), with_suffix(main_path, "-wal"), shm] {
            if !candidate.exists() {
                continue;
            }
            match lsof_status(&candidate) {
                Some(0) => return TriState::Active,
                Some(1) => {}
                // lsof errored or is missing: cannot prove the handle is closed.
                _ => any_probe_failed = true,
            }
        }
        if any_probe_failed {
            TriState::Unknown
        } else {
            TriState::Idle
        }
    }

    fn any_process_running(&self, names: &[&str]) -> TriState {
        let table = match &self.table {
            Some(t) => t,
            // No table, no proof of idleness.
            None => return TriState::Unknown,
        };
        for line in table.lines() {
            let comm = line.split_whitespace().next().unwrap_or("");
            let base = comm.rsplit('/').next().unwrap_or(comm);
            if names.iter().any(|n| base.eq_ignore_ascii_case(n)) {
                return TriState::Active;
            }
        }
        TriState::Idle
    }
}

/// Append a literal suffix to a path's final component (`db` → `db-wal`).
fn with_suffix(path: &Path, suffix: &str) -> std::path::PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(suffix);
    std::path::PathBuf::from(s)
}

/// Run `lsof -F n -- <path>` and return its exit code (None on spawn failure).
fn lsof_status(path: &Path) -> Option<i32> {
    Command::new("lsof")
        .args(["-F", "n", "--"])
        .arg(path)
        .output()
        .ok()
        .and_then(|out| out.status.code())
}

/// Capture `ps -axo pid,ppid,comm,args` and drop our own process tree plus the
/// measurement tools Mole forks (du/find/...), mirroring the shell awk filter.
/// Returns None when the snapshot fails — a short table must not vote.
fn capture_process_table() -> Option<String> {
    let out = Command::new("ps")
        .args(["-axo", "pid,ppid,comm,args"])
        .output()
        .ok()?;
    if !out.status.success() || out.stdout.is_empty() {
        return None;
    }
    // Process tables are not guaranteed UTF-8 (an app named 富途牛牛 is fine,
    // but arbitrary bytes are possible); compare lossily like LC_ALL=C bytes.
    let raw = String::from_utf8_lossy(&out.stdout);
    let self_pid = std::process::id().to_string();

    let mut parent: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut rows: Vec<(String, String)> = Vec::new(); // (pid, text-after-ppid)
    for line in raw.lines().skip(1) {
        let mut it = line.split_whitespace();
        let pid = it.next()?.to_string();
        let ppid = it.next()?.to_string();
        // Recover the untokenized remainder so multi-word args survive.
        let idx = line.find(&ppid).map(|i| i + ppid.len()).unwrap_or(0);
        let text = line[idx..].trim_start().to_string();
        parent.insert(pid.clone(), ppid);
        rows.push((pid, text));
    }

    // Mark our own ancestor chain and every descendant of this process.
    let mut mine: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut p = self_pid.clone();
    while p != "0" && p != "1" && !p.is_empty() && !mine.contains(&p) {
        mine.insert(p.clone());
        p = parent.get(&p).cloned().unwrap_or_default();
    }
    for (pid, _) in &rows {
        let mut q = pid.clone();
        let mut depth = 0;
        while q != "0" && q != "1" && !q.is_empty() && depth < 64 {
            if q == self_pid {
                mine.insert(pid.clone());
                break;
            }
            q = parent.get(&q).cloned().unwrap_or_default();
            depth += 1;
        }
    }

    let mut filtered = String::new();
    for (pid, text) in &rows {
        if mine.contains(pid) {
            continue;
        }
        let comm = text.split_whitespace().next().unwrap_or("");
        let base = comm.rsplit('/').next().unwrap_or(comm);
        if matches!(
            base,
            "du" | "find" | "mdfind" | "ps" | "grep" | "stat" | "ls" | "rm"
        ) {
            continue;
        }
        if text.to_ascii_lowercase().contains("com.tw93.mole") {
            continue;
        }
        filtered.push_str(text);
        filtered.push('\n');
    }
    Some(filtered)
}

/// Two acceptance shapes, deliberately asymmetric (parity with the shell and
/// the Mac app's ProcessGuard): full-id substring, or a delimited last label
/// corroborated by another id component on the SAME line.
fn owner_state_in_table(owner: &str, table: &str) -> TriState {
    let owner_lower = owner.to_ascii_lowercase();
    for line in table.lines() {
        if line.to_ascii_lowercase().contains(&owner_lower) {
            return TriState::Active;
        }
    }

    let leaf = owner.rsplit('.').next().unwrap_or("");
    if leaf.len() >= 4 && leaf != owner {
        let components: Vec<&str> = owner.split('.').collect();
        let corroborators: Vec<String> = components[..components.len().saturating_sub(1)]
            .iter()
            .filter(|c| c.len() >= 4 && !c.eq_ignore_ascii_case("com"))
            .map(|c| c.to_ascii_lowercase())
            .collect();
        if !corroborators.is_empty() {
            let leaf_lower = leaf.to_ascii_lowercase();
            for line in table.lines() {
                let lower = line.to_ascii_lowercase();
                if contains_delimited(&lower, &leaf_lower)
                    && corroborators.iter().any(|c| contains_delimited(&lower, c))
                {
                    return TriState::Active;
                }
            }
        }
    }
    TriState::Idle
}

/// Token search with non-alphanumeric boundaries on both sides (the shell's
/// `(^|[^A-Za-z0-9])token([^A-Za-z0-9]|$)` regex).
fn contains_delimited(haystack: &str, needle: &str) -> bool {
    let bytes = haystack.as_bytes();
    let mut from = 0;
    while let Some(pos) = haystack[from..].find(needle) {
        let start = from + pos;
        let end = start + needle.len();
        let left_ok = start == 0 || !bytes[start - 1].is_ascii_alphanumeric();
        let right_ok = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if left_ok && right_ok {
            return true;
        }
        from = start + 1;
    }
    false
}

/// Test stub with scripted answers; Unknown by default so forgetting to script
/// a probe fails closed in tests too.
pub struct StubProbes {
    pub owner_state: TriState,
    pub sqlite_state: TriState,
}

impl StubProbes {
    /// Stub that reports everything conclusively idle (the permissive case).
    pub fn idle() -> Self {
        Self {
            owner_state: TriState::Idle,
            sqlite_state: TriState::Idle,
        }
    }
}

impl LiveProbes for StubProbes {
    fn owner_process_state(&self, _owner: &str) -> TriState {
        self.owner_state
    }
    fn sqlite_in_use(&self, _main_path: &Path) -> TriState {
        self.sqlite_state
    }

    fn any_process_running(&self, _names: &[&str]) -> TriState {
        self.owner_state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corroboration_prevents_shared_binary_false_positive() {
        // ShipIt alone must not attribute VS Code's cache to Claude.
        let table = "/Applications/Claude.app/Contents/ShipIt\n";
        assert_eq!(
            owner_state_in_table("com.microsoft.VSCode.ShipIt", table),
            TriState::Idle
        );
        // Full-id substring matches directly.
        assert_eq!(
            owner_state_in_table(
                "com.anthropic.claude",
                "helper com.anthropic.claude --serve"
            ),
            TriState::Active
        );
        // Delimited leaf + corroborating component on the same line.
        assert_eq!(
            owner_state_in_table(
                "com.fusion.acmeconsole",
                "/opt/fusion/bin/acmeconsole --daemon"
            ),
            TriState::Active
        );
    }
}
