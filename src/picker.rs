use anyhow::{Context, Result};
use std::collections::HashMap;
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use wayland_client::{
    globals::{registry_queue_init, GlobalListContents},
    protocol::{
        wl_buffer, wl_callback, wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_registry,
        wl_seat, wl_shm, wl_shm_pool, wl_surface,
    },
    Connection, Dispatch, QueueHandle, WEnum,
};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{self, Layer, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{self, Anchor, KeyboardInteractivity, ZwlrLayerSurfaceV1},
};

use crate::catalog::{self, WallpaperEntry};

const DEFAULT_WALLPAPER_LUA: &str = r#"-- =============================================================================
-- WYRD WALLPAPER: DECLARATIVE WINDOW CONFIGURATION
-- =============================================================================
-- Rendered strictly by wyrd-engine (LuaRuntime + WidgetTree + Renderer).
-- Automatically inherits tokens/styles from your active theme in settings.toml,
-- while allowing full customization of layout, dimensions, widgets, and styles.
-- =============================================================================

local ok_settings, settings_mod = pcall(require, "settings")
local settings = (ok_settings and settings_mod and settings_mod.load and settings_mod.load()) or {
    theme = "catppuccin",
}

local theme_name = settings.theme or "catppuccin"
local ok_theme, theme_mod = pcall(require, "themes." .. theme_name)
if ok_theme and theme_mod and type(theme_mod.apply) == "function" then
    theme_mod.apply()
else
    pcall(function()
        require("themes.catppuccin").apply()
    end)
end

wyrd.style("wallpaper_window", {
    extends = "popup",
    border_radius = 20,
    padding = { 18, 20, 18, 20 },
})

wyrd.style("wallpaper_subtitle", {
    font_size = 12,
    opacity = 0.78,
})

wyrd.style("wallpaper_hint", {
    font_size = 11,
    opacity = 0.65,
})

wyrd.style("wallpaper_card_active", {
    extends = "quick_toggle",
    border_radius = 14,
    padding = { 4, 4, 4, 4 },
})

wyrd.style("wallpaper_card_side", {
    extends = "chip",
    border_radius = 12,
    opacity = 0.72,
    padding = { 3, 3, 3, 3 },
    hover = {
        opacity = 0.95,
    },
})

wyrd.style("wallpaper_thumb_slot", {
    extends = "chip",
    border_radius = 10,
    opacity = 0.80,
    padding = { 2, 2, 2, 2 },
    hover = {
        opacity = 1.0,
    },
})

wyrd.style("wallpaper_apply_btn", {
    extends = "media_control_btn_primary",
    border_radius = 10,
    font_size = 13,
    font_weight = "bold",
    padding = { 8, 16, 8, 16 },
})

wyrd.create({
    type = "popup",
    name = "wallpaper_picker",
    layer = "overlay",
    width = 920,
    height = 540,
    style = "wallpaper_window",
    widgets = {
        {
            type = "container",
            layout = {
                mode = "flex_col",
                gap = 14,
                width = "100%",
                height = "100%",
                justify = "space_between",
                padding = { top = 16, right = 20, bottom = 16, left = 20 },
            },
            children = {
                {
                    type = "container",
                    layout = { mode = "flex_row", align = "center", justify = "space_between" },
                    children = {
                        {
                            type = "container",
                            layout = { mode = "flex_row", align = "center", gap = 12 },
                            children = {
                                {
                                    type = "text",
                                    text = "󰸉  Wallpaper Selector",
                                    style = "header_title",
                                },
                                {
                                    type = "text",
                                    text = "{wallpaper.name}  •  {wallpaper.resolution}",
                                    style = "wallpaper_subtitle",
                                },
                            },
                        },
                        {
                            type = "container",
                            layout = { mode = "flex_row", align = "center", gap = 8 },
                            children = {
                                {
                                    type = "text",
                                    text = "{wallpaper.index} / {wallpaper.total}",
                                    style = "chip",
                                },
                                {
                                    type = "button",
                                    id = "wp_close_btn",
                                    text = "✕",
                                    action = "wallpaper:close",
                                    style = "media_control_btn",
                                    layout = { width = 30, height = 30, justify = "center", align = "center" },
                                },
                            },
                        },
                    },
                },
                {
                    type = "container",
                    layout = { mode = "flex_row", align = "center", justify = "center", gap = 14 },
                    children = {
                        {
                            type = "button",
                            id = "wp_prev_btn",
                            text = "󰅁",
                            action = "wallpaper:prev",
                            style = "media_control_btn",
                            layout = { width = 38, height = 38, justify = "center", align = "center" },
                        },
                        {
                            type = "button",
                            id = "wp_prev_card",
                            path = "{wallpaper.prev_thumb}",
                            action = "wallpaper:prev",
                            style = "wallpaper_card_side",
                            layout = { width = 185, height = 116, justify = "center", align = "center" },
                        },
                        {
                            type = "button",
                            id = "wp_active_card",
                            path = "{wallpaper.current_thumb}",
                            action = "wallpaper:apply",
                            style = "wallpaper_card_active",
                            layout = { width = 384, height = 240, justify = "center", align = "center" },
                        },
                        {
                            type = "button",
                            id = "wp_next_card",
                            path = "{wallpaper.next_thumb}",
                            action = "wallpaper:next",
                            style = "wallpaper_card_side",
                            layout = { width = 185, height = 116, justify = "center", align = "center" },
                        },
                        {
                            type = "button",
                            id = "wp_next_btn",
                            text = "󰅂",
                            action = "wallpaper:next",
                            style = "media_control_btn",
                            layout = { width = 38, height = 38, justify = "center", align = "center" },
                        },
                    },
                },
                {
                    type = "container",
                    layout = { mode = "flex_row", align = "center", justify = "center", gap = 10 },
                    children = {
                        {
                            type = "button",
                            id = "wp_slot_0",
                            path = "{wallpaper.slot_0_thumb}",
                            action = "wallpaper:select_slot_0",
                            style = "wallpaper_thumb_slot",
                            layout = { width = 152, height = 86, justify = "center", align = "center" },
                        },
                        {
                            type = "button",
                            id = "wp_slot_1",
                            path = "{wallpaper.slot_1_thumb}",
                            action = "wallpaper:select_slot_1",
                            style = "wallpaper_thumb_slot",
                            layout = { width = 152, height = 86, justify = "center", align = "center" },
                        },
                        {
                            type = "button",
                            id = "wp_slot_2",
                            path = "{wallpaper.slot_2_thumb}",
                            action = "wallpaper:select_slot_2",
                            style = "wallpaper_card_active",
                            layout = { width = 156, height = 88, justify = "center", align = "center" },
                        },
                        {
                            type = "button",
                            id = "wp_slot_3",
                            path = "{wallpaper.slot_3_thumb}",
                            action = "wallpaper:select_slot_3",
                            style = "wallpaper_thumb_slot",
                            layout = { width = 152, height = 86, justify = "center", align = "center" },
                        },
                        {
                            type = "button",
                            id = "wp_slot_4",
                            path = "{wallpaper.slot_4_thumb}",
                            action = "wallpaper:select_slot_4",
                            style = "wallpaper_thumb_slot",
                            layout = { width = 152, height = 86, justify = "center", align = "center" },
                        },
                    },
                },
                {
                    type = "container",
                    layout = { mode = "flex_row", align = "center", justify = "space_between" },
                    children = {
                        {
                            type = "text",
                            text = "←/→ or Scroll to browse   •   Click or Enter to apply   •   Esc to close",
                            style = "wallpaper_hint",
                        },
                        {
                            type = "container",
                            layout = { mode = "flex_row", align = "center", gap = 10 },
                            children = {
                                {
                                    type = "button",
                                    id = "wp_random_btn",
                                    text = "🎲  Random",
                                    action = "wallpaper:random",
                                    style = "quick_toggle",
                                    layout = { width = 115, height = 36, justify = "center", align = "center" },
                                },
                                {
                                    type = "button",
                                    id = "wp_apply_btn",
                                    text = "󰄬  Apply Wallpaper",
                                    action = "wallpaper:apply",
                                    style = "wallpaper_apply_btn",
                                    layout = { width = 175, height = 36, justify = "center", align = "center" },
                                },
                            },
                        },
                    },
                },
            },
        },
    },
})
"#;

fn same_wallpaper(entry_path: &str, current: &Path) -> bool {
    let entry_path = Path::new(entry_path);
    if entry_path == current {
        return true;
    }
    match (entry_path.canonicalize(), current.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn wallpaper_lua_config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"));
    let dir = base.join("wyrd");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("wallpaper.lua");
    if !path.is_file() {
        let _ = std::fs::write(&path, DEFAULT_WALLPAPER_LUA);
    }
    path
}

fn settings_toml_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"));
    base.join("wyrd/settings.toml")
}

fn pid_lock_path() -> PathBuf {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    runtime_dir.join("wyrd-wallpaper-picker.pid")
}

/// If another `wyrd-wallpaper select` instance is already open, signal it to close and exit
/// so pressing `Shift + Super + W` toggles the window cleanly.
fn acquire_toggle_lock() -> Option<PathBuf> {
    let lock_path = pid_lock_path();
    if let Ok(content) = std::fs::read_to_string(&lock_path) {
        if let Ok(old_pid) = content.trim().parse::<i32>() {
            if old_pid > 0 && old_pid != std::process::id() as i32 {
                let alive = unsafe { libc::kill(old_pid, 0) == 0 };
                if alive {
                    unsafe {
                        libc::kill(old_pid, libc::SIGTERM);
                    }
                    let _ = std::fs::remove_file(&lock_path);
                    return None;
                }
            }
        }
    }
    let _ = std::fs::write(&lock_path, std::process::id().to_string());
    Some(lock_path)
}

fn ensure_rounded_thumbnail(entry_path: &str) -> String {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    let thumbs_dir = runtime_dir.join("wyrd/thumbs");
    let _ = std::fs::create_dir_all(&thumbs_dir);

    let src_path = Path::new(entry_path);
    let stem = src_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("wall");
    let mtime_secs = std::fs::metadata(src_path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let sanitized: String = stem
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let thumb_path = thumbs_dir.join(format!("{}_{}.png", sanitized, mtime_secs));

    if thumb_path.is_file() {
        return thumb_path.to_string_lossy().to_string();
    }

    let tw = 384u32;
    let th = 240u32;
    if let Ok(img) = image::open(src_path) {
        let mut rgba = img
            .resize_to_fill(tw, th, image::imageops::FilterType::Triangle)
            .to_rgba8();
        let radius = 10.0f32;
        for y in 0..th {
            for x in 0..tw {
                let fx = x as f32 + 0.5;
                let fy = y as f32 + 0.5;
                let dx = if fx < radius {
                    radius - fx
                } else if fx > tw as f32 - radius {
                    fx - (tw as f32 - radius)
                } else {
                    0.0
                };
                let dy = if fy < radius {
                    radius - fy
                } else if fy > th as f32 - radius {
                    fy - (th as f32 - radius)
                } else {
                    0.0
                };
                if dx > 0.0 && dy > 0.0 {
                    let dist = (dx * dx + dy * dy).sqrt();
                    if dist > radius {
                        let alpha = ((radius + 1.0 - dist).clamp(0.0, 1.0) * 255.0) as u8;
                        let px = rgba.get_pixel_mut(x, y);
                        px[3] = ((px[3] as u16 * alpha as u16) / 255) as u8;
                    }
                }
            }
        }
        let _ = rgba.save(&thumb_path);
        if thumb_path.is_file() {
            return thumb_path.to_string_lossy().to_string();
        }
    }

    entry_path.to_string()
}

pub fn run(directory: &Path, current: Option<&Path>) -> Result<Option<PathBuf>> {
    let Some(lock_file) = acquire_toggle_lock() else {
        return Ok(None);
    };

    let entries = catalog::list(directory);
    if entries.is_empty() {
        let _ = std::fs::remove_file(&lock_file);
        anyhow::bail!("no wallpapers found in {}", directory.display());
    }
    let initial_selected = current
        .and_then(|current_path| {
            entries
                .iter()
                .position(|entry| same_wallpaper(&entry.path, current_path))
        })
        .unwrap_or(0);

    let lua_path = wallpaper_lua_config_path();
    let lua_runtime = wyrd_engine::config::lua::LuaRuntime::new(lua_path.clone())?;
    let script =
        std::fs::read_to_string(&lua_path).unwrap_or_else(|_| DEFAULT_WALLPAPER_LUA.to_string());
    let bar_config = lua_runtime
        .load_config_from_str(&script)
        .unwrap_or_default();

    let (win_w, win_h) = bar_config
        .surfaces
        .iter()
        .find(|s| s.name == "wallpaper_picker")
        .or_else(|| bar_config.surfaces.first())
        .map(|s| (s.width.unwrap_or(920).max(480), s.height.max(320)))
        .unwrap_or((920, 540));

    let thumb_paths: Vec<String> = entries
        .iter()
        .map(|e| ensure_rounded_thumbnail(&e.path))
        .collect();

    let conn = Connection::connect_to_env().context("WAYLAND_DISPLAY is not available")?;
    let (globals, mut queue) = registry_queue_init::<PickerState>(&conn)?;
    let qh = queue.handle();
    let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 1..=6, ())?;
    let shm: wl_shm::WlShm = globals.bind(&qh, 1..=2, ())?;
    let layer_shell: ZwlrLayerShellV1 = globals.bind(&qh, 1..=4, ())?;

    let mut state = PickerState {
        shm: Some(shm),
        surface: None,
        layer_surface: None,
        pointer: None,
        keyboard: None,
        width: win_w,
        height: win_h,
        pointer_x: 0.0,
        pointer_y: 0.0,
        pointer_inside: false,
        entries,
        thumb_paths,
        selected: initial_selected,
        confirmed: None,
        closed: false,
        closing: false,
        open_progress: 0.0,
        scroll_offset: 0.0,
        hover_progress: HashMap::new(),
        last_tick: std::time::Instant::now(),
        frame_dirty: true,
        pending_buffers: Vec::new(),
        render_context: wyrd_engine::render::context::RenderContext::new(1.0),
        damage_tracker: wyrd_engine::render::damage::DamageTracker::default(),
        current_tree: None,
        hovered_widget: None,
        lua_runtime,
        bar_config,
        last_lua_mtime: std::fs::metadata(&lua_path).and_then(|m| m.modified()).ok(),
        last_settings_mtime: std::fs::metadata(settings_toml_path())
            .and_then(|m| m.modified())
            .ok(),
        text_scale: 1.0,
        output_scales: HashMap::new(),
        output_scale: 1,
    };

    let surface = compositor.create_surface(&qh, ());
    let layer_surface = layer_shell.get_layer_surface(
        &surface,
        None,
        Layer::Overlay,
        "wyrd-wallpaper-picker".to_string(),
        &qh,
        (),
    );
    layer_surface.set_anchor(Anchor::empty());
    layer_surface.set_size(win_w, win_h);
    layer_surface.set_keyboard_interactivity(KeyboardInteractivity::OnDemand);
    surface.commit();
    state.surface = Some(surface);
    state.layer_surface = Some(layer_surface);

    for global in globals.contents().clone_list() {
        if global.interface == "wl_seat" {
            let _ = globals.registry().bind::<wl_seat::WlSeat, _, _>(
                global.name,
                global.version.min(7),
                &qh,
                (),
            );
        }
        if global.interface == "wl_output" {
            let _ = globals.registry().bind::<wl_output::WlOutput, _, _>(
                global.name,
                global.version.min(4),
                &qh,
                global.name,
            );
        }
    }

    queue.roundtrip(&mut state)?;
    while !state.closed {
        state.check_hot_reload();
        if state.tick_animations() {
            state.frame_dirty = true;
        }
        if state.frame_dirty && state.width > 0 && state.height > 0 {
            state.render(&qh);
            state.frame_dirty = false;
        }
        while queue.dispatch_pending(&mut state)? > 0 {}
        queue.flush()?;

        if state.closed {
            break;
        }
        if let Some(guard) = queue.prepare_read() {
            use std::os::fd::AsRawFd;
            let mut pollfd = libc::pollfd {
                fd: queue.as_fd().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let timeout_ms = if state.is_animating() { 16 } else { 50 };
            let ready = unsafe { libc::poll(&mut pollfd, 1, timeout_ms) };
            if ready > 0 {
                let _ = guard.read();
            } else {
                drop(guard);
            }
        }
        while queue.dispatch_pending(&mut state)? > 0 {}
    }

    for buf in state.pending_buffers.drain(..) {
        buf.destroy();
    }
    if let Some(ls) = state.layer_surface.take() {
        ls.destroy();
    }
    if let Some(s) = state.surface.take() {
        s.destroy();
    }
    let _ = queue.roundtrip(&mut state);
    let _ = std::fs::remove_file(&lock_file);

    Ok(state.confirmed)
}

struct PickerState {
    shm: Option<wl_shm::WlShm>,
    surface: Option<wl_surface::WlSurface>,
    layer_surface: Option<ZwlrLayerSurfaceV1>,
    pointer: Option<wl_pointer::WlPointer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    width: u32,
    height: u32,
    pointer_x: f64,
    pointer_y: f64,
    pointer_inside: bool,
    entries: Vec<WallpaperEntry>,
    thumb_paths: Vec<String>,
    selected: usize,
    confirmed: Option<PathBuf>,
    closed: bool,
    closing: bool,
    open_progress: f32,
    scroll_offset: f32,
    hover_progress: HashMap<String, f32>,
    last_tick: std::time::Instant,
    frame_dirty: bool,
    pending_buffers: Vec<wl_buffer::WlBuffer>,
    render_context: wyrd_engine::render::context::RenderContext,
    damage_tracker: wyrd_engine::render::damage::DamageTracker,
    current_tree: Option<wyrd_engine::widgets::WidgetTree>,
    hovered_widget: Option<wyrd_engine::widgets::WidgetId>,
    lua_runtime: wyrd_engine::config::lua::LuaRuntime,
    bar_config: wyrd_engine::config::BarConfig,
    last_lua_mtime: Option<SystemTime>,
    last_settings_mtime: Option<SystemTime>,
    text_scale: f32,
    output_scales: HashMap<u32, i32>,
    output_scale: i32,
}

impl PickerState {
    fn reload_lua_config(&mut self) {
        if let Ok(script) = std::fs::read_to_string(wallpaper_lua_config_path()) {
            if let Ok(new_cfg) = self.lua_runtime.load_config_from_str(&script) {
                self.bar_config = new_cfg;
                self.frame_dirty = true;
            }
        }
    }

    fn apply_wallpaper_live(path: &str) {
        let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        let wp_sock = runtime_dir.join("wyrd-wallpaper.sock");
        if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(&wp_sock) {
            use std::io::{BufRead, BufReader, Write};
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(800)));
            let req = serde_json::json!({
                "cmd": "set",
                "path": path
            });
            if writeln!(stream, "{}", req).is_ok() {
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                let _ = reader.read_line(&mut line);
            }
        }
    }

    fn check_hot_reload(&mut self) {
        let lua_mtime = std::fs::metadata(wallpaper_lua_config_path())
            .and_then(|m| m.modified())
            .ok();
        let settings_mtime = std::fs::metadata(settings_toml_path())
            .and_then(|m| m.modified())
            .ok();
        if lua_mtime != self.last_lua_mtime || settings_mtime != self.last_settings_mtime {
            self.last_lua_mtime = lua_mtime;
            self.last_settings_mtime = settings_mtime;
            self.reload_lua_config();
        }
    }

    fn recompute_output_scale(&mut self) {
        let next = self
            .output_scales
            .values()
            .copied()
            .max()
            .unwrap_or(1)
            .max(1);
        if next != self.output_scale {
            self.output_scale = next;
            self.frame_dirty = true;
        }
    }

    fn is_animating(&self) -> bool {
        self.closing
            || self.open_progress < 0.999
            || self.scroll_offset.abs() > 0.004
            || self
                .hover_progress
                .values()
                .any(|&v| v > 0.004 && v < 0.996)
    }

    fn tick_animations(&mut self) -> bool {
        let now = std::time::Instant::now();
        let dt = now
            .duration_since(self.last_tick)
            .as_secs_f32()
            .clamp(0.001, 0.05);
        self.last_tick = now;
        let mut changed = false;

        if self.closing {
            let prev = self.open_progress;
            self.open_progress = (self.open_progress - dt / 0.16).max(0.0);
            if (self.open_progress - prev).abs() > 0.0005 {
                changed = true;
            }
            if self.open_progress <= 0.01 {
                self.closed = true;
            }
        } else if self.open_progress < 1.0 {
            let prev = self.open_progress;
            self.open_progress = (self.open_progress + dt / 0.22).min(1.0);
            if (self.open_progress - prev).abs() > 0.0005 {
                changed = true;
            }
        }

        if self.scroll_offset.abs() > 0.004 {
            let decay = (-15.5 * dt).exp();
            self.scroll_offset *= decay;
            if self.scroll_offset.abs() <= 0.004 {
                self.scroll_offset = 0.0;
            }
            changed = true;
        }

        let hovered_cfg_id = self
            .hovered_widget
            .and_then(|wid| self.current_tree.as_ref().and_then(|t| t.get(wid)))
            .and_then(|n| n.id.clone());

        let tracked_ids = [
            "wp_prev_btn",
            "wp_prev_card",
            "wp_active_card",
            "wp_next_card",
            "wp_next_btn",
            "wp_slot_0",
            "wp_slot_1",
            "wp_slot_2",
            "wp_slot_3",
            "wp_slot_4",
            "wp_random_btn",
            "wp_apply_btn",
            "wp_close_btn",
        ];
        for id in tracked_ids {
            let target = if hovered_cfg_id.as_deref() == Some(id) {
                1.0f32
            } else {
                0.0f32
            };
            let cur = *self.hover_progress.get(id).unwrap_or(&0.0);
            if (target - cur).abs() > 0.004 {
                let speed = if target > cur { 14.0 } else { 11.0 };
                let next = cur + (target - cur) * (1.0 - (-speed * dt).exp());
                let next = if (target - next).abs() <= 0.004 {
                    target
                } else {
                    next
                };
                self.hover_progress.insert(id.to_string(), next);
                changed = true;
            }
        }

        changed
    }

    fn shift(&mut self, delta: isize) {
        if self.entries.is_empty() || self.closing {
            return;
        }
        let len = self.entries.len() as isize;
        let next = (self.selected as isize + delta).rem_euclid(len) as usize;
        if next != self.selected {
            self.selected = next;
            self.scroll_offset = (self.scroll_offset + delta as f32).clamp(-2.4, 2.4);
            self.frame_dirty = true;
        }
    }

    fn build_data_store(&self) -> HashMap<String, serde_json::Value> {
        let len = self.entries.len().max(1);
        let idx = self.selected.min(len - 1);
        let prev_idx = (idx + len - 1) % len;
        let next_idx = (idx + 1) % len;

        let slot_indices = [
            (idx + len * 2 - 2) % len,
            (idx + len - 1) % len,
            idx,
            (idx + 1) % len,
            (idx + 2) % len,
        ];

        let entry = &self.entries[idx];
        let stem = Path::new(&entry.path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&entry.name);

        let mut store = HashMap::new();
        store.insert(
            "wallpaper".to_string(),
            serde_json::json!({
                "name": stem,
                "filename": entry.name,
                "path": entry.path,
                "resolution": format!("{}×{}", entry.width, entry.height),
                "index": idx + 1,
                "total": len,
                "prev_thumb": self.thumb_paths[prev_idx],
                "current_thumb": self.thumb_paths[idx],
                "next_thumb": self.thumb_paths[next_idx],
                "slot_0_thumb": self.thumb_paths[slot_indices[0]],
                "slot_1_thumb": self.thumb_paths[slot_indices[1]],
                "slot_2_thumb": self.thumb_paths[slot_indices[2]],
                "slot_3_thumb": self.thumb_paths[slot_indices[3]],
                "slot_4_thumb": self.thumb_paths[slot_indices[4]],
            }),
        );
        store
    }

    fn execute_action(&mut self, action: &str) {
        let len = self.entries.len().max(1);
        match action {
            "wallpaper:prev" => self.shift(-1),
            "wallpaper:next" => self.shift(1),
            "wallpaper:close" => {
                self.closing = true;
                self.frame_dirty = true;
            }
            "wallpaper:apply" => {
                if let Some(entry) = self.entries.get(self.selected) {
                    self.confirmed = Some(PathBuf::from(&entry.path));
                    self.closing = true;
                    self.frame_dirty = true;
                }
            }
            "wallpaper:dynamic_theme" => {
                if let Some(entry) = self.entries.get(self.selected).cloned() {
                    Self::apply_wallpaper_live(&entry.path);
                }
                let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("/tmp"));
                let shell_sock = runtime_dir.join("wyrd-shell.sock");
                if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(&shell_sock) {
                    use std::io::Write;
                    let _ = stream.write_all(b"theme:set dynamic\n");
                } else {
                    let _ = std::process::Command::new("wyrd-shell")
                        .args(["--action", "theme:set dynamic"])
                        .spawn();
                }
                std::thread::sleep(std::time::Duration::from_millis(40));
                self.reload_lua_config();
            }
            "wallpaper:random" => {
                if len > 1 {
                    let nanos = SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .map(|d| d.subsec_nanos() as usize)
                        .unwrap_or(1);
                    let mut next = nanos % len;
                    if next == self.selected {
                        next = (next + 1) % len;
                    }
                    let delta = if next > self.selected { 1.5 } else { -1.5 };
                    self.selected = next;
                    self.scroll_offset = delta;
                    if let Some(entry) = self.entries.get(self.selected).cloned() {
                        Self::apply_wallpaper_live(&entry.path);
                    }
                    self.reload_lua_config();
                    self.frame_dirty = true;
                }
            }
            "wallpaper:select_slot_0" => self.shift(-2),
            "wallpaper:select_slot_1" => self.shift(-1),
            "wallpaper:select_slot_2" => {
                if let Some(entry) = self.entries.get(self.selected) {
                    self.confirmed = Some(PathBuf::from(&entry.path));
                    self.closing = true;
                    self.frame_dirty = true;
                }
            }
            "wallpaper:select_slot_3" => self.shift(1),
            "wallpaper:select_slot_4" => self.shift(2),
            other if !other.trim().is_empty() => {
                let _ = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(other)
                    .spawn();
            }
            _ => {}
        }
    }

    fn render(&mut self, qh: &QueueHandle<Self>) {
        let (Some(shm), Some(surface)) = (self.shm.as_ref(), self.surface.as_ref()) else {
            return;
        };

        let target_scale = self.output_scale.max(1) as f32;
        if (self.text_scale - target_scale).abs() > f32::EPSILON {
            self.render_context =
                wyrd_engine::render::context::RenderContext::new(target_scale.into());
            self.text_scale = target_scale;
        }

        let Some(surface_config) = self
            .bar_config
            .surfaces
            .iter()
            .find(|s| s.name == "wallpaper_picker")
            .or_else(|| self.bar_config.surfaces.first())
            .cloned()
        else {
            return;
        };

        let data_store = self.build_data_store();
        let mut tree = wyrd_engine::widgets::from_popup_config_with_store(
            &surface_config.widgets,
            &self.bar_config.styles,
            &data_store,
        );
        if let Some(root_id) = tree.root() {
            let style_opt = surface_config
                .style
                .as_deref()
                .and_then(|s| self.bar_config.styles.get(s))
                .or_else(|| self.bar_config.styles.get("wallpaper_window"))
                .or_else(|| self.bar_config.styles.get("popup"));
            if let Some(style) = style_opt {
                if let Some(root_node) = tree.get_mut(root_id) {
                    wyrd_engine::widgets::apply_style_to_node(root_node, style);
                }
            }
        }

        let w = self.width.max(1);
        let h = self.height.max(1);
        wyrd_engine::widgets::tree::measure_tree(
            &mut tree,
            &mut self.render_context,
            w as f32,
            h as f32,
        );
        wyrd_engine::widgets::tree::layout_tree(&mut tree, 0.0, 0.0, w as f32, h as f32);

        let p = self.open_progress.clamp(0.0, 1.0);
        let eased_open = if self.closing {
            p * p * (3.0 - 2.0 * p)
        } else {
            1.0 - (1.0 - p).powi(3)
        };
        let surf_scale = (0.92 + 0.08 * eased_open) as f64;
        let surf_oy = ((1.0 - eased_open) * -14.0) as f64;
        if (surf_scale - 1.0).abs() > 0.0005 {
            self.render_context.animator.start_named_easing(
                "surf_scale_0",
                surf_scale,
                surf_scale,
                std::time::Duration::ZERO,
                wyrd_engine::animator::EasingCurve::Linear,
            );
        } else {
            self.render_context.animator.remove_named("surf_scale_0");
        }
        if surf_oy.abs() > 0.01 {
            self.render_context.animator.start_named_easing(
                "surf_oy_0",
                surf_oy,
                surf_oy,
                std::time::Duration::ZERO,
                wyrd_engine::animator::EasingCurve::Linear,
            );
        } else {
            self.render_context.animator.remove_named("surf_oy_0");
        }

        let s = self.scroll_offset;
        for (_, node) in tree.iter_nodes_mut() {
            let Some(cid) = node.id.clone() else {
                continue;
            };
            let hov = *self.hover_progress.get(&cid).unwrap_or(&0.0);
            let (dx, dy, scale, alpha_opt): (f32, f32, f32, Option<f32>) = match cid.as_str() {
                "wp_active_card" => (
                    s * 188.0,
                    -s.abs().min(1.0) * 8.0 - hov * 3.5,
                    (1.0 - s.abs().min(1.0) * 0.30) * (1.0 + hov * 0.028),
                    None,
                ),
                "wp_prev_card" => (
                    s * 150.0,
                    -hov * 3.0,
                    (1.0 + (s * 0.18).clamp(-0.22, 0.28)) * (1.0 + hov * 0.045),
                    Some((0.72 + hov * 0.26 - (-s).max(0.0) * 0.15).clamp(0.45, 1.0)),
                ),
                "wp_next_card" => (
                    s * 150.0,
                    -hov * 3.0,
                    (1.0 + (-s * 0.18).clamp(-0.22, 0.28)) * (1.0 + hov * 0.045),
                    Some((0.72 + hov * 0.26 - s.max(0.0) * 0.15).clamp(0.45, 1.0)),
                ),
                "wp_slot_0" | "wp_slot_1" | "wp_slot_3" | "wp_slot_4" => (
                    s * 162.0,
                    -hov * 3.5,
                    1.0 + hov * 0.055,
                    Some((0.80 + hov * 0.20).clamp(0.5, 1.0)),
                ),
                "wp_slot_2" => (
                    s * 162.0,
                    -(1.0 - s.abs().min(1.0)) * 3.0 - hov * 2.5,
                    (1.03 - s.abs().min(1.0) * 0.05) * (1.0 + hov * 0.04),
                    None,
                ),
                "wp_prev_btn" => {
                    let pulse = (-s).clamp(0.0, 1.0) * 0.14;
                    (
                        -hov * 1.5 - pulse * 8.0,
                        0.0,
                        1.0 + hov * 0.08 + pulse,
                        None,
                    )
                }
                "wp_next_btn" => {
                    let pulse = s.clamp(0.0, 1.0) * 0.14;
                    (hov * 1.5 + pulse * 8.0, 0.0, 1.0 + hov * 0.08 + pulse, None)
                }
                "wp_random_btn" | "wp_apply_btn" | "wp_close_btn" => {
                    (0.0, -hov * 1.5, 1.0 + hov * 0.05, None)
                }
                _ => continue,
            };

            let old_w = node.final_rect.2;
            let old_h = node.final_rect.3;
            let new_w = old_w * scale;
            let new_h = old_h * scale;
            node.final_rect.0 += dx - (new_w - old_w) * 0.5;
            node.final_rect.1 += dy - (new_h - old_h) * 0.5;
            node.final_rect.2 = new_w;
            node.final_rect.3 = new_h;
            if let Some(a) = alpha_opt {
                node.style.opacity = a;
            }
        }

        self.hovered_widget = tree.hit_test(self.pointer_x, self.pointer_y);

        let Some((_, pixmap, _)) = wyrd_engine::render::render_surface(
            0,
            w,
            h,
            self.output_scale.max(1) as f64,
            true,
            &tree,
            &mut self.render_context,
            &mut self.damage_tracker,
            self.hovered_widget,
            None,
            eased_open as f64,
        ) else {
            return;
        };

        self.current_tree = Some(tree);

        let physical_w = pixmap.width();
        let physical_h = pixmap.height();
        let bytes = wyrd_engine::render::to_wayland_bgra(pixmap.data());
        let mut file = match create_shm_file(bytes.len()) {
            Ok(f) => f,
            Err(e) => {
                log::warn!("picker shm alloc: {e}");
                return;
            }
        };
        use std::io::Write;
        if let Err(e) = file.write_all(&bytes) {
            log::warn!("picker shm write: {e}");
            return;
        }
        let pool = shm.create_pool(file.as_fd(), bytes.len() as i32, qh, ());
        let buffer = pool.create_buffer(
            0,
            physical_w as i32,
            physical_h as i32,
            (physical_w * 4) as i32,
            wl_shm::Format::Argb8888,
            qh,
            (),
        );
        pool.destroy();
        surface.set_buffer_scale(self.output_scale.max(1));
        surface.attach(Some(&buffer), 0, 0);
        surface.damage_buffer(0, 0, physical_w as i32, physical_h as i32);
        surface.commit();
        self.pending_buffers.push(buffer);
    }
}

fn create_shm_file(size: usize) -> Result<std::fs::File> {
    let name = std::ffi::CString::new("wyrd-wallpaper-picker")?;
    let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        anyhow::bail!("memfd_create failed");
    }
    if unsafe { libc::ftruncate(fd, size as libc::off_t) } < 0 {
        unsafe { libc::close(fd) };
        anyhow::bail!("ftruncate failed");
    }
    use std::os::fd::FromRawFd;
    Ok(unsafe { std::fs::File::from_raw_fd(fd) })
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for PickerState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } if interface == "wl_output" => {
                let _ = registry.bind::<wl_output::WlOutput, _, _>(name, version.min(4), qh, name);
            }
            wl_registry::Event::GlobalRemove { name }
                if state.output_scales.remove(&name).is_some() =>
            {
                state.recompute_output_scale();
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, u32> for PickerState {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        &global_name: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Scale { factor } = event {
            state.output_scales.insert(global_name, factor.max(1));
            state.recompute_output_scale();
        }
    }
}

impl Dispatch<wl_compositor::WlCompositor, ()> for PickerState {
    fn event(
        _: &mut Self,
        _: &wl_compositor::WlCompositor,
        _: wl_compositor::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_shm::WlShm, ()> for PickerState {
    fn event(
        _: &mut Self,
        _: &wl_shm::WlShm,
        _: wl_shm::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_shm_pool::WlShmPool, ()> for PickerState {
    fn event(
        _: &mut Self,
        _: &wl_shm_pool::WlShmPool,
        _: wl_shm_pool::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for PickerState {
    fn event(
        state: &mut Self,
        buffer: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, wl_buffer::Event::Release) {
            state.pending_buffers.retain(|b| b != buffer);
            buffer.destroy();
        }
    }
}

impl Dispatch<wl_surface::WlSurface, ()> for PickerState {
    fn event(
        _: &mut Self,
        _: &wl_surface::WlSurface,
        _: wl_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for PickerState {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if matches!(event, wl_callback::Event::Done { .. }) {
            state.render(qh);
        }
    }
}

impl Dispatch<ZwlrLayerShellV1, ()> for PickerState {
    fn event(
        _: &mut Self,
        _: &ZwlrLayerShellV1,
        _: zwlr_layer_shell_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, ()> for PickerState {
    fn event(
        state: &mut Self,
        layer_surface: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                layer_surface.ack_configure(serial);
                if width > 0 {
                    state.width = width;
                }
                if height > 0 {
                    state.height = height;
                }
                state.frame_dirty = true;
                state.render(qh);
                state.frame_dirty = false;
            }
            zwlr_layer_surface_v1::Event::Closed => {
                state.closed = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for PickerState {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(caps),
        } = event
        {
            if caps.contains(wl_seat::Capability::Pointer) && state.pointer.is_none() {
                state.pointer = Some(seat.get_pointer(qh, ()));
            }
            if caps.contains(wl_seat::Capability::Keyboard) && state.keyboard.is_none() {
                state.keyboard = Some(seat.get_keyboard(qh, ()));
            }
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for PickerState {
    fn event(
        state: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter {
                surface_x,
                surface_y,
                ..
            } => {
                state.pointer_inside = true;
                state.pointer_x = surface_x;
                state.pointer_y = surface_y;
                if let Some(ref tree) = state.current_tree {
                    let next_hover = tree.hit_test(surface_x, surface_y);
                    if next_hover != state.hovered_widget {
                        state.hovered_widget = next_hover;
                        state.frame_dirty = true;
                    }
                }
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                state.pointer_inside = true;
                state.pointer_x = surface_x;
                state.pointer_y = surface_y;
                if let Some(ref tree) = state.current_tree {
                    let next_hover = tree.hit_test(surface_x, surface_y);
                    if next_hover != state.hovered_widget {
                        state.hovered_widget = next_hover;
                        state.frame_dirty = true;
                    }
                }
            }
            wl_pointer::Event::Leave { .. } => {
                state.pointer_inside = false;
                if state.hovered_widget.is_some() {
                    state.hovered_widget = None;
                    state.frame_dirty = true;
                }
            }
            wl_pointer::Event::Button {
                button,
                state: WEnum::Value(wl_pointer::ButtonState::Pressed),
                ..
            } => {
                if button == 0x110 {
                    let mut clicked_action = None;
                    if let Some(ref tree) = state.current_tree {
                        let mut cur = tree.hit_test(state.pointer_x, state.pointer_y);
                        while let Some(wid) = cur {
                            if let Some(node) = tree.get(wid) {
                                if let Some(ref act) = node.on_click {
                                    clicked_action = Some(act.clone());
                                    break;
                                }
                                cur = node.parent;
                            } else {
                                break;
                            }
                        }
                    }
                    if let Some(act) = clicked_action {
                        state.execute_action(&act);
                    }
                } else if button == 0x111 {
                    state.closing = true;
                    state.frame_dirty = true;
                }
            }
            wl_pointer::Event::Axis {
                axis: WEnum::Value(wl_pointer::Axis::VerticalScroll),
                value,
                ..
            } => {
                if value > 0.5 {
                    state.shift(1);
                } else if value < -0.5 {
                    state.shift(-1);
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for PickerState {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Leave { .. } => {
                if !state.pointer_inside {
                    state.closing = true;
                    state.frame_dirty = true;
                }
            }
            wl_keyboard::Event::Key {
                key,
                state: WEnum::Value(wl_keyboard::KeyState::Pressed),
                ..
            } => match key {
                1 => {
                    state.closing = true;
                    state.frame_dirty = true;
                }
                28 | 96 => {
                    if let Some(entry) = state.entries.get(state.selected) {
                        state.confirmed = Some(PathBuf::from(&entry.path));
                        state.closing = true;
                        state.frame_dirty = true;
                    }
                }
                105 | 30 | 35 => state.shift(-1),
                106 | 32 | 38 => state.shift(1),
                _ => {}
            },
            _ => {}
        }
    }
}
