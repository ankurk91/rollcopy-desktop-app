//! iPhone access over usbmuxd + lockdown + AFC: device listing, connection
//! state, device info, and read-only file access. No UI dependency.
//!
//! Read-only towards the phone (PLAN §2a): nothing in this crate's public API
//! can write, rename or delete anything on the device.

mod error;
mod media;
pub mod models;
mod session;
mod watch;

pub use error::{DeviceError, Result};
pub use media::{EntryKind, MediaFs, Stat};
pub use session::{DeviceInfo, DeviceRef, Session, list_devices, pair};
pub use watch::{NoDeviceHint, Phase, apple_usb_present, usbmuxd_installed, watch};
