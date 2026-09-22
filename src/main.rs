//! Wyrd Wallpaper Daemon.
//!
//! Creates Background-layer surfaces on all outputs using `zwlr-layer-shell-v1`,
//! scales and renders wallpaper images, and exports `$XDG_RUNTIME_DIR/wyrd/wallpaper.toml`.

mod catalog;
mod picker;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use log::info;
use serde::Deserialize;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::fd::{AsFd, AsRawFd};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use tiny_skia::{Color, Pixmap, PixmapMut, Transform};
use wayland_client::{
    globals::{registry_queue_init, GlobalListContents},
    protocol::{wl_buffer, wl_compositor, wl_output, wl_registry, wl_shm, wl_shm_pool, wl_surface},
    Connection, Dispatch, Proxy, QueueHandle,
};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{self, Layer, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{self, Anchor, ZwlrLayerSurfaceV1},
};

#[derive(Parser, Debug)]
#[command(name = "wyrd-wallpaper", about = "Wyrd Wayland wallpaper daemon")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// Path to wallpaper image
    #[arg(short, long)]
    image: Option<PathBuf>,

    /// Target output name (optional)
    #[arg(short, long)]
    output: Option<String>,
}

#[derive(Subcommand, Debug)]
enum Command {
    Run {
        #[arg(short, long)]
        image: Option<PathBuf>,
        /// Target output name (optional)
        #[arg(short, long)]
        output: Option<String>,
    },
    Set {
        image: PathBuf,
    },
    List,
    Current,
    Select,
    Color {
        #[arg(value_parser = ["auto", "manual"])]
        mode: String,
        #[arg(long)]
        accent: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
struct SocketCommand {
    cmd: String,
    path: Option<String>,
    mode: Option<String>,
    accent: Option<String>,
}

struct SocketRequest {
    command: SocketCommand,
    reply: Sender<String>,
}

fn socket_path() -> PathBuf {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    runtime_dir.join("wyrd-wallpaper.sock")
}

fn persistent_state_file_path() -> PathBuf {
    if let Ok(state_home) = std::env::var("XDG_STATE_HOME") {
        PathBuf::from(state_home)
            .join("wyrd")
            .join("wallpaper.toml")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home)
            .join(".local")
            .join("state")
            .join("wyrd")
            .join("wallpaper.toml")
    } else {
        PathBuf::from("/tmp").join("wyrd-wallpaper-state.toml")
    }
}

fn config_state_file_path() -> PathBuf {
    if let Ok(config_home) = std::env::var("XDG_CONFIG_HOME") {
        PathBuf::from(config_home)
            .join("wyrd")
            .join("wallpaper.toml")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home)
            .join(".config")
            .join("wyrd")
            .join("wallpaper.toml")
    } else {
        PathBuf::from("/tmp").join("wyrd-wallpaper-config.toml")
    }
}

fn runtime_state_file_path() -> PathBuf {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| {
        // SAFETY: `libc::geteuid` is a standard, stateless POSIX syscall with no side effects.
        let uid = unsafe { libc::geteuid() };
        format!("/run/user/{}", uid)
    });
    PathBuf::from(runtime_dir)
        .join("wyrd")
        .join("wallpaper.toml")
}

fn write_wallpaper_state(state: &wyrd_engine::state_files::WallpaperState) -> Result<()> {
    let content = toml::to_string_pretty(state)?;

    // 1. Write runtime state ($XDG_RUNTIME_DIR/wyrd/wallpaper.toml) for active IPC & live module sync
    let runtime_file = runtime_state_file_path();
    if let Some(parent) = runtime_file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let runtime_tmp = runtime_file.with_extension("toml.tmp");
    if std::fs::write(&runtime_tmp, &content).is_ok() {
        let _ = std::fs::rename(runtime_tmp, &runtime_file);
    }

    // 2. Write persistent state (~/.local/state/wyrd/wallpaper.toml) to survive reboots
    let persistent_file = persistent_state_file_path();
    if let Some(parent) = persistent_file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let persistent_tmp = persistent_file.with_extension("toml.tmp");
    std::fs::write(&persistent_tmp, &content)?;
    std::fs::rename(persistent_tmp, &persistent_file)?;

    Ok(())
}

#[derive(Debug, Default, Deserialize)]
struct PersistedWallpaperState {
    #[serde(default)]
    color_mode: Option<String>,
    #[serde(default)]
    accent: Option<String>,
    #[serde(default)]
    manual_accent: Option<String>,
    #[serde(default)]
    primary: Option<String>,
    #[serde(default)]
    outputs: HashMap<String, String>,
}

/// Restores the last-selected wallpaper and configuration from persistent storage
/// so that choices persist across system reboots.
fn read_persisted_state() -> Option<PersistedWallpaperState> {
    let candidates = [
        persistent_state_file_path(),
        config_state_file_path(),
        runtime_state_file_path(),
    ];

    for path in candidates {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(state) = toml::from_str::<PersistedWallpaperState>(&content) {
                if !state.outputs.is_empty() || state.color_mode.is_some() {
                    return Some(state);
                }
            }
        }
    }
    None
}

struct OutputState {
    output: wl_output::WlOutput,
    name: String,
    width: u32,
    height: u32,
    scale: i32,
    surface: Option<wl_surface::WlSurface>,
    layer_surface: Option<ZwlrLayerSurfaceV1>,
    configured: bool,
}

struct WallpaperState {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    layer_shell: Option<ZwlrLayerShellV1>,
    outputs: HashMap<u32, OutputState>,
    image_path: Option<PathBuf>,
    wallpaper_dir: PathBuf,
    color_mode: String,
    manual_accent: String,
    auto_accent: String,
    palette: Option<catalog::MaterialPalette>,
    output_filter: Option<String>,
    pending_buffers: Vec<wl_buffer::WlBuffer>,
}

impl WallpaperState {
    fn ensure_output_surface(&mut self, global_name: u32, qh: &QueueHandle<WallpaperState>) {
        let (Some(compositor), Some(layer_shell)) =
            (self.compositor.as_ref(), self.layer_shell.as_ref())
        else {
            return;
        };
        let Some(out) = self.outputs.get_mut(&global_name) else {
            return;
        };
        if out.surface.is_some() {
            return;
        }

        let surface = compositor.create_surface(qh, ());
        let layer_surface = layer_shell.get_layer_surface(
            &surface,
            Some(&out.output),
            Layer::Background,
            "wallpaper".to_string(),
            qh,
            global_name,
        );
        layer_surface.set_anchor(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right);
        layer_surface.set_exclusive_zone(-1);
        layer_surface.set_size(0, 0);
        surface.commit();
        out.surface = Some(surface);
        out.layer_surface = Some(layer_surface);
    }

    fn render_output(&mut self, qh: &QueueHandle<WallpaperState>, global_name: u32) {
        let Some(shm) = self.shm.as_ref() else { return };
        let Some(out) = self.outputs.get_mut(&global_name) else {
            return;
        };
        if let Some(filter) = self.output_filter.as_deref() {
            if out.name != filter {
                return;
            }
        }
        let scale = out.scale.max(1);
        let (pixel_width, pixel_height) = (
            (out.width.max(1) * scale as u32),
            (out.height.max(1) * scale as u32),
        );
        let Some(surface) = out.surface.as_ref() else {
            return;
        };

        let mut pixmap = match Pixmap::new(pixel_width, pixel_height) {
            Some(p) => p,
            None => return,
        };

        let image_path = self.image_path.as_ref();
        if let Some(path) = image_path {
            let Ok(img) = image::open(path) else { return };
            let resized = img.resize_to_fill(
                pixel_width,
                pixel_height,
                image::imageops::FilterType::Triangle,
            );
            let rgba = resized.to_rgba8();
            if let Some(source) =
                PixmapMut::from_bytes(&mut rgba.into_raw(), pixel_width, pixel_height)
            {
                pixmap.draw_pixmap(
                    0,
                    0,
                    source.as_ref(),
                    &tiny_skia::PixmapPaint::default(),
                    Transform::identity(),
                    None,
                );
            }
        } else {
            // Default elegant obsidian crimson gradient
            pixmap.fill(Color::from_rgba8(13, 6, 15, 255));
        }

        let stride = (pixel_width * 4) as i32;
        let size = (stride * pixel_height as i32) as usize;

        match create_shm_file(size) {
            Ok(mut file) => {
                use std::io::Write;
                let bgra_data = wyrd_engine::render::to_wayland_bgra(pixmap.data());
                if let Err(error) = file.write_all(&bgra_data) {
                    log::warn!(
                        "failed to write wallpaper buffer for output {}: {error}",
                        out.name
                    );
                    return;
                }
                let pool = shm.create_pool(file.as_fd(), size as i32, qh, ());
                let buffer = pool.create_buffer(
                    0,
                    pixel_width as i32,
                    pixel_height as i32,
                    stride,
                    wl_shm::Format::Argb8888,
                    qh,
                    (),
                );
                // Pool can be destroyed immediately; the buffer retains the SHM mapping.
                pool.destroy();
                surface.set_buffer_scale(scale);
                surface.attach(Some(&buffer), 0, 0);
                surface.damage_buffer(0, 0, pixel_width as i32, pixel_height as i32);
                surface.commit();
                self.pending_buffers.push(buffer);
            }
            Err(error) => {
                log::warn!(
                    "failed to create shm buffer for output {}: {error}",
                    out.name
                );
            }
        }
    }

    fn runtime_state(&self) -> wyrd_engine::state_files::WallpaperState {
        let mut outputs: HashMap<String, String> = self
            .outputs
            .values()
            .map(|output| {
                (
                    output.name.clone(),
                    self.image_path
                        .as_ref()
                        .map(|path| path.to_string_lossy().to_string())
                        .unwrap_or_default(),
                )
            })
            .filter(|(_, path)| !path.is_empty())
            .collect();
        if outputs.is_empty() {
            if let Some(ref path) = self.image_path {
                outputs.insert("default".to_string(), path.to_string_lossy().to_string());
            }
        }
        wyrd_engine::state_files::WallpaperState {
            color_mode: self.color_mode.clone(),
            accent: if self.color_mode == "manual" {
                self.manual_accent.clone()
            } else {
                self.auto_accent.clone()
            },
            auto_accent: self.auto_accent.clone(),
            primary: self
                .outputs
                .values()
                .next()
                .map(|output| output.name.clone())
                .or_else(|| Some("default".to_string())),
            outputs,
            palette: self.palette.as_ref().map(|p| p.dark.to_map()),
        }
    }

    fn persist_state(&self) {
        if let Err(error) = write_wallpaper_state(&self.runtime_state()) {
            log::warn!("failed to write wallpaper state: {error}");
        }
    }

    fn set_image(&mut self, path: PathBuf, qh: &QueueHandle<WallpaperState>) -> Result<()> {
        let path = catalog::resolve(&path, &self.wallpaper_dir)?;
        let palette = catalog::extract_palette(&path);
        self.auto_accent = palette.dark.primary.clone();
        self.palette = Some(palette);
        self.image_path = Some(path);
        self.persist_state();
        let output_keys = self.outputs.keys().copied().collect::<Vec<_>>();
        for key in output_keys {
            self.render_output(qh, key);
        }
        Ok(())
    }

    fn set_color(&mut self, mode: String, accent: Option<String>) -> Result<()> {
        if mode == "manual" {
            let accent = accent.ok_or_else(|| anyhow::anyhow!("manual mode requires --accent"))?;
            if !is_hex_color(&accent) {
                return Err(anyhow::anyhow!("accent must be a #RRGGBB color"));
            }
            if let Some(palette) = catalog::palette_from_hex(&accent) {
                self.palette = Some(palette);
            }
            self.manual_accent = accent;
        } else if mode == "auto" {
            if let Some(path) = &self.image_path {
                let palette = catalog::extract_palette(path);
                self.auto_accent = palette.dark.primary.clone();
                self.palette = Some(palette);
            }
        }
        self.color_mode = mode;
        self.persist_state();
        Ok(())
    }
}

fn is_hex_color(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value[1..].chars().all(|ch| ch.is_ascii_hexdigit())
}

fn create_shm_file(size: usize) -> Result<std::fs::File> {
    let name = format!("/wyrd-wall-shm-{}", std::process::id());
    let name_c = std::ffi::CString::new(name)?;
    // SAFETY: `name_c` is a valid null-terminated C string. We create an exclusive posix shm segment.
    let fd = unsafe {
        libc::shm_open(
            name_c.as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL,
            0o600,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: `shm_unlink` detaches the shm name immediately, preserving the open fd.
    // `ftruncate` sets the file length to the required shm size for tiny-skia pixmap.
    unsafe {
        libc::shm_unlink(name_c.as_ptr());
        libc::ftruncate(fd, size as libc::off_t);
    }
    use std::os::unix::io::FromRawFd;
    // SAFETY: `fd` is a valid, open file descriptor owned by this function.
    Ok(unsafe { std::fs::File::from_raw_fd(fd) })
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for WallpaperState {
    fn event(
        state: &mut Self,
        proxy: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } if interface == "wl_output" => {
                let output =
                    proxy.bind::<wl_output::WlOutput, _, _>(name, version.min(4), qh, name);
                state.outputs.insert(
                    name,
                    OutputState {
                        output,
                        name: format!("output-{name}"),
                        width: 0,
                        height: 0,
                        scale: 1,
                        surface: None,
                        layer_surface: None,
                        configured: false,
                    },
                );
                state.ensure_output_surface(name, qh);
            }
            wl_registry::Event::GlobalRemove { name } => {
                if let Some(output) = state.outputs.remove(&name) {
                    if let Some(layer_surface) = output.layer_surface {
                        layer_surface.destroy();
                    }
                    if let Some(surface) = output.surface {
                        surface.destroy();
                    }
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_compositor::WlCompositor, ()> for WallpaperState {
    fn event(
        _state: &mut Self,
        _: &wl_compositor::WlCompositor,
        _: wl_compositor::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_shm::WlShm, ()> for WallpaperState {
    fn event(
        _state: &mut Self,
        _: &wl_shm::WlShm,
        _: wl_shm::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_shm_pool::WlShmPool, ()> for WallpaperState {
    fn event(
        _state: &mut Self,
        _: &wl_shm_pool::WlShmPool,
        _: wl_shm_pool::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for WallpaperState {
    fn event(
        state: &mut Self,
        buffer: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, wl_buffer::Event::Release) {
            state
                .pending_buffers
                .retain(|pending| pending.id() != buffer.id());
        }
    }
}

impl Dispatch<wl_output::WlOutput, u32> for WallpaperState {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        &global_name: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Mode { width, height, .. } = event {
            if let Some(out) = state.outputs.get_mut(&global_name) {
                out.width = width as u32;
                out.height = height as u32;
            }
        } else if let wl_output::Event::Name { name } = event {
            if let Some(out) = state.outputs.get_mut(&global_name) {
                out.name = name;
            }
        } else if let wl_output::Event::Scale { factor } = event {
            if let Some(out) = state.outputs.get_mut(&global_name) {
                out.scale = factor.max(1);
            }
        }
    }
}

impl Dispatch<wl_surface::WlSurface, ()> for WallpaperState {
    fn event(
        _state: &mut Self,
        _: &wl_surface::WlSurface,
        _: wl_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrLayerShellV1, ()> for WallpaperState {
    fn event(
        _state: &mut Self,
        _: &ZwlrLayerShellV1,
        _: zwlr_layer_shell_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, u32> for WallpaperState {
    fn event(
        state: &mut Self,
        layer_surface: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        &global_name: &u32,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let zwlr_layer_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            layer_surface.ack_configure(serial);
            if let Some(out) = state.outputs.get_mut(&global_name) {
                out.width = width;
                out.height = height;
                out.configured = true;
            }
            state.render_output(qh, global_name);
        }
    }
}

fn start_socket_server(tx: Sender<SocketRequest>) -> Result<()> {
    let path = socket_path();
    let _ = std::fs::remove_file(&path);
    let listener =
        UnixListener::bind(&path).with_context(|| format!("failed to bind {}", path.display()))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let tx = tx.clone();
            thread::spawn(move || handle_socket_client(stream, tx));
        }
    });
    Ok(())
}

fn handle_socket_client(mut stream: UnixStream, tx: Sender<SocketRequest>) {
    let Ok(clone) = stream.try_clone() else {
        return;
    };
    let mut line = String::new();
    if BufReader::new(clone).read_line(&mut line).is_err() {
        return;
    }
    let response = match serde_json::from_str::<SocketCommand>(&line) {
        Ok(command) => {
            let (reply_tx, reply_rx) = mpsc::channel();
            if tx
                .send(SocketRequest {
                    command,
                    reply: reply_tx,
                })
                .is_err()
            {
                r#"{"status":"error","message":"daemon unavailable"}"#.to_string()
            } else {
                reply_rx
                    .recv()
                    .unwrap_or_else(|_| r#"{"status":"error","message":"no response"}"#.to_string())
            }
        }
        Err(error) => serde_json::json!({"status":"error","message":error.to_string()}).to_string(),
    };
    let _ = writeln!(stream, "{response}");
}

fn handle_command(
    state: &mut WallpaperState,
    request: SocketRequest,
    qh: &QueueHandle<WallpaperState>,
) {
    let result = match request.command.cmd.as_str() {
        "set" => request
            .command
            .path
            .map(PathBuf::from)
            .ok_or_else(|| anyhow::anyhow!("missing path"))
            .and_then(|path| state.set_image(path, qh))
            .map(|_| serde_json::json!({"status":"ok"})),
        "list" => {
            Ok(serde_json::json!({"status":"ok", "entries": catalog::list(&state.wallpaper_dir)}))
        }
        "current" => Ok(
            serde_json::json!({"status":"ok", "path": state.image_path, "color_mode": state.color_mode, "accent": state.runtime_state().accent, "palette": state.palette.as_ref().map(|p| p.dark.to_map())}),
        ),
        "color" => request
            .command
            .mode
            .map(|mode| state.set_color(mode, request.command.accent))
            .ok_or_else(|| anyhow::anyhow!("missing color mode"))
            .map(|_| serde_json::json!({"status":"ok"})),
        _ => Err(anyhow::anyhow!("unknown command")),
    };
    let response = match result {
        Ok(value) => value,
        Err(error) => serde_json::json!({"status":"error", "message":error.to_string()}),
    };
    let _ = request.reply.send(response.to_string());
}

fn socket_request_raw(command: serde_json::Value) -> Result<String> {
    let mut stream =
        UnixStream::connect(socket_path()).context("wallpaper daemon is not running")?;
    writeln!(stream, "{command}")?;
    let mut response = String::new();
    BufReader::new(stream).read_line(&mut response)?;
    Ok(response)
}

fn socket_request(command: serde_json::Value) -> Result<()> {
    let response = socket_request_raw(command)?;
    print!("{response}");
    Ok(())
}

/// Asks the running daemon which wallpaper is currently active, so the picker can
/// open with that wallpaper already selected. Returns `None` if the daemon isn't
/// running or has no wallpaper set yet - the picker just falls back to the first
/// entry in that case.
fn current_wallpaper_path() -> Option<PathBuf> {
    if let Ok(response) = socket_request_raw(serde_json::json!({"cmd": "current"})) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(response.trim()) {
            if let Some(path) = value.get("path").and_then(|p| p.as_str()) {
                return Some(PathBuf::from(path));
            }
        }
    }
    let wallpaper_dir = catalog::default_directory();
    read_persisted_state().and_then(|p| {
        let raw = p
            .primary
            .as_ref()
            .and_then(|prim| p.outputs.get(prim))
            .or_else(|| p.outputs.values().next())?;
        catalog::resolve(Path::new(raw), &wallpaper_dir).ok()
    })
}

fn run_client(command: Command, top_level_output: Option<String>) -> Result<()> {
    match command {
        Command::Run { image, output } => run_daemon(image, output.or(top_level_output)),
        Command::Set { image } => socket_request(serde_json::json!({"cmd":"set", "path":image})),
        Command::List => socket_request(serde_json::json!({"cmd":"list"})),
        Command::Current => socket_request(serde_json::json!({"cmd":"current"})),
        Command::Select => {
            let directory = catalog::default_directory();
            let current = current_wallpaper_path();
            if let Some(path) = picker::run(&directory, current.as_deref())? {
                socket_request(serde_json::json!({"cmd":"set", "path":path}))?;
            }
            Ok(())
        }
        Command::Color { mode, accent } => {
            socket_request(serde_json::json!({"cmd":"color", "mode":mode, "accent":accent}))
        }
    }
}

fn run_daemon(initial_image: Option<PathBuf>, output_filter: Option<String>) -> Result<()> {
    info!("Starting wyrd-wallpaper");

    let conn = Connection::connect_to_env()
        .context("WAYLAND_DISPLAY not set; cannot connect to compositor")?;
    let (globals, mut event_queue) =
        registry_queue_init::<WallpaperState>(&conn).context("failed to init Wayland registry")?;
    let qh = event_queue.handle();

    let wallpaper_dir = catalog::default_directory();
    let persisted = read_persisted_state();

    let persisted_image = persisted.as_ref().and_then(|p| {
        if let Some(ref primary) = p.primary {
            if let Some(path_str) = p.outputs.get(primary) {
                if !path_str.is_empty() {
                    if let Ok(resolved) = catalog::resolve(Path::new(path_str), &wallpaper_dir) {
                        return Some(resolved);
                    }
                }
            }
        }
        let raw_path = p.outputs.values().next()?;
        if raw_path.is_empty() {
            return None;
        }
        match catalog::resolve(Path::new(raw_path), &wallpaper_dir) {
            Ok(path) => Some(path),
            Err(error) => {
                log::warn!("persisted wallpaper is no longer valid: {error}");
                None
            }
        }
    });

    let image_path = initial_image
        .and_then(|path| catalog::resolve(&path, &wallpaper_dir).ok())
        .or(persisted_image)
        .or_else(|| {
            catalog::list(&wallpaper_dir)
                .first()
                .and_then(|entry| catalog::resolve(Path::new(&entry.path), &wallpaper_dir).ok())
        });
    if let Some(path) = &image_path {
        info!("Loading wallpaper image from {:?}", path);
    }

    let (color_mode, manual_accent) = if let Some(ref p) = persisted {
        (
            p.color_mode.clone().unwrap_or_else(|| "auto".to_string()),
            p.manual_accent
                .clone()
                .or_else(|| p.accent.clone())
                .unwrap_or_else(|| "#c72548".to_string()),
        )
    } else {
        ("auto".to_string(), "#c72548".to_string())
    };

    let palette = if color_mode == "manual" {
        catalog::palette_from_hex(&manual_accent)
    } else {
        image_path.as_deref().map(catalog::extract_palette)
    };

    let auto_accent = palette
        .as_ref()
        .map(|p| p.dark.primary.clone())
        .unwrap_or_else(|| "#c72548".to_string());

    let mut state = WallpaperState {
        compositor: None,
        shm: None,
        layer_shell: None,
        outputs: HashMap::new(),
        image_path: image_path.clone(),
        wallpaper_dir,
        color_mode,
        manual_accent,
        auto_accent,
        palette,
        output_filter,
        pending_buffers: Vec::new(),
    };

    for global in globals
        .contents()
        .clone_list()
        .into_iter()
        .filter(|g| g.interface == "wl_output")
    {
        let output = globals.registry().bind::<wl_output::WlOutput, _, _>(
            global.name,
            global.version.min(4),
            &qh,
            global.name,
        );
        state.outputs.insert(
            global.name,
            OutputState {
                output,
                name: format!("output-{}", global.name),
                width: 0,
                height: 0,
                scale: 1,
                surface: None,
                layer_surface: None,
                configured: false,
            },
        );
    }

    let compositor: wl_compositor::WlCompositor = globals
        .bind(&qh, 1..=6, ())
        .context("wl_compositor not available")?;
    let shm: wl_shm::WlShm = globals
        .bind(&qh, 1..=2, ())
        .context("wl_shm not available")?;
    let layer_shell: ZwlrLayerShellV1 = globals
        .bind(&qh, 1..=4, ())
        .context("zwlr_layer_shell_v1 not available")?;

    let output_keys = state.outputs.keys().copied().collect::<Vec<_>>();
    for key in output_keys {
        let out = state.outputs.get_mut(&key).unwrap();
        let surface = compositor.create_surface(&qh, ());
        let layer_surf = layer_shell.get_layer_surface(
            &surface,
            Some(&out.output),
            Layer::Background,
            "wallpaper".to_string(),
            &qh,
            key,
        );
        layer_surf.set_anchor(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right);
        layer_surf.set_exclusive_zone(-1);
        layer_surf.set_size(0, 0);
        surface.commit();

        out.surface = Some(surface);
        out.layer_surface = Some(layer_surf);
    }

    state.compositor = Some(compositor);
    state.shm = Some(shm);
    state.layer_shell = Some(layer_shell);

    let output_keys = state.outputs.keys().copied().collect::<Vec<_>>();
    for key in output_keys {
        state.ensure_output_surface(key, &qh);
    }

    state.persist_state();

    let (tx, rx): (Sender<SocketRequest>, Receiver<SocketRequest>) = mpsc::channel();
    start_socket_server(tx)?;

    loop {
        let output_keys = state.outputs.keys().copied().collect::<Vec<_>>();
        for key in output_keys {
            state.ensure_output_surface(key, &qh);
        }
        while let Ok(request) = rx.try_recv() {
            handle_command(&mut state, request, &qh);
        }
        while event_queue.dispatch_pending(&mut state)? > 0 {}
        event_queue.flush()?;

        let Some(guard) = event_queue.prepare_read() else {
            continue;
        };
        let mut pollfd = libc::pollfd {
            fd: event_queue.as_fd().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `pollfd` is a valid mutable reference to a stack-allocated struct of size 1.
        let ready = unsafe { libc::poll(&mut pollfd, 1, 50) };
        if ready > 0 {
            guard.read()?;
        } else {
            drop(guard);
        }
    }
}

fn main() -> Result<()> {
    env_logger::init();
    let cli = Cli::parse();
    match cli.command {
        Some(command) => run_client(command, cli.output),
        None => run_daemon(cli.image, cli.output),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_persistent_state_roundtrip() {
        let mut outputs = HashMap::new();
        outputs.insert(
            "eDP-1".to_string(),
            "/home/user/Pictures/wall.jpg".to_string(),
        );

        let state = wyrd_engine::state_files::WallpaperState {
            color_mode: "manual".to_string(),
            accent: "#123456".to_string(),
            auto_accent: "#c72548".to_string(),
            primary: Some("eDP-1".to_string()),
            outputs,
            palette: None,
        };

        let content = toml::to_string_pretty(&state).expect("Failed to serialize state");
        let persisted: PersistedWallpaperState =
            toml::from_str(&content).expect("Failed to deserialize persisted state");

        assert_eq!(persisted.color_mode.as_deref(), Some("manual"));
        assert_eq!(persisted.accent.as_deref(), Some("#123456"));
        assert_eq!(persisted.primary.as_deref(), Some("eDP-1"));
        assert_eq!(
            persisted.outputs.get("eDP-1").map(|s| s.as_str()),
            Some("/home/user/Pictures/wall.jpg")
        );
    }
}
