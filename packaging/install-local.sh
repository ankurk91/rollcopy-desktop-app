#!/usr/bin/env bash
# Installs Rollcopy for the current user (no sudo) into ~/.local:
#   ~/.local/bin/rollcopy
#   ~/.local/share/applications/io.github.ankurk91.Rollcopy.desktop
#   ~/.local/share/icons/hicolor/scalable/apps/io.github.ankurk91.Rollcopy.svg
# Usage: packaging/install-local.sh            build (release) and install
#        packaging/install-local.sh --uninstall remove those three files
set -euo pipefail

ID=io.github.ankurk91.Rollcopy
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$HOME/.local/bin/rollcopy"
APPS="$HOME/.local/share/applications"
ICONS="$HOME/.local/share/icons/hicolor/scalable/apps"

refresh() {
    command -v update-desktop-database >/dev/null && update-desktop-database -q "$APPS" || true
    command -v gtk-update-icon-cache >/dev/null &&
        gtk-update-icon-cache -q -t "$HOME/.local/share/icons/hicolor" 2>/dev/null || true
}

if [[ "${1:-}" == "--uninstall" ]]; then
    rm -f "$BIN" "$APPS/$ID.desktop" "$ICONS/$ID.svg"
    refresh
    echo "Rollcopy removed (settings and cache are kept in ~/.config, ~/.local/state, ~/.cache)."
    exit 0
fi

cargo build --release -p rollcopy-app --manifest-path "$ROOT/Cargo.toml"
install -Dm755 "$ROOT/target/release/rollcopy" "$BIN"
install -Dm644 "$ROOT/packaging/$ID.desktop" "$APPS/$ID.desktop"
install -Dm644 "$ROOT/packaging/icons/$ID.svg" "$ICONS/$ID.svg"
refresh
echo "Installed: $BIN"
case ":$PATH:" in
    *":$HOME/.local/bin:"*) ;;
    *) echo "Note: ~/.local/bin is not on PATH; the menu entry still works." ;;
esac
