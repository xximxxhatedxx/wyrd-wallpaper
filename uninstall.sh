#!/usr/bin/env bash
set -euo pipefail

BIN_DIR="${HOME}/.local/bin"
SYSTEMD_DIR="${HOME}/.config/systemd/user"

echo "==> Uninstalling wyrd-wallpaper..."
systemctl --user disable --now wyrd-wallpaper.service 2>/dev/null || true
rm -f "$SYSTEMD_DIR/wyrd-wallpaper.service"
systemctl --user daemon-reload 2>/dev/null || true
rm -f "$BIN_DIR/wyrd-wallpaper"

if [[ "${1:-}" == "--purge" ]]; then
    rm -f "${HOME}/.config/wyrd/wallpaper.lua" "${HOME}/.config/wyrd/wallpaper.toml"
    rm -f "${HOME}/.local/state/wyrd/wallpaper.toml"
fi

echo "==> wyrd-wallpaper uninstalled."
