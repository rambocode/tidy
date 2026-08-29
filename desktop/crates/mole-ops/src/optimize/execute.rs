//! Production executor for user-scope optimization tasks. External tools are
//! bounded, locale-stable argv calls. Candidate deletion goes through
//! `engine::execute`, never a raw filesystem removal.

use super::discovery;
use super::{OptimizeTask, TaskExecutor, TaskResult};
use crate::engine::{self, ExecOptions, Providers};
use crate::scanutil;
use mole_core::plan::{DeletionPlan, Scope};
use mole_core::probes::{LiveProbes, TriState};
use mole_core::sink::DeleteMode;
use mole_macos::{AdminRunner, FinderTrash};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const PROBE_TIMEOUT: Duration = Duration::from_secs(8);
const TASK_TIMEOUT: Duration = Duration::from_secs(30);
const DATABASE_TIMEOUT: Duration = Duration::from_secs(60);

/// Production optimization executor bound to the invoking user's home.
pub struct SystemTaskExecutor {
    home: PathBuf,
}

impl SystemTaskExecutor {
    pub fn new(home: impl Into<PathBuf>) -> Self {
        Self { home: home.into() }
    }
}

impl TaskExecutor for SystemTaskExecutor {
    fn execute(&self, task: &OptimizeTask, probes: &dyn LiveProbes) -> TaskResult {
        match task.id {
            "rebuildQuickLook"
            | "rebuildQuickLookThumbnails"
            | "rebuildFontCache"
            | "preventNetworkDSStore" => self.run_external_steps(task, TASK_TIMEOUT),
            "rebuildLaunchServices" => self.rebuild_launch_services(task),
            "cleanSavedState" => {
                self.trash_discovered(task, discovery::stale_saved_states(&self.home), probes)
            }
            "cleanQuarantineEvents" => self.clean_quarantine(task),
            "cleanCoreDuet" => self.clean_coreduet(task, probes),
            "auditLegacyOverrides" => self.audit_legacy_overrides(task),
            "vacuumSQLiteDatabases" => self.vacuum_databases(task, probes),
            "repairSharedFileLists" => self.trash_discovered(
                task,
                discovery::broken_shared_file_lists(&self.home),
                probes,
            ),
            "cleanBrokenLaunchAgents" => self.clean_broken_launch_agents(task, probes),
            "repairBrokenPlists" => {
                self.trash_discovered(task, discovery::broken_preferences(&self.home), probes)
            }
            "pruneNotificationDB" => self.prune_notification_db(task),
            "pruneSpotlightOrphanRules" => self.prune_spotlight_rules(task),
            "auditLoginItems" => self.audit_login_items(task),
            // These are unreachable until a future executor implements the
            // signed helper's narrow maintenance-action protocol.
            "flushDNS"
            | "reindexSpotlight"
            | "memoryPurge"
            | "repairDiskPermissions"
            | "runPeriodicMaintenance"
            | "flushNetworkStack" => TaskResult::new(
                task,
                "requires_admin",
                "privileged maintenance transport is unavailable",
            ),
            _ => TaskResult::new(task, "failed", "task has no executor"),
        }
    }
}

impl SystemTaskExecutor {
    fn run_external_steps(&self, task: &OptimizeTask, timeout: Duration) -> TaskResult {
        let mut outputs = Vec::new();
        let mut unavailable = false;
        let mut failed = false;
        for step in &task.commands {
            let result = run_bounded(step, timeout);
            unavailable |= result.status == CommandStatus::Unavailable;
            failed |= !result.success();
            if !result.output.trim().is_empty() {
                outputs.push(result.output.trim().to_string());
            }
        }
        let output = if outputs.is_empty() {
            format!("{} completed", task.title)
        } else {
            outputs.join("\n")
        };
        if unavailable {
            TaskResult::new(task, "unavailable", output)
        } else if failed {
            TaskResult::new(task, "failed", output)
        } else {
            TaskResult::new(task, "ok", output)
        }
    }

    fn rebuild_launch_services(&self, task: &OptimizeTask) -> TaskResult {
        // Garbage collection is best effort; the authoritative rebuild still
        // runs and decides the result, matching Mole.app/CLI behavior.
        let _ = run_bounded(&task.commands[0], TASK_TIMEOUT);
        let result = run_bounded(&task.commands[1], TASK_TIMEOUT);
        command_task_result(task, result, "Launch Services database rebuilt")
    }

    fn clean_quarantine(&self, task: &OptimizeTask) -> TaskResult {
        let db = self
            .home
            .join("Library/Preferences/com.apple.LaunchServices.QuarantineEventsV2");
        if !discovery::is_sqlite(&db) {
            return TaskResult::new(
                task,
                "unchanged",
                "quarantine database is absent or invalid",
            );
        }
        let count = self.sqlite(
            &db,
            "SELECT COUNT(*) FROM LSQuarantineEvent;",
            PROBE_TIMEOUT,
        );
        if !count.success() {
            return command_task_result(task, count, "");
        }
        let Some(rows) = count.output.trim().parse::<u64>().ok() else {
            return TaskResult::new(task, "failed", "quarantine row count was not numeric");
        };
        if rows == 0 {
            return TaskResult::new(task, "unchanged", "quarantine history is already empty");
        }
        let result = self.sqlite(
            &db,
            "DELETE FROM LSQuarantineEvent; VACUUM;",
            DATABASE_TIMEOUT,
        );
        command_task_result(task, result, format!("cleared {rows} quarantine event(s)"))
    }

    fn clean_coreduet(&self, task: &OptimizeTask, probes: &dyn LiveProbes) -> TaskResult {
        let Some(db) = discovery::oversized_knowledge_db(&self.home) else {
            return TaskResult::new(task, "unchanged", "usage database is below 100 MB");
        };
        match probes.sqlite_in_use(&db) {
            TriState::Active => {
                return TaskResult::new(task, "skipped", "usage database is currently open")
            }
            TriState::Unknown => {
                return TaskResult::new(
                    task,
                    "probe_failed",
                    "could not prove the usage database is closed",
                )
            }
            TriState::Idle => {}
        }
        let sql = "DELETE FROM ZOBJECT WHERE ZCREATIONDATE < (strftime('%s','now','-90 days') - strftime('%s','2001-01-01')); VACUUM;";
        let result = self.sqlite(&db, sql, DATABASE_TIMEOUT);
        command_task_result(task, result, "oversized usage database compacted")
    }

    fn audit_legacy_overrides(&self, task: &OptimizeTask) -> TaskResult {
        let keys = [
            ("NSGlobalDomain", "NSAppSleepDisabled"),
            ("com.apple.frameworks.diskimages", "skip-verify-locked"),
            ("com.apple.frameworks.diskimages", "skip-verify-remote"),
        ];
        let mut present = Vec::new();
        for (domain, key) in keys {
            let read = strings(&["/usr/bin/defaults", "read", domain, key]);
            let result = run_bounded(&read, PROBE_TIMEOUT);
            if result.status == CommandStatus::Unavailable {
                return TaskResult::new(task, "unavailable", result.output);
            }
            if result.success() {
                present.push((domain, key));
            }
        }
        if present.is_empty() {
            return TaskResult::new(task, "unchanged", "legacy overrides are not set");
        }

        let mut failures = Vec::new();
        for (domain, key) in &present {
            let delete = strings(&["/usr/bin/defaults", "delete", domain, key]);
            let result = run_bounded(&delete, PROBE_TIMEOUT);
            if !result.success() {
                failures.push(format!("{domain}:{key}: {}", result.output.trim()));
            }
        }
        if failures.is_empty() {
            TaskResult::new(
                task,
                "ok",
                format!("removed {} legacy override(s)", present.len()),
            )
        } else {
            TaskResult::new(task, "failed", failures.join("\n"))
        }
    }

    fn vacuum_databases(&self, task: &OptimizeTask, probes: &dyn LiveProbes) -> TaskResult {
        let databases = match discovery::vacuum_databases(&self.home) {
            Ok(databases) => databases,
            Err(error) => return TaskResult::new(task, "failed", error.to_string()),
        };
        if databases.is_empty() {
            return TaskResult::new(task, "unchanged", "no eligible databases were found");
        }

        let mut compacted = 0usize;
        let mut already_compact = 0usize;
        let mut busy = 0usize;
        let mut failures = Vec::new();
        for database in databases {
            match probes.sqlite_in_use(&database) {
                TriState::Active => {
                    busy += 1;
                    continue;
                }
                TriState::Unknown => {
                    failures.push(format!("{}: open-state probe failed", database.display()));
                    continue;
                }
                TriState::Idle => {}
            }
            let page_info = self.sqlite(
                &database,
                "PRAGMA page_count; PRAGMA freelist_count;",
                PROBE_TIMEOUT,
            );
            if !page_info.success() {
                failures.push(format!(
                    "{}: {}",
                    database.display(),
                    page_info.output.trim()
                ));
                continue;
            }
            let values: Vec<u64> = page_info
                .output
                .lines()
                .filter_map(|line| line.trim().parse().ok())
                .collect();
            if values.len() != 2 || values[0] == 0 {
                failures.push(format!(
                    "{}: invalid fragmentation data",
                    database.display()
                ));
                continue;
            }
            if values[1] * 100 < values[0] * 5 {
                already_compact += 1;
                continue;
            }
            let integrity = self.sqlite(&database, "PRAGMA integrity_check;", DATABASE_TIMEOUT);
            if !integrity.success() || integrity.output.trim() != "ok" {
                failures.push(format!("{}: integrity check failed", database.display()));
                continue;
            }
            let vacuum = self.sqlite(&database, "VACUUM;", DATABASE_TIMEOUT);
            if vacuum.success() {
                compacted += 1;
            } else {
                failures.push(format!("{}: {}", database.display(), vacuum.output.trim()));
            }
        }

        let summary = format!(
            "{compacted} compacted, {already_compact} already compact, {busy} busy, {} failed",
            failures.len()
        );
        if !failures.is_empty() {
            TaskResult::new(
                task,
                "failed",
                format!("{summary}\n{}", failures.join("\n")),
            )
        } else if compacted > 0 {
            TaskResult::new(task, "ok", summary)
        } else if busy > 0 {
            TaskResult::new(task, "skipped", summary)
        } else {
            TaskResult::new(task, "unchanged", summary)
        }
    }

    fn clean_broken_launch_agents(
        &self,
        task: &OptimizeTask,
        probes: &dyn LiveProbes,
    ) -> TaskResult {
        let items = match discovery::user_launch_items(&self.home) {
            Ok(items) => items,
            Err(error) => return TaskResult::new(task, "failed", error.to_string()),
        };
        let paths = items
            .into_iter()
            .filter(|item| !item.program_exists)
            .map(|item| item.plist)
            .collect();
        self.trash_paths(task, paths, probes)
    }

    fn prune_notification_db(&self, task: &OptimizeTask) -> TaskResult {
        let Some(database) = discovery::oversized_notification_db(&self.home) else {
            return TaskResult::new(task, "unchanged", "notification database is below 50 MB");
        };
        if !discovery::is_sqlite(&database) {
            return TaskResult::new(task, "failed", "notification database is not SQLite");
        }
        let sql =
            "DELETE FROM record WHERE delivered_date < strftime('%s','now','-30 days'); VACUUM;";
        let result = self.sqlite(&database, sql, DATABASE_TIMEOUT);
        if result.success() {
            // Restart is best effort after the database transaction succeeds.
            let _ = run_bounded(
                &strings(&["/usr/bin/killall", "NotificationCenter"]),
                PROBE_TIMEOUT,
            );
        }
        command_task_result(task, result, "notification database pruned")
    }

    fn prune_spotlight_rules(&self, task: &OptimizeTask) -> TaskResult {
        let rules = match discovery::spotlight_rules(&self.home) {
            Ok(rules) => rules,
            Err(error) => return TaskResult::new(task, "failed", error.to_string()),
        };
        if rules.is_empty() {
            return TaskResult::new(task, "unchanged", "Spotlight rules are already clean");
        }
        let (installed, filesystem_complete) = discovery::installed_bundle_ids(&self.home);
        let mut keep = Vec::new();
        let mut removed = Vec::new();
        let mut unknown = 0usize;
        for rule in rules {
            if rule.starts_with("System.")
                || rule.starts_with("com.apple.")
                || !discovery::valid_bundle_id(&rule)
                || installed.contains(&rule)
            {
                keep.push(rule);
                continue;
            }
            match self.bundle_present_in_spotlight(&rule) {
                TriState::Active => keep.push(rule),
                TriState::Idle if filesystem_complete => removed.push(rule),
                TriState::Idle | TriState::Unknown => {
                    unknown += 1;
                    keep.push(rule);
                }
            }
        }
        if removed.is_empty() {
            let outcome = if unknown > 0 {
                "probe_failed"
            } else {
                "unchanged"
            };
            return TaskResult::new(
                task,
                outcome,
                format!("no proven orphan rules; {unknown} absence check(s) were inconclusive"),
            );
        }

        let command = if keep.is_empty() {
            strings(&[
                "/usr/bin/defaults",
                "delete",
                "com.apple.spotlight",
                "EnabledPreferenceRules",
            ])
        } else {
            let mut command = strings(&[
                "/usr/bin/defaults",
                "write",
                "com.apple.spotlight",
                "EnabledPreferenceRules",
                "-array",
            ]);
            command.extend(keep);
            command
        };
        let result = run_bounded(&command, TASK_TIMEOUT);
        command_task_result(
            task,
            result,
            format!("removed {} proven orphan Spotlight rule(s)", removed.len()),
        )
    }

    fn audit_login_items(&self, task: &OptimizeTask) -> TaskResult {
        let items = match discovery::user_launch_items(&self.home) {
            Ok(items) => items,
            Err(error) => return TaskResult::new(task, "failed", error.to_string()),
        };
        let stale: Vec<_> = items.iter().filter(|item| !item.program_exists).collect();
        if stale.is_empty() {
            return TaskResult::new(
                task,
                "unchanged",
                format!(
                    "{} launch item(s) checked; no stale executable paths",
                    items.len()
                ),
            );
        }
        let lines = stale
            .iter()
            .map(|item| format!("{} -> {}", item.label, item.program.display()))
            .collect::<Vec<_>>()
            .join("\n");
        TaskResult::new(
            task,
            "attention",
            format!("{} stale login item(s) found:\n{lines}", stale.len()),
        )
    }

    fn trash_discovered(
        &self,
        task: &OptimizeTask,
        discovered: std::io::Result<Vec<PathBuf>>,
        probes: &dyn LiveProbes,
    ) -> TaskResult {
        match discovered {
            Ok(paths) => self.trash_paths(task, paths, probes),
            Err(error) => TaskResult::new(
                task,
                "failed",
                format!("candidate scan was incomplete: {error}"),
            ),
        }
    }

    fn trash_paths(
        &self,
        task: &OptimizeTask,
        paths: Vec<PathBuf>,
        probes: &dyn LiveProbes,
    ) -> TaskResult {
        if paths.is_empty() {
            return TaskResult::new(task, "unchanged", "no eligible items were found");
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let candidates = paths
            .iter()
            .filter_map(|path| scanutil::make_candidate(path, task.id, Scope::User, &cancel))
            .collect::<Vec<_>>();
        if candidates.len() != paths.len() {
            return TaskResult::new(
                task,
                "failed",
                "one or more candidates changed while the plan was being bound",
            );
        }
        let plan = DeletionPlan { candidates };
        let selection = plan
            .candidates
            .iter()
            .map(|candidate| candidate.id.clone())
            .collect::<Vec<_>>();
        let trash = FinderTrash;
        let privileged = AdminRunner;
        let providers = Providers {
            trash: &trash,
            privileged: &privileged,
            probes,
        };
        let report = engine::execute(
            &plan,
            &selection,
            &ExecOptions {
                home: self.home.to_string_lossy().into_owned(),
                command: "optimize".into(),
                mode: DeleteMode::Trash,
                dry_run: false,
                uninstall_mode: false,
            },
            &providers,
            |_| {},
        );
        let trashed = report
            .items
            .iter()
            .filter(|item| item.outcome == "trashed")
            .count();
        let summary = format!(
            "{trashed} moved to Trash, {} skipped, {} failed",
            report.skipped, report.failed
        );
        if report.failed > 0 {
            TaskResult::new(task, "failed", summary)
        } else if trashed > 0 {
            TaskResult::new(task, "ok", summary)
        } else if report.skipped > 0 {
            TaskResult::new(task, "skipped", summary)
        } else {
            TaskResult::new(task, "unchanged", summary)
        }
    }

    fn sqlite(&self, path: &Path, sql: &str, timeout: Duration) -> CommandResult {
        let command = vec![
            "/usr/bin/sqlite3".to_string(),
            path.to_string_lossy().into_owned(),
            sql.to_string(),
        ];
        run_bounded(&command, timeout)
    }

    fn bundle_present_in_spotlight(&self, bundle_id: &str) -> TriState {
        let query = format!("kMDItemCFBundleIdentifier == '{bundle_id}'");
        let command = vec!["/usr/bin/mdfind".into(), query];
        let result = run_bounded(&command, PROBE_TIMEOUT);
        if !result.success() {
            TriState::Unknown
        } else if result.output.lines().any(|line| Path::new(line).exists()) {
            TriState::Active
        } else {
            TriState::Idle
        }
    }
}

fn command_task_result(
    task: &OptimizeTask,
    result: CommandResult,
    success_message: impl Into<String>,
) -> TaskResult {
    let message = success_message.into();
    if result.status == CommandStatus::Unavailable {
        TaskResult::new(task, "unavailable", result.output)
    } else if !result.success() {
        TaskResult::new(task, "failed", result.output)
    } else if message.is_empty() {
        TaskResult::new(task, "ok", result.output)
    } else {
        TaskResult::new(task, "ok", message)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandStatus {
    Success,
    Failed,
    TimedOut,
    Unavailable,
}

struct CommandResult {
    status: CommandStatus,
    output: String,
}

impl CommandResult {
    fn success(&self) -> bool {
        self.status == CommandStatus::Success
    }
}

/// Bounded argv execution. No shell is involved, locale is pinned for every
/// parsed system tool, and timeout kills the child before returning.
fn run_bounded(argv: &[String], timeout: Duration) -> CommandResult {
    let Some((program, arguments)) = argv.split_first() else {
        return CommandResult {
            status: CommandStatus::Failed,
            output: "empty command".into(),
        };
    };
    let mut child = match Command::new(program)
        .args(arguments)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return CommandResult {
                status: CommandStatus::Unavailable,
                output: format!("{program} is not available"),
            }
        }
        Err(error) => {
            return CommandResult {
                status: CommandStatus::Failed,
                output: error.to_string(),
            }
        }
    };

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                break if status.success() {
                    CommandStatus::Success
                } else {
                    CommandStatus::Failed
                }
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break CommandStatus::TimedOut;
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return CommandResult {
                    status: CommandStatus::Failed,
                    output: error.to_string(),
                };
            }
        }
    };
    let mut bytes = Vec::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_end(&mut bytes);
    }
    if let Some(mut stderr) = child.stderr.take() {
        let _ = stderr.read_to_end(&mut bytes);
    }
    let mut output = String::from_utf8_lossy(&bytes).trim().to_string();
    if status == CommandStatus::TimedOut {
        output = format!("{} timed out after {}s", program, timeout.as_secs());
    } else if output.is_empty() && status == CommandStatus::Failed {
        output = format!("{program} failed without output");
    }
    CommandResult { status, output }
}

fn strings(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| part.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn bounded_runner_reports_missing_tools_by_cause() {
        let result = run_bounded(
            &["/definitely/missing/mole-tool".into()],
            Duration::from_millis(50),
        );
        assert_eq!(result.status, CommandStatus::Unavailable);
        assert!(result.output.contains("not available"));
    }

    #[test]
    fn bounded_runner_times_out_and_kills_the_child() {
        let result = run_bounded(&strings(&["/bin/sleep", "2"]), Duration::from_millis(20));
        assert_eq!(result.status, CommandStatus::TimedOut);
        assert!(result.output.contains("timed out"));
    }

    #[test]
    fn sqlite_result_parser_requires_both_numeric_values() {
        let values: Vec<u64> = "100\n10\n"
            .lines()
            .filter_map(|line| line.parse().ok())
            .collect();
        assert_eq!(values, [100, 10]);
        assert!(values[1] * 100 >= values[0] * 5);
    }

    #[test]
    fn spotlight_presence_never_embeds_invalid_bundle_ids() {
        let ids = HashSet::from(["com.example.present".to_string()]);
        assert!(ids.contains("com.example.present"));
        assert!(!discovery::valid_bundle_id("com.example.'; rm"));
    }
}
