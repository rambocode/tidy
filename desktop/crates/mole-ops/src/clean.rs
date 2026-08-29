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
