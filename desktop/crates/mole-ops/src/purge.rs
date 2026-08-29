// Purge: rebuildable project build artifacts under the user's project
// containers. Only whitelisted artifact names are ever candidates, discovery
// never descends into a found artifact, git state produces report-only
// blocker badges (never a "safe to delete" verdict), and ambiguous names
// ("bin", "target", ...) require a project marker file as evidence.

use crate::scanutil::{self, CancelFlag};
use mole_core::plan::{DeletionPlan, Scope};
use mole_core::state::load_purge_paths;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

/// Canonical purge targets (port of MOLE_PURGE_TARGETS in purge_shared.sh).
pub const PURGE_TARGETS: &[&str] = &[
    "node_modules",
    "target",
    "build",
    "dist",
    "venv",
    ".venv",
    ".pytest_cache",
    ".mypy_cache",
    ".tox",
    ".nox",
    ".ruff_cache",
    ".gradle",
    ".terragrunt-cache",
    "__pycache__",
    ".next",
    ".nuxt",
    ".output",
    "vendor",
    "bin",
    "obj",
    ".turbo",
    ".parcel-cache",
    ".dart_tool",
    ".zig-cache",
    "zig-out",
    ".angular",
    ".svelte-kit",
    ".astro",
    "coverage",
    "DerivedData",
    "Pods",
    ".cxx",
    ".expo",
    ".build",
];

/// Default search paths when the user has no purge_paths config (port of
/// MOLE_PURGE_DEFAULT_SEARCH_PATHS, minus CLI-specific worktree containers).
fn default_search_paths(home: &str) -> Vec<PathBuf> {
    [
        "www",
        "dev",
        "Projects",
        "GitHub",
        "Code",
        "Workspace",
        "Repos",
        "Development",
        "Library/CloudStorage",
    ]
    .iter()
    .map(|p| PathBuf::from(home).join(p))
    .collect()
}

/// Ambiguous artifact names and the project marker that authorizes them:
/// "bin"/"obj" need a .NET solution, "target" needs Cargo/Maven, "build"
/// and "dist" need a build-system file. No marker → not a candidate.
fn artifact_authorized(project: &Path, artifact_name: &str) -> bool {
    let has_any = |names: &[&str]| names.iter().any(|n| project.join(n).exists());
    let has_ext = |ext: &str| {
        std::fs::read_dir(project)
            .map(|entries| {
                entries
                    .flatten()
                    .any(|e| e.path().extension().is_some_and(|x| x == ext))
            })
            .unwrap_or(false)
    };
    match artifact_name {
        "bin" | "obj" => has_ext("sln") || has_ext("csproj"),
        "target" => has_any(&["Cargo.toml", "pom.xml"]),
        "build" | "dist" => has_any(&[
            "package.json",
            "build.gradle",
            "build.gradle.kts",
            "CMakeLists.txt",
            "setup.py",
            "pyproject.toml",
            "Makefile",
        ]),
        "vendor" => has_any(&["composer.json", "go.mod"]),
        _ => true,
    }
}

/// One project's discovered state for the preview.
#[derive(Debug, Serialize)]
pub struct ProjectReport {
    pub root: String,
    /// Report-only blocker badges: dirty / no-git. Never a safety verdict.
    pub blockers: Vec<String>,
}

/// Purge plan plus per-project blocker badges.
#[derive(Debug, Serialize)]
pub struct PurgePlanOutput {
    pub plan: DeletionPlan,
    pub projects: Vec<ProjectReport>,
}

/// Git working-tree blockers, report-only (dirty means "look before you
/// leap", not "unsafe"; clean means nothing beyond that).
fn git_blockers(project: &Path) -> Vec<String> {
    let mut blockers = Vec::new();
    if !project.join(".git").exists() {
        return blockers;
    }
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(project)
        .args(["status", "--porcelain"])
        .output();
    match out {
        Ok(o) if o.status.success() => {
            if !o.stdout.is_empty() {
                blockers.push("dirty".into());
            }
        }
        // A git probe that cannot answer is reported, not assumed clean.
        _ => blockers.push("git-unknown".into()),
    }
    blockers
}

/// Walk one project looking for purge targets (bounded depth, never descending
/// into a found artifact or into .git).
fn find_artifacts(project: &Path, depth: usize, out: &mut Vec<PathBuf>, cancel: &CancelFlag) {
    if depth > 4 || cancel.load(Ordering::Relaxed) {
        return;
    }
    let Ok(entries) = std::fs::read_dir(project) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.is_dir() || meta.file_type().is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".git" {
            continue;
        }
        if PURGE_TARGETS.contains(&name.as_str()) {
            if artifact_authorized(project, &name) {
                out.push(path);
            }
            // Never descend into an artifact (its contents are one unit).
            continue;
        }
        // Hidden non-target dirs stay out of discovery (CLI parity).
        if name.starts_with('.') {
            continue;
        }
        find_artifacts(&path, depth + 1, out, cancel);
    }
}

/// Build the purge plan across the configured (or default) search paths.
/// Two-level container→project probing, matching the CLI's discovery shape.
pub fn build_plan(
    home: &str,
    cancel: &CancelFlag,
    mut progress: impl FnMut(&str),
) -> PurgePlanOutput {
    let search = load_purge_paths(home)
        .map(|v| v.into_iter().map(PathBuf::from).collect::<Vec<_>>())
        .unwrap_or_else(|| default_search_paths(home));

    let mut plan = DeletionPlan::default();
    let mut projects = Vec::new();

    for container in search {
        if !container.is_dir() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&container) else {
            continue;
        };
        for entry in entries.flatten() {
            let project = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&project) else {
                continue;
            };
            if !meta.is_dir() || meta.file_type().is_symlink() {
                continue;
            }
            let base = entry.file_name().to_string_lossy().into_owned();
            if base.starts_with('.') {
                continue;
            }
            if cancel.load(Ordering::Relaxed) {
                // Cancellation returns an EMPTY plan: a timed-out producer
                // must never feed partial output into a deletion flow.
                return PurgePlanOutput {
                    plan: DeletionPlan::default(),
                    projects: Vec::new(),
                };
            }
            progress(&project.to_string_lossy());

            let mut artifacts = Vec::new();
            find_artifacts(&project, 0, &mut artifacts, cancel);
            if artifacts.is_empty() {
                continue;
            }
            let section = format!("Project: {}", project.to_string_lossy());
            let candidates =
                scanutil::parallel_candidates(&artifacts, &section, Scope::User, cancel);
            if candidates.is_empty() {
                continue;
            }
            plan.candidates.extend(candidates);
            projects.push(ProjectReport {
                root: project.to_string_lossy().into_owned(),
                blockers: git_blockers(&project),
            });
        }
    }

    PurgePlanOutput { plan, projects }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    #[test]
    fn ambiguous_targets_need_project_markers() {
        let tmp = tempfile::tempdir().unwrap();
        let rust_proj = tmp.path().join("Projects/rusty");
        std::fs::create_dir_all(rust_proj.join("target/debug")).unwrap();
        std::fs::write(rust_proj.join("Cargo.toml"), b"[package]").unwrap();
        let docs = tmp.path().join("Projects/docs");
        // "target" without Cargo.toml/pom.xml: user data, not an artifact.
        std::fs::create_dir_all(docs.join("target/archive")).unwrap();
        let js = tmp.path().join("Projects/webapp");
        std::fs::create_dir_all(js.join("node_modules/dep")).unwrap();

        let config_dir = tmp
            .path()
            .join(".config")
            .join(mole_core::brand::CONFIG_DIR);
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(
            config_dir.join("purge_paths"),
            format!("{}/Projects\n", tmp.path().display()),
        )
        .unwrap();

        let cancel: CancelFlag = Arc::new(AtomicBool::new(false));
        let out = build_plan(&tmp.path().to_string_lossy(), &cancel, |_| {});
        let paths: Vec<&str> = out
            .plan
            .candidates
            .iter()
            .map(|c| c.path.as_str())
            .collect();
        assert!(paths.iter().any(|p| p.ends_with("rusty/target")));
        assert!(paths.iter().any(|p| p.ends_with("webapp/node_modules")));
        assert!(
            !paths.iter().any(|p| p.contains("docs/target")),
            "unmarked 'target' dir must not be a candidate"
        );
    }

    #[test]
    fn cancelled_discovery_returns_empty_plan() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("Projects/x/node_modules")).unwrap();
        let config_dir = tmp
            .path()
            .join(".config")
            .join(mole_core::brand::CONFIG_DIR);
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(
            config_dir.join("purge_paths"),
            format!("{}/Projects\n", tmp.path().display()),
        )
        .unwrap();
        let cancel: CancelFlag = Arc::new(AtomicBool::new(true));
        let out = build_plan(&tmp.path().to_string_lossy(), &cancel, |_| {});
        assert!(out.plan.candidates.is_empty());
    }
}
