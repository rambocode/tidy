// Scan utilities shared by every feature: cancellable physical size
// measurement (du -skP semantics: no symlink following, allocated blocks) and
// candidate construction with identity binding at discovery time.

use mole_core::identity;
use mole_core::plan::{Candidate, Scope, SizeKb};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// Cancellation flag shared between an IPC command and its scan workers.
pub type CancelFlag = Arc<AtomicBool>;

extern "C" {
    /// From <pthread/qos.h>; not exposed by the libc crate.
    fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: i32) -> i32;
}

/// Move the calling scan worker to the Utility QoS band so heavy tree walks
/// land on efficiency cores and never compete with the UI thread. A full-speed
/// default-QoS scan showed up as >700% CPU in the process table. Best-effort:
/// on failure the thread just keeps default scheduling.
pub fn set_scan_thread_qos() {
    const QOS_CLASS_UTILITY: u32 = 0x11; // <sys/qos.h>
    unsafe {
        pthread_set_qos_class_self_np(QOS_CLASS_UTILITY, 0);
    }
}

/// Bounded worker count for parallel size measurement: half the cores, capped
/// at 4. Tree walks are metadata-I/O bound, so extra threads add CPU load and
/// heat much faster than they add throughput.
pub fn scan_workers(jobs: usize) -> usize {
    let half = std::thread::available_parallelism().map_or(2, |n| n.get() / 2);
    jobs.clamp(1, half.clamp(2, 4))
}

/// Error returned when a scan was cancelled mid-flight; partial output must
/// never feed a deletion plan.
#[derive(Debug)]
pub struct Cancelled;

/// Cancellable recursive walk behind the size measurement.
fn walk_tree(
    path: &Path,
    blocks: &mut u64,
    items: &mut u64,
    cancel: &AtomicBool,
) -> Result<bool, Cancelled> {
    if cancel.load(Ordering::Relaxed) {
        return Err(Cancelled);
    }
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        // Unreadable entries make the total a lower bound, not a failure.
        Err(_) => return Ok(false),
    };
    *blocks += meta.blocks();
    *items += 1;
    if meta.is_dir() {
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                walk_tree(&entry.path(), blocks, items, cancel)?;
            }
        }
    }
    Ok(true)
}

/// Physical size of a tree in KB plus item count, without following symlinks.
/// Returns Err(Cancelled) as soon as the flag flips — the caller discards.
/// Background users run this on utility-QoS workers (`set_scan_thread_qos`)
/// instead of duty-cycle throttling, so warmups are fast yet stay off the
/// performance cores.
pub fn dir_size_kb(path: &Path, cancel: &AtomicBool) -> Result<(SizeKb, u64), Cancelled> {
    let mut blocks = 0u64;
    let mut items = 0u64;
    walk_tree(path, &mut blocks, &mut items, cancel)?;
    Ok((SizeKb::Known(blocks * 512 / 1024), items))
}

/// Build a candidate for an existing path: snapshot identity and measure size.
/// None when the path is gone (raced away between listing and binding).
pub fn make_candidate(
    path: &Path,
    section: &str,
    scope: Scope,
    cancel: &AtomicBool,
) -> Option<Candidate> {
    let identity = identity::snapshot(path.to_str()?)?;
    let (size_kb, item_count) = dir_size_kb(path, cancel).ok()?;
    Some(Candidate {
        id: next_id(),
        path: path.to_string_lossy().into_owned(),
        size_kb,
        item_count,
        scope,
        section: section.to_string(),
        identity: Some(identity),
    })
}

/// Session-unique candidate id: monotonic counter, no RNG dependency.
pub fn next_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    format!("c{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Build a small fixture tree and return its root.
    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("tree");
        fs::create_dir_all(root.join("a/b")).unwrap();
        fs::write(root.join("f1"), vec![0u8; 4096]).unwrap();
        fs::write(root.join("a/f2"), vec![0u8; 1024]).unwrap();
        fs::write(root.join("a/b/f3"), b"x").unwrap();
        (dir, root)
    }

    /// Worker pool must stay small: parallel walks cost CPU per thread, and
    /// the pre-fix 8-worker pool read as >700% CPU in the process table.
    #[test]
    fn scan_worker_pool_is_bounded() {
        assert_eq!(scan_workers(0), 1);
        assert_eq!(scan_workers(1), 1);
        assert!(scan_workers(100) <= 4);
        assert!(scan_workers(100) >= 2);
    }

    #[test]
    fn walk_honors_cancellation() {
        let (_guard, root) = fixture();
        let cancel = AtomicBool::new(true);
        assert!(dir_size_kb(&root, &cancel).is_err());
    }
}

/// Measure many paths in parallel with a bounded worker pool; order preserved.
/// Cancelled scans yield None entries so partial results cannot masquerade as
/// complete candidates.
pub fn parallel_candidates(
    paths: &[std::path::PathBuf],
    section: &str,
    scope: Scope,
    cancel: &CancelFlag,
) -> Vec<Candidate> {
    let workers = scan_workers(paths.len());
    let queue = std::sync::Mutex::new(paths.iter().cloned().enumerate().collect::<Vec<_>>());
    let results = std::sync::Mutex::new(vec![None; paths.len()]);

    std::thread::scope(|scope_| {
        for _ in 0..workers {
            scope_.spawn(|| {
                set_scan_thread_qos();
                loop {
                    let job = queue.lock().unwrap().pop();
                    let Some((idx, path)) = job else { break };
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let candidate = make_candidate(&path, section, scope, cancel);
                    results.lock().unwrap()[idx] = candidate;
                }
            });
        }
    });

    results
        .into_inner()
        .unwrap()
        .into_iter()
        .flatten()
        .collect()
}
