//! Copying (PLAN §6 copy actions, bottom transfer queue, §7.5 conflict
//! dialog): starting copies, the destination folder, the queue panel and the
//! "already exists" prompt. The work itself runs in `rollcopy-queue`.

mod actions;
mod dialogs;
mod dock;
mod state;
mod toolbar;

pub(crate) use state::*;
// Shared between the files of this module only.
use state::{PANEL_CHROME_H, ROW_H, spawn_xdg_open, tilde};
