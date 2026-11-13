# Rollcopy — plan

> Plug in your iPhone, pick photos and videos, and get Android/PC-friendly files on your laptop.

Status: **v0.1.0 released (2026-11-12). M0–M7 done**: Check for updates finds the release. Backlog for 0.1.x in §15.

---

## 1. Name

**Rollcopy**: copy your camera roll.

- Renamed from *Unspool* before the first release (2026-11-12): unspool.app is an unreleased macOS
  photo app (it adds metadata to scanned film photos), so the names would compete in search.
- Checked 2026-11-12: free on crates.io and GitHub; the only other use found was an old shell script;
  rollcopy.com/.app/.dev had no DNS records. Rejected: Tuxroll (a dice maker uses @tuxroll),
  Tuxpix (too close to the Tuxpi photo editor), Rolldock (a shipping company), Rollferry (.com taken).
- The name avoids Apple trademarks ("i…", "iPhone"); "iPhone" goes in descriptions for search.
- Binary `rollcopy`, app id `io.github.ankurk91.Rollcopy`, repo `github.com/ankurk91/rollcopy-desktop-app`.

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

Decided 2026-11-12. In v1, Rollcopy is **read-only towards the iPhone**:

- AFC is only used to list, stat and read. The device wrapper doesn't expose write, rename, remove or mkdir at all, so the rest of the app can't call them by mistake.
- `Photos.sqlite` and its `-wal` are copied to the laptop first and opened from there. The phone's copy is never opened in place. `-shm` isn't copied. Both files are stat'ed before and after the copy, and fetched again (up to 3 times) if iOS changed them meanwhile. The local copy is then checkpointed and checked with `PRAGMA quick_check` (audit REL-07), read, and deleted (SEC-02).
- No services beyond lockdown (device info), AFC (read), battery/disk-usage queries and pairing. Pairing only adds a trust record, which is the same thing tapping Trust does with Finder or iTunes.
- Deleting from the phone stays out of scope until v2 (§11), and then needs its own design and explicit confirmation.
- On the laptop side, a copy never destroys an existing file unless it's replaced at the final rename (§7.4).

## 3. What the cable connection can do

You asked what the "p2p" connection can do. It's actually two protocols. Both were used in the manual transfer on 2026-10-19.

| Protocol | What it is | What we get | Use in Rollcopy |
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

`PhotoData/Photos.sqlite` is copied read-only along with its `-wal` file (see §2a). Fields proven on 2026-10-19 are marked ✔. The rest need checking in M1.

- ✔ Folder + file name: `ZASSET.ZDIRECTORY`, `ZFILENAME`
- ✔ In Recently Deleted: `ZTRASHEDSTATE = 1`. **Hidden by default.**
- ✔ Capture date: `ZDATECREATED` (seconds since 2001-01-01 UTC), with time-zone offset from `ZADDITIONALASSETATTRIBUTES.ZTIMEZONEOFFSET`
- ✔ Edited: a `PhotoData/Mutations/DCIM/<dir>/<stem>/Adjustments/FullSizeRender.{heic,jpg,mov}` file exists
- ✔ Live Photo: a `.MOV` with the same name stem as the photo, and `ZKINDSUBTYPE = 2` on the photo's row. The clip has no row of its own.
- ✔ (iOS 27, 2026-11-12) photo vs video: `ZKIND` 0/1. Size in pixels: `ZWIDTH`/`ZHEIGHT`. Video length: `ZDURATION`. Favourite: `ZFAVORITE`. Hidden: `ZHIDDEN`. Location: `ZLATITUDE`/`ZLONGITUDE`, where **-180 means no location**, not NULL. Time zone: `ZADDITIONALASSETATTRIBUTES.ZTIMEZONEOFFSET` (seconds) joined on `ZASSET`. Original size: `ZADDITIONALASSETATTRIBUTES.ZORIGINALFILESIZE`. `ZKINDSUBTYPE = 10` looks like screenshots.
- ✔ Edited: `ZADJUSTMENTTIMESTAMP` is set exactly for the items that have a `Mutations/` folder (every one checked).
- ✔ `DCIM/` holds more files than the database has rows: on the test phone the extra files were exactly the Live Photo `.MOV` clips and the `.AAE` edit sidecars. Every row has a file.

- ✔ (2026-11-12) **Source app:** `ZADDITIONALASSETATTRIBUTES.ZIMPORTEDBYDISPLAYNAME` ("Camera", "Snapchat", "WhatsApp", "ChatGPT", "Drive", "Screenshots"; "SpringBoard" means saved from iOS). **Camera EXIF:** `ZEXTENDEDATTRIBUTES` (`ZCAMERAMAKE`/`ZCAMERAMODEL`/`ZLENSMODEL`/`ZISO`/`ZAPERTURE`/`ZSHUTTERSPEED`/`ZFOCALLENGTH`). **Street address:** `ZREVERSELOCATIONDATA`, an NSKeyedArchiver plist with `addressString` at its root. **User albums:** `ZGENERICALBUM` with `ZKIND = 2`, joined through the table whose columns are `Z_<n>ALBUMS` / `Z_<n>ASSETS` (`Z_34ASSETS` on iOS 27; the numbers change, so the table is found by column names).

The database is optional. If it can't be read, fall back to plain `DCIM/` browsing with file dates. On the test phone the fallback found the same items and the same Live Photos as the database (checked in M1), but it can't tell which items are in Recently Deleted.

## 4. Technology

| Need | Choice | Notes |
|---|---|---|
| UI | **gpui-kit =0.9.1** (bundles gpui-component 0.9.1 + `gpui-pre` =0.3.12 + the Linux platform) | Checked in M0: gpui-component 0.9.x is built on `gpui-pre` (a crates.io snapshot of Zed's gpui), **not** `gpui` 0.2.x. `gpui_kit::application()` provides the X11/Wayland platform. No Zed git rev needed. Pinned exactly. |
| iPhone access | **idevice =0.1.68** (pure Rust, MIT): `default-features = false`, features `ring`, `usbmuxd`, `pair`, `afc`, `device_info`, `diagnostics_relay` | Builds (M0). `ring` replaces the default aws-lc crypto. No `libimobiledevice-dev` needed. Needs the system `usbmuxd` daemon (already installed). |
| Async | tokio runtime on a background thread; results sent to the UI over channels | Keep all device I/O off the UI thread. |
| Photos DB | `rusqlite` (bundled SQLite) | |
| HEIC → JPG | `libheif-rs` =3.0.0 with **`embedded-libheif`** (a bundled libheif 1.23, built with cmake against the system `libde265`) + `image` 0.25 for JPEG encoding (EXIF and ICC support built in) | M4 found that the system libheif 1.17.6 **can't open about a third of the** iOS 27 HEICs tested ("Non-existing item ID referenced": portrait/depth photos). The bundled build also links x265 and needs **`libnuma-dev`**. | Keeps EXIF data and applies rotation. |
| MOV → MP4 | `ffmpeg` command-line tool, run as a subprocess | Lossless remux: `-c copy -tag:v hvc1 -movflags +faststart`. Optional H.264 re-encode setting. |
| Thumbnails | **iOS's own** `PhotoData/Thumbnails/V2/<dir>/<file>/5005.JPG` (found in M3) | Present for every item on iOS 27, photos and videos: 480×360 for photos, 640×360 for videos, about 50–85 KB each. Copied once into the thumbnail cache. No HEIC decoding and no video copy is needed for the grid or the preview. Fallback when it's missing: a type icon. ffmpeg / libheif thumbnails only if a phone turns out not to have them. |
| Tray icon | **ksni 0.3.x** (StatusNotifierItem over D-Bus) | gpui has no tray support. On Cinnamon the tray applet must be enabled. |
| Preferences | `serde` + `toml` → `~/.config/rollcopy/config.toml` (path from the `directories` crate) | See §8a |
| Job history | `rusqlite` → `~/.local/state/rollcopy/history.sqlite` | |
| Folder picker | gpui's `prompt_for_paths` (uses the XDG desktop portal on Linux) | Fallback: `rfd` |
| Update check | `ureq` (rustls) + `semver` | GitHub Releases API, see §8c |
| Open links / files | `open` crate (`xdg-open`) | GitHub page, release page, video player |
| Thumbnail cache | `~/.cache/rollcopy/thumbs/<udid>/` | |

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

**During a copy:** if the phone locks or the cable drops, AFC returns I/O errors. Seen on 2026-10-19. The queue then **pauses** and shows "Unlock your iPhone / reconnect". It doesn't record a pile of failures. When the device comes back, the queue resumes from where it stopped. If the phone never left usbmuxd's list (a short cable glitch), the queue tries to reconnect every 2 s and resumes on its own; *Resume* also retries straight away (audit REL-04). Each AFC open, read or close during a copy gives up after 30 s and counts as a disconnect, so a wedged phone pauses the queue instead of holding a worker (REL-05).

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
│ ⬇ Rollcopy   [ Save to: ~/Pictures/Rollcopy/My iPhone  📁 ]  [Copy selected (12)]  ☰ │
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
- **Save to**: shows the destination folder for the selected device. 📁 opens a folder picker. The default is `~/Pictures/Rollcopy/<device name>/`, and the choice is remembered (§8a).
- **☰ app menu**:
  - Preferences… (Ctrl+,)
  - Check for Updates…
  - View on GitHub
  - Report an Issue (opens GitHub Issues)
  - About Rollcopy
  - Quit (Ctrl+Q)

**Left pane: devices and library**
- Device card with name, model, iOS version, storage bar and battery. "Details" opens the serial number, UDID and iCloud Backup hint.
- The library tree is grouped **year → month** by capture date. An "On-phone folders" view (`100APPLE`, …) is available as an advanced toggle.
- Recently Deleted is listed with its count but greyed out, and excluded from "copy folder". It's off by default (Preferences → Library).
- **Hidden album** (audit SEC-01, 2026-11-13): iOS locks it behind Face ID or the passcode, so its items are left out of the tree, the grid, the counts and every copy action, and their cached thumbnails are deleted. Preferences → Library → *Show Hidden album* lists them under their own **Hidden** node, never inside All media, so "Copy all" never includes them.
- **Blurred Hidden items and list view** (2026-11-13, at the user's request). With *Show Hidden album* on, Hidden items show a tiny, blurred copy of their thumbnail (`<thumb>.veil.jpg`, 48 px) with an 👁 button. Clicking it reveals that item until quit, until it's blurred again from the preview pane, or until the preference is turned off. No full-size image is fetched while an item is blurred. A grid/list toggle in the header switches to a list without thumbnails (name, date, size, file size, tags, copied mark); no thumbnails are requested in list mode. The choice is saved in `ui.toml`.

**Centre pane: thumbnail grid**
- Filter chips: All / Photos / Videos / Edited, plus Copied / Not copied (click the active one again to turn it off). Copied state comes from the destination folder, as for the badges.
- Sort by date taken: Newest first (default) / Oldest first; undated items last.
- Place and Location in the info panel have copy-to-clipboard buttons.
- Badges on each tile: ▶ video with duration, EDITED and LIVE text badges (a pencil looked clickable), and a green sync icon for items already in the current destination folder. Selection is a separate blue ✓.
- Selection: click, Ctrl+click, Shift+click, Ctrl+A, Esc to clear.
- Keyboard (added in M3 at the user's request): ← → ↑ ↓ move the focused item and update the preview, Shift+arrows extend the selection, Space / Enter open the item.
- The preview pane is resizable by dragging its left edge (220 px minimum; the grid keeps at least one column). Its width isn't saved yet; that comes with `ui.toml` in M5.
- Right-click menu: Copy, Copy folder, Show info.
- Thumbnails are virtualised and lazy-loaded. A 5,000-item library must scroll smoothly. Only rows on screen are built, and only their thumbnails are requested, newest request first. `ROLLCOPY_DEV_REPEAT=N` repeats the library N times to test this with a small phone.
- Double-click a tile opens it like "Open" (the system viewer). In-app fullscreen with ← / → is not built yet.
- Not handled yet: the cached thumbnail isn't refreshed if an item is edited on the phone after it was cached.

**Right pane: preview and info**
- Photos: a large preview. Double-click opens fullscreen with ← / → to step through.
- Videos: a poster frame. "Open ▶" copies the file to a temp location and opens it in the system player with `xdg-open`, because gpui has no video widget.
- Info: file name, type, capture date and time zone, pixel size, length, file size, edited / Live / favourite, **source app**, **camera, lens, exposure**, **place** (street address), location coordinates (no map in v1), **albums**, **what it will be saved as** (e.g. "HEIC → JPG"), and **whether it's already copied** (and where).
- The pane is hidden until an item is selected; × closes it, and the grid then takes the full width with tiles stretched to fill each row.

**Bottom: transfer queue**
- Collapsed: one summary line with done / total, bytes, failed count and time left.
- Expanded: a list with a status icon per item and the local time it finished. Failed rows show the reason, plus "Retry" and "Retry all failed". The header also has Pause/Resume, Clear finished, Export error report (CSV) and Open folder.
- Each item moves through: Queued → Reading → Converting → Verifying → Done / Failed / Skipped / Overwritten.

**Tray icon (ksni)**
- The icon changes with status: idle / device connected / copying (progress tooltip) / error.
- Menu: Show Rollcopy · `<device name>` · Pause/Resume · Quit.
- Closing the window keeps the app in the tray **only while a copy is running**. Otherwise it quits. As built in M6: closing during a copy **minimises the window** (gpui can't hide windows on Linux) and sends a "still copying" notification. *Show Rollcopy* or a click on the icon brings it back. When the copies finish it notifies and **stays in the tray** (changed 2026-11-12 at the user's request; quitting there also cut off the notification). Quit from the tray menu, or bring the window back and close it. With no tray host, or with the preference off, closing asks "Copies are still running: Keep copying / Quit".
- **One instance per session** (audit REL-03, 2026-11-13). Starting Rollcopy again, which is the normal way back from the tray, used to start a second process that shared the staging and cache files with the first one. Now the first instance holds a `flock` on `~/.local/state/rollcopy/rollcopy.lock` and owns the D-Bus name `io.github.ankurk91.Rollcopy` (`org.freedesktop.Application.Activate`). A second launch calls Activate, which brings the first window to the front, then exits. Staging files are named `<pid>-<n>.<ext>` as well, so they never clash.
- Checked 2026-11-12: Linux Mint Cinnamon has `xapp-sn-watcher` (StatusNotifierWatcher) and the xapp-status applet, and the icon shows in the top panel.
- Optional desktop notification when a queue finishes, only when Rollcopy isn't the active window (preference, on by default).
- Notifications also cover pause and resume (including a disconnect pause). They are sent over **one D-Bus connection kept open for the app's lifetime**: Cinnamon deletes an app's notifications as soon as their sender disconnects (`_onNameVanished` in notificationDaemon.js), so a connection per notification played the sound but showed nothing. `ROLLCOPY_DEBUG_NOTIFY=1` sends a test notification at startup.

**Copy actions**
- **One item:** the Copy button in the preview pane.
- **Selected items:** the toolbar button "Copy selected (n)".
- **Whole folder:** right-click a month or year → "Copy folder".

## 7. Copy pipeline

These are the rules proven in the manual transfer, turned into code.

1. **Pick the source.** Use the edited version (`FullSizeRender.*`) if it exists, otherwise the original.
2. **Convert to compatible formats** (`rollcopy-convert`; checked on real files 2026-11-12):
   - HEIC → JPG keeps the Display P3 ICC profile, Make/Model, the capture time with its offset, and GPS. The pixels are rotated by libheif, so the EXIF orientation is reset to 1. A 12 MP photo takes about 2.6 s in a debug build.
   - MOV → MP4 keeps creation time, GPS and Make, maps only video and audio (iPhone MOVs carry five Apple data tracks), tags HEVC as `hvc1`, and drops the Dolby Vision box, leaving plain HLG.
   - JPG / PNG / GIF / MP4 → copied byte for byte.
   - HEIC → JPG, quality 92, keeping EXIF and applying rotation.
   - MOV (HEVC) → MP4 by lossless remux. Preference: "Re-encode to H.264 for old devices" (off by default).
   - **iPhone videos are HDR** (checked 2026-11-12): HEVC Main 10, HLG (`arib-std-b67`), BT.2020, Dolby Vision profile 8.4. Celluloid (mpv) on this laptop played one in **black and white**, while ffmpeg decodes it in full colour. A lossless remux keeps the HDR. **Decided 2026-11-12: Lossless stays the default.** The MP4 is correct: it decodes in colour with ffmpeg (software and VA-API), VLC, Android and iPhone; only Celluloid shows it in black and white. The user plays videos in VLC. The H.264 option stays as a preference (M5) for old devices. It's slow in software because of the float tone-map. **GPU path (2026-11-13):** `tonemap_vaapi` refuses iPhone HLG (it only does HDR10 with mastering metadata), and fully-GPU chains (VA-API decode → Vulkan → VA-API) fail on ffmpeg 6.1 with Mesa. What works: software decode, then `libplacebo` (Vulkan) with the **Hable** curve to match the software look, then `h264_vaapi` at QP 24. On a real 15 s 1080p HLG clip that took 7.7 s against 13.0 s in software, used a tenth of the CPU, gave a smaller file (11.7 MB against 20.9 MB) and scored SSIM 0.98 against the software result with the same brightness. Rollcopy uses it when a one-time trial encode on `/dev/dri/renderD*` succeeds, and falls back to software if the trial or a conversion fails (`ROLLCOPY_NO_GPU` forces software). Try dropping the Dolby Vision metadata and keeping only the HLG layer. The H.264 option should also tone-map to SDR: `zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv,format=yuv420p` gave correct colours.
   - Live Photos → **still only, always.** The motion clip is never copied. There's no setting for it.
3. **Arrange the destination** as `<dest>/<YYYY>/<MM>/images|videos/<IMG_xxxx>.<ext>`, using the capture date in its own time zone. `<dest>` defaults to `~/Pictures/Rollcopy/<device name>/`. A flat-folder layout is available as a preference.
4. **Write safely:** write to a hidden `.rollcopy-<pid>-<n>-<name>.part` beside the destination (unique per job, so two jobs never share one), check it (size for straight copies; decodes for converted files), then rename. Set the file's modified time to the capture date. On overwrite, the old file is only replaced at the final rename, so a failed copy never destroys it. Without overwrite the rename is no-clobber (`renameat2` NOREPLACE, or link + unlink), and the folder is fsynced afterwards.
5. **Same name, different photo** (audit REL-01, fixed 2026-11-13). iOS reuses `IMG_0001`–`IMG_9999` across `DCIM/1xxAPPLE` folders, so two items can map to one path. A file counts as an item's copy only when its modified time is the item's capture time (Rollcopy always sets it; 2 s and whole-quarter-hour slack for FAT). Any other file there is left alone, and the item goes to `IMG_0057 (2).JPG`, `(3)`, … Queued items reserve their path, and the path is re-checked when the job starts. Undated items can't be told apart, so any file at their path counts as theirs.
6. **If this item's copy already exists**, show a dialog:

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
   - There's no per-device "copied" database. ✓ badges and conflicts come from what's actually in the destination folder (the file and its modified time), so they stay correct if you move or delete files there. A copy whose modified time was changed by another program counts as not copied, and copying again makes a `(2)` file next to it rather than replacing it.
7. **Limits and cancelling** (audit 2026-11-13). ffprobe gets 60 s. ffmpeg gets max(120 s, 4× the clip length) for a remux and max(600 s, 20×) for H.264; then it's killed and the item fails. Queued or running items can be cancelled one by one or all at once; a cancelled job's temporary files are removed. Before each item, and after any failure, free space is checked: the destination needs the item's size (twice that when converting) plus 64 MB, and the cache needs the original when converting. Too little pauses the queue with "The destination disk is full" rather than failing every item (REL-06). A crash can still leave a `.rollcopy-<pid>-…part` file; the first job writing to that folder later deletes those whose process is gone (REL-09).
8. **Concurrency:** 2 AFC reads at once, plus 1–2 conversion workers. Never more than 1 ffmpeg per CPU core.

## 8. Error tracking

- Each failed item is saved with time, device, source path, stage (read / convert / write / verify), error text and retry count. History goes to `~/.local/state/rollcopy/history.sqlite`.
- The queue shows a red badge with the failed count. Clicking it filters the queue to failures.
- "Export error report" saves a CSV for support or debugging.
- Disconnects pause the queue instead of failing items (§5). Real per-file failures, such as a corrupt file or ffmpeg errors, are retried once automatically and then left for the user.

## 8a. Preferences window

Opened from ☰ → Preferences… or Ctrl+,. It's a small separate window with sections. Changes apply immediately (no OK/Apply button).

| Section | Setting | Default |
|---|---|---|
| **Destination** | Base folder | `~/Pictures/Rollcopy/` |
| | Subfolder per device | On (`<base>/<device name>/`) |
| | Folder layout | Year / Month / images·videos, or Flat |
| | When a file already exists | Ask / Always skip / Always overwrite (default **Ask**) |
| **Conversion** | JPEG quality for HEIC (dropdown of presets 80–100; the number input hit a gpui focus panic in debug builds) | 92 |
| | Videos | Lossless MP4 (default) / Re-encode to H.264 |
| **Library** | Show Hidden album (own node, never in All media) | Off |
| | Show Recently Deleted (greyed, never copied) | Off |
| | Default view | By date / On-phone folders |
| **App** | Theme | System / Light / Dark (a toolbar toggle cycles through them already, from M4; dark mode uses softer greys than the stock theme) |
| | Keep running in tray while copying | On |
| | Notify when transfers finish | On |
| | Check for updates on startup (at most once a day) | Off |
| | Reset and restart (two-step confirm): deletes `config.toml`, `ui.toml` and `history.sqlite`, then restarts. The phone, copied files and the thumbnail cache are untouched | — |
| **Advanced** | Parallel reads / conversion workers | 2 / auto |

In M5 the window has Destination, Conversion, Library (Show Recently Deleted) and App (Theme, Check for updates on startup). Not there yet: *Default view* (the on-phone folders view isn't built), *Keep running in tray* and *Notify* (they arrive with the tray in M6), and *Advanced* (worker count is fixed at 2; there's no log file yet). The toolbar's 📁 choice is stored under `[destination.devices]` and can be cleared in Preferences → Destination.
| | Open logs folder (button) | — |

**How preferences are saved:**
- File: `~/.config/rollcopy/config.toml`. Each preference carries a default (`serde(default)`), so a file from an older version still loads. Unknown keys are ignored.
- Writes go to a temp file that is then renamed into place, so a crash never leaves a half-written config.
- If the file is broken, rename it to `config.toml.bad`, start with defaults, and show a one-line notice.
- Window size and position, the last selected device, and the queue collapsed/expanded state are also remembered, in a separate file: `~/.local/state/rollcopy/ui.toml`.
- A destination folder you choose with 📁 in the toolbar is saved per device (keyed by UDID). Otherwise the base + device-name default is used.

## 8b. About dialog

A small modal opened from ☰ → About Rollcopy:

```
┌─────────────────────────────────────────┐
│               [icon 96px]               │
│                Rollcopy                  │
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
- **Credits** expands to the libraries and assets used, with their licences: gpui, gpui-component, idevice, ksni, libheif, libde265, FFmpeg, Lucide icons (MIT, via gpui-kit-assets).
- **Copy debug info** (in Credits) puts the app version, OS, desktop session, usbmuxd version and the connected iOS version on the clipboard, for bug reports.

## 8c. Check for updates

- **Trigger:** manual, from ☰ or the About dialog. Optionally also on startup, once a day at most (preference, off by default).
- **Request:** `GET https://api.github.com/repos/ankurk91/rollcopy-desktop-app/releases/latest` with header `User-Agent: Rollcopy/<version>`. No token is needed.
- **Compare** the release `tag_name` (e.g. `v0.2.0`, with the leading `v` stripped) with the running version using `semver`. Drafts and pre-releases are ignored, because `/releases/latest` already excludes them.

| Result | What the user sees |
|---|---|
| Newer release | "Rollcopy 0.2.0 is available (you have 0.1.0)" + release notes (first ~20 lines of the release body) + **[Download]** → opens the release page in the browser |
| Same or older | "You're on the latest version (0.1.0)" |
| No releases yet (404) | "No releases published yet" |
| Offline / timeout (10 s) / rate-limited (403/429) | "Couldn't check for updates — try again later" + reason under "Details" |

- The automatic startup check only shows something when an update is available, as a small banner and in the tray menu. Errors stay silent.
- The app never downloads or installs anything by itself. Updating means downloading from the release page (and later an AppImage / `.deb`).
- The last check time is stored in `ui.toml` so the once-a-day limit holds across restarts.

## 9. Icon

- Since 0.1.1: a film roll unrolling into a download arrow, purple squircle (`packaging/icons/io.github.ankurk91.Rollcopy.svg`). It was designed as a raster image for the project and redrawn as a clean SVG, because the raster had no real transparency. Before 0.1.1 it was a Tabler `device-mobile-down` placeholder.
- The same SVG is used by the `.desktop` file and notifications, and is rasterised at runtime with `resvg` for the tray (22/32/48 px, plus a green/blue/red status dot). No build-time PNGs needed.

## 10. Repo layout (planned)

```
rollcopy/
├── Cargo.toml            # workspace
├── crates/
│   ├── rollcopy-device/   # usbmuxd/lockdown/AFC wrapper, device info, pairing state machine
│   ├── rollcopy-library/  # Photos.sqlite reader, DCIM fallback, item model
│   ├── rollcopy-convert/  # HEIC→JPG, MOV→MP4, thumbnails
│   ├── rollcopy-queue/    # transfer jobs, history db, retry
│   └── rollcopy-app/      # gpui UI + ksni tray (binary: rollcopy)
├── packaging/            # .desktop file, app icon SVG, install-local.sh (~/.local, --uninstall); later .deb/AppImage
└── docs/PLAN.md
```

The device, library, convert and queue crates have no UI dependency, so they can be tested headless with a small CLI (`rollcopy-cli ls`, `rollcopy-cli info`).

## 11. Later (v2+)

- **Delete from phone after copy.** PTP-route lessons: deletes are permanent (they skip Recently Deleted), edited items can't be deleted over USB, and deleting a HEIC's virtual JPG does nothing. Needs its own design.
- Android phones over MTP.
- Upload the copied folder to S3 / cloud storage.
- Packaging as `.deb` and AppImage.

## 12. Milestones

| # | Deliverable | Done when |
|---|---|---|
| M0 | System packages, workspace skeleton, gpui "hello window" on this laptop | Window opens on X11 Cinnamon |
| M1 | `rollcopy-device` + CLI: list devices, pairing flow, device info, AFC listing, Photos DB read | `rollcopy-cli info` and `ls` match what `ideviceinfo`/`ifuse` showed |
| M2 | Window: device card, connection guide screens, library tree | Plug / unplug / Trust each show the right screen |
| M3 | Thumbnail grid, filters, preview pane, info panel | Smooth scroll over the whole library |
| M4 | Copy pipeline + queue panel + error history | Copying a whole month gives the same output as the 2026-10-19 `final/` folder |
| M5 | Preferences window + saved config, ☰ app menu, About dialog, GitHub links, Check for updates | Preferences survive a restart; update check handles newer / same / 404 / offline |
| M6 | Tray icon, notifications, `.desktop` file | Shows in Cinnamon's tray and app menu |
| M7 | Polish: keyboard shortcuts, empty states, first GitHub release + packaging | Check for updates finds that release |

**Release pipeline (M7):** `.github/workflows/ci.yml` (fmt, clippy `-D warnings`, tests on `ubuntu-24.04`, amd64) on pushes to main and on PRs. `.github/workflows/release.yml` runs on tags `vMAJOR.MINOR.PATCH` (glob filter plus a regex check, and the tag must equal the Cargo.toml version), or by hand. It builds the `.deb` and AppImage with SHA256SUMS, uploads them as an artifact, and for tags creates a **draft** release, which is published by hand. Targets: Ubuntu 24.04+ (glibc 2.39), amd64 only. The AppImage needs `usbmuxd` and `ffmpeg` on the host; the `.deb` depends on them.

**Pipeline (audit CI-01…10, 2026-11-13).** Actions use major-version tags (`@v7`, decided by the user: SHA pins are hard to maintain). Dependabot (`.github/dependabot.yml`) opens one grouped PR per ecosystem on the 15th of each month, with a 14-day cooldown, and skips the deliberately pinned gpui-kit, idevice and libheif-rs. Every cargo command uses `--locked`, and checkouts don't keep the token. CI also validates the `.desktop` file and the AppStream metainfo, and installs ffmpeg so the video test runs. The release is split: a **build** job with a read-only token, no cache and no test re-run, and a **publish** job that only downloads the artifact, attests build provenance (`gh attestation verify <file> -R ankurk91/rollcopy-desktop-app`) and creates the draft. `rust-version` is 1.92 (the `oo7` dependency needs it). Dropped at the user's request: the MSRV job and cargo-deny. Not done: a Windows check job (Linux only for now).

## 13. System setup

- Rust: **installed 2026-11-12** via rustup (stable 1.99.1, in `~/.cargo` and `~/.rustup`; rustup added `. "$HOME/.cargo/env"` to `~/.bashrc` and `~/.profile`).
- Already present: build-essential, pkg-config, libssl-dev, libxkbcommon-dev, libwayland-dev, libxcb1-dev, libfontconfig-dev, libfreetype-dev, libzstd-dev, libdbus-1-dev, Vulkan (Intel Arc), usbmuxd, ffmpeg.
- Installed for M0 (confirmed 2026-11-12): clang, cmake, libxkbcommon-x11-dev, libx11-xcb-dev, libheif-dev.
- Needed from M4: `libde265-dev` (already present) and **`libnuma-dev`** (`sudo apt install libnuma-dev`). The bundled libheif links x265, and x265's static link needs `-lnuma`.
- The system libheif is 1.17.6. It builds, but it's **too old for iOS 27 photos** (see §4). Rollcopy now uses the bundled libheif 1.23 instead.
- The usbmuxd socket (`/var/run/usbmuxd`) only exists while an Apple device is plugged in, because udev starts the daemon on demand. The connection guide (§5) must not treat a missing socket as "usbmuxd not installed" on its own. Check whether the binary is installed too.

## 14. Risks and open questions

**Risks**
- **gpui is pre-1.0.** Its API changes and its docs are thin. Mitigation: pin versions, keep UI code thin, and keep logic in the headless crates.
- ~~**gpui-component vs gpui version match.**~~ Resolved in M0: use `gpui-kit`, which pins a matching `gpui-pre`.
- **idevice on iOS 27.** Pairing and AFC must be proven in M1 before UI work starts.
- **Photos.sqlite schema changes between iOS versions.** Mitigation: detect columns at runtime and fall back to `DCIM/` browsing.
- **GNOME/Cinnamon auto-mounts the phone** (gvfs gphoto2/afc). This blocks PTP but not AFC over usbmuxd. Watch for conflicts.

**Decided (2026-11-12)**
- Destination: the user picks it. Default `~/Pictures/Rollcopy/<device name>/`.
- Live Photos: always stills. No motion clip, no setting.
- Existing files: Ask → Skip / Overwrite, with "do this for all", based on the destination folder only.
- Preferences window with saved settings, an About dialog, a GitHub menu item, and Check for updates via GitHub Releases.

**Open questions for you**
1. ~~GitHub repo URL~~: `github.com/ankurk91/rollcopy-desktop-app` (from the git remote).
2. ~~App licence~~: **MIT** (decided 2026-11-12, `LICENSE` file in the repo root).

## 15. Backlog after 0.1.0

**0.1.1 polish**
- Empty states: empty month/filter, empty transfer list, no Photos database.
- Keyboard shortcuts list in the app (☰ menu).
- In-app fullscreen viewer with ← / →.
- Refresh cached thumbnails for items edited on the phone after caching.

**Still to test**
- Wayland session (only X11 tested so far) and GNOME (its tray needs the AppIndicator extension).
- A long copy with the phone auto-locking.
- Re-pairing after the phone forgets this computer (`InvalidHostID` path).
- A second device (another iPhone, an iPad).

**Speed**
- Faster cold CI builds (a `ci` profile with unoptimised dependencies).

**Reach**
- Listings: AppImageHub, AlternativeTo, AUR; later Flathub (Flatpak needs access to the host's usbmuxd socket).
- A landing page (GitHub Pages) and a repo social-preview image.
- ~~Own app icon~~ (done for 0.1.1).

