// Installer: leftover installer artifacts (dmg/pkg/xip) in the user's
// download locations. Trash-routed, exact extensions only — a zip is NOT an
// installer without inspection, so zips stay out of scope here.

use crate::scanutil::{self, CancelFlag};
use mole_core::plan::{DeletionPlan, Scope};
use std::path::PathBuf;

/// Extensions treated as installer artifacts.
const INSTALLER_EXTS: &[&str] = &["dmg", "pkg", "xip"];

/// Build the installer cleanup plan from ~/Downloads and ~/Desktop.
pub fn build_plan(home: &str, cancel: &CancelFlag) -> DeletionPlan {
    let mut paths: Vec<PathBuf> = Vec::new();
    for dir in ["Downloads", "Desktop"] {
        let root = PathBuf::from(home).join(dir);
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            let ext = path
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            if INSTALLER_EXTS.contains(&ext.as_str()) {
                paths.push(path);
            }
        }
    }
    DeletionPlan {
        candidates: scanutil::parallel_candidates(
            &paths,
            "Installer artifacts",
            Scope::User,
            cancel,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    #[test]
    fn only_installer_extensions_are_candidates() {
        let tmp = tempfile::tempdir().unwrap();
        let dl = tmp.path().join("Downloads");
        std::fs::create_dir_all(&dl).unwrap();
        std::fs::write(dl.join("App.dmg"), b"x").unwrap();
        std::fs::write(dl.join("Tool.PKG"), b"x").unwrap();
        std::fs::write(dl.join("data.zip"), b"x").unwrap();
        std::fs::write(dl.join("notes.txt"), b"x").unwrap();

        let cancel: CancelFlag = Arc::new(AtomicBool::new(false));
        let plan = build_plan(&tmp.path().to_string_lossy(), &cancel);
        let names: Vec<&str> = plan
            .candidates
            .iter()
            .map(|c| c.path.rsplit('/').next().unwrap())
            .collect();
        assert!(names.contains(&"App.dmg"));
        assert!(names.contains(&"Tool.PKG"));
        assert!(
            !names.contains(&"data.zip"),
            "zip is not installer evidence"
        );
        assert!(!names.contains(&"notes.txt"));
    }
}
