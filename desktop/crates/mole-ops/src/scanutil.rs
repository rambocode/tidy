// Scan utilities shared by every feature: cancellable physical size
// measurement (du -skP semantics: no symlink following, allocated blocks) and
// candidate construction with identity binding at discovery time.

use mole_core::identity;
use mole_core::plan::{Candidate, Scope, SizeKb};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// Cancellation flag shared between an IPC command and its scan workers.
pub type CancelFlag = Arc<AtomicBool>;

extern "C" {
    /// From <pthread/qos.h>; not exposed by the libc crate.
    fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: i32) -> i32;
}

/// Maximum number of recursive filesystem walks allowed in the whole
/// process. A per-request cap is insufficient because independent Analyze,
/// Clean, and Apps requests can otherwise multiply each other's CPU load.
const GLOBAL_SCAN_WORKERS: usize = 2;

/// Process-wide counting semaphore for recursive metadata walks. It uses only
/// the standard library so the low-level scan boundary stays dependency-free.
struct ScanLimiter {
    active: Mutex<usize>,
    ready: Condvar,
    limit: usize,
}

impl ScanLimiter {
    const fn new(limit: usize) -> Self {
        Self {
            active: Mutex::new(0),
            ready: Condvar::new(),
            limit,
        }
    }

    /// Wait for one process-wide worker slot. Timed waits keep cancellation
    /// responsive even when every slot is occupied by another feature.
    fn acquire<'a>(&'a self, cancel: &AtomicBool) -> Result<ScanPermit<'a>, Cancelled> {
        let mut active = self.active.lock().unwrap();
        while *active >= self.limit {
            if cancel.load(Ordering::Relaxed) {
                return Err(Cancelled);
            }
            let (next, _) = self
                .ready
                .wait_timeout(active, Duration::from_millis(25))
                .unwrap();
            active = next;
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(Cancelled);
        }
        *active += 1;
        Ok(ScanPermit { limiter: self })
    }
}

/// RAII worker slot: every success, cancellation, and panic-unwind path
/// releases capacity for the next queued scan.
struct ScanPermit<'a> {
    limiter: &'a ScanLimiter,
}

impl Drop for ScanPermit<'_> {
    fn drop(&mut self) {
        let mut active = self.limiter.active.lock().unwrap();
        *active -= 1;
        self.limiter.ready.notify_one();
    }
}

static SCAN_LIMITER: ScanLimiter = ScanLimiter::new(GLOBAL_SCAN_WORKERS);

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

/// Bounded worker count for parallel size measurement. Each request stays at
/// or below the process-wide budget, while `SCAN_LIMITER` also coordinates
/// independent requests and product features.
pub fn scan_workers(jobs: usize) -> usize {
    jobs.clamp(1, GLOBAL_SCAN_WORKERS)
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
    let _permit = SCAN_LIMITER.acquire(cancel)?;
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
        assert_eq!(scan_workers(100), 2);
    }

    /// Multiple product scans share one process-wide budget. Per-call worker
    /// caps alone allowed three Analyze requests to run six hot walks after a
    /// WebView reload, which reproduced as roughly 500% CPU on macOS.
    #[test]
    fn scan_limiter_caps_concurrent_walks_across_callers() {
        let limiter = Arc::new(ScanLimiter::new(2));
        let cancel = Arc::new(AtomicBool::new(false));
        let active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let peak = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        std::thread::scope(|scope| {
            for _ in 0..6 {
                let limiter = limiter.clone();
                let cancel = cancel.clone();
                let active = active.clone();
                let peak = peak.clone();
                scope.spawn(move || {
                    let _permit = limiter.acquire(&cancel).unwrap();
                    let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    active.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });

        assert_eq!(peak.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn directory_walk_waiting_for_global_budget_is_cancellable() {
        let (_guard, root) = fixture();
        let occupied_cancel = AtomicBool::new(false);
        let _first = SCAN_LIMITER.acquire(&occupied_cancel).unwrap();
        let _second = SCAN_LIMITER.acquire(&occupied_cancel).unwrap();
        let waiting_cancel = Arc::new(AtomicBool::new(false));

        let waiter = {
            let waiting_cancel = waiting_cancel.clone();
            std::thread::spawn(move || dir_size_kb(&root, &waiting_cancel).is_err())
        };
        std::thread::sleep(Duration::from_millis(10));
        waiting_cancel.store(true, Ordering::Relaxed);

        assert!(waiter.join().unwrap());
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
