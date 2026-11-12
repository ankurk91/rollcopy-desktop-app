//! Failure history (PLAN §8): `~/.local/state/unspool/history.sqlite`, plus
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
        Ok(Self { conn })
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

fn csv(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
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
}
