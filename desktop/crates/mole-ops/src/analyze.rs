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

/// Cached subtree sizes from previous scans. A top-level scan already
/// visits every file below it, so drilling into a child is a lookup here
/// instead of a fresh walk. Cleared on manual rescan, before any deletion,
/// and after CACHE_TTL.
#[derive(Default)]
pub struct SizeCache {
    inner: Mutex<Option<CacheState>>,
}

struct CacheState {
    dirs: std::collections::HashMap<PathBuf, (u64, u64)>,
    built: std::time::Instant,
}

/// Sizes older than this are re-measured (files change under us).
const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(15 * 60);

impl SizeCache {
    /// Forget everything (before deletions, or on manual rescan).
    pub fn clear(&self) {
        *self.inner.lock().unwrap() = None;
    }

    /// Cached (kb, items) for a directory, if fresh.
    fn get(&self, path: &Path) -> Option<(u64, u64)> {
        let guard = self.inner.lock().unwrap();
        let state = guard.as_ref()?;
        if state.built.elapsed() > CACHE_TTL {
            return None;
        }
        state.dirs.get(path).copied()
    }

    /// Merge one measured tree in. A stale cache is replaced, not merged.
    fn insert(&self, tree: scanutil::TreeSizes) {
        let mut guard = self.inner.lock().unwrap();
        match guard.as_mut() {
            Some(state) if state.built.elapsed() <= CACHE_TTL => state.dirs.extend(tree.dirs),
            _ => {
                *guard = Some(CacheState {
                    dirs: tree.dirs,
                    built: std::time::Instant::now(),
                })
            }
        }
    }
}

/// Scan one directory level. Cache hit: children are looked up (files
/// stat'ed, dirs from the cache). Miss: one parallel walk of the whole
/// subtree (`measure_tree`) fills the cache for this root AND every
/// directory below it. The progress callback receives (dirs_visited, the
/// directory being read) — the total is unknown until the walk ends, so the
/// live path is what tells the reader the scan is moving.
pub fn scan_dir(
    root: &str,
    cancel: &CancelFlag,
    cache: &SizeCache,
    fresh: bool,
    progress: impl Fn(usize, &Path) + Sync,
) -> std::io::Result<DirListing> {
    let root_path = Path::new(root);
    if fresh {
        cache.clear();
    }
    let mut names: Vec<PathBuf> = std::fs::read_dir(root_path)?
        .flatten()
        .map(|e| e.path())
        .collect();
    names.sort();

    // Miss on the root itself → measure the whole subtree once. The walk
    // is cancellable; a cancelled walk caches nothing.
    if cache.get(root_path).is_none() {
        progress(0, root_path);
        match scanutil::measure_tree(root_path, cancel, &progress) {
            Ok(tree) => cache.insert(tree),
            Err(scanutil::Cancelled) => {
                return Ok(DirListing {
                    root: root.to_string(),
                    entries: Vec::new(),
                    total_kb: 0,
                    truncated: true,
                })
            }
        }
    }

    let mut entries = Vec::with_capacity(names.len());
    let total = names.len();
    for (idx, path) in names.into_iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        // Sizing this level is fast on a cache hit and slow on a miss; report
        // either way so the last stretch before the table appears is visible.
        progress(total.saturating_sub(idx), &path);
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        let is_dir = meta.is_dir();
        let (size_kb, item_count) = if is_dir {
            match cache.get(&path) {
                Some(v) => v,
                // Not kept (deeper than the walk's keep depth) or raced in
                // after the walk: measure just this one.
                None => match scanutil::dir_size_kb(&path, cancel) {
                    Ok((SizeKb::Known(kb), n)) => (kb, n),
                    Ok((SizeKb::Unknown, n)) => (0, n),
                    Err(_) => break,
                },
            }
        } else {
            use std::os::unix::fs::MetadataExt;
            (meta.blocks() * 512 / 1024, 1)
        };
        entries.push(DirEntryInfo {
            name: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path: path.to_string_lossy().into_owned(),
            size_kb,
            is_dir,
            item_count,
        });
    }

    let truncated = cancel.load(Ordering::Relaxed);
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
        let cache = SizeCache::default();
        let listing = scan_dir(
            tmp.path().to_str().unwrap(),
            &cancel,
            &cache,
            false,
            |_, _| {},
        )
        .unwrap();
        assert_eq!(listing.entries.len(), 3);
        assert_eq!(listing.entries[0].name, "big");
        assert!(!listing.truncated);
        assert!(listing.total_kb >= 200);
        let sub = listing.entries.iter().find(|e| e.name == "sub").unwrap();
        assert!(sub.is_dir);
        assert_eq!(sub.item_count, 2);

        // Drill-down is served from the cache filled by the parent scan and
        // agrees with a fresh measurement.
        assert!(cache.get(&tmp.path().join("sub")).is_some());
        let child = scan_dir(sub.path.as_str(), &cancel, &cache, false, |_, _| {}).unwrap();
        assert_eq!(child.entries.len(), 1);
        assert_eq!(child.total_kb, sub.size_kb);
        // fresh=true drops the cache.
        scan_dir(sub.path.as_str(), &cancel, &cache, true, |_, _| {}).unwrap();
        assert!(cache.get(tmp.path()).is_none());
    }

    /// A scan must prove it is alive: the callback has to fire with a real
    /// directory path, not the empty label the UI used to print.
    #[test]
    fn scan_reports_live_progress_with_paths() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("a/b")).unwrap();
        std::fs::write(tmp.path().join("a/b/f"), vec![0u8; 4096]).unwrap();

        let seen: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let cancel: CancelFlag = Arc::new(AtomicBool::new(false));
        let cache = SizeCache::default();
        scan_dir(
            tmp.path().to_str().unwrap(),
            &cancel,
            &cache,
            false,
            |_, path| seen.lock().unwrap().push(path.display().to_string()),
        )
        .unwrap();

        let seen = seen.into_inner().unwrap();
        assert!(!seen.is_empty(), "progress must fire at least once");
        assert!(
            seen.iter().all(|p| !p.is_empty()),
            "every report names a directory"
        );
        assert!(seen.iter().any(|p| p.ends_with("/a")), "{seen:?}");
    }

    #[test]
    fn cancelled_scan_reports_truncated() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a"), b"x").unwrap();
        let cancel: CancelFlag = Arc::new(AtomicBool::new(true));
        let cache = SizeCache::default();
        let listing = scan_dir(
            tmp.path().to_str().unwrap(),
            &cancel,
            &cache,
            false,
            |_, _| {},
        )
        .unwrap();
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

#[cfg(test)]
mod bench {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    /// Manual: BENCH_DIR=~ cargo test -p mole-ops --release bench_scan_dir -- --ignored --nocapture
    #[test]
    #[ignore]
    fn bench_scan_dir() {
        let root = std::env::var("BENCH_DIR").unwrap_or_else(|_| std::env::var("HOME").unwrap());
        let cancel: CancelFlag = Arc::new(AtomicBool::new(false));
        let cache = SizeCache::default();
        for round in 0..2 {
            let t = std::time::Instant::now();
            let l = scan_dir(&root, &cancel, &cache, round == 0, |_, _| {}).unwrap();
            eprintln!(
                "round {round}: {} entries, {} KB, {:?}",
                l.entries.len(),
                l.total_kb,
                t.elapsed()
            );
        }
    }
}
