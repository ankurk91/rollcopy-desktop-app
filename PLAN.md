# Unspool — plan

> Plug in your iPhone, pick photos and videos, and get Android/PC-friendly files on your laptop.

Status: **M0, M1 and M2 done (2026-11-12).** Next: M3 (thumbnail grid, filters, preview pane).

---

## 1. Name

**Unspool**: you plug in the cable and the media unspools from the phone onto the disk.

- Generic enough to cover phone media, and later Android.
- Not taken on crates.io (checked 2026-11-12).
- Binary `unspool`, app id `dev.unspool.Unspool`.

## 2. Goal and scope

**v1 goal:** copy media from an iPhone over USB to a Linux laptop, as compatible files, with as few clicks as possible.

| In v1 | Not in v1 |
|---|---|
| Ubuntu / Linux Mint (X11 and Wayland) | Windows, macOS |
| iPhone / iPad over USB cable | Wi-Fi sync, Android phones |
| Browse, preview, inspect, copy | Deleting from the phone (see §11) |
| HEIC → JPG, MOV → MP4 on copy; Live Photos as stills | Editing photos |
| Device info, connection guide, tray icon | iCloud Photos download |
| Preferences window (saved between runs), About dialog, GitHub link, Check for updates | Auto-installing updates |

## 2a. Hard rule: never lose data on the phone

Decided 2026-11-12. In v1, Unspool is **read-only towards the iPhone**:

- AFC is only used to list, stat and read. The device wrapper doesn't expose write, rename, remove or mkdir at all, so the rest of the app can't call them by mistake.
- `Photos.sqlite` and its `-wal`/`-shm` files are copied to the laptop first and opened from there, read-only. The phone's copy is never opened in place.
- No services beyond lockdown (device info), AFC (read), battery/disk-usage queries and pairing. Pairing only adds a trust record, which is the same thing tapping Trust does with Finder or iTunes.
- Deleting from the phone stays out of scope until v2 (§11), and then needs its own design and explicit confirmation.
- On the laptop side, a copy never destroys an existing file unless it's replaced at the final rename (§7.4).

## 3. What the cable connection can do

You asked what the "p2p" connection can do. It's actually two protocols. Both were used in the manual transfer on 2026-10-19.

| Protocol | What it is | What we get | Use in Unspool |
|---|---|---|---|
| **PTP** (Picture Transfer Protocol) | The camera-style view the file manager shows as "iPhone" | Photos only, in month folders. The phone converts HEIC to JPG on the fly, but Live Photo clips often fail with I/O errors. | **Not used.** It's slow, flaky, and gives no metadata. |
| **AFC over usbmuxd + lockdown** | Apple's file and service protocol, used by `ifuse` and `ideviceinfo` | Raw `DCIM/` files, the Photos library database, edited versions, and device info | **The main route.** |

### Device information available over lockdown

| Shown in the app | Source (lockdown key / domain) |
|---|---|
| Device name ("My iPhone") | `DeviceName` |
| Model (marketing name, e.g. iPhone 15) | `ProductType` → bundled lookup table (`iPhone15,4` = iPhone 15) |
| iOS version and build | `ProductVersion`, `BuildVersion` |
| Storage: total / free / used by photos | `com.apple.disk_usage`: `TotalDiskCapacity`, `TotalDataAvailable`; photos size from our own scan |
| Battery % and charging | `com.apple.mobile.battery`: `BatteryCurrentCapacity`, `BatteryIsCharging` |
| Serial / UDID (hidden under "Details") | `SerialNumber`, `UniqueDeviceID` |
| iCloud Backup on/off (as a hint) | `com.apple.mobile.backup`: `CloudBackupEnabled` |

### Per-item information from the Photos database

`PhotoData/Photos.sqlite` is copied read-only along with its `-wal` and `-shm` files. Fields proven on 2026-10-19 are marked ✔. The rest need checking in M1.

- ✔ Folder + file name: `ZASSET.ZDIRECTORY`, `ZFILENAME`
- ✔ In Recently Deleted: `ZTRASHEDSTATE = 1`. **Hidden by default.**
- ✔ Capture date: `ZDATECREATED` (seconds since 2001-01-01 UTC), with time-zone offset from `ZADDITIONALASSETATTRIBUTES.ZTIMEZONEOFFSET`
- ✔ Edited: a `PhotoData/Mutations/DCIM/<dir>/<stem>/Adjustments/FullSizeRender.{heic,jpg,mov}` file exists
- ✔ Live Photo: a `.MOV` with the same name stem as the photo, and `ZKINDSUBTYPE = 2` on the photo's row. The clip has no row of its own.
- ✔ (iOS 27, 2026-11-12) photo vs video: `ZKIND` 0/1. Size in pixels: `ZWIDTH`/`ZHEIGHT`. Video length: `ZDURATION`. Favourite: `ZFAVORITE`. Hidden: `ZHIDDEN`. Location: `ZLATITUDE`/`ZLONGITUDE`, where **-180 means no location**, not NULL. Time zone: `ZADDITIONALASSETATTRIBUTES.ZTIMEZONEOFFSET` (seconds) joined on `ZASSET`. Original size: `ZADDITIONALASSETATTRIBUTES.ZORIGINALFILESIZE`. `ZKINDSUBTYPE = 10` looks like screenshots.
- ✔ Edited: `ZADJUSTMENTTIMESTAMP` is set exactly for the items that have a `Mutations/` folder (every one checked).
- ✔ `DCIM/` holds more files than the database has rows: on the test phone the extra files were exactly the Live Photo `.MOV` clips and the `.AAE` edit sidecars. Every row has a file.

The database is optional. If it can't be read, fall back to plain `DCIM/` browsing with file dates. On the test phone the fallback found the same items and the same Live Photos as the database (checked in M1), but it can't tell which items are in Recently Deleted.

## 4. Technology

| Need | Choice | Notes |
|---|---|---|
| UI | **gpui-kit =0.9.1** (bundles gpui-component 0.9.1 + `gpui-pre` =0.3.12 + the Linux platform) | Checked in M0: gpui-component 0.9.x is built on `gpui-pre` (a crates.io snapshot of Zed's gpui), **not** `gpui` 0.2.x. `gpui_kit::application()` provides the X11/Wayland platform. No Zed git rev needed. Pinned exactly. |
| iPhone access | **idevice =0.1.68** (pure Rust, MIT): `default-features = false`, features `ring`, `usbmuxd`, `pair`, `afc`, `device_info`, `diagnostics_relay` | Builds (M0). `ring` replaces the default aws-lc crypto. No `libimobiledevice-dev` needed. Needs the system `usbmuxd` daemon (already installed). |
| Async | tokio runtime on a background thread; results sent to the UI over channels | Keep all device I/O off the UI thread. |
| Photos DB | `rusqlite` (bundled SQLite) | |
| HEIC → JPG | `libheif-rs` =3.0.0 with `default-features = false, features = ["v1_17"]` (needs `libheif-dev`) + `image` for JPEG encoding | Keeps EXIF data and applies rotation. |
| MOV → MP4 | `ffmpeg` command-line tool, run as a subprocess | Lossless remux: `-c copy -tag:v hvc1 -movflags +faststart`. Optional H.264 re-encode setting. |
| Video thumbnails | `ffmpeg -ss 1 -frames:v 1` | |
| Tray icon | **ksni 0.3.x** (StatusNotifierItem over D-Bus) | gpui has no tray support. On Cinnamon the tray applet must be enabled. |
| Preferences | `serde` + `toml` → `~/.config/unspool/config.toml` (path from the `directories` crate) | See §8a |
| Job history | `rusqlite` → `~/.local/state/unspool/history.sqlite` | |
| Folder picker | gpui's `prompt_for_paths` (uses the XDG desktop portal on Linux) | Fallback: `rfd` |
| Update check | `ureq` (rustls) + `semver` | GitHub Releases API, see §8c |
| Open links / files | `open` crate (`xdg-open`) | GitHub page, release page, video player |
| Thumbnail cache | `~/.cache/unspool/thumbs/<udid>/` | |

## 5. Connection guide

The app always shows exactly one clear next step. Each state is detected, not guessed.

```
 ┌────────────────────┐  usbmuxd socket missing  ┌──────────────────────────────────┐
 │ start              ├─────────────────────────►│ "Install usbmuxd" + copy command  │
 └─────────┬──────────┘                          └──────────────────────────────────┘
           ▼
 ┌────────────────────┐  no device listed        ┌──────────────────────────────────┐
 │ list devices       ├─────────────────────────►│ "Plug in your iPhone with a cable"│
 └─────────┬──────────┘  (Apple USB id seen but   │  + "try another cable/port" hint  │
           │             not listed → hint)       └──────────────────────────────────┘
           ▼
 ┌────────────────────┐  not paired               ┌──────────────────────────────────┐
 │ device found       ├─────────────────────────►│ "Unlock your iPhone and tap       │
 └─────────┬──────────┘                          │  Trust, then enter your passcode" │
           │                                     │  (we send the pair request; poll) │
           │             user tapped Don't Trust └──────────────────────────────────┘
           │             → "Unplug and re-plug to ask again"
           ▼
 ┌────────────────────┐  locked / AFC refused    ┌──────────────────────────────────┐
 │ paired             ├─────────────────────────►│ "Unlock your iPhone"              │
 └─────────┬──────────┘                          └──────────────────────────────────┘
           ▼
     Ready → read device info → read the Photos library
```

**During a copy:** if the phone locks or the cable drops, AFC returns I/O errors. Seen on 2026-10-19. The queue then **pauses** and shows "Unlock your iPhone / reconnect". It doesn't record a pile of failures. When the device comes back, the queue resumes from where it stopped.

**Locked phone (tested 2026-11-12):** once the phone is trusted and has been unlocked at least once since it started, AFC keeps working while it's locked. Listing, reading a HEIC and reading `Photos.sqlite` all succeeded. So "Unlock your iPhone" is only shown when the phone actually refuses: before the first unlock after a restart, or while waiting for Trust. The app shouldn't nag about the lock screen otherwise. It's not yet known whether the 2026-10-19 I/O errors came from auto-lock or from a bad cable like the one seen today. **M4 must test a long copy with the phone locked.** Until then, the queue keeps pausing on disconnect.

**Tip shown once:** set Auto-Lock to Never during long transfers (drop it if the M4 test shows locking doesn't matter).

**Who sends the pair request (found in M2, 2026-11-12):** usbmuxd 1.1 (Ubuntu 24.04 / Mint 22) pairs by itself when a phone **with no record** on this computer is plugged in. It shows the Trust dialog and saves the record. When the app also sent a request, both ran at once, usbmuxd saved first, and the app's `SavePairRecord` was refused. So:
- **No record:** wait up to 20 s for usbmuxd's record, showing "tap Trust". Only then send our own request (for systems without usbmuxd's auto-pairing). Tested: unplug → move the record aside → replug → Trust → Ready.
- **Record exists but the phone rejects it** (`InvalidHostID`, e.g. after an iOS update; seen on the morning of 2026-11-12): usbmuxd doesn't re-pair, so the app sends the request right away. If usbmuxd refuses to save, the stale local record is deleted and the save retried. **Not tested yet:** it needs the phone to forget this computer.

**Bad cable / port signature (seen 2026-11-12):** usbmuxd attaches the device for about a second and then logs "Removed device". After that the kernel logs `device descriptor read/64, error -71` and `unable to enumerate USB device`, and `lsusb` no longer lists Apple (`05ac`). This is a physical link problem, so the app should show "Try another cable or USB port (no hubs)", not the pairing screen. Detect it from a device that appears and disappears within a few seconds.

## 6. Interface

One window with three panes and a collapsible queue at the bottom. It uses a gpui-component theme and follows the system light/dark setting.

```
┌───────────────────────────────────────────────────────────────────────────────┐
│ ⬇ Unspool   [ Save to: ~/Pictures/Unspool/My iPhone  📁 ]  [Copy selected (12)]  ☰ │
├───────────────────┬───────────────────────────────────────┬───────────────────┤
│ DEVICES           │  All media ▸ 2025 ▸ September          │  PREVIEW          │
│ ● My iPhone  │  [Photos][Videos][Edited][All]  🔍     │  ┌─────────────┐  │
│   iPhone 15       │ ┌────┐┌────┐┌────┐┌────┐┌────┐┌────┐  │  │             │  │
│   iOS 27.0.1      │ │ 🖼 ││ 🖼 ││ ▶ ││ 🖼 ││ 🖼 ││ ▶ │  │  │   image /   │  │
│   ▓▓▓▓▓░░ 98/128GB│ └────┘└────┘└────┘└────┘└────┘└────┘  │  │ video frame │  │
│   🔋 84% charging │ ┌────┐┌────┐┌────┐┌────┐┌────┐┌────┐  │  └─────────────┘  │
│                   │ │ ✓ ││ ✓ ││ 🖼 ││ ▶ ││ 🖼 ││ 🖼 │  │  IMG_3480.MOV     │
│ LIBRARY           │ └────┘└────┘└────┘└────┘└────┘└────┘  │  Video · 0:42     │
│  All media   660  │                                       │  1 Sep 2025 18:04 │
│  ▸ 2026      120  │                                       │  3840×2160 HEVC   │
│  ▾ 2025      210  │                                       │  23.6 MB          │
│     September  5  │                                       │  Edited ✔         │
│     October   24  │                                       │  → saves as MP4   │
│  Recently Del. 141│ (greyed; off by default)               │  [Copy] [Open ▶]  │
├───────────────────┴───────────────────────────────────────┴───────────────────┤
│ ▾ Transfers  37/120 · 1.2 GB of 4.1 GB · 2 failed · ~3 min        [Pause] [⋯] │
│   ✔ IMG_3477.JPG   ✔ IMG_3478.MP4   ⟳ IMG_3480.MP4 ▓▓▓░░ 61%   ✖ IMG_3482 ⓘ │
└───────────────────────────────────────────────────────────────────────────────┘
```

**Toolbar**
- **Save to**: shows the destination folder for the selected device. 📁 opens a folder picker. The default is `~/Pictures/Unspool/<device name>/`, and the choice is remembered (§8a).
- **☰ app menu**:
  - Preferences… (Ctrl+,)
  - Check for Updates…
  - View on GitHub
  - Report an Issue (opens GitHub Issues)
  - About Unspool
  - Quit (Ctrl+Q)

**Left pane: devices and library**
- Device card with name, model, iOS version, storage bar and battery. "Details" opens the serial number, UDID and iCloud Backup hint.
- The library tree is grouped **year → month** by capture date. An "On-phone folders" view (`100APPLE`, …) is available as an advanced toggle.
- Recently Deleted is listed with its count but greyed out, and excluded from "copy folder".

**Centre pane: thumbnail grid**
- Filter chips: All / Photos / Videos / Edited.
- Badges on each tile: ▶ video with duration, ✎ edited, ◉ Live Photo, ✓ already in the current destination folder.
- Selection: click, Ctrl+click, Shift+click, Ctrl+A.
- Right-click menu: Copy, Copy folder, Show info.
- Thumbnails are virtualised and lazy-loaded. A 5,000-item library must scroll smoothly.

**Right pane: preview and info**
- Photos: a large preview. Double-click opens fullscreen with ← / → to step through.
- Videos: a poster frame. "Open ▶" copies the file to a temp location and opens it in the system player with `xdg-open`, because gpui has no video widget.
- Info: file name, type, capture date and time zone, pixel size, codec, length, file size, edited / Live / favourite, location as plain coordinates with no map in v1, and **what it will be saved as** (e.g. "HEIC → JPG").

**Bottom: transfer queue**
- Collapsed: one summary line with done / total, bytes, failed count and time left.
- Expanded: a list with a status icon per item. Failed rows show the reason, plus "Retry" and "Retry all failed".
- Each item moves through: Queued → Reading → Converting → Verifying → Done / Failed / Skipped / Overwritten.

**Tray icon (ksni)**
- The icon changes with status: idle / device connected / copying (progress tooltip) / error.
- Menu: Show Unspool · `<device name>` · Pause/Resume · Quit.
- Closing the window keeps the app in the tray **only while a copy is running**. Otherwise it quits.
- Optional desktop notification when a queue finishes.

**Copy actions**
- **One item:** the Copy button in the preview pane.
- **Selected items:** the toolbar button "Copy selected (n)".
- **Whole folder:** right-click a month or year → "Copy folder".

## 7. Copy pipeline

These are the rules proven in the manual transfer, turned into code.

1. **Pick the source.** Use the edited version (`FullSizeRender.*`) if it exists, otherwise the original.
2. **Convert to compatible formats:**
   - JPG / PNG / GIF / MP4 → copied byte for byte.
   - HEIC → JPG, quality 92, keeping EXIF and applying rotation.
   - MOV (HEVC) → MP4 by lossless remux. Preference: "Re-encode to H.264 for old devices" (off by default).
   - Live Photos → **still only, always.** The motion clip is never copied. There's no setting for it.
3. **Arrange the destination** as `<dest>/<YYYY>/<MM>/images|videos/<IMG_xxxx>.<ext>`, using the capture date in its own time zone. `<dest>` defaults to `~/Pictures/Unspool/<device name>/`. A flat-folder layout is available as a preference.
4. **Write safely:** write to `<name>.part`, check it (size for straight copies; decodes for converted files), then rename. Set the file's modified time to the capture date. On overwrite, the old file is only replaced at the final rename, so a failed copy never destroys it.
5. **If the file already exists** at the destination path, show a dialog:

   ```
   ┌──────────────────────────────────────────────────────────┐
   │  IMG_3480.MP4 already exists in 2025/09/videos           │
   │  Existing: 23.6 MB · 1 Sep 2025   New: 23.6 MB           │
   │                                                          │
   │  [✓] Do this for the remaining 14 conflicts              │
   │                                  [ Skip ]  [ Overwrite ] │
   └──────────────────────────────────────────────────────────┘
   ```

   - The queue pauses on that item until you answer. Other non-conflicting items keep copying.
   - Preference "When a file already exists": **Ask** (default) / Always skip / Always overwrite.
   - There's no per-device "copied" database. ✓ badges and conflicts come from what's actually in the destination folder, so they stay correct if you move or delete files there.
6. **Concurrency:** 2 AFC reads at once, plus 1–2 conversion workers. Never more than 1 ffmpeg per CPU core.

## 8. Error tracking

- Each failed item is saved with time, device, source path, stage (read / convert / write / verify), error text and retry count. History goes to `~/.local/state/unspool/history.sqlite`.
- The queue shows a red badge with the failed count. Clicking it filters the queue to failures.
- "Export error report" saves a CSV for support or debugging.
- Disconnects pause the queue instead of failing items (§5). Real per-file failures, such as a corrupt file or ffmpeg errors, are retried once automatically and then left for the user.

## 8a. Preferences window

Opened from ☰ → Preferences… or Ctrl+,. It's a small separate window with sections. Changes apply immediately (no OK/Apply button).

| Section | Setting | Default |
|---|---|---|
| **Destination** | Base folder | `~/Pictures/Unspool/` |
| | Subfolder per device | On (`<base>/<device name>/`) |
| | Folder layout | Year / Month / images·videos, or Flat |
| | When a file already exists | Ask / Always skip / Always overwrite (default **Ask**) |
| **Conversion** | JPEG quality for HEIC | 92 |
| | Videos | Lossless MP4 (default) / Re-encode to H.264 |
| **Library** | Show Recently Deleted (greyed, never copied) | On |
| | Default view | By date / On-phone folders |
| **App** | Theme | System / Light / Dark |
| | Keep running in tray while copying | On |
| | Notify when transfers finish | On |
| | Check for updates on startup (at most once a day) | Off |
| **Advanced** | Parallel reads / conversion workers | 2 / auto |
| | Open logs folder (button) | — |

**How preferences are saved:**
- File: `~/.config/unspool/config.toml`. Each preference carries a default (`serde(default)`), so a file from an older version still loads. Unknown keys are ignored.
- Writes go to a temp file that is then renamed into place, so a crash never leaves a half-written config.
- If the file is broken, rename it to `config.toml.bad`, start with defaults, and show a one-line notice.
- Window size and position, the last selected device, and the queue collapsed/expanded state are also remembered, in a separate file: `~/.local/state/unspool/ui.toml`.
- A destination folder you choose with 📁 in the toolbar is saved per device (keyed by UDID). Otherwise the base + device-name default is used.

## 8b. About dialog

A small modal opened from ☰ → About Unspool:

```
┌─────────────────────────────────────────┐
│               [icon 96px]               │
│                Unspool                  │
│          Version 0.1.0 (a1b2c3d)        │
│  Copy photos and videos from your       │
│  iPhone to Linux over a USB cable.      │
│                                         │
│  [ GitHub ]  [ Report an issue ]        │
│  [ Check for updates ]                  │
│                                         │
│  Licence: MIT · Credits ▸               │
└─────────────────────────────────────────┘
```

- **Version** comes from `CARGO_PKG_VERSION`. The short git commit is added at build time by `build.rs`.
- **Credits** expands to the libraries and assets used, with their licences: gpui, gpui-component, idevice, ksni, libheif, FFmpeg, Tabler Icons.
- **Copy debug info** (in Credits) puts the app version, OS, desktop session, usbmuxd version and the connected iOS version on the clipboard, for bug reports.

## 8c. Check for updates

- **Trigger:** manual, from ☰ or the About dialog. Optionally also on startup, once a day at most (preference, off by default).
- **Request:** `GET https://api.github.com/repos/ankurk91/unspool-desktop-app/releases/latest` with header `User-Agent: Unspool/<version>`. No token is needed.
- **Compare** the release `tag_name` (e.g. `v0.2.0`, with the leading `v` stripped) with the running version using `semver`. Drafts and pre-releases are ignored, because `/releases/latest` already excludes them.

| Result | What the user sees |
|---|---|
| Newer release | "Unspool 0.2.0 is available (you have 0.1.0)" + release notes (first ~20 lines of the release body) + **[Download]** → opens the release page in the browser |
| Same or older | "You're on the latest version (0.1.0)" |
| No releases yet (404) | "No releases published yet" |
| Offline / timeout (10 s) / rate-limited (403/429) | "Couldn't check for updates — try again later" + reason under "Details" |

- The automatic startup check only shows something when an update is available, as a small banner and in the tray menu. Errors stay silent.
- The app never downloads or installs anything by itself. Updating means downloading from the release page (and later an AppImage / `.deb`).
- The last check time is stored in `ui.toml` so the once-a-day limit holds across restarts.

## 9. Icon

- Placeholder: **Tabler Icons `device-mobile-down`** (MIT, © Paweł Kuna). Saved in `assets/icons/` with its licence file.
- At build time, render PNGs (16/32/48/64/128/256) for the tray, window and `.desktop` file. Tray variants are the same glyph with a small status dot.
- Replace it with a custom icon before any public release.

## 10. Repo layout (planned)

```
unspool/
├── Cargo.toml            # workspace
├── crates/
│   ├── unspool-device/   # usbmuxd/lockdown/AFC wrapper, device info, pairing state machine
│   ├── unspool-library/  # Photos.sqlite reader, DCIM fallback, item model
│   ├── unspool-convert/  # HEIC→JPG, MOV→MP4, thumbnails
│   ├── unspool-queue/    # transfer jobs, history db, retry
│   └── unspool-app/      # gpui UI + ksni tray (binary: unspool)
├── assets/icons/
├── packaging/            # .desktop file, udev notes, later .deb/AppImage
└── PLAN.md
```

The device, library, convert and queue crates have no UI dependency, so they can be tested headless with a small CLI (`unspool-cli ls`, `unspool-cli info`).

## 11. Later (v2+)

- **Delete from phone after copy.** PTP-route lessons: deletes are permanent (they skip Recently Deleted), edited items can't be deleted over USB, and deleting a HEIC's virtual JPG does nothing. Needs its own design.
- Android phones over MTP.
- Upload the copied folder to S3 / cloud storage.
- Packaging as `.deb` and AppImage.

## 12. Milestones

| # | Deliverable | Done when |
|---|---|---|
| M0 | System packages, workspace skeleton, gpui "hello window" on this laptop | Window opens on X11 Cinnamon |
| M1 | `unspool-device` + CLI: list devices, pairing flow, device info, AFC listing, Photos DB read | `unspool-cli info` and `ls` match what `ideviceinfo`/`ifuse` showed |
| M2 | Window: device card, connection guide screens, library tree | Plug / unplug / Trust each show the right screen |
| M3 | Thumbnail grid, filters, preview pane, info panel | Smooth scroll over the whole library |
| M4 | Copy pipeline + queue panel + error history | Copying a whole month gives the same output as the 2026-10-19 `final/` folder |
| M5 | Preferences window + saved config, ☰ app menu, About dialog, GitHub links, Check for updates | Preferences survive a restart; update check handles newer / same / 404 / offline |
| M6 | Tray icon, notifications, `.desktop` file | Shows in Cinnamon's tray and app menu |
| M7 | Polish: keyboard shortcuts, empty states, first GitHub release + packaging | Check for updates finds that release |

## 13. System setup

- Rust: **installed 2026-11-12** via rustup (stable 1.99.1, in `~/.cargo` and `~/.rustup`; rustup added `. "$HOME/.cargo/env"` to `~/.bashrc` and `~/.profile`).
- Already present: build-essential, pkg-config, libssl-dev, libxkbcommon-dev, libwayland-dev, libxcb1-dev, libfontconfig-dev, libfreetype-dev, libzstd-dev, libdbus-1-dev, Vulkan (Intel Arc), usbmuxd, ffmpeg.
- Installed for M0 (confirmed 2026-11-12): clang, cmake, libxkbcommon-x11-dev, libx11-xcb-dev, libheif-dev.
- The system libheif is 1.17.6. `libheif-rs` 3.0's default needs ≥ 1.23, so it's built with the `v1_17` feature only. Covered by `crates/unspool-convert/tests/libheif_smoke.rs`.
- The usbmuxd socket (`/var/run/usbmuxd`) only exists while an Apple device is plugged in, because udev starts the daemon on demand. The connection guide (§5) must not treat a missing socket as "usbmuxd not installed" on its own. Check whether the binary is installed too.

## 14. Risks and open questions

**Risks**
- **gpui is pre-1.0.** Its API changes and its docs are thin. Mitigation: pin versions, keep UI code thin, and keep logic in the headless crates.
- ~~**gpui-component vs gpui version match.**~~ Resolved in M0: use `gpui-kit`, which pins a matching `gpui-pre`.
- **idevice on iOS 27.** Pairing and AFC must be proven in M1 before UI work starts.
- **Photos.sqlite schema changes between iOS versions.** Mitigation: detect columns at runtime and fall back to `DCIM/` browsing.
- **GNOME/Cinnamon auto-mounts the phone** (gvfs gphoto2/afc). This blocks PTP but not AFC over usbmuxd. Watch for conflicts.

**Decided (2026-11-12)**
- Destination: the user picks it. Default `~/Pictures/Unspool/<device name>/`.
- Live Photos: always stills. No motion clip, no setting.
- Existing files: Ask → Skip / Overwrite, with "do this for all", based on the destination folder only.
- Preferences window with saved settings, an About dialog, a GitHub menu item, and Check for updates via GitHub Releases.

**Open questions for you**
1. ~~GitHub repo URL~~: `github.com/ankurk91/unspool-desktop-app` (from the git remote).
2. ~~App licence~~: **MIT** (decided 2026-11-12, `LICENSE` file in the repo root).
