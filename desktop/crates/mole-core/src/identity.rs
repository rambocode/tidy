// Path identity binding, ported from _mole_snapshot_path_identity /
// _mole_path_matches_identity. A plan holds the identity captured at scan
// time; the sink re-snapshots at the last moment and refuses when anything
// (physical parent, parent inode, target inode) changed underneath it.

use serde::{Deserialize, Serialize};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// Snapshot of where a path physically lives and what inode it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathIdentity {
    /// Physically resolved parent directory (every ancestor symlink resolved).
    pub parent: String,
    /// Parent "dev:ino".
    pub parent_id: String,
    /// Target "dev:ino" (leaf symlink NOT followed).
    pub target_id: String,
    /// Target "dev:ino:mtime" — the shell's `stat -f%d:%i:%m` expected_identity.
    pub full_identity: String,
}

/// Capture a path's identity; None when the path is gone or unstatable.
pub fn snapshot(path: &str) -> Option<PathIdentity> {
    let p = Path::new(path);
    let meta = fs::symlink_metadata(p).ok()?;

    let lexical_parent = crate::fsutil::lexical_parent(path);
    // Physical parent: canonicalize resolves every ancestor symlink, matching
    // the shell's `cd -P && pwd -P`.
    let physical_parent = fs::canonicalize(&lexical_parent).ok()?;
    let parent_meta = fs::metadata(&physical_parent).ok()?;

    Some(PathIdentity {
        parent: physical_parent.to_string_lossy().into_owned(),
        parent_id: format!("{}:{}", parent_meta.dev(), parent_meta.ino()),
        target_id: format!("{}:{}", meta.dev(), meta.ino()),
        full_identity: format!("{}:{}:{}", meta.dev(), meta.ino(), meta.mtime()),
    })
}

/// Re-snapshot and compare: true only when parent path, parent inode, and
/// target inode all still match the expectation.
pub fn matches(path: &str, expected: &PathIdentity) -> bool {
    match snapshot(path) {
        Some(now) => {
            now.parent == expected.parent
                && now.parent_id == expected.parent_id
                && now.target_id == expected.target_id
        }
        None => false,
    }
}
