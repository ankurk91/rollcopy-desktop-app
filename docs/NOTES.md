# Rollcopy — engineering notes

Findings, gotchas and quirks learned while building and testing Rollcopy, so the context survives a lost
chat or a new machine. [PLAN.md](PLAN.md) holds the plan and the decisions; this file holds the "why is
it like this" details. Keep both up to date. No personal data here (see [AGENTS.md](../AGENTS.md)).

## Ground rules that shaped the code

- **Read-only towards the phone.** `MediaFs` only lists, stats and reads; files are opened `RdOnly`.
  This is also why music and "import into iPhone" are out of scope (see the end of this file).
- **No personal data in the repo** — not in code, tests, docs, screenshots or commit messages.
  Screenshots are blurred (device name, dates, places, albums) and saved as compressed JPEG.
- **Stage commits by path.** A `git add -A` once pushed a private blog draft; history had to be
  rewritten. `.gitignore` now ignores `docs/blog-*.md` and `*.draft.md`.

## iPhone, usbmuxd and pairing

- usbmuxd 1.1 pairs by itself when it has no pair record; the app waits 20 s for that before sending its
  own pair request, or the two race and the phone shows Trust twice.
- Only usbmuxd's "no record" answer means *not paired*. Treating any `get_pair_record` error (socket
  hiccup) as unpaired caused needless re-pairing and a new Trust prompt (audit REL-08).
- `InvalidHostID` = a stale record (the phone forgot this computer): re-pair.
- Saving the pair record can be refused while usbmuxd is busy: delete and retry.
- A locked phone keeps serving AFC if it was unlocked once since boot. "Unlock your iPhone" is only
  shown when the phone actually refuses.
- Every device call has a deadline: AFC open/read/stat/list 30 s, `Session::open` / `media()` 15 s, and
  the queue adds a 45 s backstop (`Opener::timeout`). Without them a silent phone hung a job or the
  reconnect probe forever (audit NEW-02, REL-05).
- After a read error while the phone is still listed, the queue re-opens it every 2 s and resumes by
  itself (REL-04).
- In VirtualBox the user must be in the `vboxusers` group, and a USB filter for vendor `05ac` is needed
  because the iPhone re-enumerates when Trust is tapped.

## Photos library (Photos.sqlite)

- Copy `Photos.sqlite` and `-wal` off the phone, never `-shm`. Stat both before and after copying and
  re-fetch (up to 3×) if iOS changed them; then `wal_checkpoint(TRUNCATE)` and `quick_check` on the
  local copy (REL-07). The copy is deleted right after reading (privacy, SEC-02).
- Columns are detected at runtime (schema changes between iOS versions). Fields used: `ZDIRECTORY`,
  `ZFILENAME`, `ZKIND` 0/1, `ZHIDDEN`, `ZTRASHEDSTATE`, `ZFAVORITE`, `ZLATITUDE`/`ZLONGITUDE` (**-180
  means no location**, not NULL), time zone offset in `ZADDITIONALASSETATTRIBUTES`, `ZIMPORTEDBYDISPLAYNAME`
  (source app), camera/lens in `ZEXTENDEDATTRIBUTES`, place name in `ZREVERSELOCATIONDATA` (an
  NSKeyedArchiver plist), albums through a join table found by `Z_<n>ALBUMS`/`Z_<n>ASSETS` columns
  with `ZKIND = 2`, edit time in `ZADJUSTMENTTIMESTAMP`.
- Edited items: the rendered version is at `PhotoData/Mutations/<dir>/<stem>/Adjustments/FullSizeRender.*`.
- Thumbnails: the phone's own `PhotoData/Thumbnails/V2/<dir>/<file>/5005.JPG` (480×360 photos,
  640×360 videos). Anything larger than ~300 px on screen looks soft; full-size needs the original.
- Names from the phone become local paths: items whose folder/file names aren't plain (`..`, absolute,
  separators) are dropped at load (WIN-09).
- iOS reuses `IMG_0001`–`IMG_9999` across `DCIM/1xxAPPLE` folders.

## Copying and destinations

- A file counts as an item's copy only if its mtime equals the capture time (Rollcopy sets it; 2 s and
  quarter-hour slack for FAT). Other files with the same name are never touched; the item goes to
  `IMG_0057 (2).JPG` (REL-01). Undated items can't be told apart.
- Each job writes `.rollcopy-<pid>-<n>-<name>.part` beside the destination; queued jobs reserve their
  path. Orphan `.part` files of dead processes are swept when that folder is written again.
- The final move is no-clobber (`renameat2 NOREPLACE` or link+unlink). GVFS shares (`/run/user/…/gvfs`),
  sshfs and some FUSE drivers support neither, so it falls back to a plain rename when nothing is there
  (NEW-01).
- Free space is checked before each item; a full disk pauses the queue instead of failing every item.
- Cancel sets a flag that kills ffmpeg (checked every 20 ms) and stops the HEIC encoder, then aborts the
  task; `spawn_blocking` work can't be aborted otherwise (NEW-03).
- One instance per session: `flock` on `~/.local/state/rollcopy/rollcopy.lock` plus the D-Bus name
  `io.github.ankurk91.Rollcopy`; a second launch calls `org.freedesktop.Application.Activate` (REL-03).

## Conversion

- **HEIC:** Ubuntu 24.04's libheif 1.17 failed on some iPhone HEICs; libheif 1.23 is built in through
  `libheif-rs`'s `embedded-libheif` (statically linked, uses the system libde265; building it needs
  `cmake` and `libnuma-dev`). EXIF orientation is reset to 1 because libheif already rotates the
  pixels; the Display P3 ICC profile is kept or colours look washed out.
- **Release .deb hygiene:** a local build picks up whatever codec `-dev` packages are installed
  (x265, aom, dav1d…) and the .deb then depends on them. CI has none. For a clean local .deb, build with
  a CMake toolchain file that sets `CMAKE_DISABLE_FIND_PACKAGE_<X> TRUE` for the extra codecs.
- **iPhone video is HDR:** HEVC Main 10, HLG, BT.2020, Dolby Vision 8.4. The default is a lossless
  remux (`-c copy -tag:v hvc1`, video + audio only), which drops the Dolby Vision box and keeps HLG.
  Celluloid/mpv may show it black and white; VLC, Android and iPhone play it fine.
- **H.264 option:** software path is zscale + `tonemap=hable` + libx264. GPU path: `tonemap_vaapi`
  refuses iPhone HLG (HDR10-with-metadata only), and full-GPU chains fail on ffmpeg 6.1/Mesa. What works
  is CPU decode → `libplacebo` (Vulkan, Hable) → `h264_vaapi` QP 24: ~40% faster, a tenth of the CPU,
  same look. Hardware decode made it *slower* (frame copies). Used only if a one-time trial encode works.
- ffmpeg runs get time limits (remux max(120 s, 4× length), H.264 max(600 s, 20×), ffprobe 60 s).
- The ffmpeg `.part` output name doesn't end in `.mp4`, so always pass `-f mp4`.

## Desktop integration (Linux)

- **Notifications on Cinnamon** vanish instantly if the sender's D-Bus connection closes
  (`_onNameVanished`): use one long-lived zbus connection.
- **Tray:** StatusNotifierItem via `ksni`; Cinnamon needs xapp-sn-watcher, GNOME the AppIndicator
  extension. The icon is a flat one-colour glyph (`packaging/icons/tray.svg`) recoloured for the app's
  theme; the full-colour icon was mush at 16–22 px. Panels don't tell apps their colour.
- **GNOME on Wayland has no server-side decorations**: the app draws its own title bar (gpui-component
  `TitleBar`) when `window.window_decorations()` is `Client`. Gotchas: the window-button icons
  (`WindowClose/Minimize/Maximize/Restore`) must be registered in `icon_assets!` or the buttons draw
  nothing; the shadow margin of `window_border` showed as a solid grey band under software rendering
  (VMs), so shadow size is 0. `ROLLCOPY_CSD=1` (debug builds) forces this look on X11 for testing.
- **Opening files:** `xdg-open` exit code 4 means no app could open it. A fresh Ubuntu may have no video
  player at all, or one whose GStreamer lacks codecs (`gstreamer1.0-libav`); the app checks
  `xdg-mime query default` and the libav plugin and explains.
- **Show in folder:** `org.freedesktop.FileManager1.ShowItems` selects the file in Nautilus and
  Dolphin, but on Mint the service belongs to `nemo-desktop`, which starts `nemo <file>` and took ages
  to show a window, so with Nemo the folder is opened instead (checked via the service owner's
  `/proc/<pid>/comm`).
- **A stuck Nemo blocks every "open folder":** the first `nemo` owns `org.Nemo`; later launches hand
  their request to it and exit. If that one hangs (seen once, no window, main thread waiting), nothing
  opens until `nemo -q`. Not Rollcopy's doing (a Nemo started from a shell hung the same way).
- **Opening HEIC:** Mint's Xviewer (and other viewers) can't read HEIC; *Open* converts to JPG first.
- **Clickable notifications:** send a `default` action and listen for `ActionInvoked` on the same
  long-lived connection; keep a small id → action map. Simulated clicks:
  `dbus-send --type=signal /org/freedesktop/Notifications org.freedesktop.Notifications.ActionInvoked uint32:<id> string:default`.
- **Old copy shadowing a new install:** `packaging/install-local.sh` puts a binary in `~/.local/bin` and a
  menu entry in `~/.local/share/applications`, both ahead of the .deb's. Use `--uninstall`.

## gpui / gpui-kit quirks

- Pin `gpui-kit` exactly (`=0.9.1`, which pins `gpui-pre =0.3.12`); gpui-component needs it, not the
  crates.io `gpui`.
- Reading an entity while it's being updated panics ("already being updated"): pass state in instead
  of reading the view inside its own render/listener.
- The NumberInput triggered an a11y `set_focus called more than once` panic in debug; dependencies are
  built with `debug-assertions = false` and a dropdown replaced the number box.
- `use gpui_kit::*` imports gpui's `test` macro: in test modules import names explicitly, never `super::*`.
- `uniform_list` needs equal row heights; variable rows (section headers) need `list`. The grid uses
  `list` over `sections::Row`s (month headings + tile rows); `ListState::reset` scrolls to the top, so
  after a layout-only change (zoom, resize) the focused item is scrolled back into view.
- Element `on_scroll` runs in the bubble phase, after a scrollable child has used the wheel; Ctrl+scroll
  zoom registers a capture-phase `window.on_mouse_event` from a `canvas`.
- Window-wide key bindings fire while typing in a text box unless scoped: grid keys use the context
  `Rollcopy && !Input`.
- `img()` sizes itself from the image's aspect ratio and grew past its box: the viewer computes the
  image box itself. gpui also ignores EXIF orientation: viewer images are made upright first.
- `cargo fmt` reflows code; scripted edits must match the formatted text.
- The settings component sizes dropdown buttons by their label; long labels run off the window.
- gpui can't hide a window on Linux; "close to tray" minimises it.

## Build, CI and release

- System packages: see README (Building from source) and `packaging/ci-deps.sh`; CI also installs
  ffmpeg so the video tests run.
- `rust-version` is 1.92 (the `oo7` dependency needs it). Builds use current stable.
- Actions use major-version tags; Dependabot runs on the 15th monthly with a 14-day cooldown and skips
  gpui-kit, idevice and libheif-rs.
- Release: push a `vX.Y.Z` tag matching Cargo.toml → build job (read-only token) → publish job (attests
  provenance, creates a **draft**). The draft is published by hand; "Check for updates" only sees
  published releases. Bump the version in Cargo.toml, README install commands and the AppStream
  metainfo `<releases>`.
- AppImage: appimagetool only reads `*.appdata.xml`, so the metainfo is copied under that name.
- Checking release status: `gh api repos/<owner>/<repo>/releases/tags/<tag>` (a drafts-blind login can
  miss drafts in the list).

## Screenshots for the README

- Never ship real names, dates or places. Rather than blurring, replace the text: measure the
  text's ink box, find the Ubuntu Sans size whose rendering of the old text has the same width, paint
  the box with the background colour and draw the new text at the same origin. Pick replacement
  names of the same width (e.g. a neutral first name) so icons don't shift. Save as JPEG ~82 quality
  and bump the `?v=N` on the README link so GitHub's image cache refreshes.

## Ideas checked and turned down

- **Music:** stored under `iTunes_Control/Music/Fxx/` with scrambled names, metadata in a separate media
  database, and Apple Music downloads are DRM-protected. Out of scope.
- **Import into iPhone:** needs writing to the phone (against the read-only rule), and files dropped in
  `DCIM` don't appear in Photos; the real path is Apple's private sync protocol. Out of scope.
- **In-app OSM map:** OSM raster tiles are allowed for light desktop use (named User-Agent, cache 7 days,
  attribution, no prefetch) but leak the photo's area; a "Open in Google Maps" link was chosen instead.
- **Windows port:** possible (gpui has a Windows backend; idevice talks to Apple Mobile Device Service on
  127.0.0.1:27015; libheif comes from vcpkg 1.23.x on MSVC) but shelved. Needed: platform module for the
  service check, tray-icon + toast notifications, single-instance over localhost, `CREATE_NO_WINDOW` for
  ffmpeg, Windows-safe folder names, MSI via cargo-wix with an AppUserModelID shortcut.
