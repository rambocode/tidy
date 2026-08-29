// Port of validate_path_for_deletion (lib/core/file_ops.sh:606-789). The gate
// ORDER is contractual and encoded as one straight-line sequence: deny gates
// run before the /private allowlists so an allowlist can never bypass a newer
// refusal (powerlog, live caches, EDR sensors). Every rejection names its
// cause — a refusing gate must tell the user which cause it hit.

use crate::fsutil;
use crate::policy::{self, PolicyCtx};
use crate::probes::{LiveProbes, TriState};
use std::fs;
use std::path::{Path, PathBuf};

/// Why a path was refused; codes mirror the shell's log/diagnostic vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RejectReason {
    EmptyPath,
    NotAbsolute,
    Traversal,
    ControlChars,
    SymlinkIntoProtected { resolved: String },
    ResolvesIntoCritical { resolved: String },
    ResolvesIntoProtected { resolved: String },
    ActivePowerlogDb,
    LiveUserCache,
    InUseSqlite,
    EndpointSecurity,
    CriticalSystemPath,
    Protected,
}

impl RejectReason {
    /// Stable machine-readable code for logs and IPC errors.
    pub fn code(&self) -> &'static str {
        match self {
            RejectReason::EmptyPath => "empty-path",
            RejectReason::NotAbsolute => "not-absolute",
            RejectReason::Traversal => "traversal",
            RejectReason::ControlChars => "control-chars",
            RejectReason::SymlinkIntoProtected { .. } => "symlink-into-protected",
            RejectReason::ResolvesIntoCritical { .. } => "resolves-into-critical",
            RejectReason::ResolvesIntoProtected { .. } => "resolves-into-protected",
            RejectReason::ActivePowerlogDb => "active-powerlog-db",
            RejectReason::LiveUserCache => "live-user-cache",
            RejectReason::InUseSqlite => "in-use-sqlite",
            RejectReason::EndpointSecurity => "endpoint-security",
            RejectReason::CriticalSystemPath => "critical-system-path",
            RejectReason::Protected => "protected",
        }
    }
}

/// Context for one validation run: policy inputs plus the live-state probes.
pub struct ValidationCtx<'a> {
    pub policy: PolicyCtx,
    pub probes: &'a dyn LiveProbes,
}

/// The active PowerLog database; unlinking any family member can split live
/// state under PerfPowerServices (project hard rule: read-only surface).
const ACTIVE_POWERLOG_DB: &str = "/private/var/db/powerlog/Library/PerfPowerTelemetry/BackgroundProcessing/CurrentBackgroundProcessingDB.BGSQL";

/// Port of `_mole_normalize_deletion_policy_path`: collapse //, strip /./ and
/// trailing /. and trailing slash (root stays "/").
pub fn normalize_policy_path(path: &str) -> String {
    let mut p = path.to_string();
    while p.contains("//") {
        p = p.replace("//", "/");
    }
    while p.contains("/./") {
        p = p.replace("/./", "/");
    }
    while p.ends_with("/.") {
        p.truncate(p.len() - 2);
        if p.is_empty() {
            p.push('/');
        }
    }
    let trimmed = p.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Port of `_mole_is_active_powerlog_database_path` (case-insensitive family match).
pub fn is_active_powerlog_database_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let base = ACTIVE_POWERLOG_DB.to_ascii_lowercase();
    lower == base || lower == format!("{base}-wal") || lower == format!("{base}-shm")
}

/// Port of `_mole_is_sqlite_database_path`: main file or -wal/-shm/-journal
/// companion of a *.db / *.sqlite / *.sqlite3 database (extension case-insensitive).
pub fn is_sqlite_database_path(path: &str) -> bool {
    let base = path
        .strip_suffix("-wal")
        .or_else(|| path.strip_suffix("-shm"))
        .or_else(|| path.strip_suffix("-journal"))
        .unwrap_or(path)
        .to_ascii_lowercase();
    base.ends_with(".db") || base.ends_with(".sqlite") || base.ends_with(".sqlite3")
}

/// Port of `_mole_sqlite_family_base_path`: companion suffix → main database path.
pub fn sqlite_family_base_path(path: &str) -> String {
    path.strip_suffix("-wal")
        .or_else(|| path.strip_suffix("-shm"))
        .or_else(|| path.strip_suffix("-journal"))
        .unwrap_or(path)
        .to_string()
}

/// Port of `_mole_is_critical_deletion_path`: deletion policy for system roots.
/// Lexical arms first, then inode-based checks for case aliases on APFS.
pub fn is_critical_deletion_path(path: &str) -> bool {
    // Homebrew (Intel) and user-installed software: children stay deletable,
    // the roots themselves fall through to the deny arms below.
    if crate::glob::fnmatch(path, "/usr/local/*") || crate::glob::fnmatch(path, "/opt/homebrew/*") {
        return false;
    }

    let deny_globs: &[&str] = &[
        "/",
        "/bin",
        "/bin/*",
        "/dev",
        "/dev/*",
        "/sbin",
        "/sbin/*",
        "/usr",
        "/usr/*",
        "/System",
        "/System/*",
        "/Library",
        "/Library/Apple",
        "/Library/Apple/*",
        "/Library/Application Support",
        "/Library/Extensions",
        "/Library/Extensions/*",
        "/Library/Keychains",
        "/Library/Keychains/*",
        "/Applications",
        "/Applications/Finder.app",
        "/Applications/Finder.app/*",
        "/Applications/Safari.app",
        "/Applications/Safari.app/*",
        "/Volumes",
        "/opt",
        "/opt/homebrew",
        "/Users",
        "/Users/Shared",
        "/Users/Guest",
        "/Users/Guest/*",
        "/private",
        "/private/tmp",
        "/etc",
        "/etc/*",
        "/private/etc",
        "/private/etc/*",
        "/var",
        "/var/db",
        "/var/db/*",
        "/var/audit",
        "/var/audit/*",
        "/var/root",
        "/private/var",
        "/private/var/tmp",
        "/private/var/folders",
        "/private/var/db",
        "/private/var/db/*",
        "/private/var/audit",
        "/private/var/audit/*",
        "/private/var/root",
    ];
    if deny_globs.iter().any(|g| crate::glob::fnmatch(path, g)) {
        return true;
    }

    // Exactly one component under /Users is a home root: the empty-variable
    // collapse "/Users/$user/$leaf" -> "/Users/<name>" must never reach rm.
    if crate::glob::fnmatch(path, "/Users/*") && !crate::glob::fnmatch(path, "/Users/*/*") {
        return true;
    }

    // APFS case aliases: /SYSTEM is the same inode as /System but matches no
    // string arm above, so compare identities against the protected roots.
    const EXACT_ROOTS: &[&str] = &[
        "/",
        "/Applications",
        "/Library",
        "/Volumes",
        "/Network",
        "/cores",
        "/etc",
        "/home",
        "/net",
        "/tmp",
        "/var",
        "/private",
        "/private/tmp",
        "/private/var",
        "/private/var/tmp",
        "/private/var/folders",
        "/Users",
        "/opt",
        "/opt/homebrew",
    ];
    for root in EXACT_ROOTS {
        if fsutil::is_same_existing_file(path, root) {
            return true;
        }
    }

    const PROTECTED_TREES: &[&str] = &[
        "/bin",
        "/dev",
        "/sbin",
        "/usr",
        "/System",
        "/private/etc",
        "/private/var/audit",
        "/private/var/db",
        "/private/var/root",
        "/Library/Apple",
        "/Library/Extensions",
        "/Library/Keychains",
        "/Applications/Finder.app",
        "/Applications/Safari.app",
    ];
    for root in PROTECTED_TREES {
        if fsutil::is_within_existing_root(path, root) {
            return true;
        }
    }

    // Every account root stays protected even under component-casing changes
    // (/USERS/SHARED on a case-insensitive volume).
    let parent = fsutil::lexical_parent(path);
    fsutil::is_same_existing_file(&parent, "/Users")
}

/// Port of `_mole_user_cache_owner_component`: reverse-DNS first component
/// under $HOME/Library/Caches, or None when out of scope.
fn user_cache_owner_component(path: &str, home: &str) -> Option<String> {
    let prefix = format!("{}/Library/Caches", home.trim_end_matches('/'));
    let normalized = path.trim_end_matches('/');
    let rest = normalized.strip_prefix(&format!("{prefix}/"))?;
    let component = rest.split('/').next().unwrap_or("");
    // Reverse-DNS style only; named trees (Homebrew) have their own probes.
    if component.contains('.') && !component.starts_with('.') {
        Some(component.to_string())
    } else {
        None
    }
}

/// Port of `_mole_should_refuse_live_user_cache_path`: refuse when the owner
/// process is live or unknowable, or a SQLite family member is still open.
fn should_refuse_live_user_cache(path: &str, ctx: &ValidationCtx) -> bool {
    let owner = match user_cache_owner_component(path, &ctx.policy.home) {
        Some(o) => o,
        None => return false,
    };
    match ctx.probes.owner_process_state(&owner) {
        // Unknown is not proof of idleness: keep the cache.
        TriState::Active | TriState::Unknown => return true,
        TriState::Idle => {}
    }

    // Process conclusively idle; still refuse SQLite members another process
    // holds open (only a positive Active refuses here — the general SQLite
    // gate below is the one that fails closed on Unknown).
    if is_sqlite_database_path(path) {
        let base = sqlite_family_base_path(path);
        if ctx.probes.sqlite_in_use(Path::new(&base)) == TriState::Active {
            return true;
        }
    } else if Path::new(path).is_dir() {
        let mut candidates: Vec<PathBuf> = vec![Path::new(path).join("Cache.db")];
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("db")) {
                    candidates.push(p);
                }
            }
        }
        for candidate in candidates {
            if candidate.is_file() && ctx.probes.sqlite_in_use(&candidate) == TriState::Active {
                return true;
            }
        }
    }
    false
}

/// Resolve a leaf symlink's target to an absolute path, shell-style (relative
/// targets resolve through the link's directory). None when unresolvable.
fn resolve_symlink_target(path: &Path) -> Option<String> {
    let target = fs::read_link(path).ok()?;
    if target.is_absolute() {
        return Some(target.to_string_lossy().into_owned());
    }
    let link_dir = path.parent()?;
    let joined = link_dir.join(&target);
    let dir = fs::canonicalize(joined.parent()?).ok()?;
    Some(dir.join(joined.file_name()?).to_string_lossy().into_owned())
}

/// Validate a path for deletion. Ok(()) means the deletion sink may proceed;
/// Err names the specific refusing gate.
pub fn validate_path_for_deletion(path: &str, ctx: &ValidationCtx) -> Result<(), RejectReason> {
    if path.is_empty() {
        return Err(RejectReason::EmptyPath);
    }
    if !path.starts_with('/') {
        return Err(RejectReason::NotAbsolute);
    }
    // Reject `..` only as a complete component so "name..files" stays legal.
    if path.split('/').any(|component| component == "..") {
        return Err(RejectReason::Traversal);
    }
    if path.chars().any(|c| c.is_control()) {
        return Err(RejectReason::ControlChars);
    }

    let policy_path = normalize_policy_path(path);

    // Leaf symlink: the deny predicates must also hold for the resolved target.
    let p = Path::new(path);
    if fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink()) {
        if fs::read_link(p).is_err() {
            return Err(RejectReason::SymlinkIntoProtected {
                resolved: String::new(),
            });
        }
        if let Some(resolved) = resolve_symlink_target(p) {
            let resolved = normalize_policy_path(&resolved);
            if is_critical_deletion_path(&resolved) {
                return Err(RejectReason::SymlinkIntoProtected { resolved });
            }
        }
    }

    // Ancestor-symlink guard: literal-string checks see nothing dangerous when
    // an ANCESTOR is a symlink, while rm follows it into the real target.
    // Canonicalize the parent and re-run the deny predicates on the resolved
    // leaf. Deny-only: resolution never grants permission the literal lacked.
    let parent_dir = fsutil::lexical_parent(&policy_path);
    let parent = Path::new(&parent_dir);
    if parent.is_dir() {
        let mut probe = Some(parent);
        let mut ancestor_is_link = false;
        while let Some(dir) = probe {
            if dir == Path::new("/") {
                break;
            }
            if fs::symlink_metadata(dir).is_ok_and(|m| m.file_type().is_symlink()) {
                ancestor_is_link = true;
                break;
            }
            probe = dir.parent();
        }
        if ancestor_is_link {
            if let Ok(resolved_parent) = fs::canonicalize(parent) {
                let resolved_parent = resolved_parent.to_string_lossy().into_owned();
                if resolved_parent != parent_dir {
                    let leaf = policy_path.rsplit('/').next().unwrap_or("");
                    let resolved_path = normalize_policy_path(&format!("{resolved_parent}/{leaf}"));
                    if is_critical_deletion_path(&resolved_path) {
                        return Err(RejectReason::ResolvesIntoCritical {
                            resolved: resolved_path,
                        });
                    }
                    if policy::should_protect_path(&resolved_path, &ctx.policy) {
                        return Err(RejectReason::ResolvesIntoProtected {
                            resolved: resolved_path,
                        });
                    }
                }
            }
        }
    }

    // Allow: coresymbolicationd cache (safe, rebuildable system cache).
    if policy_path == "/System/Library/Caches/com.apple.coresymbolicationd/data"
        || policy_path.starts_with("/System/Library/Caches/com.apple.coresymbolicationd/data/")
    {
        return Ok(());
    }

    // Deny gates that must run BEFORE the /private allowlist below.
    if is_active_powerlog_database_path(&policy_path) {
        return Err(RejectReason::ActivePowerlogDb);
    }
    if should_refuse_live_user_cache(&policy_path, ctx) {
        return Err(RejectReason::LiveUserCache);
    }
    if is_sqlite_database_path(&policy_path) {
        let base = sqlite_family_base_path(&policy_path);
        match ctx.probes.sqlite_in_use(Path::new(&base)) {
            // Fail closed when lsof cannot answer (PR #1391).
            TriState::Active | TriState::Unknown => return Err(RejectReason::InUseSqlite),
            TriState::Idle => {}
        }
    }
    if policy::is_endpoint_security_cache_path(&policy_path) {
        return Err(RejectReason::EndpointSecurity);
    }

    // Allow: known safe paths under /private.
    const PRIVATE_ALLOW: &[&str] = &[
        "/private/tmp/*",
        "/private/var/tmp/*",
        "/private/var/log",
        "/private/var/log/*",
        "/private/var/folders/*",
        "/private/var/db/diagnostics",
        "/private/var/db/diagnostics/*",
        "/private/var/db/DiagnosticPipeline",
        "/private/var/db/DiagnosticPipeline/*",
        "/private/var/db/powerlog",
        "/private/var/db/powerlog/*",
        "/private/var/db/reportmemoryexception",
        "/private/var/db/reportmemoryexception/*",
        "/private/var/db/receipts/*.bom",
        "/private/var/db/receipts/*.plist",
    ];
    if PRIVATE_ALLOW
        .iter()
        .any(|g| crate::glob::fnmatch(&policy_path, g))
    {
        return Ok(());
    }

    if is_critical_deletion_path(&policy_path) {
        return Err(RejectReason::CriticalSystemPath);
    }
    if policy::should_protect_path(&policy_path, &ctx.policy) {
        return Err(RejectReason::Protected);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probes::StubProbes;

    fn ctx(probes: &StubProbes) -> ValidationCtx<'_> {
        ValidationCtx {
            policy: PolicyCtx {
                home: std::env::var("HOME").unwrap_or_else(|_| "/Users/tester".into()),
                uninstall_mode: false,
            },
            probes,
        }
    }

    #[test]
    fn normalization_matches_shell() {
        assert_eq!(normalize_policy_path("//etc//passwd"), "/etc/passwd");
        assert_eq!(normalize_policy_path("/etc/./passwd"), "/etc/passwd");
        assert_eq!(normalize_policy_path("/etc/."), "/etc");
        assert_eq!(normalize_policy_path("/"), "/");
        assert_eq!(normalize_policy_path("/foo/"), "/foo");
    }

    #[test]
    fn dangerous_paths_corpus_all_rejected() {
        // Same corpus, two implementations, one file: every entry must be refused.
        let corpus_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/fuzz_corpus/dangerous_paths.txt");
        let corpus = std::fs::read_to_string(&corpus_path).expect("read dangerous_paths.txt");
        let probes = StubProbes::idle();
        let c = ctx(&probes);
        let mut checked = 0;
        for line in corpus.lines() {
            let line = line.trim_end();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            assert!(
                validate_path_for_deletion(line, &c).is_err(),
                "corpus path must be rejected: {line:?}"
            );
            checked += 1;
        }
        assert!(checked >= 90, "corpus unexpectedly small: {checked}");
    }

    #[test]
    fn legitimate_targets_stay_deletable() {
        let probes = StubProbes::idle();
        let c = ctx(&probes);
        assert!(validate_path_for_deletion("/private/tmp/mole-test-scratch", &c).is_ok());
        assert!(validate_path_for_deletion("/usr/local/nonexistent-cellar-entry", &c).is_ok());
        assert!(
            validate_path_for_deletion("/Users/nobody-such-user/Library/Caches/some.app", &c)
                .is_ok()
        );
        // Firefox-style "name..files" is a legal component, not traversal.
        assert!(validate_path_for_deletion(
            "/Users/nobody-such-user/Library/Caches/name..files",
            &c
        )
        .is_ok());
    }

    #[test]
    fn unknown_probe_states_fail_closed() {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/tester".into());
        let probes = StubProbes {
            owner_state: crate::probes::TriState::Unknown,
            sqlite_state: crate::probes::TriState::Idle,
        };
        let c = ctx(&probes);
        let path = format!("{home}/Library/Caches/com.vendor.app/blob");
        assert_eq!(
            validate_path_for_deletion(&path, &c),
            Err(RejectReason::LiveUserCache)
        );

        let probes = StubProbes {
            owner_state: crate::probes::TriState::Idle,
            sqlite_state: crate::probes::TriState::Unknown,
        };
        let c = ctx(&probes);
        assert_eq!(
            validate_path_for_deletion("/private/tmp/x/store.sqlite", &c),
            Err(RejectReason::InUseSqlite)
        );
    }

    #[test]
    fn powerlog_family_is_read_only() {
        let probes = StubProbes::idle();
        let c = ctx(&probes);
        let base = "/private/var/db/powerlog/Library/PerfPowerTelemetry/BackgroundProcessing/CurrentBackgroundProcessingDB.BGSQL";
        for p in [
            base.to_string(),
            format!("{base}-wal"),
            format!("{base}-shm"),
        ] {
            assert_eq!(
                validate_path_for_deletion(&p, &c),
                Err(RejectReason::ActivePowerlogDb),
                "powerlog member must be refused: {p}"
            );
        }
        // Other powerlog children remain allowlisted.
        assert!(validate_path_for_deletion("/private/var/db/powerlog/old.PLSQL", &c).is_ok());
    }

    #[test]
    fn sqlite_path_predicates() {
        assert!(is_sqlite_database_path("/a/Cache.db"));
        assert!(is_sqlite_database_path("/a/Cache.DB-wal"));
        assert!(is_sqlite_database_path("/a/x.sqlite3-journal"));
        assert!(!is_sqlite_database_path("/a/x.txt"));
        assert_eq!(sqlite_family_base_path("/a/Cache.db-shm"), "/a/Cache.db");
    }
}
