// Clean: catalog-driven cleanup plan builder. Preview and execution consume
// the SAME plan (the CLI's `_safe_clean_impl` parity rule): protected,
// whitelisted, live-owner, and model-cache paths are filtered HERE so the
// preview equals the eligible set, and the sink re-checks policy at the
// deletion boundary anyway.
//
// Guard contract: every process probe is tri-state, and Unknown DENIES —
// a cache whose owner cannot be proven idle stays.

use crate::scanutil::{self, CancelFlag};
use mole_core::plan::{DeletionPlan, Scope};
use mole_core::policy::{self, PolicyCtx};
use mole_core::probes::{LiveProbes, TriState};
use mole_core::state::load_whitelist;
use std::path::{Path, PathBuf};

/// Browser cache targets each carry their own process guard: deleting a
/// Chromium cache under a RUNNING browser corrupts its cache index.
const BROWSER_TARGETS: &[(&str, &[&str], &[&str])] = &[
    // (cache dir under ~/Library/Caches, guard process names, skip-from-sweep names)
    (
        "Google/Chrome",
        &["Google Chrome", "Google Chrome Helper"],
        &["Google"],
    ),
    ("Microsoft Edge", &["Microsoft Edge"], &["Microsoft Edge"]),
    (
        "BraveSoftware/Brave-Browser",
        &["Brave Browser"],
        &["BraveSoftware"],
    ),
    ("Arc", &["Arc", "Arc Helper"], &["Arc"]),
    ("Firefox", &["firefox"], &["Firefox", "Mozilla"]),
];

/// Xcode build tooling names (port of xcode_build_tooling_process_state).
const XCODE_TOOLING: &[&str] = &[
    "Xcode",
    "xcodebuild",
    "xctest",
    "XCTRunner",
    "XCBBuildService",
    "swift-frontend",
];

/// Cache-owner categories for the preview cards. Keys are stable i18n ids the
/// frontend translates; matching is by lowercase substring of the cache dir
/// name (display grouping only — protection logic is untouched).
const CATEGORY_RULES: &[(&str, &[&str])] = &[
    (
        "ai",
        &[
            "claude",
            "anthropic",
            "openai",
            "chatgpt",
            "cursor",
            "copilot",
            "ollama",
            "gemini",
            "perplexity",
            "codex",
            "midjourney",
        ],
    ),
    (
        "design",
        &[
            "figma",
            "sketch",
            "adobe",
            "canva",
            "pixelmator",
            "zeplin",
            "framer",
            "blender",
        ],
    ),
    (
        "im",
        &[
            "wechat", "tencent", "qq.", ".qq", "telegram", "discord", "slack", "whatsapp",
            "dingtalk", "lark", "feishu", "teams", "zoom", "skype",
        ],
    ),
    (
        "dev",
        &[
            "jetbrains",
            "intellij",
            "pycharm",
            "goland",
            "webstorm",
            "clion",
            "rider",
            "androidstudio",
            "xcode",
            "vscode",
            "visualstudio",
            "sublime",
            "iterm",
            "warp",
            "kitty",
            "docker",
            "npm",
            "yarn",
            "pnpm",
            "gradle",
            "maven",
            "homebrew",
            "cocoapods",
            "electron",
            "rustup",
        ],
    ),
];

/// Section key for one cache dir name; falls back to the generic app bucket.
fn cache_section(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    for (key, needles) in CATEGORY_RULES {
        if needles.iter().any(|n| lower.contains(n)) {
            return key;
        }
    }
    "app_cache"
}

/// Human-ish owner name from a reverse-DNS id: last component, skipping
/// generic tails like "app"/"helper"/"desktop".
fn owner_display(component: &str) -> String {
    const GENERIC: &[&str] = &["app", "helper", "mac", "desktop", "cache", "client"];
    let parts: Vec<&str> = component.split('.').collect();
    let pick = parts
        .iter()
        .rev()
        .find(|p| !GENERIC.contains(&p.to_ascii_lowercase().as_str()) && p.len() > 2)
        .unwrap_or(parts.last().unwrap_or(&""));
    let mut chars = pick.chars();
    match chars.next() {
        Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Caches skipped because their owner is running (or unknowable): surfaced in
/// the preview header so the user knows what closing apps would unlock.
#[derive(Debug, Default, serde::Serialize)]
pub struct BlockedCaches {
    pub owners: Vec<String>,
    pub count: usize,
    pub total_kb: u64,
}

/// Clean plan plus the blocked-cache hint.
pub struct CleanPlanOutput {
    pub plan: DeletionPlan,
    pub blocked: BlockedCaches,
}

/// Build the full clean plan for one machine. System-scope candidates are
/// discovered and labeled but execution gates them on the privileged helper.
pub fn build_plan(
    home: &str,
    probes: &dyn LiveProbes,
    cancel: &CancelFlag,
    mut progress: impl FnMut(&str, usize),
) -> CleanPlanOutput {
    let mut plan = DeletionPlan::default();
    let mut blocked = BlockedCaches::default();
    let whitelist = load_whitelist(home);
    let policy_ctx = PolicyCtx {
        home: home.to_string(),
        uninstall_mode: false,
    };

    // Names under ~/Library/Caches that a dedicated section owns; the generic
    // sweep must not double-collect them.
    let mut sweep_skip: Vec<&str> = Vec::new();
    for (_, _, skip) in BROWSER_TARGETS {
        sweep_skip.extend_from_slice(skip);
    }

    // --- Section: browser caches (process-guarded) ---
    let caches_root = PathBuf::from(home).join("Library/Caches");
    let mut browser_paths: Vec<PathBuf> = Vec::new();
    for (subdir, guard_procs, _) in BROWSER_TARGETS {
        let dir = caches_root.join(subdir);
        if !dir.is_dir() {
            continue;
        }
        // Active or Unknown both deny: only a provably idle browser's cache
        // may be listed.
        if probes.any_process_running(guard_procs) != TriState::Idle {
            continue;
        }
        browser_paths.push(dir);
    }
    let browser = scanutil::parallel_candidates(&browser_paths, "browser", Scope::User, cancel);
    progress("browser", browser.len());
    plan.candidates.extend(browser);

    // --- Section: user app caches (generic sweep with the full filter set) ---
    let mut sweep_paths: Vec<(PathBuf, &'static str)> = Vec::new();
    let mut blocked_paths: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&caches_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if sweep_skip.iter().any(|s| *s == name) {
                continue;
            }
            let path_str = path.to_string_lossy().into_owned();
            // Same eligible-set filters the CLI applies before preview:
            if policy::should_protect_path(&path_str, &policy_ctx) {
                continue;
            }
            if policy::is_path_whitelisted(&path_str, &whitelist.patterns) {
                continue;
            }
            if policy::holds_compiled_model_cache(&path_str) {
                continue;
            }
            // Live reverse-DNS owner guard at plan time (the sink re-checks):
            // Active and Unknown both keep the cache out of the preview.
            if name.contains('.')
                && !name.starts_with('.')
                && probes.owner_process_state(&name) != TriState::Idle
            {
                // Surface what closing the owner would unlock.
                let display = owner_display(&name);
                if !display.is_empty() && !blocked.owners.contains(&display) {
                    blocked.owners.push(display);
                }
                blocked_paths.push(path);
                continue;
            }
            let section = cache_section(&name);
            sweep_paths.push((path, section));
        }
    }
    // Group by category so parallel_candidates keeps one section label per call.
    for key in ["app_cache", "ai", "design", "im", "dev"] {
        let paths: Vec<PathBuf> = sweep_paths
            .iter()
            .filter(|(_, k)| *k == key)
            .map(|(p, _)| p.clone())
            .collect();
        if paths.is_empty() {
            continue;
        }
        let group = scanutil::parallel_candidates(&paths, key, Scope::User, cancel);
        progress(key, group.len());
        plan.candidates.extend(group);
    }

    // Measure the blocked caches (hint only, never candidates).
    blocked.count = blocked_paths.len();
    for path in &blocked_paths {
        if let Ok((mole_core::plan::SizeKb::Known(kb), _)) = scanutil::dir_size_kb(path, cancel) {
            blocked.total_kb += kb;
        }
    }
    blocked.owners.sort();

    // --- Section: user logs (mole's own logs are policy-protected) ---
    let logs_root = PathBuf::from(home).join("Library/Logs");
    let mut log_paths: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&logs_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            let path_str = path.to_string_lossy().into_owned();
            if policy::should_protect_path(&path_str, &policy_ctx)
                || policy::is_path_whitelisted(&path_str, &whitelist.patterns)
            {
                continue;
            }
            log_paths.push(path);
        }
    }
    let logs = scanutil::parallel_candidates(&log_paths, "logs", Scope::User, cancel);
    progress("logs", logs.len());
    plan.candidates.extend(logs);

    // --- Section: developer caches ---
    let mut dev_paths: Vec<PathBuf> = Vec::new();
    let npm_cache = PathBuf::from(home).join(".npm/_cacache");
    if npm_cache.is_dir() {
        dev_paths.push(npm_cache);
    }
    let derived = PathBuf::from(home).join("Library/Developer/Xcode/DerivedData");
    // DerivedData only when the whole Xcode build tool family is provably idle.
    if derived.is_dir() && probes.any_process_running(XCODE_TOOLING) == TriState::Idle {
        if let Ok(entries) = std::fs::read_dir(&derived) {
            for entry in entries.flatten() {
                dev_paths.push(entry.path());
            }
        }
    }
    let dev = scanutil::parallel_candidates(&dev_paths, "dev", Scope::User, cancel);
    progress("dev", dev.len());
    plan.candidates.extend(dev);

    // --- Section: temp files (user's $TMPDIR and /private/tmp) ---
    let temp_paths = temp_candidates(&temp_roots(), &whitelist.patterns, &policy_ctx);
    let temp = scanutil::parallel_candidates(&temp_paths, "temp", Scope::User, cancel);
    progress("temp", temp.len());
    plan.candidates.extend(temp);

    // --- Section: system caches/logs (admin — helper-gated at execution) ---
    let mut system_paths: Vec<PathBuf> = Vec::new();
    for root in ["/Library/Caches", "/Library/Logs"] {
        if let Ok(entries) = std::fs::read_dir(root) {
            for entry in entries.flatten() {
                let path = entry.path();
                let path_str = path.to_string_lossy().into_owned();
                if policy::should_protect_path(&path_str, &policy_ctx)
                    || policy::is_path_whitelisted(&path_str, &whitelist.patterns)
                {
                    continue;
                }
                system_paths.push(path);
            }
        }
    }
    let system = scanutil::parallel_candidates(&system_paths, "system", Scope::System, cancel);
    progress("system", system.len());
    plan.candidates.extend(system);

    CleanPlanOutput { plan, blocked }
}

/// Temp entries younger than this stay: macOS itself only reaps the per-user
/// temp dir after 3 days without access, so anything newer may be live.
const TEMP_MIN_AGE: std::time::Duration = std::time::Duration::from_secs(3 * 86_400);

/// The per-user Darwin temp dir (what $TMPDIR points to) plus /private/tmp.
/// confstr is used instead of $TMPDIR because the app bundle's environment
/// may not carry the variable.
fn temp_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut buf = vec![0u8; 1024];
    // SAFETY: buf outlives the call and its length is passed exactly.
    let n = unsafe {
        libc::confstr(
            libc::_CS_DARWIN_USER_TEMP_DIR,
            buf.as_mut_ptr() as *mut libc::c_char,
            buf.len(),
        )
    };
    if n > 0 && (n as usize) <= buf.len() {
        let s = String::from_utf8_lossy(&buf[..n as usize - 1]).into_owned();
        roots.push(PathBuf::from(s.trim_end_matches('/')));
    } else if let Ok(t) = std::env::var("TMPDIR") {
        roots.push(PathBuf::from(t.trim_end_matches('/')));
    }
    roots.push(PathBuf::from("/private/tmp"));
    roots
}

/// Eligible temp entries: owned by this user, untouched for TEMP_MIN_AGE,
/// not a socket, not an Apple/launchd runtime entry, not protected or
/// whitelisted. Everything else is somebody's live scratch space.
fn temp_candidates(roots: &[PathBuf], whitelist: &[String], ctx: &PolicyCtx) -> Vec<PathBuf> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    // SAFETY: getuid has no preconditions.
    let uid = unsafe { libc::getuid() };
    let now = std::time::SystemTime::now();
    let mut out = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name.starts_with("com.apple.") {
                continue;
            }
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.uid() != uid || meta.file_type().is_socket() {
                continue;
            }
            let fresh = meta
                .modified()
                .ok()
                .and_then(|m| now.duration_since(m).ok())
                .is_none_or(|age| age < TEMP_MIN_AGE);
            if fresh {
                continue;
            }
            let path_str = path.to_string_lossy().into_owned();
            if policy::should_protect_path(&path_str, ctx)
                || policy::is_path_whitelisted(&path_str, whitelist)
                || policy::is_endpoint_security_cache_path(&path_str)
            {
                continue;
            }
            out.push(path);
        }
    }
    out
}

/// Preview-parity check used by tests: no candidate in a built plan may be
/// protected or whitelisted (the filters must have run at plan time).
#[cfg(test)]
pub fn assert_plan_is_eligible(plan: &DeletionPlan, home: &str) {
    let whitelist = load_whitelist(home);
    let ctx = PolicyCtx {
        home: home.to_string(),
        uninstall_mode: false,
    };
    for c in &plan.candidates {
        assert!(
            !policy::should_protect_path(&c.path, &ctx),
            "protected path leaked into plan: {}",
            c.path
        );
        assert!(
            !policy::is_path_whitelisted(&c.path, &whitelist.patterns),
            "whitelisted path leaked into plan: {}",
            c.path
        );
    }
}

/// True when the given path exists and is a directory (helper for callers
/// composing extra sections).
pub fn dir_exists(path: &Path) -> bool {
    path.is_dir()
}

#[cfg(test)]
mod real_machine {
    use super::*;
    use mole_core::probes::SystemProbes;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    /// Read-only smoke on the REAL home: build a clean plan with live probes
    /// and re-assert eligibility. Never deletes. Run explicitly with
    /// `cargo test -p mole-ops real_home -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_home_plan_is_eligible() {
        let home = std::env::var("HOME").unwrap();
        let probes = SystemProbes::new();
        let cancel = Arc::new(AtomicBool::new(false));
        let out = build_plan(&home, &probes, &cancel, |section, count| {
            eprintln!("{section}: {count} candidates");
        });
        eprintln!(
            "total: {} candidates, {} KB known; blocked: {} owners / {} KB",
            out.plan.candidates.len(),
            out.plan.known_total_kb(),
            out.blocked.owners.len(),
            out.blocked.total_kb
        );
        crate::clean::assert_plan_is_eligible(&out.plan, &home);
    }
}

#[cfg(test)]
mod tests {
    /// Old, user-owned, non-Apple temp entries are eligible; fresh, hidden
    /// and com.apple.* entries are not.
    #[test]
    fn temp_candidates_skip_fresh_and_runtime_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(10 * 86_400);
        for name in ["old-scratch", "com.apple.launchd.xyz", ".hidden-old"] {
            let p = root.join(name);
            std::fs::write(&p, b"x").unwrap();
            std::fs::File::options()
                .write(true)
                .open(&p)
                .unwrap()
                .set_modified(old)
                .unwrap();
        }
        std::fs::write(root.join("fresh"), b"x").unwrap();
        let ctx = super::PolicyCtx {
            home: root.to_string_lossy().into_owned(),
            uninstall_mode: false,
        };
        let got = super::temp_candidates(std::slice::from_ref(&root), &[], &ctx);
        let names: Vec<String> = got
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["old-scratch".to_string()]);
    }

    use super::*;
    use mole_core::probes::StubProbes;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    /// Build a fake home with caches; verify filters and guards shape the plan.
    #[test]
    fn plan_filters_protected_whitelisted_and_live() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_string_lossy().into_owned();
        let caches = tmp.path().join("Library/Caches");
        std::fs::create_dir_all(caches.join("com.idle.app")).unwrap();
        std::fs::create_dir_all(caches.join("com.live.app")).unwrap();
        std::fs::create_dir_all(caches.join("plaindir")).unwrap();
        std::fs::create_dir_all(caches.join("keepme")).unwrap();
        std::fs::create_dir_all(caches.join("com.apple.FontRegistry.cache")).unwrap();
        std::fs::create_dir_all(tmp.path().join("Library/Logs/SomeApp")).unwrap();
        let config_dir = tmp
            .path()
            .join(".config")
            .join(mole_core::brand::CONFIG_DIR);
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(
            config_dir.join("whitelist"),
            format!("{}\n", caches.join("keepme").display()),
        )
        .unwrap();

        // Owner probe: everything idle except... StubProbes is uniform, so run
        // twice — once idle (com.live.app included), once unknown (excluded).
        let cancel = Arc::new(AtomicBool::new(false));

        let idle = StubProbes::idle();
        let out = build_plan(&home, &idle, &cancel, |_, _| {});
        let plan = &out.plan;
        let paths: Vec<&str> = plan.candidates.iter().map(|c| c.path.as_str()).collect();
        assert!(paths.iter().any(|p| p.ends_with("com.idle.app")));
        assert!(paths.iter().any(|p| p.ends_with("plaindir")));
        assert!(paths.iter().any(|p| p.ends_with("Logs/SomeApp")));
        assert!(
            !paths.iter().any(|p| p.ends_with("keepme")),
            "whitelist filter"
        );
        assert_plan_is_eligible(&plan, &home);

        let unknown = StubProbes {
            owner_state: mole_core::probes::TriState::Unknown,
            sqlite_state: mole_core::probes::TriState::Idle,
        };
        let out2 = build_plan(&home, &unknown, &cancel, |_, _| {});
        let paths2: Vec<&str> = out2
            .plan
            .candidates
            .iter()
            .map(|c| c.path.as_str())
            .collect();
        // Unknown owner state DENIES: reverse-DNS caches drop out of the plan,
        // plain-named dirs (no owner concept) stay.
        assert!(!paths2.iter().any(|p| p.ends_with("com.idle.app")));
        assert!(paths2.iter().any(|p| p.ends_with("plaindir")));
        assert!(out2.blocked.count >= 2, "blocked caches must be surfaced");
        assert!(!out2.blocked.owners.is_empty());
    }
}
