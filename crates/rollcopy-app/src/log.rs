//! A small log file for bug reports (audit MNT-04). Started from the app
//! menu, stderr goes nowhere, so lines also go to
//! `~/.local/state/rollcopy/rollcopy.log`, which is kept under about 1 MB
//! (the previous one is kept as `rollcopy.log.1`). Only events and errors are
//! logged: no device names, serials or file names.

use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

const MAX_BYTES: u64 = 1024 * 1024;

static FILE: Mutex<Option<File>> = Mutex::new(None);

/// Opens the log file; until then lines go to stderr only.
pub fn init(path: &Path) {
    if let Some(dir) = path.parent() {
        let _ = crate::config::create_private_dir(dir);
    }
    if std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let mut old = path.as_os_str().to_owned();
        old.push(".1");
        let _ = std::fs::rename(path, old);
    }
    let file = File::options().create(true).append(true).open(path).ok();
    *FILE.lock().unwrap_or_else(|e| e.into_inner()) = file;
}

/// Writes one line to stderr and the log file.
pub fn write(args: std::fmt::Arguments<'_>) {
    let line = format!(
        "{} {args}",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    );
    eprintln!("{line}");
    if let Some(f) = FILE.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        let _ = writeln!(f, "{line}");
    }
}

/// `applog!("…", args)`: like `eprintln!`, and into the log file.
#[macro_export]
macro_rules! applog {
    ($($arg:tt)*) => {
        $crate::log::write(format_args!($($arg)*))
    };
}
