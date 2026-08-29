//! Update actions with scan-to-action rebinding. Package-manager rows remain
//! read-only and return their exact terminal command; App Store/Sparkle/
//! Electron rows re-read bundle identity before opening the original updater.

use super::command::{self, Output, Status};
use super::{bundle_identity, AppUpdate, UpdateResult};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const OPEN_TIMEOUT: Duration = Duration::from_secs(10);

trait Runner {
    fn run(&self, argv: &[String], timeout: Duration, cancelled: &AtomicBool) -> Output;
}

struct SystemRunner;

impl Runner for SystemRunner {
    fn run(&self, argv: &[String], timeout: Duration, cancelled: &AtomicBool) -> Output {
        command::run(argv, timeout, cancelled)
    }
}

pub fn run_updates(updates: &[AppUpdate], cancelled: &AtomicBool) -> Vec<UpdateResult> {
    run_with_runner(updates, cancelled, &SystemRunner)
}

fn run_with_runner(
    updates: &[AppUpdate],
    cancelled: &AtomicBool,
    runner: &dyn Runner,
) -> Vec<UpdateResult> {
    let mut results = Vec::with_capacity(updates.len());
    for update in updates {
        if cancelled.load(Ordering::Relaxed) {
            results.push(result(update, "cancelled", "cancelled", "update cancelled"));
        } else if update.ignored {
            results.push(result(
                update,
                "skipped",
                "ignored",
                "update is currently hidden",
            ));
        } else if update.action == "terminal" {
            results.push(result(
                update,
                "external",
                "terminal_required",
                update
                    .command_hint
                    .as_deref()
                    .map(|command| format!("run in Terminal: {command}"))
                    .unwrap_or_else(|| "use the package manager in Terminal".into()),
            ));
        } else {
            results.push(run_delegated(update, cancelled, runner));
        }
    }
    results
}

fn run_delegated(update: &AppUpdate, cancelled: &AtomicBool, runner: &dyn Runner) -> UpdateResult {
    let app_path = update.app_path.as_deref().map(Path::new);
    if let (Some(path), Some(expected_bundle)) = (app_path, update.bundle_id.as_deref()) {
        let Some((current_bundle, current_version)) = bundle_identity(path) else {
            return result(
                update,
                "failed",
                "app_missing",
                "installed app is no longer readable",
            );
        };
        if current_bundle != expected_bundle {
            return result(
                update,
                "failed",
                "app_identity_changed",
                "installed app identity changed after scanning",
            );
        }
        if !super::version_is_newer(&update.latest, &current_version) {
            return result(
                update,
                "updated",
                "already_current",
                "app is already up to date",
            );
        }
    }
    let argv = match update.action.as_str() {
        "open_app_store" => {
            let Some(url) = update
                .external_url
                .as_deref()
                .filter(|url| valid_app_store_url(url))
            else {
                return result(
                    update,
                    "failed",
                    "unsafe_url",
                    "invalid App Store destination",
                );
            };
            command::strings(&["/usr/bin/open", url])
        }
        "open_app" => {
            let Some(path) = app_path.filter(|path| path.is_dir()) else {
                return result(update, "failed", "app_missing", "installed app is missing");
            };
            vec!["/usr/bin/open".into(), path.to_string_lossy().into_owned()]
        }
        "open_website" => {
            let Some(url) = update
                .external_url
                .as_deref()
                .filter(|url| valid_https_url(url))
            else {
                return result(update, "failed", "unsafe_url", "invalid update website");
            };
            command::strings(&["/usr/bin/open", url])
        }
        _ => {
            return result(
                update,
                "failed",
                "invalid_action",
                "unsupported update action",
            )
        }
    };
    let output = runner.run(&argv, OPEN_TIMEOUT, cancelled);
    if output.success() {
        result(
            update,
            "external",
            "delegated",
            match update.action.as_str() {
                "open_app_store" => "opened the App Store",
                "open_app" => "opened the app's own updater",
                _ => "opened the official update page",
            },
        )
    } else {
        let (outcome, cause) = classify_command_failure(&output);
        result(update, outcome, cause, output.text)
    }
}

fn classify_command_failure(output: &Output) -> (&'static str, &'static str) {
    match output.status {
        Status::Cancelled => ("cancelled", "cancelled"),
        Status::TimedOut => ("failed", "timeout"),
        Status::Unavailable => ("failed", "tool_unavailable"),
        Status::Success => ("external", "delegated"),
        Status::Failed => ("failed", "open_failed"),
    }
}

fn valid_app_store_url(value: &str) -> bool {
    let prefix = "macappstore://itunes.apple.com/app/id";
    value
        .strip_prefix(prefix)
        .is_some_and(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()))
}

fn valid_https_url(value: &str) -> bool {
    value.starts_with("https://")
        && value.len() <= 4096
        && !value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == b' ')
}

fn result(
    update: &AppUpdate,
    outcome: &str,
    cause: &str,
    message: impl Into<String>,
) -> UpdateResult {
    UpdateResult {
        id: update.id.clone(),
        outcome: outcome.into(),
        cause: cause.into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct MockRunner {
        calls: RefCell<Vec<Vec<String>>>,
        outputs: RefCell<Vec<Output>>,
    }

    impl Runner for MockRunner {
        fn run(&self, argv: &[String], _timeout: Duration, _cancelled: &AtomicBool) -> Output {
            self.calls.borrow_mut().push(argv.to_vec());
            self.outputs.borrow_mut().remove(0)
        }
    }

    fn update(action: &str, app_path: Option<String>) -> AppUpdate {
        AppUpdate {
            id: "update-test".into(),
            kind: "app".into(),
            name: "Demo".into(),
            bundle_id: Some("com.demo.app".into()),
            app_path,
            installed: "1.0".into(),
            latest: "2.0".into(),
            source: "sparkle".into(),
            action: action.into(),
            release_notes: None,
            command_hint: None,
            ignored: false,
            external_url: None,
        }
    }

    #[test]
    fn app_store_url_accepts_only_numeric_product_ids() {
        assert!(valid_app_store_url(
            "macappstore://itunes.apple.com/app/id123456"
        ));
        assert!(!valid_app_store_url("https://apps.apple.com/app/id123"));
        assert!(!valid_app_store_url(
            "macappstore://itunes.apple.com/app/id123?x=1"
        ));
    }

    #[test]
    fn delegated_action_rebinds_bundle_identity_before_opening() {
        let home = tempfile::tempdir().unwrap();
        let app = home.path().join("Demo.app");
        std::fs::create_dir_all(app.join("Contents")).unwrap();
        let mut info = plist::Dictionary::new();
        info.insert(
            "CFBundleIdentifier".into(),
            plist::Value::String("com.demo.app".into()),
        );
        info.insert(
            "CFBundleShortVersionString".into(),
            plist::Value::String("1.0".into()),
        );
        plist::Value::Dictionary(info)
            .to_file_xml(app.join("Contents/Info.plist"))
            .unwrap();
        let runner = MockRunner {
            calls: Default::default(),
            outputs: RefCell::new(vec![Output {
                status: Status::Success,
                stdout: String::new(),
                text: String::new(),
            }]),
        };
        let results = run_with_runner(
            &[update("open_app", Some(app.to_string_lossy().into_owned()))],
            &AtomicBool::new(false),
            &runner,
        );
        assert_eq!(results[0].outcome, "external");
        assert_eq!(runner.calls.borrow()[0][0], "/usr/bin/open");
        assert_eq!(runner.calls.borrow()[0][1], app.to_string_lossy());
    }

    #[test]
    fn homebrew_rows_never_execute_package_manager_commands() {
        let mut update = update("terminal", None);
        update.source = "homebrew".into();
        update.command_hint = Some("brew upgrade --cask demo".into());
        let runner = MockRunner {
            calls: Default::default(),
            outputs: Default::default(),
        };
        let results = run_with_runner(&[update], &AtomicBool::new(false), &runner);
        assert_eq!(results[0].outcome, "external");
        assert_eq!(results[0].cause, "terminal_required");
        assert!(runner.calls.borrow().is_empty());
    }
}
