# Flatpak

How Rollcopy runs inside Flatpak, what was tested, and what a Flatpak
manifest has to provide. This is background for whoever writes the manifest;
it is not a manifest.

> **Flathub rules (checked 2026-11-26).** Flathub manifests must not contain
> AI-generated or AI-assisted content, AI tools must not open or answer the
> submission pull request, and AI-generated material in the app must be
> disclosed. Rollcopy's code was written with AI assistance, so that has to
> be disclosed, and the manifest and the PR have to be the maintainer's own.
> See [Flathub's requirements](https://docs.flathub.org/docs/for-app-authors/requirements).

## The phone: a relay for usbmuxd

Rollcopy talks to the iPhone through the host's usbmuxd socket,
`/run/usbmuxd`. Tested on Linux Mint 22 with Flatpak 1.14.6 (2026-11-26):

| Situation | Plain `--filesystem=/run/usbmuxd` |
|---|---|
| Phone plugged in before the app starts | Works |
| App started first, phone plugged in later | Never sees the phone |
| Phone unplugged and plugged in again | "Connection refused" until restart |
| `--filesystem=/run` instead | Not allowed by Flatpak |

usbmuxd makes a new socket each time a phone is plugged in, and a sandbox
only sees the one that existed when it started. So inside Flatpak (detected
by `/.flatpak-info`) Rollcopy:

1. points all phone access at a relay socket in its runtime folder,
   `$XDG_RUNTIME_DIR/app/$FLATPAK_ID/usbmuxd`, which every sandbox of the
   app shares;
2. while that relay doesn't answer, asks the Flatpak portal for a fresh
   sandbox (`flatpak-spawn`, never `--host`) running
   `rollcopy --usbmuxd-relay`. A fresh sandbox sees the current usbmuxd
   socket. The relay passes bytes through and adds no way to change the
   phone;
3. the relay ends when its usbmuxd socket goes stale (re-plug) or when the
   app is gone (it watches a lock the app holds), and the app starts a new
   one within 3 seconds.

Tested by hand: plugged in after start, unplugged and re-plugged while
running, the phone came back within a second each time, with no restart.
Code: `crates/rollcopy-device/src/{mux,relay}.rs`, `crates/rollcopy-app/src/flatpak.rs`.

usbmuxd itself must be installed on the host (the description says so); the
sandbox can't check, so the "plug in" screen mentions it.

## Other sandbox changes in the app

- **D-Bus:** the app uses zbus 5. zbus 6 pipelines its connection handshake,
  which `xdg-dbus-proxy` before 0.1.6 can't parse ("Invalid message header
  read"); Ubuntu 24.04 and Mint 22 ship 0.1.5. With zbus 6, notifications
  and the single-instance name failed inside Flatpak.
- **Tray:** registers without its own `StatusNotifierItem-<pid>-<n>` D-Bus
  name inside Flatpak (ksni `disable_dbus_name`); the sandbox may not own it.
- **Open:** `xdg-open` inside the runtime goes through the OpenURI portal,
  which asks which app to use. The player check (`xdg-mime`, GStreamer
  plugins) is skipped; the sandbox can't see the host's apps.
- **Show in folder:** the OpenURI portal's `OpenDirectory`, with the file as
  an fd, instead of `FileManager1` over `dbus-send`.
- **Updates:** "Check for updates" is hidden; Flathub updates the app, and
  the check pointed at GitHub's .deb and AppImage. So the app needs no
  network access.
- Folder picker: gpui uses the FileChooser portal already.

## What a manifest needs

**Runtime:** `org.freedesktop.Platform` / `Sdk` (26.08 was tested), with
`org.freedesktop.Sdk.Extension.rust-stable` to build.

**Permissions** (`finish-args`) used in the local test, and why:

| Permission | Why |
|---|---|
| `--share=ipc`, `--socket=x11`, `--socket=wayland`, `--device=dri` | The window (gpui renders with Vulkan/GL) |
| `--filesystem=/run/usbmuxd` | The phone, through the relay (above) |
| `--filesystem=xdg-pictures` | The default destination, `~/Pictures/Rollcopy`; other folders come through the folder picker portal |
| `--talk-name=org.freedesktop.Notifications` | "Copies finished" notifications |
| `--talk-name=org.kde.StatusNotifierWatcher` | The tray icon while copying |

Not needed: `--share=network`, `--device=all`, `--talk-name=org.freedesktop.Flatpak`
(the relay uses the normal portal, not host commands).

**Build pieces** (Flathub builds offline, from source):

- **Rust crates:** vendored for an offline build; Flathub's
  `flatpak-cargo-generator` turns `Cargo.lock` into sources.
- **libheif:** bundled by `libheif-rs` (`embedded-libheif`), built with
  cmake. It needs `libde265` (HEVC decoding) available to link against.
  Rollcopy only decodes HEIC, so no HEVC encoder (x265) is needed.
- **ffmpeg and ffprobe programs** in `PATH` (`/app/bin`): MOV → MP4 runs
  them. The lossless remux needs the mov/mp4 demuxer and muxer and the
  HEVC/H.264/AAC parsers. The optional H.264 mode also needs libx264 and
  zimg (`zscale`, `tonemap`); its GPU path (libplacebo, VA-API) falls back
  to software when missing.
- **Desktop file, icon and metainfo** from `packaging/`. The metainfo passes
  `flatpak-builder-lint appstream`.

## Testing a local build

A throwaway package that wraps a host-built binary (copying the few
libraries the runtime lacks) is enough to test the sandbox behaviour above;
keep it out of the repo. Check: phone found when plugged in after start;
found again after a re-plug; Open, Copy, Show in folder, the dock's Open
folder, Save to; the tray; Report an Issue opens the browser.
