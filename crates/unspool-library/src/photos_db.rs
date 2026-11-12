//! Reader for a *local copy* of `PhotoData/Photos.sqlite` (PLAN §3).
//!
//! The schema changes between iOS versions, so columns are detected at
//! runtime: only `ZDIRECTORY` and `ZFILENAME` are required, and every other
//! field is read when present and left empty otherwise.

use std::collections::HashSet;
use std::path::Path;

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OpenFlags, Row};

use crate::asset::{Asset, MediaKind, kind_from_extension};

/// Seconds between the Unix epoch and Apple's (2001-01-01T00:00:00Z).
const APPLE_EPOCH_OFFSET: f64 = 978_307_200.0;
/// `ZKINDSUBTYPE` value for Live Photos (verified on iOS 27).
const SUBTYPE_LIVE_PHOTO: i64 = 2;
/// Photos stores "no location" as -180/-180 rather than NULL.
const NO_COORD: f64 = -180.0;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("unsupported Photos.sqlite schema: {0}")]
    Schema(String),
}

fn columns(conn: &Connection, table: &str) -> rusqlite::Result<HashSet<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = stmt.query_map([], |r| r.get::<_, String>(1))?;
    names.collect()
}

/// Opens the database copy read-only and returns every asset row, trashed
/// ones included (callers decide what to show).
pub fn read_assets(db_path: &Path) -> Result<Vec<Asset>, DbError> {
    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;

    let a = columns(&conn, "ZASSET")?;
    if !(a.contains("ZDIRECTORY") && a.contains("ZFILENAME")) {
        return Err(DbError::Schema(
            "ZASSET has no ZDIRECTORY/ZFILENAME columns".into(),
        ));
    }
    let attrs = columns(&conn, "ZADDITIONALASSETATTRIBUTES").unwrap_or_default();
    let join = attrs.contains("ZASSET");

    // Each optional column becomes `NULL` when missing, keeping indexes fixed.
    let col = |name: &str| {
        if a.contains(name) {
            format!("a.{name}")
        } else {
            "NULL".to_owned()
        }
    };
    let attr = |name: &str| {
        if join && attrs.contains(name) {
            format!("x.{name}")
        } else {
            "NULL".to_owned()
        }
    };
    let sql = format!(
        "SELECT a.ZDIRECTORY, a.ZFILENAME, {kind}, {subtype}, {date}, {tz}, {w}, {h}, {dur},
                {fav}, {hidden}, {trashed}, {lat}, {lon}, {size}
         FROM ZASSET a {join_clause}
         WHERE a.ZDIRECTORY IS NOT NULL AND a.ZFILENAME IS NOT NULL
         ORDER BY {order}",
        kind = col("ZKIND"),
        subtype = col("ZKINDSUBTYPE"),
        date = col("ZDATECREATED"),
        tz = attr("ZTIMEZONEOFFSET"),
        w = col("ZWIDTH"),
        h = col("ZHEIGHT"),
        dur = col("ZDURATION"),
        fav = col("ZFAVORITE"),
        hidden = col("ZHIDDEN"),
        trashed = col("ZTRASHEDSTATE"),
        lat = col("ZLATITUDE"),
        lon = col("ZLONGITUDE"),
        size = attr("ZORIGINALFILESIZE"),
        join_clause = if join {
            "LEFT JOIN ZADDITIONALASSETATTRIBUTES x ON x.ZASSET = a.Z_PK"
        } else {
            ""
        },
        order = if a.contains("ZDATECREATED") {
            "a.ZDATECREATED, a.ZFILENAME"
        } else {
            "a.ZFILENAME"
        },
    );

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], row_to_asset)?;
    let mut out = Vec::new();
    for r in rows {
        if let Some(asset) = r? {
            out.push(asset);
        }
    }
    Ok(out)
}

fn row_to_asset(r: &Row<'_>) -> rusqlite::Result<Option<Asset>> {
    let directory: String = r.get(0)?;
    let filename: String = r.get(1)?;
    let zkind: Option<i64> = r.get(2)?;
    let kind = match zkind {
        Some(0) => MediaKind::Photo,
        Some(1) => MediaKind::Video,
        // Unknown or missing ZKIND: trust the extension; skip non-media.
        _ => match kind_from_extension(&filename) {
            Some(k) => k,
            None => return Ok(None),
        },
    };
    let mut a = Asset::new(directory, filename, kind);

    let subtype: Option<i64> = r.get(3)?;
    a.live_photo = kind == MediaKind::Photo && subtype == Some(SUBTYPE_LIVE_PHOTO);
    a.captured = r.get::<_, Option<f64>>(4)?.and_then(apple_time);
    a.tz_offset_secs = r
        .get::<_, Option<i64>>(5)?
        .and_then(|v| i32::try_from(v).ok());
    a.width = r.get::<_, Option<i64>>(6)?.and_then(positive_u32);
    a.height = r.get::<_, Option<i64>>(7)?.and_then(positive_u32);
    a.duration_secs = r.get::<_, Option<f64>>(8)?.filter(|d| *d > 0.0);
    a.favorite = r.get::<_, Option<i64>>(9)?.unwrap_or(0) != 0;
    a.hidden = r.get::<_, Option<i64>>(10)?.unwrap_or(0) != 0;
    a.trashed = r.get::<_, Option<i64>>(11)?.unwrap_or(0) == 1;
    let lat: Option<f64> = r.get(12)?;
    let lon: Option<f64> = r.get(13)?;
    a.location = match (lat, lon) {
        (Some(la), Some(lo)) if la != NO_COORD && lo != NO_COORD => Some((la, lo)),
        _ => None,
    };
    a.original_size = r
        .get::<_, Option<i64>>(14)?
        .and_then(|v| u64::try_from(v).ok());
    Ok(Some(a))
}

fn apple_time(secs: f64) -> Option<DateTime<Utc>> {
    let unix = secs + APPLE_EPOCH_OFFSET;
    DateTime::from_timestamp(unix.floor() as i64, ((unix.fract()) * 1e9) as u32)
}

fn positive_u32(v: i64) -> Option<u32> {
    u32::try_from(v).ok().filter(|v| *v > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db(schema: &str, rows: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let conn = Connection::open(dir.path().join("Photos.sqlite")).unwrap();
        conn.execute_batch(schema).unwrap();
        conn.execute_batch(rows).unwrap();
        dir
    }

    const FULL: &str = "
        CREATE TABLE ZASSET (Z_PK INTEGER PRIMARY KEY, ZDIRECTORY TEXT, ZFILENAME TEXT,
            ZKIND INTEGER, ZKINDSUBTYPE INTEGER, ZDATECREATED REAL, ZWIDTH INTEGER,
            ZHEIGHT INTEGER, ZDURATION REAL, ZFAVORITE INTEGER, ZHIDDEN INTEGER,
            ZTRASHEDSTATE INTEGER, ZLATITUDE REAL, ZLONGITUDE REAL);
        CREATE TABLE ZADDITIONALASSETATTRIBUTES (Z_PK INTEGER PRIMARY KEY, ZASSET INTEGER,
            ZTIMEZONEOFFSET INTEGER, ZORIGINALFILESIZE INTEGER);";

    #[test]
    fn reads_full_schema() {
        let d = db(
            FULL,
            "INSERT INTO ZASSET VALUES
               (1,'DCIM/100APPLE','IMG_0001.HEIC',0,2,778377600.5,4032,3024,0,1,0,0,12.5,77.25),
               (2,'DCIM/100APPLE','IMG_0002.MOV',1,0,778377700,1920,1080,42.5,0,0,1,-180,-180);
             INSERT INTO ZADDITIONALASSETATTRIBUTES VALUES (1,1,19800,1234567),(2,2,NULL,NULL);",
        );
        let assets = read_assets(&d.path().join("Photos.sqlite")).unwrap();
        assert_eq!(assets.len(), 2);

        let p = &assets[0];
        assert_eq!(p.kind, MediaKind::Photo);
        assert!(p.live_photo && p.favorite && !p.trashed);
        // 778377600 s after 2001-01-01 = 2025-09-01T00:00:00Z
        assert_eq!(
            p.captured.unwrap().to_rfc3339(),
            "2025-09-01T00:00:00.500+00:00"
        );
        assert_eq!(p.tz_offset_secs, Some(19800));
        assert_eq!((p.width, p.height), (Some(4032), Some(3024)));
        assert_eq!(p.location, Some((12.5, 77.25)));
        assert_eq!(p.original_size, Some(1234567));

        let v = &assets[1];
        assert_eq!(v.kind, MediaKind::Video);
        assert!(v.trashed && !v.live_photo);
        assert_eq!(v.duration_secs, Some(42.5));
        assert_eq!(v.location, None, "-180 means no location");
    }

    #[test]
    fn tolerates_minimal_schema() {
        let d = db(
            "CREATE TABLE ZASSET (Z_PK INTEGER PRIMARY KEY, ZDIRECTORY TEXT, ZFILENAME TEXT);",
            "INSERT INTO ZASSET VALUES (1,'DCIM/100APPLE','IMG_1.JPG'),(2,'DCIM/100APPLE','IMG_1.AAE');",
        );
        let assets = read_assets(&d.path().join("Photos.sqlite")).unwrap();
        assert_eq!(assets.len(), 1, "non-media rows are skipped");
        assert_eq!(assets[0].kind, MediaKind::Photo);
        assert_eq!(assets[0].captured, None);
    }

    #[test]
    fn rejects_unrelated_db() {
        let d = db("CREATE TABLE ZASSET (Z_PK INTEGER PRIMARY KEY);", "");
        assert!(matches!(
            read_assets(&d.path().join("Photos.sqlite")),
            Err(DbError::Schema(_))
        ));
    }
}
