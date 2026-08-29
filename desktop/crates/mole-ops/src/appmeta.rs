// App metadata extras for the Apps view: icon extraction (icns → PNG via the
// system `sips`, no image crates), and login-item listing
// (plist Program/ProgramArguments parsed as
// ABSOLUTE paths only, per the project's launchd-parsing rule).

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Extract an app's icon as PNG bytes (128px), or None when unavailable.
/// Uses `sips` so there is no image-decoding dependency in the app.
pub fn app_icon_png(app_path: &str) -> Option<Vec<u8>> {
    let resources = Path::new(app_path).join("Contents/Resources");
    let icns = find_icns(app_path, &resources)?;

    // Unique temp target per source path hash; sips refuses stdin/stdout.
    let mut hash: u64 = 1469598103934665603;
    for b in app_path.as_bytes() {
        hash = (hash ^ u64::from(*b)).wrapping_mul(1099511628211);
    }
    let out = std::env::temp_dir().join(format!("mole-icon-{hash:x}.png"));

    let status = Command::new("sips")
        .args(["-s", "format", "png", "--resampleHeightWidth", "128", "128"])
        .arg(&icns)
        .arg("--out")
        .arg(&out)
        .output()
        .ok()?;
    if !status.status.success() {
        return None;
    }
    let bytes = std::fs::read(&out).ok();
    // SAFE: removes the sips temp file this function just created — the same
    // mktemp-scratch exception the shell contract grants; routing it through
    // the sink would add Trash + an audit-log entry to a scratch file.
    #[allow(clippy::disallowed_methods)]
    let _ = std::fs::remove_file(&out);
    bytes
}

/// Locate the app's .icns: CFBundleIconFile first, then the first .icns in
/// Resources (asset-catalog-only apps yield None → letter avatar fallback).
fn find_icns(app_path: &str, resources: &Path) -> Option<PathBuf> {
    let info = Path::new(app_path).join("Contents/Info.plist");
    if let Ok(value) = plist::Value::from_file(&info) {
        if let Some(name) = value
            .as_dictionary()
            .and_then(|d| d.get("CFBundleIconFile"))
            .and_then(|v| v.as_string())
        {
            let mut candidate = resources.join(name);
            if candidate.extension().is_none() {
                candidate.set_extension("icns");
            }
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    std::fs::read_dir(resources)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "icns"))
}

/// One launchd login item (agent or daemon).
#[derive(Debug, Serialize)]
pub struct LoginItem {
    pub label: String,
    /// The plist file itself.
    pub path: String,
    /// Absolute executable path, or None when the plist's program value is
    /// not an absolute path (never guessed — project parsing rule).
    pub program: Option<String>,
    /// Whether the target executable still exists (false = likely orphan).
    pub program_exists: bool,
    /// "user" (removable via Trash) or "system" (display-only).
    pub scope: &'static str,
    /// launchd override state: false when the label is disabled in its domain.
    pub enabled: bool,
}

/// Labels disabled in one launchd domain, from `launchctl print-disabled`.
/// Read-only; an unreadable domain yields an empty set (items then read as
/// enabled, which matches launchd's default).
fn disabled_labels(domain: &str) -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    let Ok(out) = Command::new("launchctl")
        .args(["print-disabled", domain])
        .output()
    else {
        return set;
    };
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let line = line.trim();
        // Shapes across macOS versions: "label" => disabled | => true
        if let Some((label_part, state)) = line.split_once("=>") {
            let label = label_part.trim().trim_matches('"').to_string();
            let state = state.trim().trim_end_matches(';');
            if !label.is_empty() && (state == "disabled" || state == "true") {
                set.insert(label);
            }
        }
    }
    set
}

/// List login items from the user and system launchd directories.
pub fn list_login_items(home: &str) -> Vec<LoginItem> {
    let dirs: [(PathBuf, &'static str); 3] = [
        (PathBuf::from(home).join("Library/LaunchAgents"), "user"),
        (PathBuf::from("/Library/LaunchAgents"), "system"),
        (PathBuf::from("/Library/LaunchDaemons"), "system"),
    ];
    let uid = unsafe { libc::getuid() };
    let user_disabled = disabled_labels(&format!("gui/{uid}"));
    let system_disabled = disabled_labels("system");
    let mut items = Vec::new();
    for (dir, scope) in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "plist") {
                continue;
            }
            let (label, program) = read_launchd_plist(&path);
            let program_exists = program.as_deref().is_some_and(|p| Path::new(p).exists());
            let label = label.unwrap_or_else(|| {
                path.file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });
            let disabled = if scope == "user" {
                user_disabled.contains(&label)
            } else {
                system_disabled.contains(&label)
            };
            items.push(LoginItem {
                label,
                path: path.to_string_lossy().into_owned(),
                program,
                program_exists,
                scope,
                enabled: !disabled,
            });
        }
    }
    items.sort_by_key(|i| i.label.to_lowercase());
    items
}

/// Parse Label and the program path from a launchd plist. Program values that
/// are not absolute paths are rejected (returned as None), and parse errors
/// never masquerade as data.
fn read_launchd_plist(path: &Path) -> (Option<String>, Option<String>) {
    let Ok(value) = plist::Value::from_file(path) else {
        return (None, None);
    };
    let Some(dict) = value.as_dictionary() else {
        return (None, None);
    };
    let label = dict
        .get("Label")
        .and_then(|v| v.as_string())
        .map(String::from);
    let raw_program = dict
        .get("Program")
        .and_then(|v| v.as_string())
        .map(String::from)
        .or_else(|| {
            dict.get("ProgramArguments")
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .and_then(|v| v.as_string())
                .map(String::from)
        });
    // Absolute paths only: a bare command name is not evidence of anything.
    let program = raw_program.filter(|p| p.starts_with('/'));
    (label, program)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launchd_plist_accepts_absolute_program_only() {
        let tmp = tempfile::tempdir().unwrap();
        let agents = tmp.path().join("Library/LaunchAgents");
        std::fs::create_dir_all(&agents).unwrap();
        let write = |name: &str, program: &str| {
            std::fs::write(
                agents.join(name),
                format!(
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>{}</string>
<key>ProgramArguments</key><array><string>{}</string></array>
</dict></plist>"#,
                    name.trim_end_matches(".plist"),
                    program
                ),
            )
            .unwrap();
        };
        write("com.example.abs.plist", "/usr/bin/true");
        write("com.example.rel.plist", "not-a-path");

        let items = list_login_items(&tmp.path().to_string_lossy());
        let abs = items.iter().find(|i| i.label == "com.example.abs").unwrap();
        assert_eq!(abs.program.as_deref(), Some("/usr/bin/true"));
        assert!(abs.program_exists);
        let rel = items.iter().find(|i| i.label == "com.example.rel").unwrap();
        // A bare command name never becomes a program path.
        assert_eq!(rel.program, None);
        assert_eq!(abs.scope, "user");
    }

    #[test]
    fn launchctl_toggle_is_test_guarded() {
        // The project launchctl rule: tests must never reach the real command.
        std::env::set_var("MOLE_TEST_NO_AUTH", "1");
        let err = set_login_agent_enabled("com.example.x", "/tmp/x.plist", false).unwrap_err();
        assert_eq!(err, "test-mode-guard");
        std::env::remove_var("MOLE_TEST_NO_AUTH");
    }

    #[test]
    fn embedded_items_found_in_fake_bundle() {
        let tmp = tempfile::tempdir().unwrap();
        let li = tmp
            .path()
            .join("Applications/Demo.app/Contents/Library/LoginItems/DemoHelper.app");
        std::fs::create_dir_all(&li).unwrap();
        // The scan always includes the real /Applications too; assert only on
        // the planted bundle under the temp home.
        let items = list_embedded_login_items(&tmp.path().to_string_lossy());
        let demo = items.iter().find(|i| i.app_name == "Demo").unwrap();
        assert_eq!(demo.item_name, "DemoHelper");
        assert_eq!(demo.kind, "login");
    }
}

/// One login item or helper embedded inside an app bundle. These are owned by
/// their app (SMAppService); Mole shows them for confirmation only.
#[derive(Debug, Serialize)]
pub struct EmbeddedLoginItem {
    pub app_name: String,
    /// Parent app bundle (icon source).
    pub app_path: String,
    pub item_name: String,
    /// "login" (Contents/Library/LoginItems) or "helper" (LaunchServices).
    pub kind: &'static str,
}

/// Scan app bundles for embedded login items and privileged helpers.
pub fn list_embedded_login_items(home: &str) -> Vec<EmbeddedLoginItem> {
    let roots = [
        PathBuf::from("/Applications"),
        PathBuf::from(home).join("Applications"),
    ];
    let mut items = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let app = entry.path();
            if app.extension().is_none_or(|e| e != "app") {
                continue;
            }
            let app_name = app
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let scan = [
                (app.join("Contents/Library/LoginItems"), "login"),
                (app.join("Contents/Library/LaunchServices"), "helper"),
            ];
            for (dir, kind) in scan {
                let Ok(children) = std::fs::read_dir(&dir) else {
                    continue;
                };
                for child in children.flatten() {
                    let name = child
                        .path()
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    if name.is_empty() {
                        continue;
                    }
                    items.push(EmbeddedLoginItem {
                        app_name: app_name.clone(),
                        app_path: app.to_string_lossy().into_owned(),
                        item_name: name,
                        kind,
                    });
                }
            }
        }
    }
    items.sort_by_key(|i| (i.app_name.to_lowercase(), i.item_name.to_lowercase()));
    items
}

/// Enable/disable a USER-scope launchd agent via launchctl (gui domain — no
/// authorization prompt). System-domain items are refused by the caller.
/// MOLE_TEST_MODE / MOLE_TEST_NO_AUTH hard-deny per the project launchctl rule.
pub fn set_login_agent_enabled(label: &str, plist_path: &str, enable: bool) -> Result<(), String> {
    if std::env::var("MOLE_TEST_MODE").as_deref() == Ok("1")
        || std::env::var("MOLE_TEST_NO_AUTH").as_deref() == Ok("1")
    {
        return Err("test-mode-guard".to_string());
    }
    let uid = unsafe { libc::getuid() };
    let domain_label = format!("gui/{uid}/{label}");
    let domain = format!("gui/{uid}");

    // Order matters: enable clears the override BEFORE bootstrapping; disable
    // boots the job out first so the override cannot race a respawn.
    let run = |args: &[&str]| {
        Command::new("launchctl")
            .args(args)
            .output()
            .map_err(|e| e.to_string())
    };
    if enable {
        run(&["enable", &domain_label])?;
        // Already-loaded jobs make bootstrap fail benignly; ignore its status.
        let _ = run(&["bootstrap", &domain, plist_path]);
    } else {
        let _ = run(&["bootout", &domain_label]);
        run(&["disable", &domain_label])?;
    }
    Ok(())
}
