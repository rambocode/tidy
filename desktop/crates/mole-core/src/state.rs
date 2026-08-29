// Shared user state: the SAME files the CLI reads and writes
// (~/.config/mole/whitelist, purge_paths), so both surfaces stay consistent.
// The whitelist grammar is a logic port of load_mole_whitelist
// (lib/core/base.sh:343-426) — it cannot be code-generated, so a parity test
// against bash guards it instead.

use std::fs;
use std::path::PathBuf;

/// Sentinel whitelist token that opts Finder metadata sweeps out of cleanup.
pub const FINDER_METADATA_SENTINEL: &str = "FINDER_METADATA";

/// Convenience defaults used only when the user has NO whitelist file
/// (replaceable, unlike the safety set). Port of DEFAULT_WHITELIST_PATTERNS.
fn default_whitelist_patterns(home: &str) -> Vec<String> {
    [
        "/Library/Caches/ms-playwright*",
        "/.gradle/caches/*",
        "/.gradle/daemon/*",
        "/.ollama/models/*",
        "/Library/Caches/com.nssurge.surge-mac/*",
        "/Library/Application Support/com.nssurge.surge-mac/*",
        "/Library/Caches/org.R-project.R/R/renv/*",
        "/Library/Caches/JetBrains*",
        "/Library/Caches/com.jetbrains.toolbox*",
        "/Library/Caches/tealdeer/tldr-pages",
        "/Library/Application Support/JetBrains*",
        "/Library/Caches/com.apple.finder",
        "/Library/Mobile Documents*",
    ]
    .iter()
    .map(|suffix| format!("{home}{suffix}"))
    .chain(std::iter::once(FINDER_METADATA_SENTINEL.to_string()))
    .collect()
}

/// Hard safety patterns merged unconditionally even over a user's custom file
/// (removing these breaks search/fonts/iCloud, not just a rebuild). Port of
/// SAFETY_WHITELIST_PATTERNS.
fn safety_whitelist_patterns(home: &str) -> Vec<String> {
    std::iter::once(FINDER_METADATA_SENTINEL.to_string())
        .chain(
            [
                "/Library/Caches/com.apple.FontRegistry*",
                "/Library/Caches/com.apple.spotlight*",
                "/Library/Caches/com.apple.Spotlight*",
                "/Library/Caches/CloudKit*",
                "/Library/Caches/pypoetry/virtualenvs*",
            ]
            .iter()
            .map(|suffix| format!("{home}{suffix}")),
        )
        .collect()
}

/// Result of loading the whitelist: valid patterns plus per-line warnings.
#[derive(Debug, Default)]
pub struct Whitelist {
    pub patterns: Vec<String>,
    pub warnings: Vec<String>,
}

/// Path of the clean whitelist file for a given home.
pub fn whitelist_file(home: &str) -> PathBuf {
    PathBuf::from(home).join(format!(".config/{}/whitelist", crate::brand::CONFIG_DIR))
}

/// Port of `load_mole_whitelist`: read, validate, expand, dedupe, then merge
/// defaults (file absent) or hard-safety entries (always).
pub fn load_whitelist(home: &str) -> Whitelist {
    let mut wl = Whitelist::default();
    let file = whitelist_file(home);

    if let Ok(content) = fs::read_to_string(&file) {
        for raw in content.lines() {
            let mut line = raw.trim().to_string();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // ~ / $HOME / ${HOME} all expand to the invoking user's home.
            if let Some(rest) = line.strip_prefix('~') {
                line = format!("{home}{rest}");
            }
            line = line.replace("$HOME", home).replace("${HOME}", home);

            if line.contains("..") {
                wl.warnings
                    .push(format!("Path traversal not allowed: {line}"));
                continue;
            }
            if line != FINDER_METADATA_SENTINEL {
                if line.chars().any(|c| c.is_control()) {
                    wl.warnings.push(format!("Invalid path format: {line}"));
                    continue;
                }
                if !line.starts_with('/') {
                    wl.warnings.push(format!("Must be absolute path: {line}"));
                    continue;
                }
            }
            if line.contains("//") {
                wl.warnings.push(format!("Consecutive slashes: {line}"));
                continue;
            }
            // System-path denylist: whitelisting these would mask real damage.
            const DENY: &[&str] = &[
                "/",
                "/System",
                "/System/*",
                "/bin",
                "/bin/*",
                "/sbin",
                "/sbin/*",
                "/usr/bin",
                "/usr/bin/*",
                "/usr/sbin",
                "/usr/sbin/*",
                "/etc",
                "/etc/*",
                "/var/db",
                "/var/db/*",
            ];
            if DENY.iter().any(|g| crate::glob::fnmatch(&line, g)) {
                wl.warnings.push(format!("Protected system path: {line}"));
                continue;
            }
            if !wl.patterns.contains(&line) {
                wl.patterns.push(line);
            }
        }
    } else {
        wl.patterns = default_whitelist_patterns(home);
    }

    // Expand a leading ~ once more (defaults may carry it), then merge the
    // hard-safety set for every command that loads this shared policy.
    wl.patterns = wl
        .patterns
        .iter()
        .map(|p| {
            if let Some(rest) = p.strip_prefix('~') {
                format!("{home}{rest}")
            } else {
                p.clone()
            }
        })
        .collect();
    for safety in safety_whitelist_patterns(home) {
        if !wl.patterns.contains(&safety) {
            wl.patterns.push(safety);
        }
    }
    wl
}

/// Write the whitelist file in the CLI's format: header comment + patterns.
pub fn save_whitelist(home: &str, patterns: &[String]) -> std::io::Result<()> {
    let file = whitelist_file(home);
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut out = String::from("# Mole cleanup whitelist\n# One absolute path or glob per line; lines starting with # are comments.\n\n");
    let mut seen: Vec<&String> = Vec::new();
    for p in patterns {
        if !seen.contains(&p) {
            out.push_str(p);
            out.push('\n');
            seen.push(p);
        }
    }
    fs::write(file, out)
}

/// Path of the purge scan-directories config for a given home.
pub fn purge_paths_file(home: &str) -> PathBuf {
    std::env::var("PURGE_PATHS_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(home).join(format!(".config/{}/purge_paths", crate::brand::CONFIG_DIR))
        })
}

/// Read purge scan directories: one path per line, ~ expansion, # comments;
/// an empty/missing file means defaults (returned as None).
pub fn load_purge_paths(home: &str) -> Option<Vec<String>> {
    let content = fs::read_to_string(purge_paths_file(home)).ok()?;
    let paths: Vec<String> = content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            if let Some(rest) = l.strip_prefix('~') {
                format!("{home}{rest}")
            } else {
                l.to_string()
            }
        })
        .collect();
    if paths.is_empty() {
        None
    } else {
        Some(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a temp home with a whitelist file and load it.
    fn load_with(content: &str) -> (tempfile::TempDir, Whitelist) {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_string_lossy().into_owned();
        let dir = tmp
            .path()
            .join(format!(".config/{}", crate::brand::CONFIG_DIR));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("whitelist"), content).unwrap();
        let wl = load_whitelist(&home);
        (tmp, wl)
    }

    #[test]
    fn grammar_validation_matches_shell() {
        let (tmp, wl) = load_with(
            "# comment\n\n~/Library/Caches/keep\n$HOME/other\nrelative/path\n/has//double\n/etc\n/ok/path\n/ok/path\n",
        );
        let home = tmp.path().to_string_lossy().into_owned();
        assert!(wl.patterns.contains(&format!("{home}/Library/Caches/keep")));
        assert!(wl.patterns.contains(&format!("{home}/other")));
        assert!(wl.patterns.contains(&"/ok/path".to_string()));
        // Rejected lines each carry a named warning.
        assert!(wl
            .warnings
            .iter()
            .any(|w| w.contains("Must be absolute path")));
        assert!(wl
            .warnings
            .iter()
            .any(|w| w.contains("Consecutive slashes")));
        assert!(wl
            .warnings
            .iter()
            .any(|w| w.contains("Protected system path")));
        // Dedupe: /ok/path appears once.
        assert_eq!(wl.patterns.iter().filter(|p| *p == "/ok/path").count(), 1);
    }

    #[test]
    fn safety_patterns_merge_over_user_file() {
        let (tmp, wl) = load_with("/just/one/entry\n");
        let home = tmp.path().to_string_lossy().into_owned();
        // A user file replaces convenience defaults but never the hard-safety set.
        assert!(wl
            .patterns
            .contains(&format!("{home}/Library/Caches/com.apple.FontRegistry*")));
        assert!(wl.patterns.contains(&FINDER_METADATA_SENTINEL.to_string()));
        assert!(!wl.patterns.iter().any(|p| p.contains("ms-playwright")));
    }

    #[test]
    fn missing_file_uses_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_string_lossy().into_owned();
        let wl = load_whitelist(&home);
        assert!(wl.patterns.iter().any(|p| p.contains("ms-playwright")));
        assert!(wl.patterns.contains(&FINDER_METADATA_SENTINEL.to_string()));
    }

    #[test]
    fn traversal_rejected_even_for_sentinel_lookalikes() {
        let (_tmp, wl) = load_with("/tmp/../etc\n");
        assert!(wl.warnings.iter().any(|w| w.contains("Path traversal")));
        assert!(!wl.patterns.iter().any(|p| p.contains("..")));
    }
}
