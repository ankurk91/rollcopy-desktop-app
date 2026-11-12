//! Transfer jobs, conflicts, retry, and the error history database
//! (PLAN §7–8). No UI dependency.
//!
//! Read-only towards the phone: jobs only list, stat and read through
//! `rollcopy_device::MediaFs`.

mod history;
mod job;
mod layout;
mod queue;

pub use history::{FailureRecord, History};
pub use layout::{ConflictPolicy, Layout, Settings, dest_path, dest_path_in, output_name};
pub use queue::{JobId, JobState, QueueCommand, QueueEvent, QueueHandle, Stage, spawn};
