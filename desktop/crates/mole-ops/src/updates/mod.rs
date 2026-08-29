//! Software-update domain aligned with Mole.app's source model.
//!
//! Discovery recognizes Homebrew casks/formulae, Mac App Store receipts,
//! Sparkle feeds, Electron updater metadata, and safe external fallbacks.
//! Homebrew remains preview-only (Mole is not a package manager); App Store
//! and app-owned updaters are delegated to their original trusted surface.

mod channels;
mod command;
mod execute;
mod homebrew;
mod persistence;

use crate::uninstall::AppInfo;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::{SystemTime, UNIX_EPOCH};

pub use execute::run_updates;
pub use persistence::{load_catalog, save_catalog, set_ignored};

/// One app or Homebrew package update. Deserialize exists only for the
/// persisted display cache; authority fields are never serialized, so they
/// always come back as None and a loaded row can never drive an action.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppUpdate {
    /// Stable source-qualified identity, also used for ignore state.
    pub id: String,
    /// `app` or `package`.
    pub kind: String,
    pub name: String,
    #[serde(skip_serializing, default)]
    pub(crate) bundle_id: Option<String>,
    #[serde(skip_serializing, default)]
    pub(crate) app_path: Option<String>,
    pub installed: String,
    pub latest: String,
    /// `homebrew`, `app_store`, `sparkle`, `electron`, or `website`.
    pub source: String,
    /// `terminal`, `open_app_store`, `open_app`, or `open_website`.
    pub action: String,
    pub release_notes: Option<String>,
    /// Read-only terminal command for package-manager owned updates.
    pub command_hint: Option<String>,
    pub ignored: bool,
    /// Exact HTTPS or macappstore destination for delegated updates.
    #[serde(skip_serializing, default)]
    pub(crate) external_url: Option<String>,
}

/// Read-only app whose native channel was checked and found current.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpToDateApp {
    pub name: String,
    pub version: String,
    pub source: String,
}

/// Full response for the Updates view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateCatalog {
    pub updates: Vec<AppUpdate>,
    pub up_to_date: Vec<UpToDateApp>,
    /// Per-source diagnostic codes. A failed source never reads as up to date.
    pub warnings: Vec<String>,
    pub checked_at: u64,
}

/// One update action result.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct UpdateResult {
    pub id: String,
    /// updated / external / still_pending / skipped / failed / cancelled.
    pub outcome: String,
    pub cause: String,
    pub message: String,
}

/// Scan all supported update channels. Each network-bearing native channel is
/// bounded; failures are preserved in `warnings`, never converted to an empty
/// "all current" result.
pub fn scan(home: &str, apps: &[AppInfo], cancelled: &AtomicBool) -> UpdateCatalog {
    let ignored = persistence::load_ignored(home);
    let brew = homebrew::scan(home, apps, cancelled);
    let managed_paths: HashSet<String> = brew.managed_app_paths.clone();
    let native_apps: Vec<AppInfo> = apps
        .iter()
        .filter(|app| !managed_paths.contains(&app.path))
        .cloned()
        .collect();
    let native = channels::scan(&native_apps, cancelled);

    let mut updates = brew.updates;
    updates.extend(native.updates);
    for update in &mut updates {
        update.ignored = ignored.contains(&update.id);
    }
    updates.sort_by(|left, right| {
        left.ignored
            .cmp(&right.ignored)
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });

    let mut warnings = brew.warnings;
    warnings.extend(native.warnings);
    warnings.sort();
    warnings.dedup();
    let mut up_to_date = native.up_to_date;
    up_to_date.sort_by_key(|app| app.name.to_lowercase());
    UpdateCatalog {
        updates,
        up_to_date,
        warnings,
        checked_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    }
}

/// Strict token grammar shared by scan and execution.
pub(crate) fn valid_package_token(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric()
                || (index > 0 && matches!(byte, b'@' | b'+' | b'.' | b'_' | b'-'))
        })
}

/// Read exact app identity again before a delegated action.
pub(crate) fn bundle_identity(path: &Path) -> Option<(String, String)> {
    let value = plist::Value::from_file(path.join("Contents/Info.plist")).ok()?;
    let dictionary = value.as_dictionary()?;
    let bundle_id = dictionary
        .get("CFBundleIdentifier")
        .and_then(plist::Value::as_string)?
        .to_string();
    let version = dictionary
        .get("CFBundleShortVersionString")
        .or_else(|| dictionary.get("CFBundleVersion"))
        .and_then(plist::Value::as_string)
        .unwrap_or_default()
        .to_string();
    Some((bundle_id, version))
}

pub(crate) fn app_info_plist(path: &Path) -> Option<plist::Dictionary> {
    plist::Value::from_file(path.join("Contents/Info.plist"))
        .ok()?
        .into_dictionary()
}

/// Stable opaque id: source authority stays in the backend snapshot rather
/// than being encoded into a webview-controlled token/path/URL.
pub(crate) fn update_id(source_key: &str) -> String {
    let mut hash = 1469598103934665603_u64;
    for byte in source_key.as_bytes() {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(1099511628211);
    }
    format!("update-{hash:016x}")
}

/// Conservative version comparison: numeric components dominate; stable
/// versions outrank prereleases. Ambiguous equal-prefix text does not produce
/// an update.
pub(crate) fn version_is_newer(candidate: &str, installed: &str) -> bool {
    let candidate = candidate.trim().trim_start_matches(['v', 'V']);
    let installed = installed.trim().trim_start_matches(['v', 'V']);
    let candidate_pre = is_prerelease(candidate);
    let installed_pre = is_prerelease(installed);
    let candidate_parts = numeric_parts(candidate);
    let installed_parts = numeric_parts(installed);
    let length = candidate_parts.len().max(installed_parts.len());
    for index in 0..length {
        let left = *candidate_parts.get(index).unwrap_or(&0);
        let right = *installed_parts.get(index).unwrap_or(&0);
        if left != right {
            return left > right;
        }
    }
    installed_pre && !candidate_pre
}

fn numeric_parts(value: &str) -> Vec<u64> {
    let lower = value.to_ascii_lowercase();
    let core_end = prerelease_markers()
        .iter()
        .filter_map(|marker| lower.find(marker))
        .min()
        .unwrap_or(value.len());
    lower[..core_end]
        .split(|character: char| !character.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse().ok())
        .collect()
}

fn is_prerelease(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    prerelease_markers()
        .iter()
        .any(|marker| lower.contains(marker))
}

fn prerelease_markers() -> &'static [&'static str] {
    &[
        "alpha",
        "beta",
        "canary",
        "candidate",
        "dev",
        "nightly",
        "preview",
        "pre",
        "rc",
        "snapshot",
        "testflight",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_compare_is_numeric_and_prerelease_safe() {
        assert!(version_is_newer("2.10.0", "2.9.9"));
        assert!(version_is_newer("v3.0", "2.99"));
        assert!(version_is_newer("2.0", "2.0-rc1"));
        assert!(!version_is_newer("2.0-beta1", "2.0"));
        assert!(!version_is_newer("2.0", "2.0"));
    }

    #[test]
    fn package_tokens_cannot_become_paths_or_flags() {
        for allowed in ["flashspace", "font-fira-code", "openssl@3", "libc++"] {
            assert!(valid_package_token(allowed), "{allowed}");
        }
        for denied in ["", "--formula", "../evil", "a/b", "bad token", ";touch"] {
            assert!(!valid_package_token(denied), "{denied}");
        }
    }

    #[test]
    fn frontend_payload_never_exposes_action_authority() {
        let update = AppUpdate {
            id: "homebrew:cask:demo".into(),
            kind: "app".into(),
            name: "Demo".into(),
            bundle_id: Some("com.demo.app".into()),
            app_path: Some("/Applications/Demo.app".into()),
            installed: "1".into(),
            latest: "2".into(),
            source: "homebrew".into(),
            action: "terminal".into(),
            release_notes: None,
            command_hint: Some("brew upgrade --cask demo".into()),
            ignored: false,
            external_url: Some("https://example.test/update".into()),
        };
        let value = serde_json::to_value(update).unwrap();
        let object = value.as_object().unwrap();
        assert!(!object.contains_key("bundle_id"));
        assert!(!object.contains_key("app_path"));
        assert!(!object.contains_key("external_url"));
        assert_eq!(object["command_hint"], "brew upgrade --cask demo");
    }
}
