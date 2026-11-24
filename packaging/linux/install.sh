#!/usr/bin/env bash
# Per-user install of Kadex on Linux (no sudo). Usage: install.sh [--rebuild] | --uninstall
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BIN_SRC="$ROOT_DIR/target/release/kadex"
BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}"
APP_DIR="$DATA_DIR/applications"
ICON_DIR="$DATA_DIR/icons/hicolor/scalable/apps"
# Kadex windows use these app-ids; panels and launchers look the icon up by app-id.
WINDOW_APP_IDS="kadex.sticky kadex.editor kadex.overlay"

refresh_caches() {
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "$APP_DIR" >/dev/null 2>&1 || true
    fi
    if command -v gtk-update-icon-cache >/dev/null 2>&1; then
        gtk-update-icon-cache -f -t "$DATA_DIR/icons/hicolor" >/dev/null 2>&1 || true
    fi
}

if [ "${1:-}" = "--uninstall" ]; then
    "$BIN_DIR/kadex" quit >/dev/null 2>&1 || true
    rm -f "$BIN_DIR/kadex" "$APP_DIR/kadex.desktop" "$ICON_DIR/kadex.svg"
    for id in $WINDOW_APP_IDS; do rm -f "$ICON_DIR/$id.svg"; done
    refresh_caches
    echo "Kadex removed. Your notes were left untouched."
    exit 0
fi

if [ ! -x "$BIN_SRC" ] || [ "${1:-}" = "--rebuild" ]; then
    echo "Building kadex (release)..."
    cargo build --release --locked --manifest-path "$ROOT_DIR/Cargo.toml"
fi

# Stop a running daemon so the binary can be replaced and picked up.
"$BIN_DIR/kadex" quit >/dev/null 2>&1 || true

mkdir -p "$BIN_DIR" "$APP_DIR" "$ICON_DIR"
install -Dm755 "$BIN_SRC" "$BIN_DIR/kadex"
install -Dm644 "$ROOT_DIR/packaging/kadex.desktop" "$APP_DIR/kadex.desktop"
install -Dm644 "$ROOT_DIR/assets/icon.svg" "$ICON_DIR/kadex.svg"
for id in $WINDOW_APP_IDS; do
    install -Dm644 "$ROOT_DIR/assets/icon.svg" "$ICON_DIR/$id.svg"
done
refresh_caches

echo "Installed:"
echo "  binary:  $BIN_DIR/kadex"
echo "  desktop: $APP_DIR/kadex.desktop"
echo "  icon:    $ICON_DIR/kadex.svg"
case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *) echo "  note: $BIN_DIR is not on your PATH" ;;
esac
echo ""
echo "Niri window rules and keybinds (paste into ~/.config/niri/config.kdl):"
echo "  $ROOT_DIR/packaging/niri/kadex.kdl"
