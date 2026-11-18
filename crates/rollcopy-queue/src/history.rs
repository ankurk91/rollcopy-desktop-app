//! Failure history (PLAN §8): `~/.local/state/rollcopy/history.sqlite`, plus
//! CSV export for bug reports.

use std::io::Write;
use std::path::Path;

use rusqlite::{Connection, params};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureRecord {
    /// RFC 3339 local time.
    pub time: String,
    pub udid: String,
    pub source: String,
    pub dest: String,
    pub stage: String,
    pub error: String,
    pub retries: u32,
}

pub struct History {
    conn: Connection,
}

/// Failures kept; older ones are deleted when the history is opened
/// (review R2). Plenty for a bug report, and a few MB at most.
const KEEP: i64 = 5_000;

impl History {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS failures (
                id INTEGER PRIMARY KEY,
                time TEXT NOT NULL,
                udid TEXT NOT NULL,
                source TEXT NOT NULL,
                dest TEXT NOT NULL,
                stage TEXT NOT NULL,
                error TEXT NOT NULL,
                retries INTEGER NOT NULL
            );",
        )?;
        let h = Self { conn };
        h.prune(KEEP)?;
        Ok(h)
    }

    /// Keeps only the newest `keep` failures.
    fn prune(&self, keep: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "DELETE FROM failures WHERE id <= (SELECT MAX(id) - ?1 FROM failures)",
            [keep],
        )?;
        Ok(())
    }

    pub fn record(&self, r: &FailureRecord) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO failures (time, udid, source, dest, stage, error, retries)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                r.time, r.udid, r.source, r.dest, r.stage, r.error, r.retries
            ],
        )?;
        Ok(())
    }

    /// Most recent first.
    pub fn failures(&self, limit: usize) -> rusqlite::Result<Vec<FailureRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT time, udid, source, dest, stage, error, retries
             FROM failures ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |r| {
            Ok(FailureRecord {
                time: r.get(0)?,
                udid: r.get(1)?,
                source: r.get(2)?,
                dest: r.get(3)?,
                stage: r.get(4)?,
                error: r.get(5)?,
                retries: r.get(6)?,
            })
        })?;
        rows.collect()
    }

    /// Writes all failures as CSV ("Export error report").
    pub fn export_csv(&self, out: &Path) -> std::io::Result<usize> {
        let rows = self
            .failures(usize::MAX >> 1)
            .map_err(std::io::Error::other)?;
        let mut f = std::io::BufWriter::new(std::fs::File::create(out)?);
        writeln!(f, "time,udid,source,dest,stage,error,retries")?;
        for r in &rows {
            writeln!(
                f,
                "{},{},{},{},{},{},{}",
                csv(&r.time),
                csv(&r.udid),
                csv(&r.source),
                csv(&r.dest),
                csv(&r.stage),
                csv(&r.error),
                r.retries
            )?;
        }
        f.flush()?;
        Ok(rows.len())
    }
}

/// One CSV cell. A cell starting with `=`, `+`, `-`, `@`, tab or CR gets
/// a leading `'`, so LibreOffice and Excel show it as text instead of
/// running it as a formula (OWASP "CSV injection"; review S6).
fn csv(s: &str) -> String {
    let s = if s.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{s}")
    } else {
        s.to_owned()
    };
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_exports() {
        let dir = tempfile::tempdir().unwrap();
        let h = History::open(&dir.path().join("state/history.sqlite")).unwrap();
        let r = FailureRecord {
            time: "2026-11-12T15:00:00+05:30".into(),
            udid: "U1".into(),
            source: "/DCIM/100APPLE/IMG_1.HEIC".into(),
            dest: "/d/IMG_1.JPG".into(),
            stage: "convert".into(),
            error: "bad \"data\", truncated".into(),
            retries: 1,
        };
        h.record(&r).unwrap();
        assert_eq!(h.failures(10).unwrap(), vec![r]);
        let out = dir.path().join("errors.csv");
        assert_eq!(h.export_csv(&out).unwrap(), 1);
        let text = std::fs::read_to_string(out).unwrap();
        assert!(text.contains("\"bad \"\"data\"\", truncated\""));
    }

    #[test]
    fn cells_never_run_as_formulas() {
        assert_eq!(csv("=HYPERLINK(\"x\")"), "\"'=HYPERLINK(\"\"x\"\")\"");
        assert_eq!(csv("@SUM(A1)"), "'@SUM(A1)");
        assert_eq!(csv("-1+2"), "'-1+2");
        assert_eq!(
            csv("/DCIM/100APPLE/IMG_0001.HEIC"),
            "/DCIM/100APPLE/IMG_0001.HEIC"
        );
        assert_eq!(csv("read failed, retrying"), "\"read failed, retrying\"");
    }

    #[test]
    fn keeps_only_the_newest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.sqlite");
        let h = History::open(&path).unwrap();
        let r = |n: u32| FailureRecord {
            time: format!("2026-11-12T15:00:{n:02}+05:30"),
            udid: "U1".into(),
            source: format!("/DCIM/100APPLE/IMG_{n:04}.HEIC"),
            dest: "/d".into(),
            stage: "read".into(),
            error: "x".into(),
            retries: 0,
        };
        for n in 1..=5 {
            h.record(&r(n)).unwrap();
        }
        h.prune(3).unwrap();
        let left: Vec<_> = h
            .failures(10)
            .unwrap()
            .into_iter()
            .map(|f| f.source)
            .collect();
        assert_eq!(
            left,
            [5, 4, 3].map(|n| format!("/DCIM/100APPLE/IMG_{n:04}.HEIC"))
        );
        // Opening again prunes to KEEP, which leaves these alone.
        drop(h);
        assert_eq!(History::open(&path).unwrap().failures(10).unwrap().len(), 3);
    }
}
