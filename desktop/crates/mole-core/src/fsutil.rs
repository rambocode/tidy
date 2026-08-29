// Filesystem identity helpers shared by policy and validation. These mirror the
// shell's `-ef` / `stat -f%d:%i` checks: APFS is case-insensitive but
// case-preserving, so string comparison alone cannot prove two paths differ.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// (device, inode) pair identifying a file independent of its spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileId {
    pub dev: u64,
    pub ino: u64,
}

/// Stat a path without following a leaf symlink; None when it does not exist.
pub fn lstat_id(path: &Path) -> Option<FileId> {
    let meta = fs::symlink_metadata(path).ok()?;
    Some(FileId {
        dev: meta.dev(),
        ino: meta.ino(),
    })
}

/// Stat a path following symlinks; None when it does not exist.
pub fn stat_id(path: &Path) -> Option<FileId> {
    let meta = fs::metadata(path).ok()?;
    Some(FileId {
        dev: meta.dev(),
        ino: meta.ino(),
    })
}

/// Port of `_mole_path_is_same_existing_file`: both exist and are the same inode.
pub fn is_same_existing_file(path: &str, protected: &str) -> bool {
    match (stat_id(Path::new(path)), stat_id(Path::new(protected))) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// Port of `_mole_path_is_within_existing_root`: walk lexical ancestors of
/// `path` and test each against `protected_root` by inode, so case aliases and
/// odd spellings still resolve to the protected tree.
pub fn is_within_existing_root(path: &str, protected_root: &str) -> bool {
    let root_id = match stat_id(Path::new(protected_root)) {
        Some(id) => id,
        None => return false,
    };
    let mut probe = path.to_string();
    while probe.starts_with('/') {
        if let Some(id) = stat_id(Path::new(&probe)) {
            if id == root_id {
                return true;
            }
        }
        if probe == "/" {
            break;
        }
        match probe.rfind('/') {
            Some(0) => probe = "/".to_string(),
            Some(idx) => probe.truncate(idx),
            None => break,
        }
    }
    false
}

/// Lexical parent of a path string, mirroring `${path%/*}` with the "/" fallback.
pub fn lexical_parent(path: &str) -> String {
    match path.rfind('/') {
        Some(0) => "/".to_string(),
        Some(idx) => path[..idx].to_string(),
        None => "/".to_string(),
    }
}
