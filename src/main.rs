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
        /// Target output name (optional)
        #[arg(short, long)]
        output: Option<String>,
    },
    List,
    Current,
    Select {
        /// Target output name (optional)
        #[arg(short, long)]
        output: Option<String>,
    },
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
    output: Option<String>,
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

fn write_wallpaper_state(state: &wyrd_engine::state_files::WallpaperState) -> Result<()> {
    wyrd_engine::state_files::write_wallpaper_state(state)?;

    // Notify wyrd-shell via UNIX socket so dynamic Material You theme updates immediately
    let shell_sock = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("wyrd-shell.sock");
    if let Ok(mut stream) = UnixStream::connect(&shell_sock) {
        let _ = stream.set_write_timeout(Some(std::time::Duration::from_millis(100)));
        let _ = stream.write_all(b"wallpaper:changed\n");
    }

    Ok(())
}

fn read_persisted_state() -> Option<wyrd_engine::state_files::WallpaperState> {
    wyrd_engine::state_files::read_persisted_wallpaper_state()
}

struct OutputState {
    output: wl_output::WlOutput,
    name: String,
    image_path: Option<PathBuf>,
    width: u32,
    height: u32,
    scale: i32,
    surface: Option<wl_surface::WlSurface>,
    layer_surface: Option<ZwlrLayerSurfaceV1>,
    configured: bool,
    current_bgra: Vec<u8>,
    prev_bgra: Vec<u8>,
    target_bgra: Vec<u8>,
    transition_start: Option<std::time::Instant>,
}

struct WallpaperState {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    layer_shell: Option<ZwlrLayerShellV1>,
    outputs: HashMap<u32, OutputState>,
    per_output_paths: HashMap<String, PathBuf>,
    image_path: Option<PathBuf>,
    decoded_image: Option<image::DynamicImage>,
    wallpaper_dir: PathBuf,
    color_mode: String,
    manual_accent: String,
    auto_accent: String,
    palette: Option<catalog::MaterialPalette>,
    output_filter: Option<String>,
    pending_buffers: Vec<wl_buffer::WlBuffer>,
    last_transition_step: Option<std::time::Instant>,
}

fn deterministic_primary<'a, T>(
    outputs: impl Iterator<Item = (&'a str, &'a T)>,
) -> Option<(&'a str, &'a T)> {
    outputs.min_by(|(left, _), (right, _)| left.cmp(right))
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

    fn decode_bgra_for_size(
        &mut self,
        target_path: Option<&Path>,
        pixel_width: u32,
        pixel_height: u32,
    ) -> Vec<u8> {
        let mut pixmap = match Pixmap::new(pixel_width, pixel_height) {
            Some(p) => p,
            None => return Vec::new(),
        };
        let effective_path = target_path.or(self.image_path.as_deref());
        let custom_decoded = if effective_path != self.image_path.as_deref() {
            effective_path.and_then(|p| image::open(p).ok())
        } else {
            if self.decoded_image.is_none() {
                if let Some(path) = self.image_path.as_ref() {
                    self.decoded_image = image::open(path).ok();
                }
            }
            None
        };
        let img_ref = custom_decoded.as_ref().or(self.decoded_image.as_ref());
        if let Some(img) = img_ref {
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
            pixmap.fill(Color::from_rgba8(13, 6, 15, 255));
        }
        wyrd_engine::render::to_wayland_bgra(pixmap.data())
    }

    fn commit_bgra_to_output(
        shm: &wl_shm::WlShm,
        qh: &QueueHandle<WallpaperState>,
        out: &OutputState,
        bgra_data: &[u8],
        pending_buffers: &mut Vec<wl_buffer::WlBuffer>,
    ) {
        let scale = out.scale.max(1);
        let pixel_width = out.width.max(1) * scale as u32;
        let pixel_height = out.height.max(1) * scale as u32;
        let Some(surface) = out.surface.as_ref() else {
            return;
        };
        let stride = (pixel_width * 4) as i32;
        let size = bgra_data.len();
        if size != (stride as usize) * (pixel_height as usize) {
            return;
        }
        if let Ok(mut file) = create_shm_file(size) {
            if file.write_all(bgra_data).is_err() {
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
            pool.destroy();
            surface.set_buffer_scale(scale);
            surface.attach(Some(&buffer), 0, 0);
            surface.damage_buffer(0, 0, pixel_width as i32, pixel_height as i32);
            surface.commit();
            pending_buffers.push(buffer);
        }
    }

    fn render_output(&mut self, qh: &QueueHandle<WallpaperState>, global_name: u32) {
        let (pixel_width, pixel_height, output_image_path) = {
            let Some(out) = self.outputs.get(&global_name) else {
                return;
            };
            if let Some(filter) = self.output_filter.as_deref() {
                if out.name != filter {
                    return;
                }
            }
            if out.surface.is_none() {
                return;
            }
            let scale = out.scale.max(1) as u32;
            let specific = out
                .image_path
                .clone()
                .or_else(|| self.per_output_paths.get(&out.name).cloned())
                .or_else(|| self.image_path.clone());
            (
                out.width.max(1) * scale,
                out.height.max(1) * scale,
                specific,
            )
        };

        let target_bgra =
            self.decode_bgra_for_size(output_image_path.as_deref(), pixel_width, pixel_height);
        if target_bgra.is_empty() {
            return;
        }

        let Some(shm) = self.shm.as_ref() else { return };
        let Some(out) = self.outputs.get_mut(&global_name) else {
            return;
        };

        if !out.current_bgra.is_empty() && out.current_bgra.len() == target_bgra.len() {
            out.prev_bgra.clone_from(&out.current_bgra);
            out.target_bgra = target_bgra;
            out.transition_start = Some(std::time::Instant::now());
        } else {
            out.current_bgra = target_bgra;
            out.target_bgra = Vec::new();
            out.prev_bgra = Vec::new();
            out.transition_start = None;
            Self::commit_bgra_to_output(shm, qh, out, &out.current_bgra, &mut self.pending_buffers);
        }
        if !self.any_output_animating()
            && self
                .outputs
                .values()
                .all(|o| !o.configured || !o.current_bgra.is_empty())
        {
            self.decoded_image = None;
            #[cfg(all(target_os = "linux", target_env = "gnu"))]
            unsafe {
                libc::malloc_trim(0);
            }
        }
    }

    fn any_output_animating(&self) -> bool {
        self.outputs.values().any(|o| o.transition_start.is_some())
    }

    fn step_transitions(&mut self, qh: &QueueHandle<WallpaperState>) {
        const DURATION_SECS: f32 = 0.44;
        self.last_transition_step = Some(std::time::Instant::now());
        let Some(shm) = self.shm.as_ref() else { return };
        let keys: Vec<u32> = self
            .outputs
            .iter()
            .filter_map(|(&k, o)| o.transition_start.map(|_| k))
            .collect();
        for key in keys {
            let Some(out) = self.outputs.get_mut(&key) else {
                continue;
            };
            let Some(start) = out.transition_start else {
                continue;
            };
            let elapsed = start.elapsed().as_secs_f32();
            let t = (elapsed / DURATION_SECS).clamp(0.0, 1.0);
            let scale = out.scale.max(1) as u32;
            let pw = out.width.max(1) * scale;
            let ph = out.height.max(1) * scale;

            if t >= 1.0 || out.prev_bgra.len() != out.target_bgra.len() {
                out.current_bgra = std::mem::take(&mut out.target_bgra);
                out.prev_bgra = Vec::new();
                out.transition_start = None;
                Self::commit_bgra_to_output(
                    shm,
                    qh,
                    out,
                    &out.current_bgra,
                    &mut self.pending_buffers,
                );
            } else {
                blend_wallpaper_transition(
                    &out.prev_bgra,
                    &out.target_bgra,
                    &mut out.current_bgra,
                    pw,
                    ph,
                    t,
                );
                Self::commit_bgra_to_output(
                    shm,
                    qh,
                    out,
                    &out.current_bgra,
                    &mut self.pending_buffers,
                );
            }
        }
        if !self.any_output_animating() {
            self.last_transition_step = None;
            self.decoded_image = None;
            #[cfg(all(target_os = "linux", target_env = "gnu"))]
            unsafe {
                libc::malloc_trim(0);
            }
        }
    }

    fn runtime_state(&self) -> wyrd_engine::state_files::WallpaperState {
        let mut outputs: HashMap<String, String> = self
            .per_output_paths
            .iter()
            .map(|(k, v)| (k.clone(), v.to_string_lossy().to_string()))
            .collect();
        for output in self.outputs.values() {
            let effective = output
                .image_path
                .as_ref()
                .or_else(|| self.per_output_paths.get(&output.name))
                .or(self.image_path.as_ref());
            if let Some(path) = effective {
                outputs.insert(output.name.clone(), path.to_string_lossy().to_string());
            }
        }
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
            primary: deterministic_primary(
                self.outputs
                    .values()
                    .map(|output| (output.name.as_str(), output)),
            )
            .map(|(name, _)| name.to_string())
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

    fn set_image(
        &mut self,
        path: PathBuf,
        target_output: Option<String>,
        qh: &QueueHandle<WallpaperState>,
    ) -> Result<()> {
        let path = catalog::resolve(&path, &self.wallpaper_dir)?;
        if let Some(target_name) = target_output {
            self.per_output_paths
                .insert(target_name.clone(), path.clone());
            let mut target_keys = Vec::new();
            for (&key, out) in self.outputs.iter_mut() {
                if out.name == target_name {
                    out.image_path = Some(path.clone());
                    target_keys.push(key);
                }
            }
            for key in target_keys {
                self.render_output(qh, key);
            }
            let primary_name = deterministic_primary(
                self.outputs
                    .values()
                    .map(|output| (output.name.as_str(), output)),
            )
            .map(|(name, _)| name);
            if self.image_path.is_none() || primary_name == Some(target_name.as_str()) {
                self.image_path = Some(path.clone());
                let palette = catalog::extract_palette(&path);
                self.auto_accent = palette.dark.primary.clone();
                self.palette = Some(palette);
            }
        } else {
            self.decoded_image = image::open(&path).ok();
            self.image_path = Some(path.clone());
            self.per_output_paths.clear();
            for out in self.outputs.values_mut() {
                out.image_path = None;
            }
            let output_keys = self.outputs.keys().copied().collect::<Vec<_>>();
            for key in output_keys {
                self.render_output(qh, key);
            }
            let palette = catalog::extract_palette(&path);
            self.auto_accent = palette.dark.primary.clone();
            self.palette = Some(palette);
        }
        self.step_transitions(qh);
        self.persist_state();
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

fn blend_wallpaper_transition(
    prev: &[u8],
    target: &[u8],
    dst: &mut [u8],
    width: u32,
    height: u32,
    t: f32,
) {
    let eased = 1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3);
    let cx = width as f32 * 0.5;
    let cy = height as f32 * 0.5;
    let inv_max_dist_sq = 1.0 / (cx * cx + cy * cy).max(1.0);
    let wave_front = eased * 1.45;
    let inv_feather = 1.0 / 0.45f32;
    let w_usize = width as usize;

    let dx2_norm: Vec<f32> = (0..w_usize)
        .map(|x| {
            let dx = x as f32 - cx;
            dx * dx * inv_max_dist_sq
        })
        .collect();

    for y in 0..height as usize {
        let dy = y as f32 - cy;
        let dy2_norm = dy * dy * inv_max_dist_sq;
        let row_offset = y * w_usize * 4;
        for (x, &dx2) in dx2_norm.iter().enumerate() {
            let dist_sq_norm = dx2 + dy2_norm;
            let radial = ((wave_front - dist_sq_norm) * inv_feather).clamp(0.0, 1.0);
            let radial_smooth = radial * radial * (3.0 - 2.0 * radial);
            let alpha_f = (radial_smooth * 0.68 + eased * 0.32).clamp(0.0, 1.0);
            let a = (alpha_f * 256.0) as u32;
            let inv = 256 - a;
            let idx = row_offset + x * 4;
            dst[idx] = ((prev[idx] as u32 * inv + target[idx] as u32 * a) >> 8) as u8;
            dst[idx + 1] = ((prev[idx + 1] as u32 * inv + target[idx + 1] as u32 * a) >> 8) as u8;
            dst[idx + 2] = ((prev[idx + 2] as u32 * inv + target[idx + 2] as u32 * a) >> 8) as u8;
            dst[idx + 3] = 255;
        }
    }
}

fn create_shm_file(size: usize) -> Result<std::fs::File> {
    let name = format!("/wyrd-wall-shm-{}", std::process::id());
    let name_c = std::ffi::CString::new(name)?;
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
    unsafe {
        libc::shm_unlink(name_c.as_ptr());
    }
    if unsafe { libc::ftruncate(fd, size as libc::off_t) } != 0 {
        let error = std::io::Error::last_os_error();
        unsafe {
            libc::close(fd);
        }
        return Err(error.into());
    }
    use std::os::unix::io::FromRawFd;
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
                        image_path: None,
                        width: 0,
                        height: 0,
                        scale: 1,
                        surface: None,
                        layer_surface: None,
                        configured: false,
                        current_bgra: Vec::new(),
                        prev_bgra: Vec::new(),
                        target_bgra: Vec::new(),
                        transition_start: None,
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
            buffer.destroy();
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
        qh: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Mode { width, height, .. } = event {
            if let Some(out) = state.outputs.get_mut(&global_name) {
                out.width = width as u32;
                out.height = height as u32;
            }
        } else if let wl_output::Event::Name { name } = event {
            let mut should_rerender = false;
            if let Some(out) = state.outputs.get_mut(&global_name) {
                let per_out = state.per_output_paths.get(&name).cloned();
                if per_out != out.image_path {
                    out.image_path = per_out;
                    should_rerender = out.configured;
                }
                out.name = name;
            }
            if should_rerender {
                state.render_output(qh, global_name);
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
    };
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
    let target_output = request.command.output.clone();
    let result = match request.command.cmd.as_str() {
        "set" => request
            .command
            .path
            .map(PathBuf::from)
            .ok_or_else(|| anyhow::anyhow!("missing path"))
            .and_then(|path| state.set_image(path, target_output, qh))
            .map(|_| serde_json::json!({"status":"ok"})),
        "list" => {
            Ok(serde_json::json!({"status":"ok", "entries": catalog::list(&state.wallpaper_dir)}))
        }
        "current" => {
            let runtime = state.runtime_state();
            Ok(serde_json::json!({
                "status":"ok",
                "path": state.image_path,
                "outputs": runtime.outputs,
                "color_mode": state.color_mode,
                "accent": runtime.accent,
                "palette": state.palette.as_ref().map(|p| p.dark.to_map())
            }))
        }
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
            .or_else(|| {
                deterministic_primary(p.outputs.iter().map(|(name, path)| (name.as_str(), path)))
                    .map(|(_, path)| path)
            })?;
        catalog::resolve(Path::new(raw), &wallpaper_dir).ok()
    })
}

fn run_client(command: Command, top_level_output: Option<String>) -> Result<()> {
    match command {
        Command::Run { image, output } => run_daemon(image, output.or(top_level_output)),
        Command::Set { image, output } => socket_request(serde_json::json!({
            "cmd": "set",
            "path": image,
            "output": output.or(top_level_output),
        })),
        Command::List => socket_request(serde_json::json!({"cmd":"list"})),
        Command::Current => socket_request(serde_json::json!({"cmd":"current"})),
        Command::Select { output } => {
            let output = output.or(top_level_output);
            let directory = catalog::default_directory();
            let current = current_wallpaper_path();
            if let Some(selection) = picker::run(&directory, current.as_deref())? {
                if !selection.applied_to_daemon || output.is_some() {
                    socket_request(serde_json::json!({
                        "cmd": "set",
                        "path": selection.path,
                        "output": output,
                    }))?;
                }
            }
            Ok(())
        }
        Command::Color { mode, accent } => {
            socket_request(serde_json::json!({"cmd":"color", "mode":mode, "accent":accent}))
        }
    }
}

fn acquire_single_instance_lock() -> Option<std::fs::File> {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    let lock_path = runtime_dir.join("wyrd-wallpaper.lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .ok()?;
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc != 0 {
        return None;
    }
    Some(file)
}

fn run_daemon(initial_image: Option<PathBuf>, output_filter: Option<String>) -> Result<()> {
    let Some(_instance_lock) = acquire_single_instance_lock() else {
        info!("wyrd-wallpaper daemon is already running; exiting duplicate instance");
        return Ok(());
    };
    info!("Starting wyrd-wallpaper");
    let (tx, rx): (Sender<SocketRequest>, Receiver<SocketRequest>) = mpsc::channel();
    start_socket_server(tx)?;

    let conn = Connection::connect_to_env()
        .context("WAYLAND_DISPLAY not set; cannot connect to compositor")?;
    let (globals, mut event_queue) =
        registry_queue_init::<WallpaperState>(&conn).context("failed to init Wayland registry")?;
    let qh = event_queue.handle();

    let wallpaper_dir = catalog::default_directory();
    let persisted = read_persisted_state();

    let mut per_output_paths: HashMap<String, PathBuf> = HashMap::new();
    if initial_image.is_none() {
        if let Some(ref p) = persisted {
            for (out_name, raw_path) in &p.outputs {
                if !raw_path.is_empty() {
                    if let Ok(resolved) = catalog::resolve(Path::new(raw_path), &wallpaper_dir) {
                        per_output_paths.insert(out_name.clone(), resolved);
                    }
                }
            }
        }
    }

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
        let raw_path =
            deterministic_primary(p.outputs.iter().map(|(name, path)| (name.as_str(), path)))?.1;
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
            if p.color_mode.is_empty() {
                "auto".to_string()
            } else {
                p.color_mode.clone()
            },
            if p.accent.is_empty() {
                "#c72548".to_string()
            } else {
                p.accent.clone()
            },
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
        per_output_paths,
        decoded_image: image_path.as_deref().and_then(|p| image::open(p).ok()),
        image_path: image_path.clone(),
        wallpaper_dir,
        color_mode,
        manual_accent,
        auto_accent,
        palette,
        output_filter,
        pending_buffers: Vec::new(),
        last_transition_step: None,
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
                image_path: None,
                width: 0,
                height: 0,
                scale: 1,
                surface: None,
                layer_surface: None,
                configured: false,
                current_bgra: Vec::new(),
                prev_bgra: Vec::new(),
                target_bgra: Vec::new(),
                transition_start: None,
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
        let Some(out) = state.outputs.get_mut(&key) else {
            continue;
        };
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

    loop {
        let output_keys = state.outputs.keys().copied().collect::<Vec<_>>();
        for key in output_keys {
            state.ensure_output_surface(key, &qh);
        }
        while let Ok(request) = rx.try_recv() {
            handle_command(&mut state, request, &qh);
        }
        if state.any_output_animating() {
            let ready_for_frame = state
                .last_transition_step
                .map(|last| last.elapsed() >= std::time::Duration::from_millis(15))
                .unwrap_or(true);
            if ready_for_frame {
                state.step_transitions(&qh);
            }
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
        let timeout_ms = if state.any_output_animating() {
            state
                .last_transition_step
                .map(|last| {
                    16i32
                        .saturating_sub(last.elapsed().as_millis() as i32)
                        .max(1)
                })
                .unwrap_or(1)
        } else {
            50
        };
        let ready = unsafe { libc::poll(&mut pollfd, 1, timeout_ms) };
        if ready > 0 {
            if let Err(wayland_client::backend::WaylandError::Io(ref err)) = guard.read() {
                if err.kind() != std::io::ErrorKind::WouldBlock {
                    return Err(anyhow::anyhow!("Wayland read error: {err}"));
                }
            }
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
        let persisted: wyrd_engine::state_files::WallpaperState =
            toml::from_str(&content).expect("Failed to deserialize persisted state");

        assert_eq!(persisted.color_mode.as_str(), "manual");
        assert_eq!(persisted.accent.as_str(), "#123456");
        assert_eq!(persisted.primary.as_deref(), Some("eDP-1"));
        assert_eq!(
            persisted.outputs.get("eDP-1").map(|s| s.as_str()),
            Some("/home/user/Pictures/wall.jpg")
        );
    }

    #[test]
    fn select_accepts_output_option_after_subcommand() {
        let cli = Cli::try_parse_from(["wyrd-wallpaper", "select", "--output", "HDMI-A-1"])
            .expect("select should accept --output after the subcommand");

        assert!(matches!(
            cli.command,
            Some(Command::Select {
                output: Some(output)
            }) if output == "HDMI-A-1"
        ));
    }

    #[test]
    fn deterministic_primary_selection_ignores_hashmap_order() {
        let mut first = HashMap::new();
        first.insert("eDP-1".to_string(), "laptop.jpg".to_string());
        first.insert("HDMI-A-1".to_string(), "external.jpg".to_string());

        let mut second = HashMap::new();
        second.insert("HDMI-A-1".to_string(), "external.jpg".to_string());
        second.insert("eDP-1".to_string(), "laptop.jpg".to_string());

        let primary_name = |outputs: &HashMap<String, String>| {
            deterministic_primary(outputs.iter().map(|(name, path)| (name.as_str(), path)))
                .map(|(name, _)| name.to_string())
        };

        assert_eq!(primary_name(&first), Some("HDMI-A-1".to_string()));
        assert_eq!(primary_name(&first), primary_name(&second));
    }

    #[test]
    fn test_is_hex_color_edge_cases() {
        assert!(is_hex_color("#000000"));
        assert!(is_hex_color("#ffffff"));
        assert!(is_hex_color("#aBcDeF"));
        assert!(is_hex_color("#123456"));
        assert!(!is_hex_color("123456"));
        assert!(!is_hex_color("#12345"));
        assert!(!is_hex_color("#1234567"));
        assert!(!is_hex_color("#12345g"));
        assert!(!is_hex_color(""));
    }

    #[test]
    fn test_blend_wallpaper_transition_bounds() {
        let prev = vec![10u8, 20, 30, 255, 40, 50, 60, 255];
        let target = vec![200u8, 210, 220, 255, 230, 240, 250, 255];
        let mut dst = vec![0u8; 8];

        // At t = 0.0, output should equal prev
        blend_wallpaper_transition(&prev, &target, &mut dst, 2, 1, 0.0);
        assert_eq!(dst, prev);

        // At t = 1.0, output should equal target
        blend_wallpaper_transition(&prev, &target, &mut dst, 2, 1, 1.0);
        assert_eq!(dst, target);
    }

    #[test]
    fn test_socket_command_deserialization() {
        let cmd: SocketCommand =
            serde_json::from_str(r#"{"cmd":"set","path":"/tmp/wall.png","output":"HDMI-A-1"}"#)
                .expect("Failed to deserialize SocketCommand");
        assert_eq!(cmd.cmd, "set");
        assert_eq!(cmd.path.as_deref(), Some("/tmp/wall.png"));
        assert_eq!(cmd.output.as_deref(), Some("HDMI-A-1"));
    }

    #[test]
    fn test_parse_safe_shell_action() {
        assert_eq!(picker::parse_safe_shell_action("rm -rf /", true), None);
        assert_eq!(
            picker::parse_safe_shell_action("exec:notify-send hi", false),
            None
        );
        assert_eq!(
            picker::parse_safe_shell_action("exec:notify-send hi", true),
            Some("notify-send hi")
        );
        assert_eq!(picker::parse_safe_shell_action("exec:   ", true), None);
    }

    #[test]
    fn test_is_wyrd_wallpaper_process_rejects_invalid_pids() {
        assert!(!picker::is_wyrd_wallpaper_process(0));
        assert!(!picker::is_wyrd_wallpaper_process(-1));
        assert!(!picker::is_wyrd_wallpaper_process(1));
    }
}
