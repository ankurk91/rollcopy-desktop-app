//! Reads the Photos library (`PhotoData/Photos.sqlite`) into an item model,
//! falling back to plain `DCIM/` browsing. No UI dependency.

mod asset;
mod load;
pub mod photos_db;

pub use asset::{Asset, MediaKind, kind_from_extension};
pub use load::{Library, Source, apply_edits, fetch_photos_db, load, scan_dcim, scan_edits};
