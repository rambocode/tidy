//! Mole.app-compatible optimization catalog and execution contract.
//!
//! The catalog contains the same 22 maintenance subjects as Mole.app 1.12.1.
//! Discovery stays separate from execution so conditional tasks can report
//! `unchanged` without mutating the machine. File removals are delegated to
//! the shared desktop deletion engine; system maintenance refuses until a
//! signed helper exposes a narrow maintenance transport.

mod catalog;
mod discovery;
mod execute;

use mole_core::probes::{LiveProbes, TriState};
use serde::Serialize;

pub use catalog::tasks;
pub use execute::SystemTaskExecutor;

/// One optimization task shown to the user before it runs.
#[derive(Debug, Clone, Serialize)]
pub struct OptimizeTask {
    /// Stable Mole.app task identity.
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    /// Explain-before-execute steps. External commands are literal argv;
    /// `mole:*` steps name an internal scan or Trash operation.
    pub commands: Vec<Vec<String>>,
    /// True only for tasks that require a signed privileged maintenance call.
    pub requires_admin: bool,
    /// Apps that must be closed. Unknown process state refuses as well.
    pub guard_processes: &'static [&'static str],
}

/// Result of one requested task.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TaskResult {
    pub id: String,
    /// ok / unchanged / skipped / unavailable / attention / failed /
    /// requires_admin / apps_running / probe_failed / unknown-task.
    pub outcome: String,
    pub output: String,
}

impl TaskResult {
    /// Construct a result while preserving the stable task id.
    pub(crate) fn new(task: &OptimizeTask, outcome: &str, output: impl Into<String>) -> Self {
        Self {
            id: task.id.to_string(),
            outcome: outcome.to_string(),
            output: output.into(),
        }
    }
}

/// Task execution boundary. Tests inject a recorder; production uses the
/// bounded executor in `execute.rs`.
pub trait TaskExecutor {
    /// Whether this executor can send named maintenance actions through the
    /// signed helper. Path deletion support alone is not enough.
    fn supports_privileged_maintenance(&self) -> bool {
        false
    }

    /// Execute one already-authorized user-scope task.
    fn execute(&self, task: &OptimizeTask, probes: &dyn LiveProbes) -> TaskResult;
}

/// Execute selected tasks in request order. Every refusal has its own cause;
/// a failed process probe never becomes proof that an app is closed.
pub fn run_tasks(
    catalog: &[OptimizeTask],
    task_ids: &[String],
    executor: &dyn TaskExecutor,
    helper_available: bool,
    probes: &dyn LiveProbes,
) -> Vec<TaskResult> {
    let mut results = Vec::with_capacity(task_ids.len());
    for id in task_ids {
        let Some(task) = catalog.iter().find(|task| task.id == id) else {
            results.push(TaskResult {
                id: id.clone(),
                outcome: "unknown-task".into(),
                output: "task is not in the Mole.app-compatible catalog".into(),
            });
            continue;
        };

        if task.requires_admin && (!helper_available || !executor.supports_privileged_maintenance())
        {
            results.push(TaskResult::new(
                task,
                "requires_admin",
                "the signed helper does not expose this maintenance action; no fallback elevation was attempted",
            ));
            continue;
        }

        if !task.guard_processes.is_empty() {
            match probes.any_process_running(task.guard_processes) {
                TriState::Active => {
                    results.push(TaskResult::new(
                        task,
                        "apps_running",
                        format!(
                            "close {} first, then run this task again",
                            task.guard_processes.join(", ")
                        ),
                    ));
                    continue;
                }
                TriState::Unknown => {
                    results.push(TaskResult::new(
                        task,
                        "probe_failed",
                        format!(
                            "could not verify {} are closed; the task was not run",
                            task.guard_processes.join(", ")
                        ),
                    ));
                    continue;
                }
                TriState::Idle => {}
            }
        }

        results.push(executor.execute(task, probes));
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use mole_core::probes::StubProbes;
    use std::cell::RefCell;

    struct RecordingExecutor(RefCell<Vec<String>>);

    impl TaskExecutor for RecordingExecutor {
        fn execute(&self, task: &OptimizeTask, _probes: &dyn LiveProbes) -> TaskResult {
            self.0.borrow_mut().push(task.id.to_string());
            TaskResult::new(task, "ok", "mocked")
        }
    }

    #[test]
    fn catalog_matches_mole_app_1_12_1_subjects() {
        let catalog = tasks("/Users/tester");
        let ids: Vec<&str> = catalog.iter().map(|task| task.id).collect();
        assert_eq!(
            ids,
            [
                "rebuildQuickLook",
                "rebuildQuickLookThumbnails",
                "flushDNS",
                "reindexSpotlight",
                "memoryPurge",
                "cleanSavedState",
                "cleanQuarantineEvents",
                "cleanCoreDuet",
                "preventNetworkDSStore",
                "auditLegacyOverrides",
                "rebuildFontCache",
                "rebuildLaunchServices",
                "repairDiskPermissions",
                "runPeriodicMaintenance",
                "vacuumSQLiteDatabases",
                "repairSharedFileLists",
                "cleanBrokenLaunchAgents",
                "repairBrokenPlists",
                "pruneNotificationDB",
                "pruneSpotlightOrphanRules",
                "auditLoginItems",
                "flushNetworkStack",
            ]
        );
        assert_eq!(catalog.len(), 22);
        assert!(catalog.iter().all(|task| !task.title.is_empty()));
        assert!(catalog.iter().all(|task| !task.description.is_empty()));
        assert!(catalog.iter().all(|task| !task.commands.is_empty()));
    }

    #[test]
    fn admin_tasks_name_the_transport_refusal_and_never_execute() {
        let catalog = tasks("/Users/tester");
        let executor = RecordingExecutor(Default::default());
        let results = run_tasks(
            &catalog,
            &["flushDNS".into(), "rebuildQuickLook".into()],
            &executor,
            true,
            &StubProbes::idle(),
        );
        assert_eq!(results[0].outcome, "requires_admin");
        assert!(results[0].output.contains("signed helper"));
        assert_eq!(results[1].outcome, "ok");
        assert_eq!(&*executor.0.borrow(), &["rebuildQuickLook"]);
    }

    #[test]
    fn guarded_tasks_refuse_running_and_unknown_process_states() {
        let catalog = tasks("/Users/tester");
        for (state, expected) in [
            (TriState::Active, "apps_running"),
            (TriState::Unknown, "probe_failed"),
        ] {
            let executor = RecordingExecutor(Default::default());
            let probes = StubProbes {
                owner_state: state,
                sqlite_state: TriState::Idle,
            };
            let results = run_tasks(
                &catalog,
                &["vacuumSQLiteDatabases".into()],
                &executor,
                false,
                &probes,
            );
            assert_eq!(results[0].outcome, expected);
            assert!(executor.0.borrow().is_empty());
        }
    }

    #[test]
    fn unknown_task_is_distinct_from_a_failed_task() {
        let executor = RecordingExecutor(Default::default());
        let results = run_tasks(
            &[],
            &["missing".into()],
            &executor,
            false,
            &StubProbes::idle(),
        );
        assert_eq!(results[0].outcome, "unknown-task");
    }
}
