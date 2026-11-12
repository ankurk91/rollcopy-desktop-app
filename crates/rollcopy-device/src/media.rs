use std::path::Path;
use std::time::{Duration, SystemTime};

use idevice::afc::AfcClient;
use idevice::afc::opcode::AfcFopenMode;
use tokio::io::AsyncWriteExt;

use crate::error::{DeviceError, Result};

/// Bytes requested per AFC read. idevice splits larger requests into its own
/// maximum transfer size; 1 MiB keeps round trips low without much memory.
const READ_CHUNK: usize = 1 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
    Other,
}

#[derive(Debug, Clone)]
pub struct Stat {
    pub kind: EntryKind,
    pub size: u64,
    /// Device-reported modification time (UTC).
    pub modified: SystemTime,
}

/// Read-only view of the phone's media filesystem over AFC.
///
/// This is the only file access the rest of Rollcopy gets (PLAN §2a). It can
/// list, stat and read. It deliberately has no write, rename, remove, mkdir,
/// link or set-time methods, and files are only ever opened `RdOnly`. The
/// underlying `AfcClient` stays private so nothing else can reach those calls.
pub struct MediaFs {
    afc: AfcClient,
}

impl std::fmt::Debug for MediaFs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MediaFs")
    }
}

fn not_found_as(path: &str) -> impl FnOnce(idevice::IdeviceError) -> DeviceError + '_ {
    move |e| match DeviceError::from(e) {
        DeviceError::NotFound(_) => DeviceError::NotFound(path.to_owned()),
        other => other,
    }
}

impl MediaFs {
    pub(crate) fn new(afc: AfcClient) -> Self {
        Self { afc }
    }

    /// Entry names in a directory, without `.` and `..`, sorted.
    pub async fn list_dir(&mut self, path: &str) -> Result<Vec<String>> {
        let mut names = self.afc.list_dir(path).await.map_err(not_found_as(path))?;
        names.retain(|n| n != "." && n != "..");
        names.sort();
        Ok(names)
    }

    pub async fn stat(&mut self, path: &str) -> Result<Stat> {
        let fi = self
            .afc
            .get_file_info(path)
            .await
            .map_err(not_found_as(path))?;
        let kind = match fi.st_ifmt.as_str() {
            "S_IFREG" => EntryKind::File,
            "S_IFDIR" => EntryKind::Dir,
            "S_IFLNK" => EntryKind::Symlink,
            _ => EntryKind::Other,
        };
        let modified = fi.modified.and_utc();
        let modified = u64::try_from(modified.timestamp_nanos_opt().unwrap_or(0))
            .map(|ns| SystemTime::UNIX_EPOCH + Duration::from_nanos(ns))
            .unwrap_or(SystemTime::UNIX_EPOCH);
        Ok(Stat {
            kind,
            size: fi.size as u64,
            modified,
        })
    }

    /// Whether a path exists. Other errors (disconnects) are returned.
    pub async fn exists(&mut self, path: &str) -> Result<bool> {
        match self.stat(path).await {
            Ok(_) => Ok(true),
            Err(DeviceError::NotFound(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Reads a whole file into memory. For small files only (thumbnails,
    /// metadata); use [`MediaFs::copy_to_local`] for media.
    pub async fn read(&mut self, path: &str) -> Result<Vec<u8>> {
        let mut f = self
            .afc
            .open(path, AfcFopenMode::RdOnly)
            .await
            .map_err(not_found_as(path))?;
        let data = f.read_entire().await?;
        f.close().await?;
        Ok(data)
    }

    /// Streams a device file to `dest` on the laptop, calling `progress` with
    /// the running byte count. Writes straight to `dest`; atomic `.part` +
    /// rename handling belongs to the caller (PLAN §7.4). Returns bytes copied.
    pub async fn copy_to_local(
        &mut self,
        path: &str,
        dest: &Path,
        mut progress: impl FnMut(u64),
    ) -> Result<u64> {
        let mut f = self
            .afc
            .open(path, AfcFopenMode::RdOnly)
            .await
            .map_err(not_found_as(path))?;
        let mut out = tokio::fs::File::create(dest).await?;
        let mut total = 0u64;
        loop {
            let chunk = f.read_n(READ_CHUNK).await?;
            if chunk.is_empty() {
                break;
            }
            out.write_all(&chunk).await?;
            total += chunk.len() as u64;
            progress(total);
        }
        out.flush().await?;
        out.sync_all().await?;
        f.close().await?;
        Ok(total)
    }
}
