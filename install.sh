#!/usr/bin/env bash
set -euo pipefail

BIN_DIR="${HOME}/.local/bin"
SYSTEMD_DIR="${HOME}/.config/systemd/user"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "==> Installing wyrd-wallpaper v0.1.0..."
mkdir -p "$BIN_DIR" "$SYSTEMD_DIR"

if [ -f "$SCRIPT_DIR/target/release/wyrd-wallpaper" ]; then
    install -m 755 "$SCRIPT_DIR/target/release/wyrd-wallpaper" "$BIN_DIR/wyrd-wallpaper"
else
    echo "==> Building wyrd-wallpaper (release)..."
    cargo build --release --manifest-path "$SCRIPT_DIR/Cargo.toml"
    install -m 755 "$SCRIPT_DIR/target/release/wyrd-wallpaper" "$BIN_DIR/wyrd-wallpaper"
fi

if [ -f "$SCRIPT_DIR/systemd/wyrd-wallpaper.service" ]; then
    sed "s|/usr/bin/wyrd-wallpaper|${BIN_DIR}/wyrd-wallpaper|g" \
        "$SCRIPT_DIR/systemd/wyrd-wallpaper.service" > "$SYSTEMD_DIR/wyrd-wallpaper.service"
    systemctl --user daemon-reload 2>/dev/null || true
    systemctl --user enable --now wyrd-wallpaper.service 2>/dev/null || true
fi

echo "==> wyrd-wallpaper installed to ${BIN_DIR}/wyrd-wallpaper"
