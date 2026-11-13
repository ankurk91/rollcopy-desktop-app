//! Reader for a *local copy* of `PhotoData/Photos.sqlite` (PLAN §3).
//!
//! The schema changes between iOS versions, so columns are detected at
//! runtime: only `ZDIRECTORY` and `ZFILENAME` are required, and every other
//! field is read when present and left empty otherwise.

use std::collections::{HashMap, HashSet};
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

/// Makes a fetched copy self-contained and checks it (audit REL-07): folds
/// the WAL into the main file, then runs `PRAGMA quick_check`. Works on the
/// local copy only, which is why it may open it read-write; the phone's
/// database is never opened.
pub fn prepare(db_path: &Path) -> Result<(), DbError> {
    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
    let check: String = conn.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
    if check != "ok" {
        return Err(DbError::Schema(format!(
            "database copy is damaged: {check}"
        )));
    }
    Ok(())
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
    let ext = columns(&conn, "ZEXTENDEDATTRIBUTES").unwrap_or_default();
    let join_ext = ext.contains("ZASSET");

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
    let extc = |name: &str| {
        if join_ext && ext.contains(name) {
            format!("e.{name}")
        } else {
            "NULL".to_owned()
        }
    };
    let sql = format!(
        "SELECT a.ZDIRECTORY, a.ZFILENAME, {kind}, {subtype}, {date}, {tz}, {w}, {h}, {dur},
                {fav}, {hidden}, {trashed}, {lat}, {lon}, {size},
                a.Z_PK, {app}, {place}, {make}, {model}, {lens}, {iso}, {aperture},
                {shutter}, {focal}, {adjusted}
         FROM ZASSET a {join_clause} {ext_clause}
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
        app = attr("ZIMPORTEDBYDISPLAYNAME"),
        place = attr("ZREVERSELOCATIONDATA"),
        make = extc("ZCAMERAMAKE"),
        model = extc("ZCAMERAMODEL"),
        lens = extc("ZLENSMODEL"),
        iso = extc("ZISO"),
        aperture = extc("ZAPERTURE"),
        shutter = extc("ZSHUTTERSPEED"),
        focal = extc("ZFOCALLENGTH"),
        adjusted = col("ZADJUSTMENTTIMESTAMP"),
        ext_clause = if join_ext {
            "LEFT JOIN ZEXTENDEDATTRIBUTES e ON e.ZASSET = a.Z_PK"
        } else {
            ""
        },
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

    let albums = read_albums(&conn).unwrap_or_default();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], row_to_asset)?;
    let mut out = Vec::new();
    for r in rows {
        if let Some((pk, mut asset)) = r? {
            if let Some(names) = albums.get(&pk) {
                asset.albums = names.clone();
            }
            out.push(asset);
        }
    }
    Ok(out)
}

/// `Z_<digits><suffix>`: Core Data names join columns by entity number,
/// which changes between iOS versions (`Z_34ALBUMS`, `Z_3ASSETS` on iOS 27).
fn is_numbered(col: &str, suffix: &str) -> bool {
    col.strip_prefix("Z_")
        .and_then(|r| r.strip_suffix(suffix))
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// User album titles per asset primary key. Finds the album↔asset join
/// table by its column names; returns an empty map if there is none.
fn read_albums(conn: &Connection) -> rusqlite::Result<HashMap<i64, Vec<String>>> {
    let mut out: HashMap<i64, Vec<String>> = HashMap::new();
    let album_cols = columns(conn, "ZGENERICALBUM").unwrap_or_default();
    if !album_cols.contains("ZTITLE") || !album_cols.contains("ZKIND") {
        return Ok(out);
    }
    let tables: Vec<String> = conn
        .prepare(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name LIKE 'Z\\_%ASSETS' ESCAPE '\\'",
        )?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let found = tables.iter().find_map(|t| {
        let cols = columns(conn, t).ok()?;
        let album = cols.iter().find(|c| is_numbered(c, "ALBUMS"))?.clone();
        let asset = cols.iter().find(|c| is_numbered(c, "ASSETS"))?.clone();
        Some((t.clone(), album, asset))
    });
    let Some((table, album_col, asset_col)) = found else {
        return Ok(out);
    };
    let trashed = if album_cols.contains("ZTRASHEDSTATE") {
        "AND (g.ZTRASHEDSTATE IS NULL OR g.ZTRASHEDSTATE = 0)"
    } else {
        ""
    };
    // ZKIND 2 = albums the user made (verified on iOS 27).
    let sql = format!(
        "SELECT j.{asset_col}, g.ZTITLE FROM {table} j
         JOIN ZGENERICALBUM g ON g.Z_PK = j.{album_col}
         WHERE g.ZKIND = 2 AND g.ZTITLE IS NOT NULL {trashed}
         ORDER BY g.ZTITLE"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
    for row in rows {
        let (pk, title) = row?;
        out.entry(pk).or_default().push(title);
    }
    Ok(out)
}

/// The street address in `ZREVERSELOCATIONDATA`, an NSKeyedArchiver plist
/// whose root object has an `addressString`.
pub fn place_from_archive(bytes: &[u8]) -> Option<String> {
    let v = plist::Value::from_reader(std::io::Cursor::new(bytes)).ok()?;
    let dict = v.as_dictionary()?;
    let objects = dict.get("$objects")?.as_array()?;
    fn deref<'a>(objects: &'a [plist::Value], v: &'a plist::Value) -> Option<&'a plist::Value> {
        match v {
            plist::Value::Uid(u) => objects.get(u.get() as usize),
            other => Some(other),
        }
    }
    let top = dict.get("$top")?.as_dictionary()?.get("root")?;
    let root = deref(objects, top)?.as_dictionary()?;
    let address = deref(objects, root.get("addressString")?)?
        .as_string()?
        .trim();
    (!address.is_empty()).then(|| address.to_owned())
}

fn camera_name(make: Option<String>, model: Option<String>) -> Option<String> {
    let model = model.map(|m| m.trim().to_owned()).filter(|m| !m.is_empty());
    let make = make.map(|m| m.trim().to_owned()).filter(|m| !m.is_empty());
    match (make, model) {
        (Some(make), Some(model)) => {
            // "EXAMPLE CORPORATION" + "EXAMPLE X100" → "EXAMPLE X100".
            let brand = make.split_whitespace().next().unwrap_or("");
            if model.to_lowercase().starts_with(&brand.to_lowercase()) {
                Some(model)
            } else {
                Some(format!("{make} {model}"))
            }
        }
        (None, Some(model)) => Some(model),
        (Some(make), None) => Some(make),
        (None, None) => None,
    }
}

fn exposure_text(
    iso: Option<f64>,
    aperture: Option<f64>,
    shutter: Option<f64>,
    focal: Option<f64>,
) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(iso) = iso.filter(|v| *v > 0.0) {
        parts.push(format!("ISO {}", iso.round() as i64));
    }
    if let Some(f) = aperture.filter(|v| *v > 0.0) {
        parts.push(format!("f/{f:.1}"));
    }
    if let Some(s) = shutter.filter(|v| *v > 0.0) {
        parts.push(if s < 1.0 {
            format!("1/{} s", (1.0 / s).round() as i64)
        } else {
            format!("{s:.1} s")
        });
    }
    if let Some(mm) = focal.filter(|v| *v > 0.0) {
        let t = format!("{mm:.2}");
        parts.push(format!(
            "{} mm",
            t.trim_end_matches('0').trim_end_matches('.')
        ));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn row_to_asset(r: &Row<'_>) -> rusqlite::Result<Option<(i64, Asset)>> {
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
    let pk: i64 = r.get(15)?;
    a.source_app = r
        .get::<_, Option<String>>(16)?
        .map(|s| {
            s.trim_matches(|c: char| c.is_whitespace() || c == '\u{200e}')
                .to_owned()
        })
        .filter(|s| !s.is_empty());
    a.place = r
        .get::<_, Option<Vec<u8>>>(17)?
        .and_then(|b| place_from_archive(&b));
    a.camera = camera_name(r.get(18)?, r.get(19)?);
    a.lens = r
        .get::<_, Option<String>>(20)?
        .filter(|s| !s.trim().is_empty());
    a.exposure = exposure_text(r.get(21)?, r.get(22)?, r.get(23)?, r.get(24)?);
    a.edited_at = r
        .get::<_, Option<f64>>(25)?
        .map(|s| (s + APPLE_EPOCH_OFFSET) as i64);
    Ok(Some((pk, a)))
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
    fn reads_source_camera_place_albums() {
        let d = db(
            &format!(
                "{FULL}
                ALTER TABLE ZADDITIONALASSETATTRIBUTES ADD COLUMN ZIMPORTEDBYDISPLAYNAME TEXT;
                CREATE TABLE ZEXTENDEDATTRIBUTES (Z_PK INTEGER PRIMARY KEY, ZASSET INTEGER,
                    ZCAMERAMAKE TEXT, ZCAMERAMODEL TEXT, ZLENSMODEL TEXT, ZISO INTEGER,
                    ZAPERTURE REAL, ZSHUTTERSPEED REAL, ZFOCALLENGTH REAL);
                CREATE TABLE ZGENERICALBUM (Z_PK INTEGER PRIMARY KEY, ZKIND INTEGER, ZTITLE TEXT,
                    ZTRASHEDSTATE INTEGER);
                CREATE TABLE Z_33KEYASSETS (Z_33ALBUMSBEINGKEYASSETS INTEGER, Z_3KEYASSETS INTEGER);
                CREATE TABLE Z_34ASSETS (Z_34ALBUMS INTEGER, Z_3ASSETS INTEGER, Z_FOK_3ASSETS INTEGER);"
            ),
            "INSERT INTO ZASSET (Z_PK, ZDIRECTORY, ZFILENAME, ZKIND) VALUES
                (1,'DCIM/100APPLE','IMG_1.HEIC',0),(2,'DCIM/100APPLE','IMG_2.JPG',0);
             INSERT INTO ZADDITIONALASSETATTRIBUTES (Z_PK, ZASSET, ZIMPORTEDBYDISPLAYNAME)
                VALUES (1,1,'Camera'),(2,2,'\u{200e}WhatsApp');
             INSERT INTO ZEXTENDEDATTRIBUTES VALUES
                (1,1,'Apple','iPhone 15','iPhone 15 back camera',160,1.6,0.0303,5.96),
                (2,2,'EXAMPLE CORPORATION','EXAMPLE X100',NULL,NULL,NULL,NULL,NULL);
             INSERT INTO ZGENERICALBUM VALUES (51,2,'Holiday',0),(52,2,'Trips',0),
                (53,2,'Old',1),(4,3571,'progress-sync',0);
             INSERT INTO Z_34ASSETS VALUES (51,1,0),(52,1,0),(53,1,0),(4,1,0);
             INSERT INTO Z_33KEYASSETS VALUES (52,2);",
        );
        let assets = read_assets(&d.path().join("Photos.sqlite")).unwrap();
        let a = assets.iter().find(|a| a.filename == "IMG_1.HEIC").unwrap();
        assert_eq!(a.source_app.as_deref(), Some("Camera"));
        assert_eq!(a.camera.as_deref(), Some("Apple iPhone 15"));
        assert_eq!(
            a.exposure.as_deref(),
            Some("ISO 160 · f/1.6 · 1/33 s · 5.96 mm")
        );
        assert_eq!(a.albums, vec!["Holiday".to_owned(), "Trips".to_owned()]);
        let b = assets.iter().find(|a| a.filename == "IMG_2.JPG").unwrap();
        assert_eq!(b.source_app.as_deref(), Some("WhatsApp"));
        assert_eq!(b.camera.as_deref(), Some("EXAMPLE X100"));
        assert!(
            b.albums.is_empty(),
            "key-asset links are not album membership"
        );
    }

    #[test]
    fn decodes_place_archive() {
        use plist::{Dictionary, Uid, Value};
        let mut root = Dictionary::new();
        root.insert("addressString".into(), Value::Uid(Uid::new(2)));
        let mut top = Dictionary::new();
        top.insert("root".into(), Value::Uid(Uid::new(1)));
        let mut archive = Dictionary::new();
        archive.insert("$top".into(), Value::Dictionary(top));
        archive.insert(
            "$objects".into(),
            Value::Array(vec![
                Value::String("$null".into()),
                Value::Dictionary(root),
                Value::String("1 Example Street, Springfield".into()),
            ]),
        );
        let mut bytes = Vec::new();
        Value::Dictionary(archive)
            .to_writer_binary(&mut bytes)
            .unwrap();
        assert_eq!(
            place_from_archive(&bytes).as_deref(),
            Some("1 Example Street, Springfield")
        );
        assert_eq!(place_from_archive(b"junk"), None);
    }

    #[test]
    fn reads_edit_time() {
        let d = db(
            "CREATE TABLE ZASSET (Z_PK INTEGER PRIMARY KEY, ZDIRECTORY TEXT, ZFILENAME TEXT,
                ZKIND INTEGER, ZADJUSTMENTTIMESTAMP REAL);",
            "INSERT INTO ZASSET VALUES (1,'DCIM/100APPLE','IMG_1.HEIC',0,778377600),
                (2,'DCIM/100APPLE','IMG_2.HEIC',0,NULL);",
        );
        let assets = read_assets(&d.path().join("Photos.sqlite")).unwrap();
        // 778377600 s after 2001-01-01 = 2025-09-01T00:00:00Z = 1756684800.
        assert_eq!(assets[0].edited_at, Some(1_756_684_800));
        assert_eq!(assets[1].edited_at, None);
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

    /// A copy taken while iOS was writing: the newest rows are only in the
    /// WAL, and there is no `-shm` (it isn't copied).
    #[test]
    fn wal_rows_are_read_without_shm() {
        let live = tempfile::tempdir().unwrap();
        let path = live.path().join("Photos.sqlite");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;
             CREATE TABLE ZASSET (Z_PK INTEGER PRIMARY KEY, ZDIRECTORY TEXT, ZFILENAME TEXT);
             INSERT INTO ZASSET VALUES (1, 'DCIM/100APPLE', 'IMG_0001.HEIC');",
        )
        .unwrap();
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .unwrap();
        conn.execute(
            "INSERT INTO ZASSET VALUES (2, 'DCIM/100APPLE', 'IMG_0002.HEIC')",
            [],
        )
        .unwrap();

        // Copy as fetched: main file and WAL, while the writer is still open.
        let copy = tempfile::tempdir().unwrap();
        let dst = copy.path().join("Photos.sqlite");
        std::fs::copy(&path, &dst).unwrap();
        std::fs::copy(
            live.path().join("Photos.sqlite-wal"),
            copy.path().join("Photos.sqlite-wal"),
        )
        .unwrap();
        drop(conn);

        prepare(&dst).unwrap();
        let names: Vec<String> = read_assets(&dst)
            .unwrap()
            .into_iter()
            .map(|a| a.filename)
            .collect();
        assert_eq!(names.len(), 2, "{names:?}");
    }

    #[test]
    fn damaged_copy_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Photos.sqlite");
        std::fs::write(&path, b"SQLite format 3\0 but then garbage").unwrap();
        assert!(prepare(&path).is_err());
    }
}
