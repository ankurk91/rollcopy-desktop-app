# Troubleshooting

Common problems with [Rollcopy](../README.md) and how to fix them. Rollcopy only ever reads from
the phone, so none of these can harm what's on it.

- **"Open folder" / "Show in folder" does nothing or takes ages (Linux Mint):** a stuck Nemo
  window process makes every new folder wait for it. Run `nemo -q`, then try again.
- **Photos open in an error in the image viewer:** Rollcopy's *Open* turns HEIC into JPG first, so
  any viewer can show it; copies you make are JPG already.
- **"The connection keeps dropping" / phone not detected:** try another cable or USB port, without
  a hub. Many cheap cables only charge.
- **"Unlock your iPhone and tap Trust" doesn't go away:** unplug, unlock the phone, plug in again and
  tap **Trust**. If you tapped **Don't Trust**, unplugging and plugging in asks again.
- **Saving to a network share:** shares opened in Files (`smb://…`, under `/run/user/…/gvfs/`), sshfs
  and some USB-drive FUSE drivers work. Rollcopy still never replaces a file that's already there.
- **Videos don't play (fresh Ubuntu):** Ubuntu's video player can't decode iPhone (HEVC) or app
  (H.264) videos until its codecs are installed: `sudo apt install gstreamer1.0-libav`, or install
  VLC. Rollcopy says so when you *Open* a video and the codecs are missing. The copied files are fine.
- **Videos play in black and white:** iPhone videos are HDR (HLG / Dolby Vision). Some Linux players,
  such as Celluloid/mpv on some setups, show these in black and white. The file is fine; VLC,
  Android and iPhone play it in colour. To get standard-colour H.264 files, choose
  *Preferences → Conversion → Videos → H.264 (slower)*.
- **Something failed:** the transfer list at the bottom shows the reason. *Export error report*
  saves a CSV, and `~/.local/state/rollcopy/rollcopy.log` has the app's log.

## Still stuck?

Please report problems with *☰ → Report an Issue*: it opens a GitHub issue form with the version,
Linux version and phone model filled in (never the phone's name or serial). Or use
[Issues](https://github.com/ankurk91/rollcopy-desktop-app/issues) directly.

See also the [FAQ](FAQ.md).
