// Protection policy, ported from lib/core/app_protection.sh. Evaluation order
// is contractual: the shell checks uninstallable-Apple-apps BEFORE
// system-critical bundles, and container cache paths skip the pattern sweeps.
// Data lists are generated at build time from lib/core/app_protection_data.sh.

use crate::fsutil;
use crate::glob::{fnmatch, fnmatch_nocase};

/// One row of OFFICIAL_UNINSTALLER_RULES, split at build time.
pub struct OfficialUninstallerRule {
    pub vendor: &'static str,
    pub bundle_prefixes: &'static [&'static str],
    pub name_fragments: &'static [&'static str],
}

/// Build-time generated protection lists (single source of truth: the shell data file).
pub mod data {
    include!(concat!(env!("OUT_DIR"), "/app_protection_data.rs"));
}

/// Inputs `should_protect_path` needs beyond the path itself.
pub struct PolicyCtx {
    /// The invoking user's home directory (whitelist/Codex arms are anchored to it).
    pub home: String,
    /// MOLE_UNINSTALL_MODE: user explicitly chose to remove this app.
    pub uninstall_mode: bool,
}

/// Port of `bundle_matches_pattern`: bash `[[ id == pattern ]]`, case-sensitive glob.
pub fn bundle_matches_pattern(bundle_id: &str, pattern: &str) -> bool {
    !pattern.is_empty() && fnmatch(bundle_id, pattern)
}

/// Port of `is_critical_system_component`: case-insensitive keyword scan.
pub fn is_critical_system_component(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    let lower = token.to_ascii_lowercase();
    [
        "backgroundtaskmanagement",
        "loginitems",
        "systempreferences",
        "systemsettings",
        "settings",
        "preferences",
        "controlcenter",
        "biometrickit",
        "sfl",
        "tcc",
    ]
    .iter()
    .any(|kw| lower.contains(kw))
}

/// Port of `should_protect_from_uninstall`: Apple-uninstallable wins over critical.
pub fn should_protect_from_uninstall(bundle_id: &str) -> bool {
    if data::APPLE_UNINSTALLABLE_APPS
        .iter()
        .any(|p| bundle_matches_pattern(bundle_id, p))
    {
        return false;
    }
    data::SYSTEM_CRITICAL_BUNDLES
        .iter()
        .any(|p| bundle_matches_pattern(bundle_id, p))
}

/// Port of `official_uninstaller_vendor`: vendor name when the app must go
/// through its official uninstaller instead of Mole's generic delete path.
pub fn official_uninstaller_vendor(
    bundle_id: &str,
    display_name: &str,
    app_path: &str,
) -> Option<&'static str> {
    let bundle = bundle_id.to_ascii_lowercase();
    let name = display_name.to_ascii_lowercase();
    let base = app_path
        .rsplit('/')
        .next()
        .unwrap_or("")
        .trim_end_matches(".app")
        .to_ascii_lowercase();

    for rule in data::OFFICIAL_UNINSTALLER_RULES {
        if rule
            .bundle_prefixes
            .iter()
            .any(|p| !p.is_empty() && bundle.starts_with(p))
        {
            return Some(rule.vendor);
        }
        if rule
            .name_fragments
            .iter()
            .any(|f| !f.is_empty() && (name.contains(f) || base.contains(f)))
        {
            return Some(rule.vendor);
        }
    }
    None
}

/// Case-arm helper: does the id match any glob in the list (case-sensitive)?
fn any_glob(id: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|p| fnmatch(id, p))
}

/// Port of `should_protect_data`: is this bundle's data protected during cleanup?
/// The arm order and the early-return DATA_PROTECTED_BUNDLES arm are contractual.
pub fn should_protect_data(bundle_id: &str) -> bool {
    // Arms are the shell `case` ladder, in source order.
    if any_glob(
        bundle_id,
        &[
            "com.apple.*",
            "loginwindow",
            "dock",
            "systempreferences",
            "finder",
            "safari",
        ],
    ) || any_glob(bundle_id, &["org.cups.*"])
        || any_glob(
            bundle_id,
            &[
                "backgroundtaskmanagement*",
                "keychain*",
                "security*",
                "bluetooth*",
                "wifi*",
                "network*",
                "tcc",
            ],
        )
        || any_glob(
            bundle_id,
            &[
                "notification*",
                "accessibility*",
                "universalaccess*",
                "HIToolbox*",
            ],
        )
        || any_glob(
            bundle_id,
            &[
                "*inputmethod*",
                "*InputMethod*",
                "*IME",
                "textinput*",
                "TextInput*",
            ],
        )
        || any_glob(
            bundle_id,
            &[
                "keyboard*",
                "Keyboard*",
                "inputsource*",
                "InputSource*",
                "keylayout*",
                "KeyLayout*",
            ],
        )
        || any_glob(
            bundle_id,
            &[
                "GlobalPreferences",
                ".GlobalPreferences",
                "org.pqrs.Karabiner*",
            ],
        )
        || any_glob(
            bundle_id,
            &[
                "com.1password.*",
                "com.agilebits.*",
                "com.lastpass.*",
                "com.dashlane.*",
                "com.bitwarden.*",
            ],
        )
        || any_glob(
            bundle_id,
            &[
                "com.jetbrains.*",
                "JetBrains*",
                "com.microsoft.*",
                "com.visualstudio.*",
            ],
        )
        || any_glob(
            bundle_id,
            &[
                "com.sublimetext.*",
                "com.sublimehq.*",
                "Cursor",
                "Claude",
                "ChatGPT",
                "com.openai.codex",
                "Codex",
                "codex-runtimes",
                "Ollama",
            ],
        )
        || any_glob(bundle_id, &["com.clash.app"])
        || any_glob(
            bundle_id,
            &[
                "com.nssurge.*",
                "com.v2ray.*",
                "com.clash.*",
                "ClashX*",
                "Surge*",
                "Shadowrocket*",
                "Quantumult*",
            ],
        )
        || any_glob(
            bundle_id,
            &[
                "clash-*",
                "Clash-*",
                "*-clash",
                "*-Clash",
                "clash.*",
                "Clash.*",
                "clash_*",
                "*clash-verge*",
                "*Clash-Verge*",
                "clashverge*",
                "ClashVerge*",
            ],
        )
        || any_glob(
            bundle_id,
            &["com.docker.*", "com.getpostman.*", "com.insomnia.*"],
        )
    {
        return true;
    }

    // Chinese-vendor arm: check only the detailed list, then stop (shell
    // early-returns here instead of falling through to the fallback sweep).
    if any_glob(
        bundle_id,
        &[
            "com.tencent.*",
            "com.sogou.*",
            "com.baidu.*",
            "com.googlecode.*",
            "im.rime.*",
        ],
    ) {
        return data::DATA_PROTECTED_BUNDLES
            .iter()
            .any(|p| bundle_matches_pattern(bundle_id, p));
    }

    data::DATA_PROTECTED_BUNDLES
        .iter()
        .any(|p| bundle_matches_pattern(bundle_id, p))
}

/// Port of `is_endpoint_security_cache_path`: EDR/MDM sensor state under the
/// per-user Darwin folders; deleting it trips tamper detection.
pub fn is_endpoint_security_cache_path(path: &str) -> bool {
    let in_scope = path.starts_with("/private/var/folders/")
        || path.starts_with("/var/folders/")
        || fsutil::is_within_existing_root(path, "/private/var/folders");
    if !in_scope {
        return false;
    }
    let lower = path.to_ascii_lowercase();
    data::ENDPOINT_SECURITY_BUNDLE_PREFIXES
        .iter()
        .any(|prefix| lower.contains(&prefix.to_ascii_lowercase()))
}

/// Port of `is_orbstack_runtime_path`: live container filesystem images.
pub fn is_orbstack_runtime_path(path: &str) -> bool {
    [
        "*/Library/Group Containers/*dev.orbstack",
        "*/Library/Group Containers/*dev.orbstack/*",
        "*/.orbstack",
        "*/.orbstack/*",
    ]
    .iter()
    .any(|p| fnmatch_nocase(path, p))
}

/// Port of `holds_compiled_model_cache`: E5RT compiled model bundles must stay.
pub fn holds_compiled_model_cache(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    let trimmed = path.trim_end_matches('/');
    if trimmed.ends_with("/com.apple.e5rt.e5bundlecache") {
        return true;
    }
    std::path::Path::new(trimmed)
        .join("com.apple.e5rt.e5bundlecache")
        .is_dir()
}

/// Port of `should_protect_path`: the central "is this path protected from
/// deletion" policy. Step numbering follows the shell source.
pub fn should_protect_path(path: &str, ctx: &PolicyCtx) -> bool {
    if path.is_empty() {
        return false;
    }

    let mut container_cache_path = false;

    // Codex Desktop rebuildable Chromium cache leaves (children only).
    let home = ctx.home.trim_end_matches('/');
    let known_rebuildable_cache_path = [
        "/Library/Caches/Codex/Default/Cache/",
        "/Library/Caches/Codex/Default/Code Cache/",
        "/Library/Caches/Codex/Default/Partitions/codex-browser-app/Cache/",
        "/Library/Caches/Codex/Default/Partitions/codex-browser-app/Code Cache/",
        "/Library/Caches/Codex/codex-browser-app/Cache/",
        "/Library/Caches/Codex/codex-browser-app/Code Cache/",
    ]
    .iter()
    .any(|suffix| {
        path.strip_prefix(home)
            .is_some_and(|rest| rest.starts_with(suffix) && rest.len() > suffix.len())
    });

    if is_orbstack_runtime_path(path) {
        return true;
    }

    // 1. Keyword-based matching for system components.
    if any_glob(
        path,
        &[
            "*[Ss]ystem[Ss]ettings*",
            "*[Ss]ystem[Pp]references*",
            "*[Cc]ontrol[Cc]enter*",
            "*com.apple.[Ss]ettings*",
            "*com.apple.[Ss]ETTINGS*",
            "*com.apple.[Nn]otes*",
            "*com.apple.[Nn]OTES*",
        ],
    ) {
        return true;
    }

    // 2. Caches critical for system UI rendering.
    if any_glob(
        path,
        &[
            "*com.apple.systempreferences.cache*",
            "*com.apple.Settings.cache*",
            "*com.apple.controlcenter.cache*",
            "*com.apple.finder.cache*",
            "*com.apple.dock.cache*",
            "*/Library/Containers/com.apple.Settings*",
            "*/Library/Containers/com.apple.SystemSettings*",
            "*/Library/Containers/com.apple.controlcenter*",
            "*/Library/Group Containers/com.apple.systempreferences*",
            "*/Library/Group Containers/com.apple.Settings*",
            "*/Library/Group Containers/*dev.orbstack",
            "*/Library/Group Containers/*dev.orbstack/*",
            "*/.orbstack",
            "*/.orbstack/*",
            "*/com.apple.sharedfilelist/*com.apple.Settings*",
            "*/com.apple.sharedfilelist/*com.apple.SystemSettings*",
            "*/com.apple.sharedfilelist/*systempreferences*",
        ],
    ) {
        return true;
    }

    // 3. Bundle id extracted from sandbox container paths.
    if let Some(bundle_id) = container_bundle_id(path) {
        // Cache/tmp inside a container is regenerable by definition; let the
        // sweeps through instead of blocking on the blanket com.apple.* match.
        if path.contains("/Data/Library/Caches/") || path.contains("/Data/tmp/") {
            container_cache_path = true;
        } else if !ctx.uninstall_mode && should_protect_data(&bundle_id) {
            return true;
        }
    }

    // 4. Hardcoded critical patterns.
    if any_glob(
        path,
        &[
            "*com.apple.Settings*",
            "*com.apple.SystemSettings*",
            "*com.apple.controlcenter*",
            "*com.apple.finder*",
            "*com.apple.dock*",
        ],
    ) {
        return true;
    }

    // 4b. Endpoint security / EDR agent caches.
    if is_endpoint_security_cache_path(path) {
        return true;
    }

    // 5. Critical preference files and user data (high-risk denylist overlay).
    if any_glob(
        path,
        &[
            "*/Library/Preferences/com.apple.dock.plist",
            "*/Library/Preferences/com.apple.finder.plist",
            // Mole's shared audit trail stays protected. Keep the exact legacy
            // Cleaner paths during the desktop placeholder-name migration so
            // an older installation's logs cannot become cleanup candidates.
            "*/Library/Logs/mole",
            "*/Library/Logs/mole/",
            "*/Library/Logs/mole/*",
            "*/Library/Logs/Cleaner",
            "*/Library/Logs/Cleaner/",
            "*/Library/Logs/Cleaner/*",
            "*/Library/Application Support/Codex",
            "*/Library/Application Support/Codex/*",
            "*/Library/Logs/com.openai.codex",
            "*/Library/Logs/com.openai.codex/*",
            "*/.codex/sessions",
            "*/.codex/sessions/*",
            "*/.codex/auth.json",
            "*/.codex/history.jsonl",
            "*/.codex/state_*.sqlite",
            "*/.codex/logs_*.sqlite",
            "*/.codex/session_index.jsonl",
            "*/.codex/cache/session_index.jsonl",
            "*/.codex/cache/codex_app_directory",
            "*/.codex/cache/codex_app_directory/*",
            "*/ByHost/com.apple.bluetooth.*",
            "*/ByHost/com.apple.wifi.*",
            "*/Library/Preferences/com.apple.networkextension*.plist",
            "*/Library/Mobile Documents*",
            "*/Mobile Documents*",
            "*/Library/Accounts",
            "*/Library/Accounts/*",
            "*/Library/Keychains",
            "*/Library/Keychains/*",
            "*/Library/Mail",
            "*/Library/Mail/*",
            "*/Library/Calendars",
            "*/Library/Contacts",
            "*/Library/Contacts/*",
            "/Library/Audio/Plug-Ins/Components",
            "/Library/Audio/Plug-Ins/Components/*",
            "/Library/Audio/Plug-Ins/VST",
            "/Library/Audio/Plug-Ins/VST/*",
            "/Library/Audio/Plug-Ins/VST3",
            "/Library/Audio/Plug-Ins/VST3/*",
            "/Library/Application Support/iZotope",
            "/Library/Application Support/iZotope/*",
            "*/Library/Application Support/iZotope",
            "*/Library/Application Support/iZotope/*",
            "/Library/Application Support/LaserSoft Imaging",
            "/Library/Application Support/LaserSoft Imaging/*",
            "*/Library/Preferences/com.native-instruments*",
            "*/Library/Preferences/com.avid.mediacomposer*.plist",
            "*/Library/Preferences/com.fabfilter.*.[0-9].plist",
            "*/Library/Preferences/com.fabfilter.*.[0-9][0-9].plist",
            "*/Library/Preferences/com.paceap.*.plist",
            "/private/var/folders/*/C/com.native-instruments*",
            "/private/var/folders/*/C/com.avid.mediacomposer*",
            "/private/var/folders/*/C/com.paceap.eden.iLokLicenseManager*",
            "*/Library/Caches/ms-playwright",
            "*/Library/Caches/ms-playwright/*",
            "*/Library/Caches/app.cotypist.Cotypist",
            "*/Library/Caches/app.cotypist.Cotypist/*",
            "*/Library/Caches/com.displaylink.DisplayLinkUserAgent",
            "*/Library/Caches/com.displaylink.DisplayLinkUserAgent/*",
            "*/Library/Caches/com.lasersoft-imaging.SilverFast9",
            "*/Library/Caches/com.lasersoft-imaging.SilverFast9/*",
            "*/Library/Caches/com.lasersoft-imaging.SilverFast-9-Installer",
            "*/Library/Caches/com.lasersoft-imaging.SilverFast-9-Installer/*",
            "*/Library/Caches/Adobe *",
            "*/Library/Caches/* Adobe*",
            "*/Library/Caches/com.apple.containermanagerd",
            "*/Library/Caches/com.apple.containermanagerd/*",
            "*/Library/Caches/com.apple.homed",
            "*/Library/Caches/com.apple.homed/*",
            "*/Library/Caches/com.apple.ap.adprivacyd",
            "*/Library/Caches/com.apple.ap.adprivacyd/*",
            "*/Library/Caches/FamilyCircle",
            "*/Library/Caches/FamilyCircle/*",
            "*/Library/Caches/com.apple.HomeKit",
            "*/Library/Caches/com.apple.HomeKit/*",
            "*/Library/Caches/com.apple.WorkflowKit.BackgroundShortcutRunner.ShortcutsSandboxCache",
            "*/Library/Caches/com.apple.WorkflowKit.BackgroundShortcutRunner.ShortcutsSandboxCache/*",
            "*/Library/Caches/com.apple.siriactionsd.ShortcutsSandboxCache",
            "*/Library/Caches/com.apple.siriactionsd.ShortcutsSandboxCache/*",
            "*/Library/Application Support/com.apple.idleassetsd",
            "*/Library/Application Support/com.apple.idleassetsd/*",
            "*/Library/Application Support/com.apple.wallpaper",
            "*/Library/Application Support/com.apple.wallpaper/*",
            "*com.apple.coreaudio*",
            "*com.apple.audio.*",
            "*coreaudiod*",
        ],
    ) {
        return true;
    }

    // 6. Full-path sweep against the protected bundle patterns, skipped for
    // container cache/tmp and known-rebuildable Codex leaves (already vetted).
    if !container_cache_path && !known_rebuildable_cache_path {
        if ctx.uninstall_mode {
            if data::APPLE_UNINSTALLABLE_APPS
                .iter()
                .any(|p| bundle_matches_pattern(path, p))
            {
                return false;
            }
            if data::SYSTEM_CRITICAL_BUNDLES
                .iter()
                .any(|p| bundle_matches_pattern(path, p))
            {
                return true;
            }
        } else if data::SYSTEM_CRITICAL_BUNDLES
            .iter()
            .chain(data::DATA_PROTECTED_BUNDLES.iter())
            .any(|p| bundle_matches_pattern(path, p))
        {
            return true;
        }

        // 7. Filename-level data protection (cleanup mode only).
        if !ctx.uninstall_mode {
            let filename = path.rsplit('/').next().unwrap_or(path);
            if should_protect_data(filename) {
                return true;
            }
        }
    }

    false
}

/// Extract the bundle id component from `.../Library/Containers/<id>/...` or
/// `.../Library/Group Containers/<id>/...` (shell regex port).
fn container_bundle_id(path: &str) -> Option<String> {
    for marker in ["/Library/Containers/", "/Library/Group Containers/"] {
        if let Some(idx) = path.find(marker) {
            let rest = &path[idx + marker.len()..];
            let id = rest.split('/').next().unwrap_or("");
            if !id.is_empty() {
                return Some(id.to_string());
            }
        }
    }
    None
}

/// Port of `is_path_whitelisted`: exact / glob / parent-of / child-of matching
/// against the user's normalized whitelist patterns.
pub fn is_path_whitelisted(target_path: &str, patterns: &[String]) -> bool {
    if target_path.is_empty() || patterns.is_empty() {
        return false;
    }
    let normalized_target = collapse_slashes(target_path.trim_end_matches('/'));

    for pattern in patterns {
        let check_pattern = collapse_slashes(pattern.trim_end_matches('/'));
        let has_glob = check_pattern.contains(['*', '?', '[']);

        if normalized_target == check_pattern || fnmatch(&normalized_target, &check_pattern) {
            return true;
        }
        // Target is a parent of a whitelisted path: keep it to preserve children.
        if check_pattern.starts_with(&format!("{normalized_target}/")) {
            return true;
        }
        // Target is a child of a whitelisted directory (literal patterns only).
        if !has_glob && normalized_target.starts_with(&format!("{check_pattern}/")) {
            return true;
        }
    }
    false
}

/// Collapse `//` runs into `/` (whitelist entries and glob concatenations, #724).
fn collapse_slashes(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut prev_slash = false;
    for ch in path.chars() {
        if ch == '/' {
            if !prev_slash {
                out.push(ch);
            }
            prev_slash = true;
        } else {
            out.push(ch);
            prev_slash = false;
        }
    }
    if out.is_empty() {
        out.push('/');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> PolicyCtx {
        PolicyCtx {
            home: "/Users/tester".into(),
            uninstall_mode: false,
        }
    }

    #[test]
    fn generated_data_has_anchors() {
        assert!(data::SYSTEM_CRITICAL_BUNDLES.contains(&"com.apple.finder"));
        assert!(data::SYSTEM_CRITICAL_BUNDLES.len() > 40);
        assert!(!data::DATA_PROTECTED_BUNDLES.is_empty());
        assert!(!data::OFFICIAL_UNINSTALLER_RULES.is_empty());
        assert_eq!(data::DATA_SHA256.len(), 64);
    }

    /// Anti-drift gate: the vendored data file must stay byte-identical to
    /// the CLI's source of truth. Vendoring keeps the desktop build
    /// self-contained, but without this check the two copies could fork
    /// silently — CI triggers on lib/core/app_protection_data.sh changes and
    /// this test is what actually turns a stale copy into a red build.
    /// Skipped outside the repo tree (self-contained source builds).
    #[test]
    fn vendored_protection_data_matches_cli_source() {
        let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let cli = manifest.join("../../../lib/core/app_protection_data.sh");
        if !cli.exists() {
            return;
        }
        let vendored = std::fs::read(manifest.join("data/app_protection_data.sh")).unwrap();
        let cli_bytes = std::fs::read(cli).unwrap();
        assert_eq!(
            vendored, cli_bytes,
            "vendored protection data drifted from lib/core/app_protection_data.sh — \
             copy the CLI file over crates/mole-core/data/app_protection_data.sh"
        );
    }

    #[test]
    fn protects_system_settings_everywhere() {
        let c = ctx();
        assert!(should_protect_path(
            "/Users/tester/Library/Caches/com.apple.SystemSettings",
            &c
        ));
        assert!(should_protect_path(
            "/Users/tester/Library/Preferences/com.apple.dock.plist",
            &c
        ));
        assert!(should_protect_path(
            "/Users/tester/Library/Logs/mole/operations.log",
            &c
        ));
        assert!(should_protect_path(
            "/Users/tester/Library/Logs/Cleaner/app.log",
            &c
        ));
    }

    #[test]
    fn container_cache_children_pass_through() {
        let c = ctx();
        // Container cache/tmp under a data-protected bundle stays deletable.
        assert!(!should_protect_path(
            "/Users/tester/Library/Containers/com.docker.docker/Data/Library/Caches/blob",
            &c
        ));
        // The container root of a protected bundle does not.
        assert!(should_protect_path(
            "/Users/tester/Library/Containers/com.docker.docker/Data/Documents",
            &c
        ));
    }

    #[test]
    fn uninstall_mode_lifts_data_protection() {
        let mut c = ctx();
        c.uninstall_mode = true;
        assert!(!should_protect_path(
            "/Users/tester/Library/Containers/com.docker.docker/Data/Documents",
            &c
        ));
        assert!(should_protect_from_uninstall("com.apple.finder"));
        assert!(!should_protect_from_uninstall("com.apple.dt.Xcode"));
    }

    #[test]
    fn official_uninstaller_rules_match() {
        assert_eq!(
            official_uninstaller_vendor("com.crowdstrike.falcon.Agent", "", ""),
            Some("CrowdStrike")
        );
        assert_eq!(
            official_uninstaller_vendor("com.example.app", "Jamf Connect", ""),
            Some("Jamf")
        );
        assert_eq!(
            official_uninstaller_vendor("com.example.app", "Example", ""),
            None
        );
    }

    #[test]
    fn whitelist_matching_shapes() {
        let patterns = vec![
            "/Users/t/Library/Caches/keepme".to_string(),
            "/Users/t/Library/Caches/glob*".to_string(),
        ];
        assert!(is_path_whitelisted(
            "/Users/t/Library/Caches/keepme",
            &patterns
        ));
        assert!(is_path_whitelisted(
            "/Users/t/Library/Caches/keepme/child",
            &patterns
        ));
        // Parent of a whitelisted path is protected to preserve the child.
        assert!(is_path_whitelisted("/Users/t/Library/Caches", &patterns));
        assert!(is_path_whitelisted(
            "/Users/t/Library/Caches/globX",
            &patterns
        ));
        assert!(!is_path_whitelisted(
            "/Users/t/Library/Caches/other",
            &patterns
        ));
        // Double slashes collapse before matching (#724).
        assert!(is_path_whitelisted(
            "/Users/t/Library//Caches/keepme",
            &patterns
        ));
    }
}
