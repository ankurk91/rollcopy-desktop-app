# AGENTS.md

Notes for AI coding agents working on Rollcopy. Humans: see [README.md](README.md).

## Project

Linux desktop app (Rust, [gpui](https://github.com/zed-industries/zed) via `gpui-kit`) that copies photos and
videos from an iPhone over USB. The plan, decisions and findings are in [PLAN.md](PLAN.md); read it first.

```
crates/rollcopy-device   usbmuxd/lockdown/AFC: listing, pairing, device info, read-only file access
crates/rollcopy-library  Photos.sqlite reader, DCIM fallback, item model
crates/rollcopy-convert  HEIC→JPG (libheif), MOV→MP4 (ffmpeg), checks
crates/rollcopy-queue    copy jobs, conflicts, retry, failure history
crates/rollcopy-app      gpui UI, tray, preferences (binary: rollcopy)
crates/rollcopy-cli      headless CLI for testing the crates above
packaging/               .desktop file, icon, install script, AppImage/deb helpers
```

## Commands

```sh
cargo fmt --all
cargo clippy --workspace --all-targets   # must be warning-free
cargo test --workspace
cargo run -p rollcopy-app                 # the app
cargo run -p rollcopy-cli -- info         # needs a connected, trusted iPhone
```

System packages are listed in the README (Building from source).

## Rules

- **Never write to, rename or delete anything on the phone.** Device access is read-only by design
  (`MediaFs` exposes list/stat/read only). Don't add write paths; don't run tools that modify the device.
- **No personal data in the repo**: no real device names, UDIDs, serials, photos, file lists, place names
  or counts from someone's library, in code, tests, docs or commit messages. Use neutral examples
  (`My iPhone`, `IMG_0001.HEIC`, `1 Example Street`).
- Keep UI code thin; logic belongs in the headless crates with unit tests.
- Pin `gpui-kit` exactly; gpui is pre-1.0.
- Keep `PLAN.md` up to date when a decision or finding changes.
