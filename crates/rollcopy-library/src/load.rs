use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rollcopy_device::{DeviceError, EntryKind, MediaFs, Session};

use crate::asset::{Asset, MediaKind, kind_from_extension};
use crate::photos_db;

/// Files fetched from `/PhotoData`. The `-shm` index is never copied: it
/// only describes the phone's live WAL and could mislead SQLite here.
const DB_FILES: [&str; 2] = ["Photos.sqlite", "Photos.sqlite-wal"];
/// Local files removed before a fetch and after reading.
const LOCAL_FILES: [&str; 3] = ["Photos.sqlite", "Photos.sqlite-wal", "Photos.sqlite-shm"];
/// Fetches tried when the database changes on the phone while it's copied.
const SNAPSHOT_TRIES: usize = 3;

/// Where the item list came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    PhotosDb,
    /// Plain `DCIM/` listing; `reason` says why the database wasn't used.
    DcimScan {
        reason: String,
    },
}

#[derive(Debug, Clone)]
pub struct Library {
    pub assets: Vec<Asset>,
    pub source: Source,
}

/// Copies `Photos.sqlite` and its `-wal` from the phone into `dest_dir`
/// (PLAN §2a: never opened in place). Each file goes to a `.part` file
/// first and is renamed when complete.
///
/// iOS keeps writing while the two files are copied one after the other, so
/// both are stat'ed before and after; if either changed, the copy is taken
/// again, up to [`SNAPSHOT_TRIES`] times (audit REL-07). Local leftovers
/// (an old `-wal`, any `-shm`) are removed first, so SQLite never pairs a
/// stale WAL with a newer database.
pub async fn fetch_photos_db(fs: &mut MediaFs, dest_dir: &Path) -> Result<PathBuf, DeviceError> {
    tokio::fs::create_dir_all(dest_dir).await?;
    for attempt in 1..=SNAPSHOT_TRIES {
        for name in LOCAL_FILES {
            match tokio::fs::remove_file(dest_dir.join(name)).await {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            }
        }
        let before = remote_state(fs).await?;
        if before[0].is_none() {
            return Err(DeviceError::NotFound(format!("/PhotoData/{}", DB_FILES[0])));
        }
        for (name, state) in DB_FILES.iter().zip(&before) {
            if state.is_none() {
                continue;
            }
            let part = dest_dir.join(format!("{name}.part"));
            fs.copy_to_local(&format!("/PhotoData/{name}"), &part, |_| {})
                .await?;
            tokio::fs::rename(&part, dest_dir.join(name)).await?;
        }
        if remote_state(fs).await? == before || attempt == SNAPSHOT_TRIES {
            // After the last try, use what we have: `prepare` checks it.
            break;
        }
    }
    Ok(dest_dir.join(DB_FILES[0]))
}

/// Size and modified time of each of [`DB_FILES`] on the phone.
async fn remote_state(
    fs: &mut MediaFs,
) -> Result<[Option<(u64, std::time::SystemTime)>; 2], DeviceError> {
    let mut out = [None, None];
    for (slot, name) in out.iter_mut().zip(DB_FILES) {
        match fs.stat(&format!("/PhotoData/{name}")).await {
            Ok(st) => *slot = Some((st.size, st.modified)),
            Err(DeviceError::NotFound(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(out)
}

/// Fetches the database and reads it, fetching once more if the copy
/// turns out damaged. The copy is deleted afterwards either way.
async fn read_from_db(
    fs: &mut MediaFs,
    dir: &Path,
) -> Result<Result<Vec<Asset>, String>, DeviceError> {
    let mut last = String::new();
    for _ in 0..2 {
        let path = match fetch_photos_db(fs, dir).await {
            Ok(p) => p,
            Err(e) => {
                remove_photos_db(dir);
                if e.is_connection_problem() {
                    return Err(e);
                }
                return Ok(Err(e.to_string()));
            }
        };
        let read = tokio::task::spawn_blocking(move || {
            photos_db::prepare(&path)?;
            photos_db::read_assets(&path)
        })
        .await
        .map_err(|e| DeviceError::Other(e.to_string()))?;
        remove_photos_db(dir);
        match read {
            Ok(assets) => return Ok(Ok(assets)),
            Err(e) => last = e.to_string(),
        }
    }
    Ok(Err(last))
}

/// Finds edited versions: `PhotoData/Mutations/<dir>/<stem>/Adjustments/
/// FullSizeRender.<ext>`. Keyed by (directory, stem).
pub async fn scan_edits(
    fs: &mut MediaFs,
    directories: &BTreeSet<String>,
) -> Result<HashMap<(String, String), String>, DeviceError> {
    let mut out = HashMap::new();
    for dir in directories {
        let base = format!("/PhotoData/Mutations/{dir}");
        let stems = match fs.list_dir(&base).await {
            Ok(s) => s,
            Err(DeviceError::NotFound(_)) => continue,
            Err(e) => return Err(e),
        };
        for stem in stems {
            let adj = format!("{base}/{stem}/Adjustments");
            let names = match fs.list_dir(&adj).await {
                Ok(n) => n,
                Err(DeviceError::NotFound(_)) => continue,
                Err(e) => return Err(e),
            };
            if let Some(render) = names.iter().find(|n| n.starts_with("FullSizeRender.")) {
                out.insert((dir.clone(), stem), format!("{adj}/{render}"));
            }
        }
    }
    Ok(out)
}

/// Attaches each edited render to its asset, matching photo renders to
/// photos and video renders to videos.
pub fn apply_edits(assets: &mut [Asset], edits: &HashMap<(String, String), String>) {
    for a in assets {
        let key = (a.directory.clone(), a.stem().to_owned());
        if let Some(render) = edits.get(&key)
            && kind_from_extension(render) == Some(a.kind)
        {
            a.edited_render = Some(render.clone());
        }
    }
}

/// Fallback when the Photos database can't be read: list `DCIM/` directly.
/// Dates are file modification times; `.AAE` sidecars are skipped; a `.MOV`
/// sharing its stem with a photo is treated as that photo's Live Photo clip.
pub async fn scan_dcim(fs: &mut MediaFs) -> Result<Vec<Asset>, DeviceError> {
    let mut assets = Vec::new();
    for sub in fs.list_dir("/DCIM").await? {
        if sub.starts_with('.') {
            continue;
        }
        let dir = format!("DCIM/{sub}");
        let names = match fs.list_dir(&format!("/{dir}")).await {
            Ok(n) => n,
            Err(DeviceError::NotFound(_)) => continue,
            Err(e) => return Err(e),
        };
        let photo_stems: HashSet<&str> = names
            .iter()
            .filter(|n| kind_from_extension(n) == Some(MediaKind::Photo))
            .filter_map(|n| n.rsplit_once('.').map(|(s, _)| s))
            .collect();

        let mut in_dir: Vec<Asset> = Vec::new();
        let mut live_stems: HashSet<String> = HashSet::new();
        for name in &names {
            let Some(kind) = kind_from_extension(name) else {
                continue;
            };
            let stem = name.rsplit_once('.').map_or(name.as_str(), |(s, _)| s);
            if kind == MediaKind::Video
                && name.to_ascii_lowercase().ends_with(".mov")
                && photo_stems.contains(stem)
            {
                live_stems.insert(stem.to_owned());
                continue;
            }
            let st = fs.stat(&format!("/{dir}/{name}")).await?;
            if st.kind != EntryKind::File {
                continue;
            }
            let mut a = Asset::new(dir.clone(), name.clone(), kind);
            a.captured = Some(DateTime::<Utc>::from(st.modified));
            a.original_size = Some(st.size);
            in_dir.push(a);
        }
        for a in &mut in_dir {
            a.live_photo = a.kind == MediaKind::Photo && live_stems.contains(a.stem());
        }
        assets.extend(in_dir);
    }
    assets.sort_by(|a, b| (a.captured, &a.filename).cmp(&(b.captured, &b.filename)));
    Ok(assets)
}

/// Deletes the local database copy. It holds the phone's whole library
/// history (places, people, captions), so it isn't kept once read
/// (audit SEC-02).
pub fn remove_photos_db(dir: &Path) {
    for name in LOCAL_FILES {
        let _ = std::fs::remove_file(dir.join(name));
        let _ = std::fs::remove_file(dir.join(format!("{name}.part")));
    }
    let _ = std::fs::remove_dir(dir);
}

/// Loads the library: Photos DB copy if possible, else a `DCIM/` scan; then
/// attaches edited versions. `cache_dir` receives the database copy, which
/// is deleted again once read.
///
/// Device errors (lock, disconnect) are returned; database problems only
/// switch to the fallback.
pub async fn load(session: &Session, cache_dir: &Path) -> Result<Library, DeviceError> {
    let mut fs = session.media().await?;

    let from_db = read_from_db(&mut fs, cache_dir).await?;

    let (mut assets, source) = match from_db {
        Ok(a) if !a.is_empty() => (a, Source::PhotosDb),
        Ok(_) => (
            scan_dcim(&mut fs).await?,
            Source::DcimScan {
                reason: "Photos database has no items".into(),
            },
        ),
        Err(reason) => (scan_dcim(&mut fs).await?, Source::DcimScan { reason }),
    };

    // Names become local paths: drop anything that isn't a plain name.
    assets.retain(Asset::has_safe_names);
    let dirs: BTreeSet<String> = assets.iter().map(|a| a.directory.clone()).collect();
    let edits = scan_edits(&mut fs, &dirs).await?;
    apply_edits(&mut assets, &edits);
    Ok(Library { assets, source })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_copy_is_removed() {
        let dir = std::env::temp_dir().join(format!("rollcopy-dbtest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in DB_FILES {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        std::fs::write(dir.join("Photos.sqlite.part"), b"x").unwrap();
        remove_photos_db(&dir);
        assert!(!dir.exists());
    }

    #[test]
    fn edits_match_by_stem_and_kind() {
        let mut assets = vec![
            Asset::new("DCIM/100APPLE", "IMG_0057.HEIC", MediaKind::Photo),
            Asset::new("DCIM/100APPLE", "IMG_0058.MOV", MediaKind::Video),
            Asset::new("DCIM/100APPLE", "IMG_0059.JPG", MediaKind::Photo),
        ];
        let base = "/PhotoData/Mutations/DCIM/100APPLE";
        let edits = HashMap::from([
            (
                ("DCIM/100APPLE".into(), "IMG_0057".into()),
                format!("{base}/IMG_0057/Adjustments/FullSizeRender.jpg"),
            ),
            (
                ("DCIM/100APPLE".into(), "IMG_0058".into()),
                format!("{base}/IMG_0058/Adjustments/FullSizeRender.mov"),
            ),
        ]);
        apply_edits(&mut assets, &edits);
        assert!(
            assets[0]
                .edited_render
                .as_deref()
                .unwrap()
                .ends_with(".jpg")
        );
        assert!(
            assets[1]
                .edited_render
                .as_deref()
                .unwrap()
                .ends_with(".mov")
        );
        assert!(!assets[2].is_edited());
    }
}
