//! A small log file for bug reports (audit MNT-04). Started from the app
//! menu, stderr goes nowhere, so lines also go to
//! `~/.local/state/rollcopy/rollcopy.log`, which is kept under about 1 MB
//! (the previous one is kept as `rollcopy.log.1`). Only events and errors are
//! logged: no device names, serials or file names.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const MAX_BYTES: u64 = 1024 * 1024;

/// The open log file, and how big it is, to rotate while running too: a
/// tray session can last weeks (review R1).
struct LogFile {
    file: File,
    path: PathBuf,
    size: u64,
}

impl LogFile {
    /// Opens `path` for appending, first moving it to `.1` if it's over
    /// [`MAX_BYTES`].
    fn open(path: &Path, max: u64) -> Option<Self> {
        let size = std::fs::metadata(path).map_or(0, |m| m.len());
        if size > max {
            let _ = std::fs::rename(path, rotated(path));
        }
        let file = File::options().create(true).append(true).open(path).ok()?;
        let size = file.metadata().map_or(0, |m| m.len());
        Some(Self {
            file,
            path: path.to_path_buf(),
            size,
        })
    }

    fn write_line(&mut self, line: &str, max: u64) {
        if writeln!(self.file, "{line}").is_ok() {
            self.size += line.len() as u64 + 1;
        }
        if self.size > max
            && let Some(fresh) = Self::open(&self.path, max)
        {
            *self = fresh;
        }
    }
}

fn rotated(path: &Path) -> PathBuf {
    let mut old = path.as_os_str().to_owned();
    old.push(".1");
    PathBuf::from(old)
}

static FILE: Mutex<Option<LogFile>> = Mutex::new(None);

/// Opens the log file; until then lines go to stderr only.
pub fn init(path: &Path) {
    if let Some(dir) = path.parent() {
        let _ = crate::config::create_private_dir(dir);
    }
    *FILE.lock().unwrap_or_else(|e| e.into_inner()) = LogFile::open(path, MAX_BYTES);
}

/// Writes one line to stderr and the log file.
pub fn write(args: std::fmt::Arguments<'_>) {
    let line = format!(
        "{} {args}",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    );
    eprintln!("{line}");
    if let Some(f) = FILE.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        f.write_line(&line, MAX_BYTES);
    }
}

/// `applog!("…", args)`: like `eprintln!`, and into the log file.
#[macro_export]
macro_rules! applog {
    ($($arg:tt)*) => {
        $crate::log::write(format_args!($($arg)*))
    };
}

#[cfg(test)]
mod tests {
    use super::{LogFile, rotated};

    /// Past the limit while running, the log moves to `.1` and starts again.
    #[test]
    fn rotates_while_running() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollcopy.log");
        let mut log = LogFile::open(&path, 100).unwrap();
        for _ in 0..5 {
            log.write_line(&"x".repeat(30), 100);
        }
        let old = std::fs::read_to_string(rotated(&path)).unwrap();
        assert_eq!(old.lines().count(), 4);
        let now = std::fs::read_to_string(&path).unwrap();
        assert_eq!(now.lines().count(), 1);
    }
}
