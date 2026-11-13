//! Transfer jobs, conflicts, retry, and the error history database
//! (PLAN §7–8). No UI dependency.
//!
//! Read-only towards the phone: jobs only list, stat and read through
//! `rollcopy_device::MediaFs`.

mod history;
mod job;
mod layout;
mod queue;
mod space;

pub use history::{FailureRecord, History};
pub use layout::{
    ConflictPolicy, Layout, Settings, Target, dest_path, dest_path_in, find_copy, is_copy_of,
    output_name, resolve_dest,
};
pub use queue::{
    DeviceOpener, JobId, JobState, Opener, QueueCommand, QueueEvent, QueueHandle, Stage, spawn,
    spawn_with,
};
