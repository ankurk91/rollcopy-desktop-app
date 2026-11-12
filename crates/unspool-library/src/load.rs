use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use unspool_device::{DeviceError, EntryKind, MediaFs, Session};

use crate::asset::{Asset, MediaKind, kind_from_extension};
use crate::photos_db;

const DB_FILES: [&str; 3] = ["Photos.sqlite", "Photos.sqlite-wal", "Photos.sqlite-shm"];

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

/// Copies `Photos.sqlite` and its `-wal`/`-shm` files from the phone into
/// `dest_dir` (PLAN §2a: never opened in place). Each file goes to a `.part`
/// file first and is renamed when complete. A local `-wal`/`-shm` left over
/// from an earlier copy is removed when the phone has none, so SQLite never
/// replays a stale WAL against a newer database.
pub async fn fetch_photos_db(fs: &mut MediaFs, dest_dir: &Path) -> Result<PathBuf, DeviceError> {
    tokio::fs::create_dir_all(dest_dir).await?;
    for name in DB_FILES {
        let src = format!("/PhotoData/{name}");
        let dest = dest_dir.join(name);
        if !fs.exists(&src).await? {
            if name == DB_FILES[0] {
                return Err(DeviceError::NotFound(src));
            }
            match tokio::fs::remove_file(&dest).await {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                _ => continue,
            }
        }
        let part = dest_dir.join(format!("{name}.part"));
        fs.copy_to_local(&src, &part, |_| {}).await?;
        tokio::fs::rename(&part, &dest).await?;
    }
    Ok(dest_dir.join(DB_FILES[0]))
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

/// Loads the library: Photos DB copy if possible, else a `DCIM/` scan; then
/// attaches edited versions. `cache_dir` receives the database copy.
///
/// Device errors (lock, disconnect) are returned; database problems only
/// switch to the fallback.
pub async fn load(session: &Session, cache_dir: &Path) -> Result<Library, DeviceError> {
    let mut fs = session.media().await?;

    let from_db = match fetch_photos_db(&mut fs, cache_dir).await {
        Ok(path) => {
            let path2 = path.clone();
            tokio::task::spawn_blocking(move || photos_db::read_assets(&path2))
                .await
                .map_err(|e| DeviceError::Other(e.to_string()))?
                .map_err(|e| e.to_string())
        }
        Err(e) if e.is_connection_problem() => return Err(e),
        Err(e) => Err(e.to_string()),
    };

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

    let dirs: BTreeSet<String> = assets.iter().map(|a| a.directory.clone()).collect();
    let edits = scan_edits(&mut fs, &dirs).await?;
    apply_edits(&mut assets, &edits);
    Ok(Library { assets, source })
}

#[cfg(test)]
mod tests {
    use super::*;

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
