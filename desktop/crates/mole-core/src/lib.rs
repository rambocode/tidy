// mole-core: Mole's deletion safety layer, ported from lib/core/*.sh.
// This crate is the ONLY place in the desktop workspace allowed to delete
// files; everything else must go through `sink::delete`.

pub mod brand;
pub mod fsutil;
pub mod glob;
pub mod history;
pub mod identity;
pub mod logging;
pub mod plan;
pub mod policy;
pub mod probes;
pub mod providers;
pub mod sink;
pub mod state;
pub mod validate;

pub use plan::{Candidate, DeletionPlan, SizeKb};
pub use sink::{delete, DeleteMode, DeleteOutcome, OpContext};
pub use validate::{validate_path_for_deletion, RejectReason};
