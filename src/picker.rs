use anyhow::{Context, Result};
use std::collections::{HashMap, VecDeque};
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, PixmapMut, Rect, Transform};
use wayland_client::{
    globals::{registry_queue_init, GlobalListContents},
    protocol::{
        wl_buffer, wl_callback, wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_registry,
        wl_seat, wl_shm, wl_shm_pool, wl_surface,
    },
    Connection, Dispatch, Proxy, QueueHandle, WEnum,
};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{self, Layer, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{self, Anchor, KeyboardInteractivity, ZwlrLayerSurfaceV1},
};

use crate::catalog::{self, WallpaperEntry};

/// Upper bound on cached thumbnails kept in memory at once. The carousel only ever
/// shows a handful of cards at a time (±2.5 steps from center), so this gives generous
/// headroom while keeping memory bounded for very large wallpaper collections.
const MAX_CACHED_THUMBNAILS: usize = 24;

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

/// Opens the wallpaper picker. `current` is the path of the wallpaper that's already
/// active, if known - when set, the picker opens with that wallpaper centered instead
/// of always starting at the first entry in the directory.
pub fn run(directory: &Path, current: Option<&Path>) -> Result<Option<PathBuf>> {
    let entries = catalog::list(directory);
    if entries.is_empty() {
        anyhow::bail!("no wallpapers found in {}", directory.display());
    }
    let initial_selected = current
        .and_then(|current_path| {
            entries
                .iter()
                .position(|entry| same_wallpaper(&entry.path, current_path))
        })
        .unwrap_or(0);

    let conn = Connection::connect_to_env().context("WAYLAND_DISPLAY is not available")?;
    let (globals, mut queue) = registry_queue_init::<PickerState>(&conn)?;
    let qh = queue.handle();
    let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 1..=6, ())?;
    let shm: wl_shm::WlShm = globals.bind(&qh, 1..=2, ())?;
    let layer_shell: ZwlrLayerShellV1 = globals.bind(&qh, 1..=4, ())?;

    let entry_count = entries.len();
    let mut state = PickerState {
        shm: Some(shm.clone()),
        surface: None,
        layer_surface: None,
        pointer: None,
        keyboard: None,
        width: 0,
        height: 0,
        pointer_x: 0.0,
        pointer_y: 0.0,
        entries,
        selected: initial_selected,
        confirmed: None,
        closed: false,
        frame_dirty: true,
        frame_callback: None,
        pending_buffers: Vec::new(),
        thumbnails: std::iter::repeat_with(|| None).take(entry_count).collect(),
        thumbnail_load_order: VecDeque::new(),
        anim_from: 0.0,
        anim_to: 0.0,
        anim_started: None,
        render_context: wyrd_engine::render::context::RenderContext::new(1.0),
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
    // No anchor → compositor centers the surface on screen.
    // Explicit size creates a compact floating panel rather than a fullscreen overlay.
    layer_surface.set_anchor(Anchor::empty());
    layer_surface.set_size(900, 560);
    layer_surface.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
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
    while state.confirmed.is_none() && !state.closed {
        if state.frame_dirty && state.width > 0 && state.height > 0 {
            state.render(&qh);
            state.frame_dirty = false;
        }
        queue.blocking_dispatch(&mut state)?;
    }

    // Clean shutdown: explicitly destroy the layer surface and wl_surface before dropping.
    // Without this, dropping ZwlrLayerSurfaceV1 and WlSurface without sending destroy causes
    // a Wayland protocol error that crashes the compositor session.
    for buf in state.pending_buffers.drain(..) {
        buf.destroy();
    }
    if let Some(ls) = state.layer_surface.take() {
        ls.destroy();
    }
    if let Some(s) = state.surface.take() {
        s.destroy();
    }
    // Flush destroy requests to the compositor before returning.
    let _ = queue.roundtrip(&mut state);

    Ok(state
        .confirmed
        .map(|index| PathBuf::from(state.entries[index].path.clone())))
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
    entries: Vec<WallpaperEntry>,
    selected: usize,
    confirmed: Option<usize>,
    closed: bool,
    frame_dirty: bool,
    frame_callback: Option<wl_callback::WlCallback>,
    /// Buffers committed but not yet released by the compositor. Destroyed on Release.
    pending_buffers: Vec<wl_buffer::WlBuffer>,
    thumbnails: Vec<Option<Pixmap>>,
    /// FIFO of thumbnail indices in load order, used to bound cache size.
    thumbnail_load_order: VecDeque<usize>,
    anim_from: f32,
    anim_to: f32,
    anim_started: Option<Instant>,
    render_context: wyrd_engine::render::context::RenderContext,
    /// Display scale the current `render_context` was built for; used to detect when
    /// it needs to be rebuilt for a different output scale.
    text_scale: f32,
    /// Latest scale factor advertised per wl_output global (keyed by registry name).
    output_scales: HashMap<u32, i32>,
    /// Effective scale used for the picker's own surface (max of known output scales).
    output_scale: i32,
}

impl Drop for PickerState {
    fn drop(&mut self) {
        for buf in self.pending_buffers.drain(..) {
            buf.destroy();
        }
        if let Some(ls) = self.layer_surface.take() {
            ls.destroy();
        }
        if let Some(s) = self.surface.take() {
            s.destroy();
        }
    }
}

impl PickerState {
    fn circular_offset(&self, index: usize, position: f32) -> f32 {
        let count = self.entries.len() as f32;
        let mut offset = index as f32 - position;
        if count > 1.0 {
            while offset > count / 2.0 {
                offset -= count;
            }
            while offset < -count / 2.0 {
                offset += count;
            }
        }
        offset
    }

    fn step_selection(&mut self, direction: f32) {
        if self.entries.len() < 2 {
            return;
        }
        let count = self.entries.len();
        // Get current animated display position, then normalize to [0, N)
        let raw = self.display_position_raw();
        let n = count as f32;
        let current_pos = ((raw % n) + n) % n;
        // Animate exactly one step in the given direction; anim_to is unnormalized so
        // the linear interpolation doesn't jump, then circular_offset handles the ring.
        self.anim_from = current_pos;
        self.anim_to = current_pos + direction;
        self.selected =
            (self.selected as isize + direction as isize).rem_euclid(count as isize) as usize;
        self.anim_started = Some(Instant::now());
        self.frame_dirty = true;
    }

    /// Returns raw interpolated position (may temporarily leave [0, N) during animation).
    fn display_position_raw(&mut self) -> f32 {
        let target = self.anim_to;
        let Some(started) = self.anim_started else {
            return self.selected as f32;
        };
        let progress = (started.elapsed().as_secs_f32() / 0.22).clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - progress).powi(3);
        let position = self.anim_from + (target - self.anim_from) * eased;
        if progress >= 1.0 {
            self.anim_started = None;
            // Normalize after animation completes to prevent unbounded drift on repeated cycling
            let n = self.entries.len() as f32;
            let normalized = ((target % n) + n) % n;
            self.anim_from = normalized;
            self.anim_to = normalized;
            normalized
        } else {
            position
        }
    }

    fn render(&mut self, qh: &QueueHandle<Self>) {
        let display_position = self.display_position_raw();
        let (Some(shm), Some(surface)) = (self.shm.as_ref(), self.surface.as_ref()) else {
            return;
        };

        // Rebuild text renderer only when output scale changes.
        let target_scale = self.output_scale.max(1) as f32;
        if (self.text_scale - target_scale).abs() > f32::EPSILON {
            self.render_context =
                wyrd_engine::render::context::RenderContext::new(target_scale.into());
            self.text_scale = target_scale;
        }

        let width = self.width.max(1);
        let height = self.height.max(1);
        let Some(mut pixmap) = Pixmap::new(width, height) else {
            return;
        };

        // ── Aetheria design tokens (matching the bar) ──────────────────────────
        let panel_bg = Color::from_rgba8(12, 14, 22, 235); // rgba(12,14,22,0.92)
        let surface_1 = Color::from_rgba8(22, 26, 38, 200); // card bg
        let accent = Color::from_rgba8(125, 211, 252, 255); // #7DD3FC primary
        let accent_glow = Color::from_rgba8(125, 211, 252, 45); // ring overlay
        let accent_dim = Color::from_rgba8(125, 211, 252, 20); // border
        let text_primary = Color::from_rgba8(226, 232, 240, 255); // #E2E8F0
        let text_secondary = Color::from_rgba8(148, 163, 184, 255); // #94A3B8
        let text_muted = Color::from_rgba8(100, 116, 139, 255); // #64748B
        let shadow_col = Color::from_rgba8(0, 0, 0, 160);
        let _border = Color::from_rgba8(125, 211, 252, 20); // border_subtle

        // ── Panel geometry ─────────────────────────────────────────────────────
        // Panel fills the entire surface (which is already the right compact size).
        let pw = width as f32;
        let ph = height as f32;
        let radius = 24.0_f32; // Large rounded corners, same feel as bar chips
        let pad_h = 20.0_f32; // Horizontal padding inside panel
        let pad_v = 16.0_f32; // Vertical padding

        // The surface background is fully transparent — only the panel itself is opaque.
        // This way the compositor sees the correct shape and doesn't paint a black box.
        pixmap.fill(Color::TRANSPARENT);

        // Drop shadow beneath the panel
        rounded_rect(
            &mut pixmap,
            4.0,
            8.0,
            pw - 8.0,
            ph - 8.0,
            radius + 4.0,
            shadow_col,
        );

        // Panel body
        rounded_rect(&mut pixmap, 0.0, 0.0, pw, ph, radius, panel_bg);

        // Subtle inner border glow (1px simulation via slightly inset lighter rect)
        draw_border(&mut pixmap, 0.0, 0.0, pw, ph, radius, accent_dim);

        // ── Header ────────────────────────────────────────────────────────────
        // Height of the header area
        let header_h = 48.0_f32;
        let header_y = pad_v;

        // "Wallpapers" label on the left
        wyrd_engine::render::text::TextRenderer::render(
            &mut self.render_context,
            &mut pixmap.as_mut(),
            "Wallpapers",
            "JetBrains Mono, sans-serif",
            13.0,
            text_primary,
            pad_h,
            header_y + 14.0,
            pw * 0.4,
        );

        // Counter chip on the right: "12 / 48"
        let selected_idx = self.selected.min(self.entries.len().saturating_sub(1));
        let count_label = format!("{} / {}", selected_idx + 1, self.entries.len());
        let chip_w = 64.0_f32;
        let chip_h = 26.0_f32;
        let chip_x = pw - pad_h - chip_w;
        let chip_y = header_y + (header_h - chip_h) / 2.0;
        rounded_rect(
            &mut pixmap,
            chip_x,
            chip_y,
            chip_w,
            chip_h,
            chip_h / 2.0,
            Color::from_rgba8(22, 26, 38, 180),
        );
        draw_border(
            &mut pixmap,
            chip_x,
            chip_y,
            chip_w,
            chip_h,
            chip_h / 2.0,
            accent_dim,
        );
        wyrd_engine::render::text::TextRenderer::render(
            &mut self.render_context,
            &mut pixmap.as_mut(),
            &count_label,
            "JetBrains Mono, sans-serif",
            10.5,
            text_secondary,
            chip_x + 10.0,
            chip_y + 7.0,
            chip_w - 4.0,
        );

        // Divider line below header
        let divider_y = header_y + header_h;
        fill_rect(
            &mut pixmap,
            pad_h,
            divider_y,
            pw - pad_h * 2.0,
            1.0,
            Color::from_rgba8(125, 211, 252, 18),
        );

        // ── Carousel area ─────────────────────────────────────────────────────
        // Cards live in the area between the divider and the hint bar at the bottom
        let hint_h = 32.0_f32;
        let carousel_y = divider_y + pad_v;
        let carousel_h = ph - carousel_y - hint_h - pad_v;
        let carousel_cx = pw / 2.0;
        let carousel_cy = carousel_y + carousel_h / 2.0;

        // Card dimensions: center card is 60% of carousel width, same aspect as the
        // wall image (16:9 → use 16:10 for slight breathing room).
        let card_w = (pw - pad_h * 2.0) * 0.54;
        let card_h = carousel_h.min(card_w * 0.625); // approx 16:10

        // Show at most ±2 neighbors (center + 2 each side = 5 slots)
        let max_visible = 2.5_f32;
        let mut indices: Vec<usize> = (0..self.entries.len())
            .filter(|i| self.circular_offset(*i, display_position).abs() <= max_visible)
            .collect();
        // Paint furthest from center first (painter's algorithm / depth order)
        indices.sort_by(|a, b| {
            self.circular_offset(*b, display_position)
                .abs()
                .partial_cmp(&self.circular_offset(*a, display_position).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Thumbnail target dimensions (cached at center card size)
        let thumb_w = card_w.max(2.0) as u32;
        let thumb_h = card_h.max(2.0) as u32;

        for index in &indices {
            let index = *index;
            let rel = self.circular_offset(index, display_position);
            let dist = rel.abs();
            let is_center = dist < 0.01;

            // Depth perspective: non-selected cards shrink slightly and dim
            let scale_f = if is_center {
                1.0
            } else {
                (1.0 - dist * 0.06).max(0.78)
            };
            let cw = card_w * scale_f;
            let ch = card_h * scale_f;
            let gap = pad_h * 1.2; // spacing between card edges
            let cx = carousel_cx + rel * (card_w + gap);
            let x = cx - cw / 2.0;
            let y = carousel_cy - ch / 2.0;
            let r = if is_center { 14.0 } else { 10.0 };

            // Card shadow
            rounded_rect(
                &mut pixmap,
                x + 3.0,
                y + 6.0,
                cw,
                ch,
                r,
                Color::from_rgba8(0, 0, 0, if is_center { 140 } else { 60 }),
            );

            // Lazy thumbnail load
            if self.thumbnails[index].is_none() {
                if let Ok(image) = image::open(&self.entries[index].path) {
                    let resized = image
                        .resize_to_fill(thumb_w, thumb_h, image::imageops::FilterType::Triangle)
                        .to_rgba8();
                    let mut raw = resized.into_raw();
                    if let Some(src) = PixmapMut::from_bytes(&mut raw, thumb_w, thumb_h) {
                        let mut pm = Pixmap::new(thumb_w, thumb_h);
                        if let Some(ref mut pm) = pm {
                            pm.draw_pixmap(
                                0,
                                0,
                                src.as_ref(),
                                &tiny_skia::PixmapPaint::default(),
                                Transform::identity(),
                                None,
                            );
                        }
                        if pm.is_some() {
                            self.thumbnails[index] = pm;
                            self.thumbnail_load_order.push_back(index);
                            while self.thumbnail_load_order.len() > MAX_CACHED_THUMBNAILS {
                                if let Some(evict) = self.thumbnail_load_order.pop_front() {
                                    if evict != index {
                                        self.thumbnails[evict] = None;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Draw thumbnail or placeholder
            if let Some(cached) = self.thumbnails[index].as_ref() {
                let tw = cw.max(2.0) as u32;
                let th = ch.max(2.0) as u32;
                if let Some(mut scaled) = Pixmap::new(tw, th) {
                    let transform = Transform::from_scale(
                        tw as f32 / cached.width() as f32,
                        th as f32 / cached.height() as f32,
                    );
                    scaled.draw_pixmap(
                        0,
                        0,
                        cached.as_ref(),
                        &tiny_skia::PixmapPaint::default(),
                        transform,
                        None,
                    );

                    // Dim non-selected cards
                    if !is_center {
                        let alpha = (dist * 70.0).min(120.0) as u8;
                        let mut paint = tiny_skia::Paint::default();
                        paint.set_color(Color::from_rgba8(12, 14, 22, alpha));
                        if let Some(rect) = Rect::from_xywh(0.0, 0.0, tw as f32, th as f32) {
                            scaled.fill_rect(rect, &paint, Transform::identity(), None);
                        }
                    }
                    pixmap.draw_pixmap(
                        x as i32,
                        y as i32,
                        scaled.as_ref(),
                        &tiny_skia::PixmapPaint::default(),
                        Transform::identity(),
                        None,
                    );
                }
            } else {
                // Placeholder when image hasn't loaded yet
                rounded_rect(&mut pixmap, x, y, cw, ch, r, surface_1);
                wyrd_engine::render::text::TextRenderer::render(
                    &mut self.render_context,
                    &mut pixmap.as_mut(),
                    "Loading…",
                    "sans-serif",
                    10.0,
                    text_muted,
                    x + cw / 2.0 - 24.0,
                    y + ch / 2.0 - 6.0,
                    60.0,
                );
            }

            // Selected card: accent ring + top accent bar
            if is_center {
                // Thin sky-blue glow ring
                draw_border(&mut pixmap, x, y, cw, ch, r, accent_glow);
                draw_border(
                    &mut pixmap,
                    x + 1.0,
                    y + 1.0,
                    cw - 2.0,
                    ch - 2.0,
                    r - 0.5,
                    Color::from_rgba8(125, 211, 252, 90),
                );
                // Top accent bar
                fill_rect(&mut pixmap, x, y, cw, 3.0_f32.max(1.5), accent);

                // Wallpaper filename below the carousel
                let name = Path::new(&self.entries[index].path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("");
                let name_y = carousel_y + carousel_h + 6.0;
                wyrd_engine::render::text::TextRenderer::render(
                    &mut self.render_context,
                    &mut pixmap.as_mut(),
                    name,
                    "JetBrains Mono, sans-serif",
                    11.0,
                    text_primary,
                    pad_h,
                    name_y,
                    pw - pad_h * 2.0,
                );
            }
        }

        // ── Keyboard hint bar ─────────────────────────────────────────────────
        let hint_y = ph - hint_h + 4.0;
        wyrd_engine::render::text::TextRenderer::render(
            &mut self.render_context,
            &mut pixmap.as_mut(),
            "← → scroll   Enter apply   Esc close",
            "JetBrains Mono, sans-serif",
            10.0,
            text_muted,
            0.0,
            hint_y,
            pw,
        );

        // ── Composite to Wayland ───────────────────────────────────────────────
        let buffer_scale = self.output_scale.max(1);
        let physical_w = width * buffer_scale as u32;
        let physical_h = height * buffer_scale as u32;
        let output_pixmap = if buffer_scale > 1 {
            match Pixmap::new(physical_w, physical_h) {
                Some(mut scaled) => {
                    scaled.draw_pixmap(
                        0,
                        0,
                        pixmap.as_ref(),
                        &tiny_skia::PixmapPaint::default(),
                        Transform::from_scale(buffer_scale as f32, buffer_scale as f32),
                        None,
                    );
                    scaled
                }
                None => pixmap,
            }
        } else {
            pixmap
        };

        let bytes = wyrd_engine::render::to_wayland_bgra(output_pixmap.data());
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
        surface.set_buffer_scale(buffer_scale);
        surface.attach(Some(&buffer), 0, 0);
        surface.damage_buffer(0, 0, physical_w as i32, physical_h as i32);
        if self.anim_started.is_some() {
            self.frame_callback = Some(surface.frame(qh, ()));
        }
        surface.commit();
        self.pending_buffers.push(buffer);
    }

    fn hit_index(&self, index: usize) -> Option<usize> {
        let pw = self.width as f32;
        let ph = self.height as f32;
        let pad_h = 20.0_f32;
        let pad_v = 16.0_f32;
        let header_h = 48.0_f32;
        let hint_h = 32.0_f32;
        let divider_y = pad_v + header_h;
        let carousel_y = divider_y + pad_v;
        let carousel_h = ph - carousel_y - hint_h - pad_v;
        let carousel_cx = pw / 2.0;
        let carousel_cy = carousel_y + carousel_h / 2.0;
        let card_w = (pw - pad_h * 2.0) * 0.54;
        let card_h = carousel_h.min(card_w * 0.625);

        let selected_idx = self.selected.min(self.entries.len().saturating_sub(1));
        let rel = self.circular_offset(index, selected_idx as f32);
        let dist = rel.abs();
        if dist > 2.5 {
            return None;
        }

        let is_center = dist < 0.01;
        let scale_f = if is_center {
            1.0
        } else {
            (1.0 - dist * 0.06).max(0.78)
        };
        let cw = card_w * scale_f;
        let ch = card_h * scale_f;
        let gap = pad_h * 1.2;
        let cx = carousel_cx + rel * (card_w + gap);
        let x = cx - cw / 2.0;
        let y = carousel_cy - ch / 2.0;

        if self.pointer_x >= x as f64
            && self.pointer_x <= (x + cw) as f64
            && self.pointer_y >= y as f64
            && self.pointer_y <= (y + ch) as f64
        {
            Some(index)
        } else {
            None
        }
    }
}

fn fill_rect(pixmap: &mut Pixmap, x: f32, y: f32, width: f32, height: f32, color: Color) {
    let mut paint = Paint::default();
    paint.set_color(color);
    let Some(rect) = Rect::from_xywh(x, y, width, height) else {
        return;
    };
    pixmap.fill_rect(rect, &paint, Transform::identity(), None);
}

/// Draw a subtle 1.5px rounded-rectangle border by drawing the full rect in `color` (a
/// semi-transparent tint) on top of whatever is already in the pixmap.  Because the color
/// is already semi-transparent, this naturally produces a glowing ring effect without
/// needing any punch-out or clear pass.
fn draw_border(
    pixmap: &mut Pixmap,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: f32,
    color: Color,
) {
    rounded_rect(pixmap, x, y, width, height, radius, color);
}

fn rounded_rect(
    pixmap: &mut Pixmap,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: f32,
    color: Color,
) {
    let width = width.max(1.0);
    let height = height.max(1.0);
    let radius = radius.max(0.0).min(width / 2.0).min(height / 2.0);
    let k = 0.5522848;
    let mut path = PathBuilder::new();
    path.move_to(x + radius, y);
    path.line_to(x + width - radius, y);
    path.cubic_to(
        x + width - radius + radius * k,
        y,
        x + width,
        y + radius - radius * k,
        x + width,
        y + radius,
    );
    path.line_to(x + width, y + height - radius);
    path.cubic_to(
        x + width,
        y + height - radius + radius * k,
        x + width - radius + radius * k,
        y + height,
        x + width - radius,
        y + height,
    );
    path.line_to(x + radius, y + height);
    path.cubic_to(
        x + radius - radius * k,
        y + height,
        x,
        y + height - radius + radius * k,
        x,
        y + height - radius,
    );
    path.line_to(x, y + radius);
    path.cubic_to(
        x,
        y + radius - radius * k,
        x + radius - radius * k,
        y,
        x + radius,
        y,
    );
    path.close();
    let Some(path) = path.finish() else { return };
    let mut paint = Paint::default();
    paint.set_color(color);
    pixmap.fill_path(
        &path,
        &paint,
        FillRule::Winding,
        Transform::identity(),
        None,
    );
}

fn create_shm_file(size: usize) -> Result<std::fs::File> {
    let name = format!("/wyrd-wall-picker-{}", std::process::id());
    let name = std::ffi::CString::new(name)?;
    // SAFETY: `name` is a valid null-terminated C string. We create an exclusive posix shm segment.
    let fd = unsafe {
        libc::shm_open(
            name.as_ptr(),
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
        libc::shm_unlink(name.as_ptr());
        libc::ftruncate(fd, size as libc::off_t);
    }
    use std::os::fd::FromRawFd;
    // SAFETY: `fd` is a valid, open file descriptor owned by this function.
    Ok(unsafe { std::fs::File::from_raw_fd(fd) })
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for PickerState {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
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
            // Compositor has finished reading this buffer; remove and explicitly destroy it
            // to reclaim the SHM memory and Wayland object. Without this, every render frame
            // during fast scrolling leaks one WlBuffer, eventually crashing the compositor.
            state.pending_buffers.retain(|b| {
                if b.id() == buffer.id() {
                    b.destroy();
                    false
                } else {
                    true
                }
            });
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
            state.output_scale = state.output_scales.values().copied().max().unwrap_or(1);
            state.frame_dirty = true;
        }
    }
}
impl Dispatch<zwlr_layer_shell_v1::ZwlrLayerShellV1, ()> for PickerState {
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
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, wl_callback::Event::Done { .. }) {
            state.frame_callback = None;
            state.frame_dirty = state.anim_started.is_some();
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
        if let wl_keyboard::Event::Key {
            key,
            state: key_state,
            ..
        } = event
        {
            if key_state != WEnum::Value(wl_keyboard::KeyState::Pressed) {
                return;
            }
            match key {
                105 => state.step_selection(-1.0),
                106 => state.step_selection(1.0),
                28 => state.confirmed = Some(state.selected),
                1 => state.closed = true,
                _ => return,
            }
            state.frame_dirty = true;
        }
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, ()> for PickerState {
    fn event(
        state: &mut Self,
        surface: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_layer_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            surface.ack_configure(serial);
            state.width = width;
            state.height = height;
            state.frame_dirty = true;
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
        if let wl_seat::Event::Capabilities { capabilities } = event {
            if let Ok(capabilities) = capabilities.into_result() {
                if capabilities.contains(wl_seat::Capability::Pointer) && state.pointer.is_none() {
                    state.pointer = Some(seat.get_pointer(qh, ()));
                }
                if capabilities.contains(wl_seat::Capability::Keyboard) && state.keyboard.is_none()
                {
                    state.keyboard = Some(seat.get_keyboard(qh, ()));
                }
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
            }
            | wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                state.pointer_x = surface_x;
                state.pointer_y = surface_y;
                state.frame_dirty = true;
            }
            wl_pointer::Event::Axis { value, .. } => {
                if value > 0.0 {
                    state.step_selection(-1.0);
                } else if value < 0.0 {
                    state.step_selection(1.0);
                }
                state.frame_dirty = true;
            }
            wl_pointer::Event::Button {
                button,
                state: button_state,
                ..
            } if button == 272
                && button_state == WEnum::Value(wl_pointer::ButtonState::Pressed) =>
            {
                if let Some(index) =
                    (0..state.entries.len()).find(|index| state.hit_index(*index).is_some())
                {
                    state.selected = index;
                    state.confirmed = Some(index);
                }
            }
            _ => {}
        }
    }
}
