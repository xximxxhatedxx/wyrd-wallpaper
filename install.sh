#!/usr/bin/env bash
# =============================================================================
# Wyrd Wallpaper - Wayland Wallpaper Daemon & Material You Palette Installer
# =============================================================================
set -euo pipefail

if [ -t 1 ]; then
    BOLD="\033[1m"
    GREEN="\033[1;32m"
    CYAN="\033[1;36m"
    YELLOW="\033[1;33m"
    RED="\033[1;31m"
    DIM="\033[2m"
    RESET="\033[0m"
else
    BOLD=""
    GREEN=""
    CYAN=""
    YELLOW=""
    RED=""
    DIM=""
    RESET=""
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

DEFAULT_BIN_DIR="$HOME/.local/bin"
DEFAULT_CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/wyrd"
DEFAULT_STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/wyrd"
DEFAULT_WALLPAPERS_DIR="$HOME/Pictures/Wallpapers"
SYSTEMD_USER_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"

BIN_DIR="$DEFAULT_BIN_DIR"
CONFIG_DIR="$DEFAULT_CONFIG_DIR"
STATE_DIR="$DEFAULT_STATE_DIR"
WALLPAPERS_DIR="$DEFAULT_WALLPAPERS_DIR"
NON_INTERACTIVE=false
FORCE_BUILD=false
NO_SYSTEMD=false
DRY_RUN=false
COLOR_MODE="auto"

print_usage() {
    echo -e "${BOLD}Wyrd Wallpaper Installer${RESET}

Usage:
  ./install.sh [options]

Options:
      --bin-dir <DIR>         Destination directory for binary (default: ~/.local/bin)
      --wallpapers-dir <DIR>  Default wallpaper directory (default: ~/Pictures/Wallpapers)
      --color-mode <MODE>     Initial palette mode: auto or manual (default: auto)
  -b, --build                 Force rebuild from source using cargo
      --no-systemd            Skip installing and starting systemd user service
      --uninstall             Uninstall wyrd-wallpaper and remove systemd service
  -y, --yes, --non-interactive  Run non-interactively with defaults
      --dry-run               Show planned actions without modifying any files
  -h, --help                  Show this help message
"
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --bin-dir)
            BIN_DIR="$2"
            shift 2
            ;;
        --bin-dir=*)
            BIN_DIR="${1#*=}"
            shift
            ;;
        --wallpapers-dir)
            WALLPAPERS_DIR="$2"
            shift 2
            ;;
        --wallpapers-dir=*)
            WALLPAPERS_DIR="${1#*=}"
            shift
            ;;
        --color-mode)
            COLOR_MODE="$2"
            shift 2
            ;;
        --color-mode=*)
            COLOR_MODE="${1#*=}"
            shift
            ;;
        -b|--build)
            FORCE_BUILD=true
            shift
            ;;
        --no-systemd)
            NO_SYSTEMD=true
            shift
            ;;
        --uninstall)
            exec "$SCRIPT_DIR/uninstall.sh"
            ;;
        -y|--yes|--non-interactive)
            NON_INTERACTIVE=true
            shift
            ;;
        --dry-run)
            DRY_RUN=true
            shift
            ;;
        -h|--help)
            print_usage
            exit 0
            ;;
        *)
            echo -e "${RED}Unknown argument:${RESET} $1"
            print_usage
            exit 1
            ;;
    esac
done

echo -e "${CYAN}${BOLD}"
echo "================================================================="
echo "                  Wyrd Wallpaper Installer"
echo "================================================================="
echo -e "${RESET}"

run_cmd() {
    if [ "$DRY_RUN" = true ]; then
        echo -e "${DIM}[dry-run] $*${RESET}"
    else
        "$@"
    fi
}

find_binary() {
    if [ -f "$SCRIPT_DIR/bin/wyrd-wallpaper" ] && [ "$FORCE_BUILD" = false ]; then
        echo "$SCRIPT_DIR/bin/wyrd-wallpaper"
        return
    fi
    local arch
    arch="$(uname -m)"
    if [ -f "$SCRIPT_DIR/target/${arch}-unknown-linux-gnu/release/wyrd-wallpaper" ] && [ "$FORCE_BUILD" = false ]; then
        echo "$SCRIPT_DIR/target/${arch}-unknown-linux-gnu/release/wyrd-wallpaper"
        return
    fi
    if [ -f "$SCRIPT_DIR/target/release/wyrd-wallpaper" ] && [ "$FORCE_BUILD" = false ]; then
        echo "$SCRIPT_DIR/target/release/wyrd-wallpaper"
        return
    fi
    echo ""
}

# 1. Interactive options if TTY and not -y
if [ "$NON_INTERACTIVE" = false ] && [ -t 0 ]; then
    echo -e "${BOLD}Select Material You palette extraction mode:${RESET}"
    echo "  [1] Dynamic (auto-extract Material You colors from active wallpaper) [Default]"
    echo "  [2] Manual (preserve fixed theme accent colors)"
    echo ""
    read -rp "Your choice [1-2, Enter=1]: " CHOICE
    case "$CHOICE" in
        2) COLOR_MODE="manual" ;;
        *) COLOR_MODE="auto" ;;
    esac
    echo ""
fi

# 2. Locate or build binary
SRC_BIN="$(find_binary)"
if [ -z "$SRC_BIN" ]; then
    echo -e "${CYAN}==> Building wyrd-wallpaper from source (release)...${RESET}"
    if ! command -v cargo >/dev/null 2>&1; then
        echo -e "${RED}Error: 'cargo' not found. Install Rust (https://rustup.rs) or provide a prebuilt binary.${RESET}"
        exit 1
    fi
    run_cmd cargo build --release --locked --manifest-path "$SCRIPT_DIR/Cargo.toml"
    SRC_BIN="$SCRIPT_DIR/target/release/wyrd-wallpaper"
else
    echo -e "${GREEN}✓${RESET} Using release binary: ${DIM}$SRC_BIN${RESET}"
fi

# 3. Install binary and create directories
echo -e "${CYAN}==> Installing binary and preparing directories...${RESET}"
run_cmd mkdir -p "$BIN_DIR" "$CONFIG_DIR" "$STATE_DIR" "$WALLPAPERS_DIR"
run_cmd install -Dm755 "$SRC_BIN" "$BIN_DIR/wyrd-wallpaper"
echo -e "  ${GREEN}✓${RESET} Binary installed to ${BOLD}$BIN_DIR/wyrd-wallpaper${RESET}"
echo -e "  ${GREEN}✓${RESET} Wallpaper catalog directory ready at ${BOLD}$WALLPAPERS_DIR${RESET}"

# 4. Systemd user service setup
if [ "$NO_SYSTEMD" = false ] && [ -f "$SCRIPT_DIR/systemd/wyrd-wallpaper.service" ]; then
    echo -e "${CYAN}==> Configuring systemd user service...${RESET}"
    run_cmd mkdir -p "$SYSTEMD_USER_DIR"
    if [ "$DRY_RUN" = false ]; then
        local_exec_dir="${BIN_DIR/#$HOME/%h}"
        escaped_exec_dir="$(printf '%s' "$local_exec_dir" | sed 's/[\\&|]/\\&/g')"
        sed "s|^ExecStart=.*|ExecStart=${escaped_exec_dir}/wyrd-wallpaper run|" \
            "$SCRIPT_DIR/systemd/wyrd-wallpaper.service" > "$SYSTEMD_USER_DIR/wyrd-wallpaper.service"
        if command -v systemctl >/dev/null 2>&1; then
            systemctl --user daemon-reload 2>/dev/null || true
            systemctl --user enable --now wyrd-wallpaper.service 2>/dev/null || true
            echo -e "  ${GREEN}✓${RESET} Enabled and started ${BOLD}wyrd-wallpaper.service${RESET}"
        fi
    else
        echo -e "${DIM}[dry-run] write $SYSTEMD_USER_DIR/wyrd-wallpaper.service & enable service${RESET}"
    fi
fi

# 5. Apply initial color mode if daemon is running
if [ "$DRY_RUN" = false ] && [ -S "${XDG_RUNTIME_DIR:-/tmp}/wyrd-wallpaper.sock" ]; then
    "$BIN_DIR/wyrd-wallpaper" color "$COLOR_MODE" >/dev/null 2>&1 || true
fi

# 6. Check PATH
if [[ ":$PATH:" != *":$BIN_DIR:"* ]]; then
    echo ""
    echo -e "${YELLOW}Note:${RESET} ${BOLD}$BIN_DIR${RESET} is not in your PATH."
    echo "Add it to your shell config:"
    echo "  bash/zsh: export PATH=\"\$HOME/.local/bin:\$PATH\""
    echo "  fish:     fish_add_path ~/.local/bin"
fi

echo ""
echo -e "${GREEN}${BOLD}Installation complete!${RESET}"
echo -e "  • Run daemon manually:  ${CYAN}wyrd-wallpaper run${RESET}"
echo -e "  • Open gallery picker:  ${CYAN}wyrd-wallpaper select${RESET}"
echo -e "  • Set wallpaper:        ${CYAN}wyrd-wallpaper set ~/Pictures/Wallpapers/image.jpg${RESET}"
