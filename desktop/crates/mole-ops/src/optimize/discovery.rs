//! Read-only discovery for conditional optimization tasks. Partial directory
//! scans return an error so an incomplete prefix never becomes a deletion
//! plan.

use plist::Value;
use std::collections::HashSet;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const MAX_DISCOVERED_PATHS: usize = 512;
const SQLITE_MAX_BYTES: u64 = 100 * 1024 * 1024;
const NOTIFICATION_MIN_BYTES: u64 = 50 * 1024 * 1024;
const SAVED_STATE_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// A user launch item whose plist gave exact executable evidence.
#[derive(Debug, Clone)]
pub(crate) struct LaunchItem {
    pub label: String,
    pub plist: PathBuf,
    pub program: PathBuf,
    pub program_exists: bool,
}

/// True only for an existing file with the SQLite 3 header.
pub(crate) fn is_sqlite(path: &Path) -> bool {
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut header = [0_u8; 16];
    file.read_exact(&mut header).is_ok() && &header == b"SQLite format 3\0"
}

/// Small Mail, Messages, and Safari databases eligible for fragmentation
/// inspection. Globs are expanded structurally rather than by a shell.
pub(crate) fn vacuum_databases(home: &Path) -> io::Result<Vec<PathBuf>> {
    let mut paths = vec![
        home.join("Library/Messages/chat.db"),
        home.join("Library/Safari/History.db"),
        home.join("Library/Safari/TopSites.db"),
    ];
    let mail = home.join("Library/Mail");
    if mail.is_dir() {
        for entry in read_dir_complete(&mail)? {
            let path = entry.path();
            if !entry.file_name().to_string_lossy().starts_with('V') || !path.is_dir() {
                continue;
            }
            let data = path.join("MailData");
            if !data.is_dir() {
                continue;
            }
            for db in read_dir_complete(&data)? {
                if db
                    .file_name()
                    .to_string_lossy()
                    .starts_with("Envelope Index")
                {
                    paths.push(db.path());
                }
            }
        }
    }
    paths.retain(|path| {
        is_sqlite(path)
            && fs::metadata(path).is_ok_and(|metadata| metadata.len() <= SQLITE_MAX_BYTES)
    });
    paths.sort();
    paths.dedup();
    Ok(paths)
}

/// Direct `.savedState` children older than 30 days. Nested traversal is not
/// needed because the task removes the whole exact bundle state directory.
pub(crate) fn stale_saved_states(home: &Path) -> io::Result<Vec<PathBuf>> {
    let root = home.join("Library/Saved Application State");
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let now = SystemTime::now();
    let mut paths = Vec::new();
    for entry in read_dir_complete(&root)? {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            continue;
        }
        if path
            .extension()
            .is_none_or(|extension| extension != "savedState")
        {
            continue;
        }
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        if now
            .duration_since(modified)
            .is_ok_and(|age| age > SAVED_STATE_AGE)
        {
            push_bounded(&mut paths, path)?;
        }
    }
    Ok(paths)
}

/// Invalid third-party preference plists. Apple/global/loginwindow preferences
/// are explicit non-targets even when malformed.
pub(crate) fn broken_preferences(home: &Path) -> io::Result<Vec<PathBuf>> {
    let root = home.join("Library/Preferences");
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in read_dir_complete(&root)? {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if path
            .extension()
            .is_none_or(|extension| extension != "plist")
            || name.starts_with("com.apple.")
            || name.starts_with(".GlobalPreferences")
            || name == "loginwindow.plist"
        {
            continue;
        }
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            continue;
        }
        if Value::from_file(&path).is_err() {
            push_bounded(&mut paths, path)?;
        }
    }
    Ok(paths)
}

/// Invalid shared-file-list databases, excluding recent-document lists which
/// are user history rather than repairable Finder state.
pub(crate) fn broken_shared_file_lists(home: &Path) -> io::Result<Vec<PathBuf>> {
    let root = home.join("Library/Application Support/com.apple.sharedfilelist");
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    walk_files(&root, 0, 8, &mut |path| {
        let lower = path.to_string_lossy().to_ascii_lowercase();
        let extension = path.extension().and_then(|extension| extension.to_str());
        if matches!(extension, Some("sfl2" | "sfl3"))
            && !lower.contains("applicationrecentdocuments")
            && Value::from_file(path).is_err()
        {
            push_bounded(&mut paths, path.to_path_buf())?;
        }
        Ok(())
    })?;
    Ok(paths)
}

/// Exact user LaunchAgent inventory. A missing or relative Program value is
/// not deletion evidence and is therefore omitted.
pub(crate) fn user_launch_items(home: &Path) -> io::Result<Vec<LaunchItem>> {
    let root = home.join("Library/LaunchAgents");
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut items = Vec::new();
    for entry in read_dir_complete(&root)? {
        let plist = entry.path();
        if plist
            .extension()
            .is_none_or(|extension| extension != "plist")
        {
            continue;
        }
        let metadata = fs::symlink_metadata(&plist)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            continue;
        }
        let Ok(value) = Value::from_file(&plist) else {
            continue;
        };
        let Some(dictionary) = value.as_dictionary() else {
            continue;
        };
        let label = dictionary
            .get("Label")
            .and_then(Value::as_string)
            .unwrap_or_default()
            .to_string();
        if label.starts_with("com.apple.") {
            continue;
        }
        let raw_program = dictionary
            .get("Program")
            .and_then(Value::as_string)
            .or_else(|| {
                dictionary
                    .get("ProgramArguments")
                    .and_then(Value::as_array)
                    .and_then(|arguments| arguments.first())
                    .and_then(Value::as_string)
            });
        let Some(raw_program) = raw_program else {
            continue;
        };
        let program = PathBuf::from(raw_program);
        if !program.is_absolute() {
            continue;
        }
        items.push(LaunchItem {
            label,
            plist,
            program_exists: program.exists(),
            program,
        });
    }
    Ok(items)
}

/// Supported Notification Center DB path when it is large enough to prune.
pub(crate) fn oversized_notification_db(home: &Path) -> Option<PathBuf> {
    let group = home.join("Library/Group Containers/group.com.apple.usernoted/db2/db");
    if file_is_over(&group, NOTIFICATION_MIN_BYTES) {
        return Some(group);
    }
    let darwin = std::env::var_os("DARWIN_USER_DIR").map(PathBuf::from);
    let legacy = darwin.map(|root| root.join("com.apple.notificationcenter/db2/db"));
    legacy.filter(|path| file_is_over(path, NOTIFICATION_MIN_BYTES))
}

/// CoreDuet database when the main/WAL/SHM family exceeds 100 MB.
pub(crate) fn oversized_knowledge_db(home: &Path) -> Option<PathBuf> {
    let db = home.join("Library/Application Support/Knowledge/knowledgeC.db");
    if !is_sqlite(&db) {
        return None;
    }
    let total = [
        db.clone(),
        with_suffix(&db, "-wal"),
        with_suffix(&db, "-shm"),
    ]
    .iter()
    .filter_map(|path| fs::metadata(path).ok())
    .map(|metadata| metadata.len())
    .sum::<u64>();
    (total > SQLITE_MAX_BYTES).then_some(db)
}

/// Current Spotlight `EnabledPreferenceRules` string array.
pub(crate) fn spotlight_rules(home: &Path) -> io::Result<Vec<String>> {
    let plist = home.join("Library/Preferences/com.apple.spotlight.plist");
    if !plist.is_file() {
        return Ok(Vec::new());
    }
    let value = Value::from_file(&plist).map_err(io::Error::other)?;
    let Some(array) = value
        .as_dictionary()
        .and_then(|dictionary| dictionary.get("EnabledPreferenceRules"))
        .and_then(Value::as_array)
    else {
        return Ok(Vec::new());
    };
    Ok(array
        .iter()
        .filter_map(Value::as_string)
        .map(str::to_string)
        .collect())
}

/// Bundle IDs found in the ordinary app roots. `complete=false` means at
/// least one existing root was unreadable, so absence must remain Unknown.
pub(crate) fn installed_bundle_ids(home: &Path) -> (HashSet<String>, bool) {
    let mut roots = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
        PathBuf::from("/System/Library/CoreServices"),
        home.join("Applications"),
    ];
    if let Ok(volumes) = fs::read_dir("/Volumes") {
        roots.extend(
            volumes
                .flatten()
                .map(|entry| entry.path().join("Applications")),
        );
    }

    let mut ids = HashSet::new();
    let mut complete = true;
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        if scan_app_root(&root, 0, 6, &mut ids).is_err() {
            complete = false;
        }
    }
    (ids, complete)
}

/// Strict reverse-DNS token accepted before embedding a bundle ID in a
/// Spotlight metadata query or defaults argv.
pub(crate) fn valid_bundle_id(value: &str) -> bool {
    if value.starts_with('.') || !value.contains('.') {
        return false;
    }
    value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn scan_app_root(
    root: &Path,
    depth: usize,
    max_depth: usize,
    ids: &mut HashSet<String>,
) -> io::Result<()> {
    if depth > max_depth {
        return Ok(());
    }
    for entry in read_dir_complete(root)? {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            continue;
        }
        if path.extension().is_some_and(|extension| extension == "app") {
            if let Ok(value) = Value::from_file(path.join("Contents/Info.plist")) {
                if let Some(id) = value
                    .as_dictionary()
                    .and_then(|dictionary| dictionary.get("CFBundleIdentifier"))
                    .and_then(Value::as_string)
                {
                    ids.insert(id.to_string());
                }
            }
            continue;
        }
        scan_app_root(&path, depth + 1, max_depth, ids)?;
    }
    Ok(())
}

fn walk_files(
    root: &Path,
    depth: usize,
    max_depth: usize,
    visit: &mut impl FnMut(&Path) -> io::Result<()>,
) -> io::Result<()> {
    if depth > max_depth {
        return Ok(());
    }
    for entry in read_dir_complete(root)? {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            walk_files(&path, depth + 1, max_depth, visit)?;
        } else if metadata.is_file() {
            visit(&path)?;
        }
    }
    Ok(())
}

fn read_dir_complete(path: &Path) -> io::Result<Vec<fs::DirEntry>> {
    fs::read_dir(path)?.collect()
}

fn push_bounded(paths: &mut Vec<PathBuf>, path: PathBuf) -> io::Result<()> {
    if paths.len() >= MAX_DISCOVERED_PATHS {
        return Err(io::Error::other("optimization candidate limit exceeded"));
    }
    paths.push(path);
    Ok(())
}

fn file_is_over(path: &Path, bytes: u64) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.is_file() && metadata.len() > bytes)
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn preference_scan_targets_only_broken_third_party_files() {
        let dir = tempfile::tempdir().unwrap();
        let prefs = dir.path().join("Library/Preferences");
        fs::create_dir_all(&prefs).unwrap();
        fs::write(prefs.join("com.example.broken.plist"), b"not a plist").unwrap();
        fs::write(prefs.join("com.apple.broken.plist"), b"not a plist").unwrap();
        Value::Dictionary(Default::default())
            .to_file_xml(prefs.join("com.example.valid.plist"))
            .unwrap();

        let paths = broken_preferences(dir.path()).unwrap();
        assert_eq!(paths, [prefs.join("com.example.broken.plist")]);
    }

    #[test]
    fn launch_agent_requires_an_absolute_missing_program() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join("Library/LaunchAgents");
        fs::create_dir_all(&agents).unwrap();
        let missing = agents.join("com.example.missing.plist");
        let relative = agents.join("com.example.relative.plist");
        let mut missing_dictionary = plist::Dictionary::new();
        missing_dictionary.insert(
            "Label".to_string(),
            Value::String("com.example.missing".into()),
        );
        missing_dictionary.insert(
            "Program".to_string(),
            Value::String("/missing/example".into()),
        );
        Value::Dictionary(missing_dictionary)
            .to_file_xml(&missing)
            .unwrap();
        let mut relative_dictionary = plist::Dictionary::new();
        relative_dictionary.insert(
            "Label".to_string(),
            Value::String("com.example.relative".into()),
        );
        relative_dictionary.insert("Program".to_string(), Value::String("bin/helper".into()));
        Value::Dictionary(relative_dictionary)
            .to_file_xml(&relative)
            .unwrap();

        let items = user_launch_items(dir.path()).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].plist, missing);
        assert!(!items[0].program_exists);
    }

    #[test]
    fn unreadable_scan_never_returns_partial_deletion_candidates() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir
            .path()
            .join("Library/Application Support/com.apple.sharedfilelist");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("broken.sfl2"), b"broken").unwrap();
        let denied = root.join("denied");
        fs::create_dir(&denied).unwrap();
        fs::set_permissions(&denied, fs::Permissions::from_mode(0o000)).unwrap();

        // Root can still read mode-000 directories, so only assert the strong
        // contract when the platform actually denies the read.
        let result = broken_shared_file_lists(dir.path());
        if fs::read_dir(&denied).is_err() {
            assert!(result.is_err());
        }
        fs::set_permissions(&denied, fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[test]
    fn bundle_id_validation_rejects_query_syntax() {
        assert!(valid_bundle_id("com.example.app"));
        assert!(!valid_bundle_id("com.example.' || true"));
        assert!(!valid_bundle_id("single"));
        assert!(!valid_bundle_id(".com.example"));
    }
}
