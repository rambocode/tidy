// In-helper validation: the privileged side of the trust boundary. The GUI
// process already validates, but the helper MUST NOT trust its caller — every
// request re-runs the full deny stack here, plus the privileged-specific
// rules: no user-mutable ancestor on the path, and Trash moves stage through
// root-owned /Library staging before the invoking user takes ownership.

use mole_core::policy::PolicyCtx;
use mole_core::probes::SystemProbes;
use mole_core::validate::{validate_path_for_deletion, ValidationCtx};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// Root-owned staging area for privileged Trash moves. Must live under
/// /Library (root-owned, not user-mutable) per the project contract.
pub const STAGE_ROOT: &str = "/Library/Application Support/Mole/PrivilegedTrashStage";

/// Why the helper refused a request; every code names its cause.
#[derive(Debug, PartialEq, Eq)]
pub enum HelperRefusal {
    /// Path failed the standard validation stack (code attached).
    Validation(String),
    /// A user-mutable ancestor makes a privileged pathname unsafe.
    MutableAncestor,
    /// The path is inside the invoking user's home — user-scope work must not
    /// escalate (least privilege).
    UserScopePath,
}

/// Validate one privileged request. `invoking_home` is the requesting user's
/// home (the helper resolves it from the XPC audit token, never from input).
pub fn validate_privileged_request(path: &str, invoking_home: &str) -> Result<(), HelperRefusal> {
    // User-home paths never need root: refuse the escalation outright.
    let home = invoking_home.trim_end_matches('/');
    if !home.is_empty() && (path == home || path.starts_with(&format!("{home}/"))) {
        return Err(HelperRefusal::UserScopePath);
    }

    let probes = SystemProbes::new();
    let ctx = ValidationCtx {
        policy: PolicyCtx {
            home: invoking_home.to_string(),
            uninstall_mode: false,
        },
        probes: &probes,
    };
    validate_path_for_deletion(path, &ctx)
        .map_err(|reason| HelperRefusal::Validation(reason.code().to_string()))?;

    if privileged_path_has_mutable_ancestor(path) {
        return Err(HelperRefusal::MutableAncestor);
    }
    Ok(())
}

/// Privileged-side port of `_mole_privileged_path_has_mutable_ancestor`,
/// evaluated with the helper's own stat authority: any parent that is a
/// symlink, non-root-owned, group/other-writable, or unstatable refuses.
pub fn privileged_path_has_mutable_ancestor(path: &str) -> bool {
    let mut probe = lexical_parent(path);
    loop {
        let p = Path::new(&probe);
        let meta = match fs::symlink_metadata(p) {
            Ok(m) => m,
            // Unknown must refuse.
            Err(_) => return true,
        };
        if meta.file_type().is_symlink() {
            return true;
        }
        if meta.uid() != 0 || (meta.mode() & 0o022) != 0 {
            return true;
        }
        if probe == "/" {
            break;
        }
        probe = lexical_parent(&probe);
    }
    false
}

/// Lexical parent with the "/" fallback (mirrors mole-core::fsutil).
fn lexical_parent(path: &str) -> String {
    match path.rfind('/') {
        Some(0) => "/".to_string(),
        Some(idx) => path[..idx].to_string(),
        None => "/".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_home_paths_never_escalate() {
        let err = validate_privileged_request("/Users/someone/Library/Caches/x", "/Users/someone")
            .unwrap_err();
        assert_eq!(err, HelperRefusal::UserScopePath);
    }

    #[test]
    fn critical_paths_are_refused_with_the_validation_code() {
        // /etc is a symlink to /private/etc on macOS, so the ancestor-symlink
        // guard fires first and reports resolves-into-critical; a literal
        // critical path reports critical-system-path. Both refuse.
        let err = validate_privileged_request("/etc/passwd", "/Users/someone").unwrap_err();
        assert!(matches!(
            err,
            HelperRefusal::Validation(ref c)
                if c == "critical-system-path" || c == "resolves-into-critical"
        ));
        let err2 = validate_privileged_request("/System/Library/CoreServices", "/Users/someone")
            .unwrap_err();
        assert!(matches!(err2, HelperRefusal::Validation(ref c) if c == "critical-system-path"));
    }

    #[test]
    fn mutable_ancestor_detection_refuses_temp_trees() {
        // A temp dir is owned by the invoking user, not root: mutable.
        let tmp = tempfile::tempdir().unwrap();
        let inner = tmp.path().join("x");
        std::fs::write(&inner, b"1").unwrap();
        assert!(privileged_path_has_mutable_ancestor(
            inner.to_str().unwrap()
        ));
        // /private/var/log has a root-owned, non-writable ancestor chain.
        assert!(!privileged_path_has_mutable_ancestor(
            "/private/var/log/install.log"
        ));
    }
}
