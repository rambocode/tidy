// Analyze: cancellable one-level directory listing with physical sizes
// (ncdu-style drill-down), plus a plan builder for ad-hoc deletion that is
// Trash-only by contract (analyze-driven cleanup must stay recoverable).

use crate::scanutil::{self, CancelFlag};
use mole_core::plan::{DeletionPlan, Scope, SizeKb};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Mutex;

/// One row in the drill-down table.
#[derive(Debug, Clone, Serialize)]
pub struct DirEntryInfo {
    pub name: String,
    pub path: String,
    pub size_kb: u64,
    pub is_dir: bool,
    pub item_count: u64,
}

/// One level of a directory, entries sorted by size descending.
#[derive(Debug, Serialize)]
pub struct DirListing {
    pub root: String,
    pub entries: Vec<DirEntryInfo>,
    pub total_kb: u64,
    /// True when the scan was cancelled and the listing is incomplete.
    pub truncated: bool,
}

/// Scan one directory level; per-entry sizes measured in parallel. The
/// progress callback receives (entries_done, total_entries).
pub fn scan_dir(
    root: &str,
    cancel: &CancelFlag,
    progress: impl Fn(usize, usize) + Sync,
) -> std::io::Result<DirListing> {
    let root_path = Path::new(root);
    let mut names: Vec<PathBuf> = std::fs::read_dir(root_path)?
        .flatten()
        .map(|e| e.path())
        .collect();
    names.sort();

    let total = names.len();
    let done = std::sync::atomic::AtomicUsize::new(0);
    let results: Mutex<Vec<Option<DirEntryInfo>>> = Mutex::new(vec![None; total]);
    let queue: Mutex<Vec<(usize, PathBuf)>> = Mutex::new(names.into_iter().enumerate().collect());

    // Bounded pool on the Utility QoS band: full-speed default-QoS walks
    // saturated every performance core (>700% CPU for a home-dir scan).
    let workers = scanutil::scan_workers(total);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                scanutil::set_scan_thread_qos();
                loop {
                    let job = queue.lock().unwrap().pop();
                    let Some((idx, path)) = job else { break };
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let meta = match std::fs::symlink_metadata(&path) {
                        Ok(m) => m,
                        Err(_) => continue,
                    };
                    let is_dir = meta.is_dir();
                    let (size_kb, item_count) = match scanutil::dir_size_kb(&path, cancel) {
                        Ok(v) => v,
                        Err(_) => break, // cancelled: leave the slot empty
                    };
                    let entry = DirEntryInfo {
                        name: path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                        path: path.to_string_lossy().into_owned(),
                        size_kb: match size_kb {
                            SizeKb::Known(kb) => kb,
                            SizeKb::Unknown => 0,
                        },
                        is_dir,
                        item_count,
                    };
                    results.lock().unwrap()[idx] = Some(entry);
                    progress(done.fetch_add(1, Ordering::Relaxed) + 1, total);
                }
            });
        }
    });

    let truncated = cancel.load(Ordering::Relaxed);
    let mut entries: Vec<DirEntryInfo> = results
        .into_inner()
        .unwrap()
        .into_iter()
        .flatten()
        .collect();
    entries.sort_by_key(|e| std::cmp::Reverse(e.size_kb));
    let total_kb = entries.iter().map(|e| e.size_kb).sum();
    Ok(DirListing {
        root: root.to_string(),
        entries,
        total_kb,
        truncated,
    })
}

/// Build an ad-hoc deletion plan from explicit paths the user picked in the
/// analyze view. Identity-bound now, validated again at the sink; the caller
/// executes it Trash-only.
pub fn plan_delete_paths(paths: &[String], cancel: &CancelFlag) -> DeletionPlan {
    let mut plan = DeletionPlan::default();
    for path in paths {
        if let Some(candidate) =
            scanutil::make_candidate(Path::new(path), "analyze", Scope::User, cancel)
        {
            plan.candidates.push(candidate);
        }
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    #[test]
    fn scan_lists_and_sorts_by_size() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("small"), vec![0u8; 10]).unwrap();
        std::fs::write(tmp.path().join("big"), vec![0u8; 200_000]).unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        std::fs::write(tmp.path().join("sub/child"), vec![0u8; 50_000]).unwrap();

        let cancel: CancelFlag = Arc::new(AtomicBool::new(false));
        let listing = scan_dir(tmp.path().to_str().unwrap(), &cancel, |_, _| {}).unwrap();
        assert_eq!(listing.entries.len(), 3);
        assert_eq!(listing.entries[0].name, "big");
        assert!(!listing.truncated);
        assert!(listing.total_kb >= 200);
        let sub = listing.entries.iter().find(|e| e.name == "sub").unwrap();
        assert!(sub.is_dir);
        assert_eq!(sub.item_count, 2);
    }

    #[test]
    fn cancelled_scan_reports_truncated() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a"), b"x").unwrap();
        let cancel: CancelFlag = Arc::new(AtomicBool::new(true));
        let listing = scan_dir(tmp.path().to_str().unwrap(), &cancel, |_, _| {}).unwrap();
        assert!(listing.truncated);
    }

    #[test]
    fn adhoc_plan_binds_identity() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("victim");
        std::fs::write(&f, b"x").unwrap();
        let cancel: CancelFlag = Arc::new(AtomicBool::new(false));
        let plan = plan_delete_paths(&[f.to_string_lossy().into_owned()], &cancel);
        assert_eq!(plan.candidates.len(), 1);
        assert!(plan.candidates[0].identity.is_some());
    }
}
