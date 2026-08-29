//! Homebrew update discovery using its machine-readable JSON contract and
//! exact Caskroom install receipts. Display-name guesses never authorize an
//! app replacement.

use super::command::{self, Status};
use super::{update_id, valid_package_token, AppUpdate};
use crate::uninstall::AppInfo;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

const BREW_SCAN_TIMEOUT: Duration = Duration::from_secs(90);

pub(crate) struct HomebrewScan {
    pub updates: Vec<AppUpdate>,
    pub managed_app_paths: HashSet<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct OutdatedPayload {
    #[serde(default)]
    formulae: Vec<OutdatedEntry>,
    #[serde(default)]
    casks: Vec<OutdatedEntry>,
}

#[derive(Debug, Deserialize)]
struct OutdatedEntry {
    name: String,
    #[serde(default)]
    installed_versions: Vec<String>,
    current_version: String,
    #[serde(default)]
    pinned: bool,
}

pub(crate) fn scan(home: &str, apps: &[AppInfo], cancelled: &AtomicBool) -> HomebrewScan {
    let Some(brew) = locate_brew() else {
        return HomebrewScan {
            updates: Vec::new(),
            managed_app_paths: HashSet::new(),
            warnings: vec!["homebrew:not_found".into()],
        };
    };
    let prefix = brew
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new("/"));
    let managed_app_paths = caskroom_app_paths(prefix, home);
    let argv = vec![
        brew.to_string_lossy().into_owned(),
        "outdated".into(),
        "--json=v2".into(),
    ];
    let output = command::run(&argv, BREW_SCAN_TIMEOUT, cancelled);
    if !output.success() {
        let cause = match output.status {
            Status::TimedOut => "timeout",
            Status::Cancelled => "cancelled",
            Status::Unavailable => "not_found",
            Status::Failed | Status::Success => "failed",
        };
        return HomebrewScan {
            updates: Vec::new(),
            managed_app_paths,
            warnings: vec![format!("homebrew:{cause}:{}", output.text)],
        };
    }
    let payload: OutdatedPayload = match serde_json::from_str(&output.stdout) {
        Ok(payload) => payload,
        Err(error) => {
            return HomebrewScan {
                updates: Vec::new(),
                managed_app_paths,
                warnings: vec![format!("homebrew:invalid_json:{error}")],
            }
        }
    };
    let app_by_path: HashMap<&str, &AppInfo> =
        apps.iter().map(|app| (app.path.as_str(), app)).collect();
    let mut updates = Vec::new();

    for entry in payload.casks {
        if entry.pinned || !valid_package_token(&entry.name) {
            continue;
        }
        let app_path = cask_app_path(prefix, home, &entry.name);
        let app = app_path
            .as_deref()
            .and_then(|path| app_by_path.get(path).copied());
        let (name, bundle_id) = match (app, app_path.as_deref()) {
            (Some(app), _) => (app.name.clone(), nonempty(&app.bundle_id)),
            (None, Some(path)) => {
                let identity = super::bundle_identity(Path::new(path));
                (
                    Path::new(path)
                        .file_stem()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| entry.name.clone()),
                    identity.map(|value| value.0),
                )
            }
            (None, None) => (entry.name.clone(), None),
        };
        updates.push(AppUpdate {
            id: update_id(&format!("homebrew:cask:{}", entry.name)),
            kind: if app_path.is_some() { "app" } else { "package" }.into(),
            name,
            bundle_id,
            app_path,
            installed: installed_versions(&entry),
            latest: entry.current_version,
            source: "homebrew".into(),
            action: "terminal".into(),
            release_notes: None,
            command_hint: Some(format!("brew upgrade --cask {}", entry.name)),
            ignored: false,
            external_url: None,
        });
    }

    for entry in payload.formulae {
        if entry.pinned || !valid_package_token(&entry.name) {
            continue;
        }
        updates.push(AppUpdate {
            id: update_id(&format!("homebrew:formula:{}", entry.name)),
            kind: "package".into(),
            name: entry.name.clone(),
            bundle_id: None,
            app_path: None,
            installed: installed_versions(&entry),
            latest: entry.current_version,
            source: "homebrew".into(),
            action: "terminal".into(),
            release_notes: None,
            command_hint: Some(format!("brew upgrade --formula {}", entry.name)),
            ignored: false,
            external_url: None,
        });
    }

    HomebrewScan {
        updates,
        managed_app_paths,
        warnings: Vec::new(),
    }
}

pub(crate) fn locate_brew() -> Option<PathBuf> {
    let mut candidates = vec![
        PathBuf::from("/opt/homebrew/bin/brew"),
        PathBuf::from("/usr/local/bin/brew"),
    ];
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|root| root.join("brew")));
    }
    candidates.into_iter().find(|path| {
        let prefix = path.parent().and_then(Path::parent);
        path.is_absolute()
            && std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file())
            && prefix.is_some_and(|prefix| {
                prefix.join("Cellar").is_dir()
                    || prefix.join("Caskroom").is_dir()
                    || prefix.join("Library/Homebrew").is_dir()
            })
    })
}

/// Exact app artifact from Homebrew's own receipt. The receipt root and app
/// name are both fixed by the validated token; only existing app paths win.
fn cask_app_path(prefix: &Path, home: &str, token: &str) -> Option<String> {
    let cask = prefix.join("Caskroom").join(token);
    let receipt = receipt_path(&cask)?;
    let value: Value = serde_json::from_slice(&std::fs::read(receipt).ok()?).ok()?;
    let mut app_names = Vec::new();
    collect_app_artifacts(&value, &mut app_names);
    app_names.sort();
    app_names.dedup();
    for name in app_names {
        if name.contains('/') || !name.ends_with(".app") {
            continue;
        }
        for root in [
            PathBuf::from("/Applications"),
            PathBuf::from(home).join("Applications"),
        ] {
            let path = root.join(&name);
            if path.is_dir() {
                return Some(path.to_string_lossy().into_owned());
            }
        }
    }
    None
}

fn caskroom_app_paths(prefix: &Path, home: &str) -> HashSet<String> {
    let root = prefix.join("Caskroom");
    let Ok(entries) = std::fs::read_dir(root) else {
        return HashSet::new();
    };
    entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
        .filter(|token| valid_package_token(token))
        .filter_map(|token| cask_app_path(prefix, home, &token))
        .collect()
}

fn receipt_path(cask: &Path) -> Option<PathBuf> {
    let direct = cask.join(".metadata/INSTALL_RECEIPT.json");
    if direct.is_file() {
        return Some(direct);
    }
    let entries = std::fs::read_dir(cask).ok()?;
    entries
        .flatten()
        .map(|entry| entry.path().join(".metadata/INSTALL_RECEIPT.json"))
        .find(|path| path.is_file())
}

fn collect_app_artifacts(value: &Value, apps: &mut Vec<String>) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if key == "app" {
                    if let Value::Array(values) = value {
                        apps.extend(values.iter().filter_map(Value::as_str).map(str::to_string));
                    }
                } else {
                    collect_app_artifacts(value, apps);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_app_artifacts(value, apps);
            }
        }
        _ => {}
    }
}

fn installed_versions(entry: &OutdatedEntry) -> String {
    if entry.installed_versions.is_empty() {
        "unknown".into()
    } else {
        entry.installed_versions.join(", ")
    }
}

fn nonempty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_readable_outdated_payload_parses_both_kinds() {
        let payload: OutdatedPayload = serde_json::from_str(
            r#"{"formulae":[{"name":"git","installed_versions":["1"],"current_version":"2","pinned":false}],"casks":[{"name":"demo","installed_versions":["3"],"current_version":"4","pinned":false}]}"#,
        )
        .unwrap();
        assert_eq!(payload.formulae[0].name, "git");
        assert_eq!(payload.casks[0].current_version, "4");
    }

    #[test]
    fn receipt_app_evidence_is_exact_and_rejects_paths() {
        let mut apps = Vec::new();
        collect_app_artifacts(
            &serde_json::json!({"uninstall_artifacts":[{"app":["Demo.app", "../Evil.app"]}]}),
            &mut apps,
        );
        assert!(apps.contains(&"Demo.app".to_string()));
        assert!(apps.contains(&"../Evil.app".to_string()));
        assert!(apps
            .iter()
            .filter(|name| !name.contains('/'))
            .all(|name| name == "Demo.app"));
    }
}
