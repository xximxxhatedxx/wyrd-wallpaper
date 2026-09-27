# Changelog

All notable changes to `wyrd-wallpaper` will be documented in this file.

## [0.1.0] - 2026-09-27

### Added
- **Wayland Wallpaper Daemon (`wlr-layer-shell`)**:
  - Multi-monitor independent wallpaper rendering with aspect-ratio cover scaling.
  - UNIX socket IPC (`run`, `set`, `list`, `current`, `select`, `color`).
- **Material You Dynamic Palette Extraction**:
  - Automatic color extraction writing `wallpaper.toml` state for `wyrd-shell` and `wyrd-greet`.
- **Interactive Wallpaper Picker**:
  - Smooth 60 FPS carousel/filmstrip picker built on `wyrd-engine`.
