//! Headless CLI for testing the device and library crates.
//!
//! Every command is read-only towards the phone (PLAN §2a).

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use rollcopy_device::{DeviceError, EntryKind, Session, list_devices};
use rollcopy_library::{Asset, MediaKind, Source};

/// `println!` that exits quietly when stdout is closed (`| head`).
macro_rules! out {
    ($($t:tt)*) => {
        if writeln!(std::io::stdout().lock(), $($t)*).is_err() {
            std::process::exit(0);
        }
    };
}

const USAGE: &str = "\
usage: rollcopy-cli [--udid UDID] <command>

commands:
  devices              list connected devices
  info                 show device info
  ls [PATH]            list a directory on the phone (default /)
  pull PATH DEST       copy one file from the phone to DEST on this computer
  convert SRC DST [--h264]
                       convert a local HEIC to JPG or MOV to MP4 (no phone needed)
  copy --month YYYY-MM --dest DIR [--overwrite] [--h264]
                       copy one month from the phone like the app does
  watch                follow the connection guide (asks to Trust if needed)
  library [--list] [--dcim]
                       read the Photos library (--dcim: skip the database)
";

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let udid = take_flag(&mut args, "--udid");
    let result = match args.first().map(String::as_str) {
        Some("devices") => cmd_devices().await,
        Some("info") => cmd_info(udid).await,
        Some("ls") => cmd_ls(udid, args.get(1).map_or("/", String::as_str)).await,
        Some("pull") if args.len() == 3 => cmd_pull(udid, &args[1], PathBuf::from(&args[2])).await,
        Some("convert") if args.len() >= 3 => {
            let h264 = args.iter().any(|a| a == "--h264");
            return match cmd_convert(&args[1], &args[2], h264) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            };
        }
        Some("copy") => cmd_copy(udid, &args).await,
        Some("watch") => {
            rollcopy_device::watch(|p| {
                let t = chrono_now();
                out!("{t}  {p:?}")
            })
            .await;
            Ok(())
        }
        Some("library") => {
            let list = args.iter().any(|a| a == "--list");
            let dcim = args.iter().any(|a| a == "--dcim");
            cmd_library(udid, list, dcim).await
        }
        _ => {
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            if let Some(hint) = hint(&e) {
                eprintln!("hint: {hint}");
            }
            ExitCode::FAILURE
        }
    }
}

fn take_flag(args: &mut Vec<String>, name: &str) -> Option<String> {
    let i = args.iter().position(|a| a == name)?;
    args.remove(i);
    (i < args.len()).then(|| args.remove(i))
}

/// The connection guide's "next step" for each state (PLAN §5).
fn hint(e: &DeviceError) -> Option<&'static str> {
    Some(match e {
        DeviceError::UsbmuxdUnavailable(_) => {
            "plug in your iPhone with a cable (if it is plugged in, check that usbmuxd is installed)"
        }
        DeviceError::NotConnected => "plug in your iPhone with a cable; try another cable or port",
        DeviceError::NotPaired | DeviceError::StalePairing => {
            "unlock your iPhone and tap Trust, then enter your passcode (`rollcopy-cli watch` asks again)"
        }
        DeviceError::PairingPending => "tap Trust on your iPhone",
        DeviceError::PairingDenied => "unplug and re-plug your iPhone to be asked again",
        DeviceError::Locked => "unlock your iPhone",
        DeviceError::Disconnected(_) => "reconnect and unlock your iPhone",
        _ => return None,
    })
}

async fn open(udid: Option<String>) -> Result<Session, DeviceError> {
    let udid = match udid {
        Some(u) => u,
        None => {
            let devices = list_devices().await?;
            match devices.as_slice() {
                [] => return Err(DeviceError::NotConnected),
                [one] => one.udid.clone(),
                _ => {
                    return Err(DeviceError::Other(
                        "several devices connected; pick one with --udid".into(),
                    ));
                }
            }
        }
    };
    Session::open(&udid).await
}

async fn cmd_devices() -> Result<(), DeviceError> {
    for d in list_devices().await? {
        out!("{}\t{}", d.udid, if d.usb { "usb" } else { "network" });
    }
    Ok(())
}

fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

async fn cmd_info(udid: Option<String>) -> Result<(), DeviceError> {
    let mut s = open(udid).await?;
    let i = s.info().await?;
    out!("Name:          {}", i.name);
    out!("Model:         {} ({})", i.model, i.product_type);
    out!("iOS:           {} ({})", i.ios_version, i.build_version);
    if let (Some(total), Some(free)) = (i.storage_total, i.storage_free) {
        out!(
            "Storage:       {} used of {} ({} free)",
            gb(total.saturating_sub(free)),
            gb(total),
            gb(free)
        );
    }
    if let Some(p) = i.battery_percent {
        let charging = if i.battery_charging == Some(true) {
            ", charging"
        } else {
            ""
        };
        out!("Battery:       {p}%{charging}");
    }
    if let Some(b) = i.icloud_backup {
        out!("iCloud Backup: {}", if b { "on" } else { "off" });
    }
    out!(
        "Serial:        {}",
        i.serial_number.as_deref().unwrap_or("-")
    );
    out!("UDID:          {}", i.udid);
    Ok(())
}

async fn cmd_ls(udid: Option<String>, path: &str) -> Result<(), DeviceError> {
    let s = open(udid).await?;
    let mut fs = s.media().await?;
    for name in fs.list_dir(path).await? {
        let full = format!("{}/{}", path.trim_end_matches('/'), name);
        match fs.stat(&full).await {
            Ok(st) if st.kind == EntryKind::Dir => out!("{name}/"),
            Ok(st) => out!("{name}\t{}", st.size),
            Err(e) => out!("{name}\t(stat failed: {e})"),
        }
    }
    Ok(())
}

async fn cmd_pull(udid: Option<String>, path: &str, dest: PathBuf) -> Result<(), DeviceError> {
    let dest = if dest.is_dir() {
        dest.join(path.rsplit('/').next().unwrap_or(path))
    } else {
        dest
    };
    if dest.exists() {
        return Err(DeviceError::Other(format!(
            "{} already exists; not overwriting",
            dest.display()
        )));
    }
    let s = open(udid).await?;
    let mut fs = s.media().await?;
    let n = fs.copy_to_local(path, &dest, |_| {}).await?;
    out!("{n} bytes -> {}", dest.display());
    Ok(())
}

fn cache_dir(udid: &str) -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("rollcopy").join("db").join(udid)
}

fn flags(a: &Asset) -> String {
    let mut f = String::new();
    for (on, c) in [
        (a.live_photo, "live"),
        (a.is_edited(), "edited"),
        (a.favorite, "fav"),
        (a.hidden, "hidden"),
        (a.trashed, "DELETED"),
    ] {
        if on {
            if !f.is_empty() {
                f.push(',');
            }
            f.push_str(c);
        }
    }
    f
}

async fn cmd_library(udid: Option<String>, list: bool, dcim_only: bool) -> Result<(), DeviceError> {
    let s = open(udid).await?;
    let lib = if dcim_only {
        let mut fs = s.media().await?;
        let mut assets = rollcopy_library::scan_dcim(&mut fs).await?;
        let dirs = assets.iter().map(|a| a.directory.clone()).collect();
        let edits = rollcopy_library::scan_edits(&mut fs, &dirs).await?;
        rollcopy_library::apply_edits(&mut assets, &edits);
        rollcopy_library::Library {
            assets,
            source: Source::DcimScan {
                reason: "--dcim".into(),
            },
        }
    } else {
        rollcopy_library::load(&s, &cache_dir(s.udid())).await?
    };

    match &lib.source {
        Source::PhotosDb => out!("Source: Photos database"),
        Source::DcimScan { reason } => out!("Source: DCIM scan ({reason})"),
    }
    let (trashed, shown): (Vec<&Asset>, Vec<&Asset>) = lib.assets.iter().partition(|a| a.trashed);
    let count = |k| shown.iter().filter(|a| a.kind == k).count();
    out!(
        "Items: {} ({} photos, {} videos; {} live, {} edited)  Recently Deleted: {}",
        shown.len(),
        count(MediaKind::Photo),
        count(MediaKind::Video),
        shown.iter().filter(|a| a.live_photo).count(),
        shown.iter().filter(|a| a.is_edited()).count(),
        trashed.len()
    );

    let mut tree: BTreeMap<i32, BTreeMap<u32, usize>> = BTreeMap::new();
    let mut undated = 0;
    for a in &shown {
        match a.year_month() {
            Some((y, m)) => *tree.entry(y).or_default().entry(m).or_default() += 1,
            None => undated += 1,
        }
    }
    for (y, months) in tree.iter().rev() {
        out!("  {y}  {}", months.values().sum::<usize>());
        for (m, n) in months.iter().rev() {
            out!("    {m:02}  {n}");
        }
    }
    if undated > 0 {
        out!("  undated  {undated}");
    }

    if list {
        for a in &lib.assets {
            let when = a
                .captured_local()
                .map(|d| d.format("%Y-%m-%d %H:%M %:z").to_string())
                .unwrap_or_else(|| "-".into());
            let dims = match (a.width, a.height) {
                (Some(w), Some(h)) => format!("{w}x{h}"),
                _ => "-".into(),
            };
            let dur = a
                .duration_secs
                .map(|d| format!("{d:.1}s"))
                .unwrap_or_default();
            out!(
                "{}\t{when}\t{}\t{dims}\t{dur}\t{}\t{}\t{}\t{}\t{}",
                a.device_path(),
                if a.kind == MediaKind::Photo {
                    "photo"
                } else {
                    "video"
                },
                flags(a),
                a.source_app.as_deref().unwrap_or("-"),
                a.camera.as_deref().unwrap_or("-"),
                a.place.as_deref().unwrap_or("-"),
                a.albums.join("|")
            );
        }
    }
    Ok(())
}

fn chrono_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!(
        "{:02}:{:02}:{:02}Z",
        secs / 3600 % 24,
        secs / 60 % 60,
        secs % 60
    )
}

fn cmd_convert(src: &str, dst: &str, h264: bool) -> Result<(), rollcopy_convert::ConvertError> {
    use std::path::Path;
    let (src, dst) = (Path::new(src), Path::new(dst));
    let ext = src
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let t = std::time::Instant::now();
    match ext.as_str() {
        "heic" | "heif" => {
            let info =
                rollcopy_convert::heic_to_jpeg(src, dst, rollcopy_convert::DEFAULT_JPEG_QUALITY)?;
            let (w, h) = rollcopy_convert::verify_jpeg(dst)?;
            out!(
                "{}x{} exif={} icc={} verified={w}x{h} in {:.2}s",
                info.width,
                info.height,
                info.has_exif,
                info.has_icc,
                t.elapsed().as_secs_f64()
            );
        }
        _ => {
            let mode = if h264 {
                rollcopy_convert::VideoMode::H264
            } else {
                rollcopy_convert::VideoMode::Remux
            };
            let before = rollcopy_convert::probe(src)?;
            rollcopy_convert::to_mp4(src, dst, mode)?;
            let after = rollcopy_convert::verify_mp4(dst, before.duration_secs)?;
            out!(
                "{before:?} -> {after:?} in {:.2}s",
                t.elapsed().as_secs_f64()
            );
        }
    }
    Ok(())
}

fn flag_value<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    let i = args.iter().position(|a| a == name)?;
    args.get(i + 1).map(String::as_str)
}

/// Copies one month through the same queue the app uses. Conflicts are
/// skipped unless `--overwrite`.
async fn cmd_copy(udid: Option<String>, args: &[String]) -> Result<(), DeviceError> {
    use rollcopy_queue::{ConflictPolicy, JobState, QueueCommand, QueueEvent, Settings};
    let usage = || DeviceError::Other("usage: copy --month YYYY-MM --dest DIR".into());
    let month = flag_value(args, "--month").ok_or_else(usage)?;
    let (y, m) = month.split_once('-').ok_or_else(usage)?;
    let (y, m): (i32, u32) = (
        y.parse().map_err(|_| usage())?,
        m.parse().map_err(|_| usage())?,
    );
    let dest = PathBuf::from(flag_value(args, "--dest").ok_or_else(usage)?);

    let session = open(udid).await?;
    let udid = session.udid().to_owned();
    let lib = rollcopy_library::load(&session, &cache_dir(&udid)).await?;
    let assets: Vec<Asset> = lib
        .assets
        .into_iter()
        .filter(|a| !a.trashed && a.year_month() == Some((y, m)))
        .collect();
    out!("{} items in {y}-{m:02}", assets.len());
    if assets.is_empty() {
        return Ok(());
    }

    let mut settings = Settings::new(dest);
    settings.on_conflict = if args.iter().any(|a| a == "--overwrite") {
        ConflictPolicy::Overwrite
    } else {
        ConflictPolicy::Skip
    };
    if args.iter().any(|a| a == "--h264") {
        settings.video_mode = rollcopy_convert::VideoMode::H264;
    }
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let staging = cache_dir(&udid).join("staging");
    let q = rollcopy_queue::spawn(settings, staging, None, tx);
    q.send(QueueCommand::DeviceReady(udid.clone()));
    let total = assets.len();
    q.send(QueueCommand::Add { udid, assets });

    let t = std::time::Instant::now();
    let mut names = std::collections::HashMap::new();
    let (mut finished, mut failed) = (0, 0);
    while let Some(ev) = rx.recv().await {
        match ev {
            QueueEvent::Added { id, name, .. } => {
                names.insert(id, name);
            }
            QueueEvent::State { id, state } if state.is_finished() => {
                finished += 1;
                if matches!(state, JobState::Failed { .. }) {
                    failed += 1;
                }
                out!(
                    "{:>3}/{total} {:<14} {state:?}",
                    finished,
                    names.get(&id).map_or("?", |s| s.as_str())
                );
                if finished == total {
                    break;
                }
            }
            QueueEvent::Paused { reason: Some(r) } => out!("paused: {r}"),
            _ => {}
        }
    }
    out!(
        "{finished} finished, {failed} failed in {:.1}s",
        t.elapsed().as_secs_f64()
    );
    Ok(())
}
