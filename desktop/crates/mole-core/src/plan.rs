// Deletion plans: ONE struct produced by scanning, consumed unchanged by both
// preview and execute. The dry-run/execute split is structural — there is no
// second candidate-discovery path an execute call could diverge into.

use crate::identity::PathIdentity;
use serde::{Deserialize, Serialize};

/// Size in KB with an honest Unknown: the log must never record a multi-GB
/// delete as 0KB just because measurement failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SizeKb {
    Known(u64),
    Unknown,
}

impl SizeKb {
    /// Log-file representation: the number, or the literal string "unknown".
    pub fn log_field(&self) -> String {
        match self {
            SizeKb::Known(kb) => kb.to_string(),
            SizeKb::Unknown => "unknown".to_string(),
        }
    }
}

/// Whether an item needs the privileged helper (system scope) or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scope {
    User,
    System,
}

/// One deletion candidate, identity-bound at scan time and re-verified at the sink.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    /// Stable id the UI selection refers back to.
    pub id: String,
    pub path: String,
    pub size_kb: SizeKb,
    /// Number of filesystem entries inside (1 for a plain file).
    pub item_count: u64,
    pub scope: Scope,
    /// Section/group label for display (e.g. "User caches").
    pub section: String,
    /// Filesystem identity captured when the candidate was discovered.
    pub identity: Option<PathIdentity>,
}

/// A complete plan: the only currency between plan_* and execute_* calls.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeletionPlan {
    pub candidates: Vec<Candidate>,
}

impl DeletionPlan {
    /// Total of the known sizes; unknown items contribute 0 but stay visible.
    pub fn known_total_kb(&self) -> u64 {
        self.candidates
            .iter()
            .map(|c| match c.size_kb {
                SizeKb::Known(kb) => kb,
                SizeKb::Unknown => 0,
            })
            .sum()
    }

    /// Look up a candidate by its stable id (execute-side selection check).
    pub fn find(&self, id: &str) -> Option<&Candidate> {
        self.candidates.iter().find(|c| c.id == id)
    }
}
