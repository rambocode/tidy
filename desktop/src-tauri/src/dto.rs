// IPC payload shapes shared by the plan/execute commands. `ui/src/types.ts`
// mirrors these by hand — keep field names in lockstep.

use mole_core::plan::{DeletionPlan, Scope, SizeKb};
use serde::Serialize;

/// One candidate row the UI can select.
#[derive(Debug, Serialize)]
pub struct PlanItem {
    pub id: String,
    pub path: String,
    /// None = size unknown (never shown as 0).
    pub size_kb: Option<u64>,
    /// Filesystem entries inside (1 for a plain file).
    pub item_count: u64,
    /// "user" | "system" — system items need the privileged helper.
    pub scope: &'static str,
}

/// A titled group of candidates.
#[derive(Debug, Serialize)]
pub struct PlanSection {
    pub title: String,
    pub items: Vec<PlanItem>,
    pub total_kb: u64,
}

/// The preview payload for any plan_* command.
#[derive(Debug, Serialize)]
pub struct PlanSummary {
    pub plan_id: String,
    pub sections: Vec<PlanSection>,
    pub total_kb: u64,
    pub count: usize,
}

/// Progress event for long scans.
#[derive(Debug, Clone, Serialize)]
pub struct ScanEvent {
    pub label: String,
    pub count: usize,
}

/// Group a stored plan into the section-shaped preview, preserving section
/// discovery order.
pub fn summarize(plan_id: String, plan: &DeletionPlan) -> PlanSummary {
    let mut sections: Vec<PlanSection> = Vec::new();
    for candidate in &plan.candidates {
        let size = match candidate.size_kb {
            SizeKb::Known(kb) => Some(kb),
            SizeKb::Unknown => None,
        };
        let item = PlanItem {
            id: candidate.id.clone(),
            path: candidate.path.clone(),
            size_kb: size,
            item_count: candidate.item_count,
            scope: match candidate.scope {
                Scope::User => "user",
                Scope::System => "system",
            },
        };
        match sections.iter_mut().find(|s| s.title == candidate.section) {
            Some(section) => {
                section.total_kb += size.unwrap_or(0);
                section.items.push(item);
            }
            None => sections.push(PlanSection {
                title: candidate.section.clone(),
                total_kb: size.unwrap_or(0),
                items: vec![item],
            }),
        }
    }
    // Big sections first, and inside each the big items first.
    for s in &mut sections {
        s.items
            .sort_by_key(|i| std::cmp::Reverse(i.size_kb.unwrap_or(0)));
    }
    sections.sort_by_key(|s| std::cmp::Reverse(s.total_kb));
    PlanSummary {
        total_kb: plan.known_total_kb(),
        count: plan.candidates.len(),
        plan_id,
        sections,
    }
}
