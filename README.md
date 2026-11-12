# Rollcopy: copy iPhone photos and videos to Linux

[![CI](https://github.com/ankurk91/rollcopy-desktop-app/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/ankurk91/rollcopy-desktop-app/actions/workflows/ci.yml)
[![Latest release](https://img.shields.io/github/v/release/ankurk91/rollcopy-desktop-app)](https://github.com/ankurk91/rollcopy-desktop-app/releases/latest)
[![Licence: MIT](https://img.shields.io/github/license/ankurk91/rollcopy-desktop-app)](LICENSE)

A **Linux desktop app** that copies photos and videos from an iPhone over a USB cable, as files any
Android phone, PC or TV can open. No iTunes, no cloud, no account.

Plug in the phone, tap **Trust**, pick what you want, and Rollcopy copies it to
`~/Pictures/Rollcopy/<phone name>/YYYY/MM/images|videos/`, converting HEIC to JPG and MOV to MP4
on the way.

![Rollcopy on Linux Mint: an iPhone photo library by year and month, a thumbnail grid with Live, edited and video badges, the preview pane with camera details, and the transfer queue](.github/screenshot.png)

> **Status:** early (0.1.0). Linux only, tested on Linux Mint 22 (Cinnamon, X11) with iOS 27.

## Features

- **Browse the Photos library**: thumbnails by year and month, filters (photos, videos, edited,
  copied / not copied), sort by date taken, keyboard navigation.
- **Details for each item**: date and time zone, size, camera and lens, the app that saved it
  (Camera, WhatsApp, Snapchat, …), place name, location, albums.
- **Android/PC-friendly copies**:
  - HEIC → JPG (keeps colour profile and EXIF, applies rotation)
  - MOV → MP4 (lossless by default; optional H.264 re-encode for old devices)
  - Edited photos and videos are copied as edited; Live Photos as the still.
- **Safe copying**: files are written to a temporary `.part` file, checked, then renamed, so a failed
  copy never damages an existing file. Already-copied items are detected from what's in the folder.
  You choose Ask / Skip / Overwrite for files that already exist.
- **Transfer queue** with progress, pause/resume, retry and an exportable error report. Unplugging
  or locking the phone pauses the queue instead of failing items.
- **Tray icon and notifications**: closing the window during a copy keeps Rollcopy running in the tray.
- **Preferences** for the destination, folder layout, JPEG quality, video format and theme.

### Your phone is safe

Rollcopy **only reads** from the phone. It has no code to write, rename or delete anything on the
device. The only change it makes is the trust record created when you tap **Trust**, the same as
connecting to a Mac or PC.

### Network use

None, except **Check for updates** (manual, or optionally once a day), which asks GitHub for the
latest release. Nothing is uploaded.

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
sudo apt install ./rollcopy_0.1.0-1_amd64.deb
```

**AppImage** (install `usbmuxd` and `ffmpeg` first, see Requirements):

```sh
chmod +x Rollcopy-0.1.0-x86_64.AppImage
./Rollcopy-0.1.0-x86_64.AppImage
```

**From source** for your user only (see [Building from source](#building-from-source)):

```sh
packaging/install-local.sh              # builds and installs into ~/.local
packaging/install-local.sh --uninstall
```

## Using it

1. Connect the iPhone with a cable and unlock it.
2. Tap **Trust** on the phone and enter your passcode when asked.
3. Pick items (click, Ctrl/Shift+click, Ctrl+A, arrow keys) and press **Copy selected**, or
   **Copy all** for the year or month you're viewing.
4. Change where files go with **Save to** (📁) or in **Preferences** (Ctrl+,).

| Key | Action |
|---|---|
| ← → ↑ ↓ | Move between items |
| Shift + arrows | Extend the selection |
| Ctrl+A / Esc | Select all / clear the selection (Esc also closes the viewer) |
| Space / Enter / double-click | View the photo full-window (videos open in your player) |
| Ctrl+, | Preferences |
| F1 | Keyboard shortcuts |
| Ctrl+Q | Quit |

## Troubleshooting

- **"The connection keeps dropping" / phone not detected:** try another cable or USB port, without
  a hub. Many cheap cables only charge.
- **"Unlock your iPhone and tap Trust" doesn't go away:** unplug, unlock the phone, plug in again and
  tap **Trust**. If you tapped **Don't Trust**, unplugging and plugging in asks again.
- **Videos play in black and white:** iPhone videos are HDR (HLG / Dolby Vision). Some Linux players,
  such as Celluloid/mpv on some setups, show these in black and white. The file is fine; VLC,
  Android and iPhone play it in colour. To get standard-colour H.264 files, choose
  *Preferences → Conversion → Videos → Re-encode to H.264* (much slower).
- **Something failed:** the transfer panel shows the reason. *Export error report* saves a CSV;
  *☰ → About → Credits → Copy debug info* copies version and system details for a bug report.

Please report problems at [Issues](https://github.com/ankurk91/rollcopy-desktop-app/issues).

## FAQ

**How do I transfer photos from an iPhone to Ubuntu or Linux Mint?**
Install Rollcopy, connect the iPhone with a USB cable, tap **Trust** on the phone, select the photos
and click **Copy selected**. No iTunes, iCloud or Apple account is needed.

**Can it convert HEIC to JPG?**
Yes, while copying. HEIC photos become JPG (quality 92 by default, colour profile and EXIF kept) and
MOV videos become MP4. JPG, PNG and MP4 files are copied unchanged.

**Does it copy Live Photos and edited photos?**
Edited photos and videos are copied as edited. Live Photos are copied as the still photo.

**Does it work on Fedora, Arch or other distributions?**
The `.deb` is for Ubuntu 24.04+ and its derivatives. The AppImage may run on other 64-bit distributions
with glibc 2.39 or newer, with `usbmuxd` and `ffmpeg` installed; this isn't tested yet.

**Can it delete photos from the iPhone or free up space?**
No. Rollcopy only reads from the phone; it never changes or deletes anything on it.

## Where Rollcopy keeps things

| Path | What |
|---|---|
| `~/.config/rollcopy/config.toml` | Preferences |
| `~/.local/state/rollcopy/` | Window layout, failed-copy history |
| `~/.cache/rollcopy/` | Thumbnails, copies of the Photos database (safe to delete) |

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
