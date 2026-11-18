<p align="center">
  <img src="packaging/icons/io.github.ankurk91.Rollcopy.svg" width="128" height="128" alt="Rollcopy logo: a film roll unrolling into a download arrow">
</p>

# Rollcopy: copy iPhone photos and videos to Linux

[![CI](https://github.com/ankurk91/rollcopy-desktop-app/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/ankurk91/rollcopy-desktop-app/actions/workflows/ci.yml)
[![Latest release](https://img.shields.io/github/v/release/ankurk91/rollcopy-desktop-app)](https://github.com/ankurk91/rollcopy-desktop-app/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/ankurk91/rollcopy-desktop-app/total?cacheSeconds=3600)](https://somsubhra.github.io/github-release-stats/?username=ankurk91&repository=rollcopy-desktop-app&page=1&per_page=30)
[![Licence: MIT](https://img.shields.io/github/license/ankurk91/rollcopy-desktop-app)](LICENSE)
[![Built with Rust](https://img.shields.io/badge/built%20with-Rust-CE422B?logo=rust&logoColor=white)](https://www.rust-lang.org)

A **Linux desktop app** that copies photos and videos from an iPhone over a USB cable, as files any
Android phone, PC or TV can open. No iTunes, no cloud, no account.

Plug in the phone, tap **Trust**, pick what you want, and Rollcopy copies it to
`~/Pictures/Rollcopy/<phone name>/YYYY/MM/images|videos/`, converting HEIC to JPG and MOV to MP4
on the way.

![Rollcopy on Linux Mint in dark mode: a sidebar with the library by year and month, albums, Hidden and Recently Deleted and the phone at the bottom; a thumbnail grid with filter toggles, copied and Live badges and two selected photos; the info pane with photo details; and the finished transfers](.github/screenshot.jpg?v=2)

> **Status:** early (0.6.0). Linux only. Tested on Linux Mint 22 (Cinnamon, X11) with iOS 27, and on
> Ubuntu 26.04 (GNOME, Wayland) in a virtual machine.

## Features

- **Browse the Photos library**: by year and month, or by **album**, with month headings in the
  grid; zoomable thumbnails; sort by date, name, size or type; keyboard navigation.
- **Filters** for photos, videos, edited, favourites, Live Photos, screenshots and selfies, and
  copied / not copied; **search** by name, place, album, app or camera (Ctrl+F).
- **Viewer** (Space): full-window, zoom and pan (scroll, double-click for 100%, drag), details (I),
  and *Show in folder* for items you've copied.
- **List view** with resizable columns: names, dates and sizes without any thumbnails, quicker, and
  nothing on screen for people looking over your shoulder.
- **Copy new**: one click copies everything you haven't copied yet. Photos taken while the phone is
  plugged in appear with **Reload** (F5).
- **Private by default**: the Hidden album (locked by Face ID on the phone) and Recently Deleted
  aren't shown or copied unless you turn them on in Preferences, and then only under their own
  entries, never in "All media" or albums. Even then, Hidden items stay blurred (in the grid, the
  preview and the viewer) until you click the 👁 on each one.
- **Details for each item**: date and time zone, size, camera and lens, the app that saved it
  (Camera, WhatsApp, Snapchat, …), place name, location (open it in Google Maps), albums.
- **Android/PC-friendly copies**:
  - HEIC → JPG (keeps colour profile and EXIF, applies rotation)
  - MOV → MP4 (lossless by default; optional H.264 re-encode for old devices, using the GPU through
    VA-API and Vulkan when available)
  - Edited photos and videos are copied as edited; Live Photos as the still, optionally with the
    motion part as a video (Preferences → Conversion).
- **Safe copying**: files are written to a temporary file, checked, then renamed, so a failed copy
  never damages an existing file. Already-copied items are recognised in the folder by name and
  capture time. iPhones reuse names like `IMG_0057` after 9,999 photos; a different photo with the
  same name is saved as `IMG_0057 (2).JPG`, never over another file. For an item copied before, you
  choose Ask / Skip / Overwrite.
- **Transfer queue** with overall progress and time left, time taken per item, pause/resume,
  cancel, retry and an exportable error report; the panel and its columns can be resized. Unplugging or locking the phone
  pauses the queue instead of failing items, and it carries on when the phone is back. A full disk
  pauses it too.
- **Tray icon and notifications**: closing the window during a copy keeps Rollcopy running in the tray;
  starting Rollcopy again brings the window back. Click the "Transfers finished" notification to open
  the folder with your copies.
- **First-run guide**: three steps to your first copy (☰ → Getting Started shows it again).
- **Preferences** for the destination, folder layout, JPEG quality, video format, Live Photo motion,
  the Hidden album and theme.

### Your phone is safe

Rollcopy **only reads** from the phone. It has no code to write, rename or delete anything on the
device. The only change it makes is the trust record created when you tap **Trust**, the same as
connecting to a Mac or PC.

### Network use

None, except **Check for updates** (manual, or optionally once a day), which asks GitHub for the
latest release. Nothing is uploaded. The 📍 button next to a photo's location opens Google Maps in
your browser, only when you click it.

## Requirements

- Ubuntu 24.04 or newer, or a distribution based on it (Linux Mint 22+, Pop!_OS 24.04, …), 64-bit x86.
- `usbmuxd` (talks to the phone) and `ffmpeg` **6.1 or newer** (video conversion; Ubuntu 24.04
  ships 6.1.1). The optional H.264 re-encode also needs ffmpeg's `zscale` and `tonemap` filters,
  which Ubuntu's build includes:

  ```sh
  sudo apt install usbmuxd ffmpeg
  ```

- A USB cable that carries data.

## Install

Download the latest `.deb` or `.AppImage` from
[Releases](https://github.com/ankurk91/rollcopy-desktop-app/releases).

**.deb** (installs `usbmuxd` and `ffmpeg` if needed):

```sh
sudo apt install ./rollcopy_0.6.0-1_amd64.deb
```

**AppImage** (install `usbmuxd` and `ffmpeg` first, see Requirements):

```sh
chmod +x Rollcopy-0.6.0-x86_64.AppImage
./Rollcopy-0.6.0-x86_64.AppImage
```

**From source** for your user only (see [Building from source](#building-from-source)):

```sh
packaging/install-local.sh              # builds and installs into ~/.local
packaging/install-local.sh --uninstall
```

## Using it

1. Connect the iPhone with a cable and unlock it.
2. Tap **Trust** on the phone and enter your passcode when asked.
3. Press **Copy new** to copy everything you haven't copied yet. Or pick items (click,
   Ctrl/Shift+click, Ctrl+A, arrow keys) and press **Copy selected**, or **Copy all** for the year,
   month or album you're viewing. Switch between the grid and the list
   with the buttons next to *Newest first*.
4. Change where files go with **Save to** (📁) or in **Preferences** (⚙ or Ctrl+,).
5. Took new photos while plugged in? Press **↻** in the toolbar (or F5).

Keyboard: arrow keys move, Shift+arrows extend the selection, Space opens the viewer, Ctrl+F
searches and I shows or hides the info pane. Press **F1** in the app for the full list.

## Help

- [Troubleshooting](docs/TROUBLESHOOTING.md): phone not detected, Trust prompt, videos that won't
  play or play in black and white, slow "Show in folder", network shares.
- [FAQ](docs/FAQ.md): HEIC to JPG, Live Photos and edited photos, other distributions, deleting
  from the phone.

Please report problems with *☰ → Report an Issue*: it opens a GitHub issue form with the version,
Linux version and phone model filled in (never the phone's name or serial). Or use
[Issues](https://github.com/ankurk91/rollcopy-desktop-app/issues) directly.

## Where Rollcopy keeps things

| Path | What |
|---|---|
| `~/.config/rollcopy/config.toml` | Preferences |
| `~/.local/state/rollcopy/` | Window layout, failed-copy history, `rollcopy.log` (attach it to bug reports) |
| `~/.cache/rollcopy/` | Thumbnails (safe to delete) |

The cache and state folders are readable by you only. While a phone loads, Rollcopy copies its Photos
database (dates, places, albums) into `~/.cache/rollcopy/db/` and deletes it as soon as the library is read.
Full-size photos for the viewer and files you *Open* are kept in `~/.cache/rollcopy/view/` and `open/` until
Rollcopy quits. Thumbnails stay until you reset; those of Hidden-album items are deleted unless you show that album.

*Preferences → App → Reset and restart* clears all three (never your copied files or the phone).

## Building from source

Ubuntu 24.04 packages (the same list CI uses):

```sh
packaging/ci-deps.sh      # build-essential cmake clang libxkbcommon-dev libde265-dev …
```

Then, with Rust stable from [rustup](https://rustup.rs):

```sh
cargo run -p rollcopy-app                      # run the app
cargo test --workspace                        # tests
cargo clippy --workspace --all-targets        # lints
cargo deb -p rollcopy-app                      # .deb (needs cargo-deb 3.8)
packaging/build-appimage.sh                   # AppImage
```

The workspace is split into headless crates (device access, Photos library, conversion, copy queue)
and the gpui app. [docs/PLAN.md](docs/PLAN.md) has the design, decisions and what was learned about iOS along
the way. AI coding agents: see [AGENTS.md](AGENTS.md).

## Licence

[MIT](LICENSE). Built with [gpui](https://github.com/zed-industries/zed) and
[gpui-kit / gpui-component](https://github.com/longbridge/gpui-kit) (Apache-2.0),
[idevice](https://crates.io/crates/idevice) (MIT), libheif and libde265 (LGPL-3.0), FFmpeg
(run as a separate program) and [Lucide](https://lucide.dev) icons (ISC).

libheif and libde265 are under the LGPL-3.0. libde265 is linked dynamically (the system's copy).
libheif is compiled into the `rollcopy` binary through the
[libheif-rs](https://crates.io/crates/libheif-rs) crate's `embedded-libheif` feature. As the LGPL asks,
you can rebuild Rollcopy with a modified libheif: the complete source is in this repository and
[Building from source](#building-from-source) shows how; point the `libheif-sys` crate at your libheif
with a `[patch.crates-io]` entry, or turn `embedded-libheif` off to link a system libheif.
