# wyrd-wallpaper

A Wayland wallpaper daemon with Material You dynamic palette extraction. Sets your wallpaper across multiple monitors, generates a color palette from it, and writes the result to `~/.local/state/wyrd/wallpaper.toml` so other Wyrd tools (`wyrd-shell`, `wyrd-greet`) can pick up the theme automatically.

---

## Features

- **Multi-monitor wallpaper**: each output is controlled independently via `wlr-layer-shell`.
- **Material You palette extraction**: generates `accent`, `background`, `surface_container`, `on_surface`, and related tokens from the dominant colors in your wallpaper image. Supports manual override if you don't want fully automatic colors.
- **Interactive wallpaper picker**: thumbnail carousel and filmstrip UI, 60 fps scroll physics, hover elevation, and smooth open/close animations. Triggered via `wyrd-wallpaper select` or the `wallpaper` module in `wyrd-shell`.
- **Image format support**: PNG, JPEG, WebP, AVIF, SVG.
- **Systemd user service**: `install.sh` sets up autostart with `graphical-session.target`.

---

## Installation

```bash
git clone https://github.com/xximxxhatedxx/wyrd-wallpaper.git
cd wyrd-wallpaper
./install.sh
```

To uninstall:

```bash
./uninstall.sh
```

---

## Usage

```bash
# Start the daemon
wyrd-wallpaper run

# Start and set an initial image
wyrd-wallpaper run --image ~/Pictures/Wallpapers/mountain.jpg

# Target a specific output
wyrd-wallpaper run --output DP-1 --image ~/Pictures/Wallpapers/forest.png

# Set wallpaper on a running daemon
wyrd-wallpaper set ~/Pictures/Wallpapers/sunset.jpg

# List all images found in ~/Pictures/Wallpapers
wyrd-wallpaper list

# Show the active wallpaper path and generated palette
wyrd-wallpaper current

# Open the interactive picker
wyrd-wallpaper select

# Palette control
wyrd-wallpaper color auto
wyrd-wallpaper color manual --accent "#7dd3fc"
```

---

## Autostart

**Hyprland**:
```ini
exec-once = wyrd-wallpaper run
```

**Sway**:
```ini
exec wyrd-wallpaper run
```

**systemd** (set up by `install.sh`):
```bash
systemctl --user enable --now wyrd-wallpaper.service
```

---

## Related

- [`wyrd-engine`](https://github.com/xximxxhatedxx/wyrd-engine) - rendering and scripting engine
- [`wyrd-shell`](https://github.com/xximxxhatedxx/wyrd-shell) - desktop shell that picks up the generated palette
- [`wyrd-greet`](https://github.com/xximxxhatedxx/wyrd-greet) - display manager greeter that reads the wallpaper and palette at login

---

## Support

If you find it useful, you can support the project here.

[![ko-fi](https://ko-fi.com/img/githubbutton_sm.svg)](https://ko-fi.com/xximxxhatedxx)

---

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).
