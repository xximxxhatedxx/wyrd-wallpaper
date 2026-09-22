# wyrd-wallpaper

[![CI](https://github.com/xximxxhatedxx/wyrd-wallpaper/actions/workflows/ci.yml/badge.svg)](https://github.com/xximxxhatedxx/wyrd-wallpaper/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)

A dynamic, multi-monitor Wayland wallpaper daemon with automatic Material You (Material Design 3) palette extraction and live desktop theming.

Powered by [`wyrd-engine`](https://github.com/xximxxhatedxx/wyrd-engine).

---

## Features

- **Multi-Monitor Output**:
  - Independent per-monitor wallpaper assignment or unified desktop spanning.
  - Automatic display hotplugging via `wlr-layer-shell-v1`.
- **Material You Dynamic Theming**:
  - Extracts dominant vibrant color schemes and Material 3 palettes from wallpaper images.
  - Writes color schemes to `$XDG_RUNTIME_DIR/wyrd/wallpaper.toml` for automatic shell synchronization.
- **Lightweight & High-Performance**:
  - Direct hardware/CPU rendering using `tiny-skia` and `wyrd-engine`.
  - Zero background CPU/GPU usage once rendering is idle.
- **Interactive Terminal Picker**:
  - Built-in thumbnail selection menu.
  - Fast image cataloging with support for PNG, JPEG, WebP, AVIF, and SVG.
- **Systemd User Service**:
  - Auto-start with your Wayland session via `graphical-session.target`.

---

## Installation

### From crates.io (available after release cooldown)

```bash
cargo install wyrd-wallpaper
```

### From Source

```bash
git clone https://github.com/xximxxhatedxx/wyrd-wallpaper.git
cd wyrd-wallpaper
cargo install --path .
```

---

## CLI Usage

### Run Daemon

Start the wallpaper daemon:

```bash
# Launch daemon
wyrd-wallpaper run

# Launch daemon with specific initial image
wyrd-wallpaper run --image ~/Pictures/wallpapers/mountain.png

# Launch daemon targeting a specific display output
wyrd-wallpaper run --output DP-1 --image ~/Pictures/wallpapers/forest.png
```

### Control Running Daemon

Send commands to the running instance via its UNIX control socket:

```bash
# Set wallpaper dynamically
wyrd-wallpaper set ~/Pictures/wallpapers/sunset.png

# List all discovered wallpapers in the catalog (~/Pictures/wallpapers)
wyrd-wallpaper list

# Print current wallpaper path and active palette
wyrd-wallpaper current

# Open interactive picker
wyrd-wallpaper select

# Manage dynamic theming colors
wyrd-wallpaper color auto
wyrd-wallpaper color manual --accent "#7dd3fc"
```

---

## Autostart

### In Hyprland (`~/.config/hypr/hyprland.conf`):

```ini
exec-once = wyrd-wallpaper run
```

### In Sway (`~/.config/sway/config`):

```ini
exec wyrd-wallpaper run
```

### Via systemd:

```bash
systemctl --user enable --now wyrd-wallpaper.service
```

---

## License

Dual-licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.
