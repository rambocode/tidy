//! Durable ignored-update keys and the display-only catalog cache. The schema
//! is explicit and atomic writes keep interruption from turning a partial
//! JSON file into a permanent empty set.

use super::UpdateCatalog;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

const SCHEMA: u32 = 1;
const CATALOG_SCHEMA: u32 = 1;

/// Persisted last complete scan for instant next-launch paint.
#[derive(Deserialize, Serialize)]
struct CatalogFile {
    schema: u32,
    catalog: UpdateCatalog,
}

#[derive(Default, Deserialize, Serialize)]
struct IgnoredFile {
    schema: u32,
    keys: Vec<String>,
}

fn persistence_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub(crate) fn load_ignored(home: &str) -> HashSet<String> {
    let _guard = persistence_lock().lock().unwrap();
    load_unlocked(home)
}

pub fn set_ignored(home: &str, id: &str, ignored: bool) -> Result<(), String> {
    if id.is_empty() || id.len() > 512 || id.bytes().any(|byte| byte.is_ascii_control()) {
        return Err("invalid-update-id".into());
    }
    let _guard = persistence_lock().lock().unwrap();
    let mut keys = load_unlocked(home);
    if ignored {
        keys.insert(id.to_string());
    } else {
        keys.remove(id);
    }
    let mut keys: Vec<String> = keys.into_iter().collect();
    keys.sort();
    let payload = serde_json::to_vec_pretty(&IgnoredFile {
        schema: SCHEMA,
        keys,
    })
    .map_err(|error| error.to_string())?;
    let path = ignored_path(home);
    let parent = path
        .parent()
        .ok_or_else(|| "invalid-config-path".to_string())?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, payload).map_err(|error| error.to_string())?;
    std::fs::rename(&temporary, &path).map_err(|error| error.to_string())
}

fn load_unlocked(home: &str) -> HashSet<String> {
    let Ok(bytes) = std::fs::read(ignored_path(home)) else {
        return HashSet::new();
    };
    let Ok(file) = serde_json::from_slice::<IgnoredFile>(&bytes) else {
        return HashSet::new();
    };
    if file.schema != SCHEMA {
        return HashSet::new();
    }
    file.keys
        .into_iter()
        .filter(|key| !key.is_empty() && key.len() <= 512)
        .collect()
}

/// Best-effort persistence of the latest complete scan so the next launch can
/// paint the Updates tab instantly. Display-only by construction: authority
/// fields (bundle_id / app_path / external_url) are skip_serializing so they
/// never reach disk, and callers must never load this file into the action
/// snapshot — update/ignore ids still require a fresh scan.
pub fn save_catalog(home: &str, catalog: &UpdateCatalog) {
    let _guard = persistence_lock().lock().unwrap();
    let Ok(payload) = serde_json::to_vec(&CatalogFile {
        schema: CATALOG_SCHEMA,
        catalog: catalog.clone(),
    }) else {
        return;
    };
    let path = catalog_path(home);
    let Some(parent) = path.parent() else { return };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    // Same atomic tmp+rename shape as the ignored set: a crash mid-write must
    // not leave a truncated file that parses as an empty catalog.
    let temporary = path.with_extension("json.tmp");
    if std::fs::write(&temporary, payload).is_ok() {
        let _ = std::fs::rename(&temporary, &path);
    }
}

/// Last persisted scan for instant display; None on any read/schema mismatch.
pub fn load_catalog(home: &str) -> Option<UpdateCatalog> {
    let _guard = persistence_lock().lock().unwrap();
    let bytes = std::fs::read(catalog_path(home)).ok()?;
    let file = serde_json::from_slice::<CatalogFile>(&bytes).ok()?;
    if file.schema != CATALOG_SCHEMA {
        return None;
    }
    Some(file.catalog)
}

fn catalog_path(home: &str) -> PathBuf {
    PathBuf::from(home)
        .join(".config")
        .join(mole_core::brand::CONFIG_DIR)
        .join("cached_updates.json")
}

fn ignored_path(home: &str) -> PathBuf {
    PathBuf::from(home)
        .join(".config")
        .join(mole_core::brand::CONFIG_DIR)
        .join("ignored_updates.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignored_keys_round_trip_and_unignore() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path().to_string_lossy();
        set_ignored(&home, "homebrew:cask:demo", true).unwrap();
        assert!(load_ignored(&home).contains("homebrew:cask:demo"));
        set_ignored(&home, "homebrew:cask:demo", false).unwrap();
        assert!(!load_ignored(&home).contains("homebrew:cask:demo"));
    }

    #[test]
    fn catalog_round_trips_without_action_authority() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path().to_string_lossy();
        let catalog = UpdateCatalog {
            updates: vec![super::super::AppUpdate {
                id: "update-abc".into(),
                kind: "app".into(),
                name: "Demo".into(),
                bundle_id: Some("com.demo.app".into()),
                app_path: Some("/Applications/Demo.app".into()),
                installed: "1.0".into(),
                latest: "2.0".into(),
                source: "sparkle".into(),
                action: "open_app".into(),
                release_notes: None,
                command_hint: None,
                ignored: false,
                external_url: Some("https://example.test".into()),
            }],
            up_to_date: Vec::new(),
            warnings: vec!["sparkle:demo".into()],
            checked_at: 42,
        };
        save_catalog(&home, &catalog);
        let loaded = load_catalog(&home).expect("persisted catalog");
        assert_eq!(loaded.updates[0].id, "update-abc");
        assert_eq!(loaded.checked_at, 42);
        // Authority fields must not survive persistence: display cache only.
        assert_eq!(loaded.updates[0].bundle_id, None);
        assert_eq!(loaded.updates[0].app_path, None);
        assert_eq!(loaded.updates[0].external_url, None);
    }

    #[test]
    fn stale_catalog_schema_fails_closed_to_none() {
        let home = tempfile::tempdir().unwrap();
        let path = catalog_path(home.path().to_str().unwrap());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, br#"{"schema":999,"catalog":{}}"#).unwrap();
        assert!(load_catalog(home.path().to_str().unwrap()).is_none());
    }

    #[test]
    fn stale_schema_fails_closed_to_no_ignored_updates() {
        let home = tempfile::tempdir().unwrap();
        let path = ignored_path(home.path().to_str().unwrap());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, br#"{"schema":999,"keys":["x"]}"#).unwrap();
        assert!(load_ignored(home.path().to_str().unwrap()).is_empty());
    }
}
